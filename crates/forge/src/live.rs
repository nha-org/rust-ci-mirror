//! A built binary taken live on the front door (`dash deploy`, the Makefile's rust/deploy after rust/build; the forge's deploy).
//! The dash serving the port is stopped, launchd starts the live binary again, and the deploy waits on that new server's /built
//! for as long as it takes: no clock rolls it back (task rust-deploy-keeps-new-build; @nha, 2026-11-13: « move fast and break
//! things please. »; 1529's land rolled back a good build at 60 s). Only a server that exits (rust/serve started twice, its
//! `serve.started` stamped again) or answers 5xx gives way to the binary before it. A page that names neither after `GIVE_UP`
//! (a server hung before binding, an old one that outlived its TERM, no launchd to start it) fails the deploy and keeps the new
//! binary, so the forge's lock never stays held (/code-review of 771d60dda).
use dash_process as process;
use std::path::Path;
use std::time::{Duration, Instant};

const CALL: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(250);
const GIVE_UP: Duration = Duration::from_secs(1800);
/// rust-analyzer's index for /deps, about 80 s.
const CODE_MAP: Duration = Duration::from_secs(600);

/// The commit a page names on its whole line « Built from <sha>. », as `go_live` and the forge's `serving` read it.
pub(crate) fn built_from(page: &str) -> Option<String> {
    let line = page
        .lines()
        .find_map(|l| l.strip_prefix("Built from ")?.strip_suffix('.'));
    line.map(str::to_string)
}

/// What one read of /built says of the new build.
#[derive(Debug, PartialEq)]
enum Look {
    Serves,
    Fails(String),
    Wait,
}

/// A 200 whose « Built from » line names `sha` (`built_from`, as `serving` reads it); any 5xx fails: /built is answered on the
/// connection's own thread, never queued, so only a broken server answers one there. Anything else (no answer yet, the old
/// server still up) waits.
fn look(code: &str, body: &str, sha: &str) -> Look {
    match code {
        c if c.starts_with('5') => Look::Fails(format!("it answers {c}")),
        "200" if built_from(body).as_deref() == Some(sha) => Look::Serves,
        _ => Look::Wait,
    }
}

fn probe(page: &str, sha: &str) -> Look {
    let args = ["-s", "-m", "2", "-w", "\n%{http_code}", page];
    let Ok((_, text, _)) = process::answer("curl", &args, None, &[], CALL) else {
        return Look::Wait;
    };
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    look(code.trim(), body, sha)
}

/// The dash processes listening on `port`, never another program there (the older app's beam before rust/front-door).
fn dashes_on(port: &str) -> Vec<String> {
    let tcp = format!("tcp:{port}");
    let pids = process::run("lsof", &["-ti", &tcp, "-sTCP:LISTEN"], None, CALL).unwrap_or_default();
    pids.lines()
        .filter(|pid| {
            let named = process::run("lsof", &["-a", "-p", pid, "-Fc"], None, CALL);
            named.is_ok_and(|n| n.lines().any(|l| l == "cdash"))
        })
        .map(str::to_string)
        .collect()
}

fn stop_dashes(port: &str) -> usize {
    let pids = dashes_on(port);
    for pid in &pids {
        let _ = process::run("kill", &[pid], None, CALL);
    }
    pids.len()
}

fn port_of(page: &str) -> Result<&str, String> {
    let host = page.split("://").nth(1).unwrap_or(page);
    let host = host.split('/').next().unwrap_or_default();
    host.rsplit_once(':')
        .map(|(_, p)| p)
        .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| format!("{page}: no port"))
}

/// `live`'s binary on the server behind `page` (its /built). Ok: the sha it serves. Err: why not, and whether the binary before
/// it is back.
pub fn go_live(live: &Path, page: &str) -> Result<String, String> {
    go_live_within(live, page, GIVE_UP)
}

fn go_live_within(live: &Path, page: &str, give_up: Duration) -> Result<String, String> {
    let port = port_of(page)?;
    let commit = live.join("commit");
    let sha = std::fs::read_to_string(&commit).map_err(|e| format!("{}: {e}", commit.display()))?;
    let sha = sha.trim();
    let started = live.join("serve.started");
    let stamp = || std::fs::metadata(&started).and_then(|m| m.modified()).ok();
    let mut seen = stamp();
    let mut starts = 0;
    if stop_dashes(port) == 0 {
        eprintln!("deploy: no dash was serving on :{port}");
    }
    let began = Instant::now();
    loop {
        match probe(page, sha) {
            Look::Serves => return Ok(format!("{sha} serves on :{port}")),
            Look::Fails(why) => return Err(roll_back(live, port, sha, &why)),
            Look::Wait => {}
        }
        let now = stamp();
        if now != seen {
            (seen, starts) = (now, starts + 1);
        }
        // the first start is the new server's own; a second one means it ended without naming its build
        if starts >= 2 {
            let why = "it exited before its /built named it (rust/serve started twice)";
            return Err(roll_back(live, port, sha, why));
        }
        if began.elapsed() >= give_up {
            let mins = give_up.as_secs() / 60;
            return Err(format!(
                "deploy: :{port}'s /built names no {sha} after {mins} min, and its server neither exited nor answered 5xx: \
                 the new binary stays; see ~/Library/Logs/rust-serve.log"
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// The code map /deps draws, built by the live binary once it serves: about 80 s, so the forge runs it after its lock.
pub fn code_map(live: &Path) -> String {
    let (dash, data) = (live.join("dash"), live.join("data"));
    let args = ["code", "map", "--data", &data.to_string_lossy()];
    match process::run(&dash.to_string_lossy(), &args, None, CODE_MAP) {
        Ok(_) => "a new code map".into(),
        Err(e) => format!("no new code map, /deps keeps the last: {e}"),
    }
}

fn roll_back(live: &Path, port: &str, sha: &str, why: &str) -> String {
    let (dash, prev) = (live.join("dash"), live.join("dash.prev"));
    let failed = format!("deploy: {sha} on :{port}: {why}; see ~/Library/Logs/rust-serve.log");
    if !prev.exists() {
        return format!("{failed}; no binary before it to go back to");
    }
    if let Err(e) = std::fs::rename(&prev, &dash) {
        return format!("{failed}; the binary before it not restored: {e}");
    }
    // no commit.prev: the binary before it was built with none written; `commit` then never names the build that failed
    let commit = live.join("commit");
    let back = match std::fs::rename(live.join("commit.prev"), &commit) {
        Ok(()) => std::fs::read_to_string(&commit).unwrap_or_default(),
        Err(_) => {
            let _ = std::fs::remove_file(&commit);
            "a commit not recorded".into()
        }
    };
    stop_dashes(port);
    format!("{failed}; back on the binary of {}", back.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::SystemTime;

    /// What the stand-in page does at each request, in turn; the last repeats.
    #[derive(Clone)]
    enum Step {
        /// closes with no answer, as a server still starting
        Drop,
        /// answers this status and body
        Answer(&'static str, &'static str),
        /// stamps serve.started (rust/serve started), then closes with no answer
        Start(u64),
    }

    fn live_with_two_builds() -> (dash_meter::TestDir, std::path::PathBuf) {
        let dir = dash_meter::TestDir::new("go-live");
        let live = dir.0.join("rust");
        std::fs::create_dir_all(&live).expect("live");
        std::fs::write(live.join("dash"), "new").expect("dash");
        std::fs::write(live.join("dash.prev"), "old").expect("prev");
        std::fs::write(live.join("commit"), "abc\n").expect("commit");
        std::fs::write(live.join("commit.prev"), "old\n").expect("commit.prev");
        std::fs::write(live.join("serve.started"), "").expect("started");
        (dir, live)
    }

    fn page(live: &Path, steps: Vec<Step>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("addr").port();
        let started = live.join("serve.started");
        std::thread::spawn(move || {
            for (i, conn) in listener.incoming().enumerate() {
                let Ok(mut conn) = conn else { continue };
                let mut buf = [0u8; 1024];
                let _ = conn.read(&mut buf);
                let step = steps[i.min(steps.len() - 1)].clone();
                match step {
                    Step::Drop => {}
                    Step::Start(n) => {
                        let at = SystemTime::now() + Duration::from_secs(n);
                        let f = std::fs::File::options().write(true).open(&started);
                        f.and_then(|f| f.set_modified(at)).expect("stamped");
                    }
                    Step::Answer(status, body) => {
                        let len = body.len();
                        let head = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
                        );
                        let _ = conn.write_all(format!("{head}{body}").as_bytes());
                    }
                }
            }
        });
        format!("http://127.0.0.1:{port}/built")
    }

    fn read(live: &Path, name: &str) -> String {
        std::fs::read_to_string(live.join(name)).expect(name)
    }

    /// Done-when 1: no answer, the old build's line and a start all wait, however long; the new line keeps the new binary.
    #[dash_test]
    fn a_slow_new_server_is_waited_on_and_kept() {
        let (_dir, live) = live_with_two_builds();
        let steps = vec![
            Step::Drop,
            Step::Answer("200 OK", "Built from old.\n"),
            Step::Start(1),
            Step::Drop,
            Step::Drop,
            Step::Answer("200 OK", "Rust tasks\n\nBuilt from abc.\n"),
        ];
        let page = page(&live, steps);
        let said = go_live(&live, &page).expect("kept");
        assert!(said.starts_with("abc serves on :"), "{said}");
        assert_eq!(read(&live, "dash"), "new", "the new binary stays");
        assert_eq!(read(&live, "commit"), "abc\n");
    }

    /// Done-when 1: a 5xx rolls back to the binary before it, even with the line in its body.
    #[dash_test]
    fn a_new_server_answering_5xx_is_rolled_back() {
        let (_dir, live) = live_with_two_builds();
        let steps = vec![
            Step::Drop,
            Step::Answer("500 Internal Server Error", "Built from abc.\n"),
        ];
        let page = page(&live, steps);
        let e = go_live(&live, &page).expect_err("rolled back");
        assert!(e.contains("it answers 500"), "{e}");
        assert!(e.ends_with("back on the binary of old"), "{e}");
        assert_eq!(read(&live, "dash"), "old");
        assert_eq!(read(&live, "commit"), "old\n");
    }

    /// Done-when 1: rust/serve started twice with no line named: the new server exited, rolled back.
    #[dash_test]
    fn a_new_server_that_exits_is_rolled_back() {
        let (_dir, live) = live_with_two_builds();
        let steps = vec![Step::Start(1), Step::Drop, Step::Start(2), Step::Drop];
        let page = page(&live, steps);
        let e = go_live(&live, &page).expect_err("rolled back");
        assert!(e.contains("it exited"), "{e}");
        assert_eq!(read(&live, "dash"), "old");
        assert_eq!(read(&live, "commit"), "old\n");
    }

    /// Done-when 1: a page that never names it gives up, failing, and keeps the new binary: no clock rolls it back.
    #[dash_test]
    fn a_server_never_named_gives_up_keeping_the_new_binary() {
        let (_dir, live) = live_with_two_builds();
        let page = page(&live, vec![Step::Answer("200 OK", "Built from old.\n")]);
        let e = go_live_within(&live, &page, Duration::from_secs(1)).expect_err("gave up");
        assert!(e.contains("the new binary stays"), "{e}");
        assert_eq!(read(&live, "dash"), "new");
        assert_eq!(read(&live, "commit"), "abc\n");
    }

    /// A rollback with no commit.prev never leaves `commit` naming the build that failed.
    #[dash_test]
    fn a_rollback_with_no_commit_before_names_none() {
        let (_dir, live) = live_with_two_builds();
        std::fs::remove_file(live.join("commit.prev")).expect("no commit.prev");
        let page = page(&live, vec![Step::Answer("503 Service Unavailable", "")]);
        let e = go_live(&live, &page).expect_err("rolled back");
        assert!(
            e.ends_with("back on the binary of a commit not recorded"),
            "{e}"
        );
        assert_eq!(read(&live, "dash"), "old");
        assert!(!live.join("commit").exists(), "no commit named");
    }

    /// rust_deploy_make_test.exs's cases, from `make rust/serves`: the whole line only, never a row's words or a prefix.
    #[dash_test]
    fn only_a_200_with_the_whole_line_serves() {
        assert_eq!(
            look("200", "Rust tasks\n\nBuilt from abc.\n", "abc"),
            Look::Serves
        );
        let row = "t\tfinished\t1\tnow\tmerged\t\tBuilt from abc.\n\nBuilt from old.\n";
        assert_eq!(look("200", row, "abc"), Look::Wait);
        assert_eq!(look("200", "Built from abcd\n", "abc"), Look::Wait);
        assert_eq!(look("000", "", "abc"), Look::Wait);
        assert_eq!(look("404", "Built from abc.\n", "abc"), Look::Wait);
        let failed = Look::Fails("it answers 503".into());
        assert_eq!(look("503", "Built from abc.\n", "abc"), failed);
        assert_eq!(port_of("http://127.0.0.1:7788/built"), Ok("7788"));
        assert!(port_of("http://127.0.0.1/built").is_err());
    }

    /// Done-when 2: the Makefile's rust/deploy is rust/build and one line, `dash deploy`; the mirror, rust/ alone, stops here.
    #[dash_test]
    fn the_makefile_s_rust_deploy_is_one_line_calling_dash_deploy() {
        let Some(repo) = dash_meter::monorepo() else {
            return;
        };
        let makefile = std::fs::read_to_string(repo.join("Makefile")).expect("the Makefile");
        let mut lines = makefile
            .lines()
            .skip_while(|l| !l.starts_with("rust/deploy:"));
        assert_eq!(lines.next(), Some("rust/deploy: rust/build"));
        let recipe: Vec<&str> = lines.take_while(|l| l.starts_with('\t')).collect();
        let want = "\t@\"$(RUST_LIVE)/dash\" deploy --live \"$(RUST_LIVE)\" --port $(RUST_PORT)";
        assert_eq!(recipe, [want]);
    }
}
