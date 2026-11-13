//! Jev (TypeSafe AI, the model `dash doc ready` asks), the binary's one client: the inbox's check pass and the RFC judge
//! (`rfc_judge`) both ask through it (task rust-task-rfc-judge moved it out of inbox.rs). Each question is a « noul », 0 to 1;
//! the API refuses a `text` one (400, tried 2026-11-11). Over curl, its key from bin/secret in curl's environment, never its
//! command line. CI and a test never pay (`paid_refused`).
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

use dash_hooks::outbound::{JEV_LOG, JEV_LOG_BEFORE};

pub const URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const MODEL: &str = "jev-1.13.0";
/// One call, at most.
pub const CALL: Duration = Duration::from_secs(60);
/// Its price in US dollars per input token, $0.042 a million (Klar's ledger, qualification/jev); its output is not priced.
pub const USD_PER_INPUT_TOKEN: f64 = 0.042 / 1e6;

/// Why no paid call runs here, when CI or nextest runs it.
pub fn paid_refused() -> Option<String> {
    let under = ["CI", "NEXTEST"]
        .into_iter()
        .find(|v| std::env::var_os(v).is_some())?;
    Some(format!(
        "{under} is set: a check never pays Jev or an agent there"
    ))
}

/// The FNV-1a hash of `s`: short, and the same in every build.
pub fn fnv(s: &str) -> u64 {
    (s.bytes()).fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Jev's key, read once a process: bin/secret is a Python start each.
pub fn key() -> Result<&'static String, String> {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(k) = KEY.get() {
        return Ok(k);
    }
    let bin = crate::pages::repo()
        .join("bin/secret")
        .display()
        .to_string();
    let key = dash_process::run(
        &bin,
        &["get", "typesafe", "TYPESAFE_API_KEY"],
        None,
        Duration::from_secs(20),
    )?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("no TYPESAFE_API_KEY in bin/secret's typesafe".into());
    }
    Ok(KEY.get_or_init(|| key))
}

/// One call: `questions` (a JSON object of questions) on `state`, its answer whole (`answers`, `usage`). `agent` names the
/// caller in its User-Agent. Refused under CI or a test before the key is read.
pub fn ask(
    questions: &serde_json::Value,
    state: serde_json::Value,
    agent: &str,
) -> Result<serde_json::Value, String> {
    if let Some(no) = paid_refused() {
        return Err(no);
    }
    let body =
        serde_json::json!({"state": state, "model": MODEL, "questions": questions}).to_string();
    let key = key()?;
    let args = curl_args(&body, agent);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (status, text, reason) =
        dash_process::answer("curl", &args, None, &[("KEY", key.as_str())], CALL)
            .map_err(String::from)?;
    let (got, code) = text
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", text.trim()));
    if !status.success() || code != "200" {
        return Err(format!(
            "Jev answered {code}: {}{}",
            got.chars().take(200).collect::<String>(),
            reason.trim()
        ));
    }
    serde_json::from_str(got).map_err(|e| format!("Jev's answer: {e}"))
}

/// curl's words for one call: its own limit, `--max-time`, 5 s under `CALL`, the process's, so curl answers its timeout before
/// the process is ended; the key from its environment, never these words.
fn curl_args(body: &str, agent: &str) -> Vec<String> {
    let max = (CALL.as_secs() - 5).to_string();
    let words = [
        "-sS",
        "--max-time",
        &max,
        "--variable",
        "%KEY",
        "--expand-header",
        "Authorization: Bearer {{KEY}}",
        "-H",
        "Content-Type: application/json",
        "-A",
        agent,
        "--data-binary",
        body,
        "-w",
        "\n%{http_code}",
        URL,
    ];
    words.iter().map(|w| w.to_string()).collect()
}

/// Jev's « noul » for `question` in its answer.
pub fn noul(answer: &serde_json::Value, question: &str) -> Result<f64, String> {
    let n = answer["answers"][question]["noul"].as_f64();
    n.ok_or_else(|| format!("Jev gave no score for {question}"))
}

/// The input tokens an answer was billed for: 0 when it does not say.
pub fn input_tokens(answer: &serde_json::Value) -> u64 {
    answer["usage"]["input_tokens"].as_u64().unwrap_or(0)
}

/// The older app's database under a repository root (`tasks::older_db`'s, for a root given).
const OLDER_DB: &str = "private/dashboard/dashboard_dev.db";

/// One Jev attempt (ats.py's row) as a line of the main checkout's log (`pages::repo`), never a worktree's: a verdict asked
/// from a task's session lands where the older app reads it (`Host::jev_log`, `dash jev log`).
pub fn log(row: &serde_json::Value) -> Result<(), String> {
    log_at(&crate::pages::repo(), row)
}

/// The line under `root`, synced, then fed to the older app: once moved (`dash jev move`), the log never changes under the
/// older app's store, so it never reads it again (task rust-jev-log-off-store-lock). Before the move, the line goes to the
/// file the older app reads itself. A feed that fails is said and caught up by the next, the line being in the log.
pub(crate) fn log_at(root: &Path, row: &serde_json::Value) -> Result<(), String> {
    let mut path = dash_hooks::outbound::jev_log_target(root)?;
    let before = root.join(JEV_LOG_BEFORE);
    // the file before is never created: moved away since it was named (`dash jev move`), the line goes to the log
    let open = |p: &Path| {
        std::fs::OpenOptions::new()
            .create(p != before)
            .append(true)
            .open(p)
    };
    let mut f = match open(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && path == before => {
            path = dash_hooks::outbound::jev_log_target(root)?;
            open(&path)
        }
        f => f,
    }
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let named = |e: std::io::Error| format!("{}: {e}", path.display());
    f.write_all(format!("{row}\n").as_bytes()).map_err(named)?;
    // written: a sync that fails is said, never an error, or ats.py would write the line a second time
    if let Err(e) = f.sync_data() {
        eprintln!("Jev's log: {} not synced: {e}", path.display());
    }
    if let Err(why) = feed(root) {
        eprintln!("Jev's log: the older app not fed ({why}): the next line catches up");
    }
    Ok(())
}

/// Feeds the older app the log's lines it lacks, those after its last one: none while the log's file before the move stands,
/// since the older app reads that one itself. The count fed.
pub fn feed(root: &Path) -> Result<usize, String> {
    let log = root.join(dash_hooks::outbound::JEV_LOG);
    if root.join(dash_hooks::outbound::JEV_LOG_BEFORE).exists() || !log.exists() {
        return Ok(0);
    }
    dash_store::older_jev::feed(&root.join(OLDER_DB), |last, held| {
        lines_after(&log, last, held)
    })
}

/// The lines of `log` after the one that reads as `last` (the same JSON, else the same text), read from its end: a call
/// reads the last megabyte, not the 64 MB before it, and the whole log only when that megabyte lacks `last`. Every line when
/// `last` is none. A `last` the log lacks (a union merge that reordered lines, a hand edit) falls back on the count the older
/// app `held`: the lines past it, said on stderr; a log shorter than that count is an error, nothing fed. A last line with no
/// newline yet (another writer's append halfway) waits for the next feed.
fn lines_after(log: &Path, last: Option<&str>, held: usize) -> Result<Vec<String>, String> {
    let named = |e: std::io::Error| format!("{}: {e}", log.display());
    let f = File::open(log).map_err(named)?;
    let len = f.metadata().map_err(named)?.len();
    let want = last.map(|l| (l, serde_json::from_str::<serde_json::Value>(l).ok()));
    let mut window: u64 = 1 << 20;
    loop {
        let start = match want {
            Some(_) => len.saturating_sub(window),
            None => 0,
        };
        (&f).seek(SeekFrom::Start(start)).map_err(named)?;
        let mut bytes = vec![];
        (&f).take(len - start)
            .read_to_end(&mut bytes)
            .map_err(named)?;
        // a window that starts inside a line (or a character) drops it: the next, wider one reads it whole
        let whole = match start {
            0 => &bytes[..],
            _ => bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(&[][..], |at| &bytes[at + 1..]),
        };
        let whole = &whole[..whole
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |at| at + 1)];
        let whole = std::str::from_utf8(whole).map_err(|e| format!("{}: {e}", log.display()))?;
        let lines: Vec<&str> = whole.lines().filter(|l| !l.is_empty()).collect();
        let Some((l, as_json)) = &want else {
            return Ok(lines.into_iter().map(String::from).collect());
        };
        // parsed only when the line names the row's key: a whole read parses a handful of lines, not 64 MB of them
        let key = as_json.as_ref().and_then(|v| v["key"].as_str());
        let same = |line: &&str| match as_json {
            Some(v) => {
                key.is_none_or(|k| line.contains(k))
                    && serde_json::from_str::<serde_json::Value>(line)
                        .ok()
                        .as_ref()
                        == Some(v)
            }
            None => line == l,
        };
        if let Some(at) = lines.iter().rposition(same) {
            return Ok(lines[at + 1..].iter().map(|s| s.to_string()).collect());
        }
        if start == 0 {
            let past = lines.get(held..).ok_or(format!(
                "the older app holds {held} Jev rows, {} has {} lines: nothing fed",
                log.display(),
                lines.len()
            ))?;
            eprintln!(
                "Jev's log: the older app's last row is not in {}: fed the lines past its {held}",
                log.display()
            );
            return Ok(past.iter().map(|s| s.to_string()).collect());
        }
        // one whole read next: windows of 4, 16, 64 MB each from the end reread 85 MB before it (review of PR 1520)
        window = len;
    }
}

const USAGE_FEED: &str =
    "dash jev feed (the older app's model gets the lines of Jev's log it lacks)";
const USAGE_LOG: &str =
    "dash jev log (each line of stdin, one Jev attempt as JSON, logged and fed to the older app)";

fn feed_command(args: &[&str], mut out: File) {
    if !args.is_empty() {
        eprintln!("usage: {USAGE_FEED}");
        std::process::exit(2);
    }
    match feed(&crate::pages::repo()) {
        Ok(n) => writeln!(out, "{n} lines fed").unwrap_or_default(),
        Err(why) => {
            eprintln!("dash jev feed: {why}");
            std::process::exit(1);
        }
    }
}

/// ats.py's writer: exit 1 when a line is not logged, so it appends that one itself.
fn log_command(args: &[&str], _out: File) {
    if !args.is_empty() {
        eprintln!("usage: {USAGE_LOG}");
        std::process::exit(2);
    }
    for line in std::io::stdin().lines() {
        let logged = line
            .map_err(|e| format!("stdin: {e}"))
            .and_then(|l| match l.trim() {
                "" => Ok(()),
                l => log(&serde_json::from_str(l).map_err(|e| format!("not JSON: {e}"))?),
            });
        if let Err(why) = logged {
            eprintln!("dash jev log: {why}");
            std::process::exit(1);
        }
    }
}

const USAGE_MOVE: &str = "dash jev move (once, right after task rust-jev-log-off-store-lock's merge: Jev's log moved off the older app's source, the older app fed, the move shipped)";

/// The move in the main checkout, then the older app fed, then `dash ship` of both paths: committed before any rebase's
/// autostash brings the old file back, where the older app would read it whole again under its lock (review of PR 1520).
fn move_command(args: &[&str], mut out: File) {
    if !args.is_empty() {
        eprintln!("usage: {USAGE_MOVE}");
        std::process::exit(2);
    }
    let repo = crate::pages::repo();
    // run again once shipped: nothing to move nor to ship
    let tracked = std::process::Command::new("git")
        .args(["cat-file", "-e", &format!("HEAD:{JEV_LOG_BEFORE}")])
        .current_dir(&repo)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !tracked && !repo.join(JEV_LOG_BEFORE).exists() {
        writeln!(out, "Jev's log already moved and shipped, at {JEV_LOG}").unwrap_or_default();
        return;
    }
    let moved = dash_hooks::outbound::jev_log_move(&repo).and_then(|_| feed(&repo));
    let fed = moved.unwrap_or_else(|why| {
        eprintln!("dash jev move: {why}");
        std::process::exit(1);
    });
    writeln!(out, "Jev's log at {JEV_LOG}, {fed} lines fed").unwrap_or_default();
    let message = "[chore][rust-jev-log-off-store-lock] Jev's log moves off the older app's source";
    let shipped = std::env::current_exe().and_then(|bin| {
        std::process::Command::new(bin)
            .args(["ship", message, JEV_LOG_BEFORE, JEV_LOG])
            .env("DASH_REPO", &repo)
            .current_dir(&repo)
            .status()
    });
    match shipped {
        Ok(s) if s.success() => {}
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("dash jev move: dash ship: {e}");
            std::process::exit(1);
        }
    }
}

inventory::submit! {
    crate::usage::Command { usage: USAGE_FEED, words: &["jev", "feed"], run: Some(feed_command) }
}

inventory::submit! {
    crate::usage::Command { usage: USAGE_MOVE, words: &["jev", "move"], run: Some(move_command) }
}

inventory::submit! {
    crate::usage::Command { usage: USAGE_LOG, words: &["jev", "log"], run: Some(log_command) }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dash_meter::TestDir;
    use serde_json::json;

    /// The older app's tables, as much of them as the feed reads, its log's rows `held`.
    fn older(root: &Path, held: &[&str]) -> dash_store::Connection {
        std::fs::create_dir_all(root.join("private/dashboard")).expect("dir");
        let db = dash_store::Connection::open(root.join(OLDER_DB)).expect("db");
        db.execute_batch(
            "CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, at TEXT, data TEXT, body TEXT,
               PRIMARY KEY (source, pos));
             CREATE TABLE tasks (collection TEXT, id TEXT);
             CREATE TABLE leads (collection TEXT, id TEXT);",
        )
        .expect("tables");
        for (pos, line) in held.iter().enumerate() {
            let data = serde_json::from_str::<serde_json::Value>(line)
                .expect("json")
                .to_string();
            db.execute(
                "INSERT INTO events (source, pos, data) VALUES ('ats/jev.jsonl', ?1, ?2)",
                (pos as i64, data),
            )
            .expect("row");
        }
        db
    }

    /// The log's rows as the older app holds them, oldest first.
    fn held(db: &dash_store::Connection) -> Vec<serde_json::Value> {
        let mut q = db
            .prepare("SELECT data FROM events WHERE source = 'ats/jev.jsonl' ORDER BY pos")
            .expect("query");
        q.query_map([], |r| r.get::<_, String>(0))
            .expect("rows")
            .map(|d| serde_json::from_str(&d.expect("row")).expect("json"))
            .collect()
    }

    /// The bytes this process has read through any file so far: Linux's `rchar`, every read call, the page cache's included.
    /// `None` on the Mac, which counts only reads that reached the disk.
    fn bytes_read() -> Option<u64> {
        let io = std::fs::read_to_string("/proc/self/io").ok()?;
        let line = io.lines().find(|l| l.starts_with("rchar:"))?;
        line["rchar:".len()..].trim().parse().ok()
    }

    /// Done-when 1: a Jev call's line costs no read of the whole log. On a 64 MB log the older app holds up to its last line,
    /// three calls' lines are synced and fed reading under 2 MB each, lines_after's 1 MB window and the older app's pages,
    /// counted on Linux (CI's mirror), where a missing count fails the test; a read of the whole log into memory fails its
    /// 50 MB budget on the Mac too. The wall clock is a loose ceiling, 5 s a call, as
    /// a shared runner's fsync took 1.4 s (mirror run 43306836185 failed the 200 ms bound this replaced, task
    /// rust-store-rebuild-write-flake); the older app's reread was 37 s, 2026-11-12.
    #[dash_meter::dash_test]
    fn a_jev_line_on_a_64_mb_log_reads_its_tail_alone() {
        let dir = TestDir::new("jev-log-64mb");
        let path = dir.0.join(JEV_LOG);
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        let mut w = std::io::BufWriter::new(File::create(&path).expect("log"));
        let state = "x".repeat(16 * 1024);
        let mut last = String::new();
        for i in 0..4096 {
            // ats.py's form: sorted keys, a space after each separator
            last = format!(
                "{{\"key\": \"k{i}\", \"request\": {{\"body\": {{\"state\": \"{state}\"}}}}, \"status\": 200}}"
            );
            writeln!(w, "{last}").expect("line");
        }
        drop(w);
        assert!(std::fs::metadata(&path).expect("size").len() >= 64 << 20);
        let db = older(&dir.0, &[&last]);
        let rows: Vec<_> = (0..3)
            .map(|i| json!({"key": format!("new{i}"), "status": 200, "ts": "2026-11-12T18:00:00"}))
            .collect();
        let before = bytes_read();
        assert!(
            before.is_some() || !cfg!(target_os = "linux"),
            "/proc/self/io unread: the bytes read go uncounted"
        );
        for row in &rows {
            let began = std::time::Instant::now();
            log_at(&dir.0, row).expect("logged");
            let took = began.elapsed();
            assert!(took < Duration::from_secs(5), "a call took {took:?}");
        }
        if let (Some(before), Some(after)) = (before, bytes_read()) {
            let read = after - before;
            assert!(
                read < 3 * (2 << 20),
                "three calls read {read} bytes, past 2 MB each"
            );
        }
        assert_eq!(held(&db)[1..], rows[..]);
    }

    /// Before the move a line goes to the file the older app reads itself, nothing fed. `dash jev move` moves it and feeds the
    /// older app what it lacks: here the line its last reread missed and the one after. A full copy of the old file back
    /// beside the log (a rebase's autostash, review of PR 1520) adds nothing to the log or the older app; a copy holding a line
    /// the log lacks adds that line alone; a link left by a move that stopped halfway adds nothing either.
    #[dash_meter::dash_test]
    fn the_move_feeds_what_the_older_app_lacks_and_a_copy_back_adds_only_new_lines() {
        let dir = TestDir::new("jev-log-move");
        let line = |k: &str| json!({"key": k, "status": 200});
        let [a, b, c, d, x, e, g] = ["a", "b", "c", "d", "x", "e", "g"].map(line);
        let (before, log) = (dir.0.join(JEV_LOG_BEFORE), dir.0.join(JEV_LOG));
        std::fs::create_dir_all(dir.0.join("private/ats")).expect("dir");
        std::fs::write(&before, format!("{a}\n{b}\n")).expect("before");
        let db = older(&dir.0, &[&a.to_string()]);
        log_at(&dir.0, &c).expect("logged");
        assert_eq!(held(&db), vec![a.clone()], "the older app reads it itself");
        assert!(!log.exists());
        dash_hooks::outbound::jev_log_move(&dir.0).expect("moved");
        assert_eq!(feed(&dir.0), Ok(2));
        assert!(!before.exists());
        let text = |want: &[&serde_json::Value]| {
            let lines: Vec<String> = want.iter().map(|v| format!("{v}\n")).collect();
            assert_eq!(std::fs::read_to_string(&log).expect("log"), lines.concat());
        };
        text(&[&a, &b, &c]);
        std::fs::write(&before, format!("{a}\n{b}\n")).expect("a full copy back");
        log_at(&dir.0, &d).expect("logged");
        assert!(!before.exists());
        text(&[&a, &b, &c, &d]);
        std::fs::write(&before, format!("{a}\n{x}")).expect("a copy with a new line");
        log_at(&dir.0, &e).expect("logged");
        text(&[&a, &b, &c, &d, &x, &e]);
        std::fs::hard_link(&log, &before).expect("a move stopped halfway");
        log_at(&dir.0, &g).expect("logged");
        assert!(!before.exists());
        text(&[&a, &b, &c, &d, &x, &e, &g]);
        assert_eq!(held(&db), vec![a, b, c, d, x, e, g]);
        assert_eq!(
            dash_hooks::outbound::jev_log_move(&dir.0),
            Ok(log.clone()),
            "moved once"
        );
        let mut half = std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .expect("log");
        write!(half, "{{\"key\": \"h\"").expect("half a line");
        assert_eq!(feed(&dir.0), Ok(0), "half a line waits");
        writeln!(half, "}}").expect("its end");
        assert_eq!(feed(&dir.0), Ok(1));
    }

    /// The older app's last row missing from the log (a union merge that reordered it, a hand edit): the lines past the count
    /// it holds are fed, so it catches up; a log shorter than that count feeds nothing, and says so.
    #[dash_meter::dash_test]
    fn a_last_row_the_log_lacks_falls_back_on_the_count() {
        let dir = TestDir::new("jev-log-lost");
        let path = dir.0.join(JEV_LOG);
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        std::fs::write(&path, "{\"key\": \"a\"}\n{\"key\": \"b\"}\n").expect("log");
        let db = older(&dir.0, &["{\"key\": \"gone\"}"]);
        assert_eq!(feed(&dir.0), Ok(1));
        assert_eq!(held(&db)[1], json!({"key": "b"}));
        let short = TestDir::new("jev-log-short");
        let path = short.0.join(JEV_LOG);
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        std::fs::write(&path, "{\"key\": \"a\"}\n").expect("log");
        let db = older(&short.0, &["{\"key\": \"x\"}", "{\"key\": \"y\"}"]);
        let said = feed(&short.0).expect_err("refused");
        assert!(said.contains("holds 2 Jev rows"), "{said}");
        assert_eq!(held(&db).len(), 2);
    }

    /// A test never pays: the call is refused before the key is read, nextest setting NEXTEST.
    #[dash_meter::dash_test]
    fn a_test_never_pays_jev() {
        let said = ask(&serde_json::json!({}), serde_json::json!({}), "test").expect_err("refused");
        assert!(said.contains("is set: a check never pays Jev"), "{said}");
    }

    /// Done-when 4, calls bounded: each call is bounded twice, curl's own `--max-time 55` and the process ended at `CALL`, 60 s;
    /// the key never in its words.
    #[dash_meter::dash_test]
    fn each_call_is_bounded_by_curl_and_by_its_process() {
        let args = curl_args("{}", "site_perso/rfc-check");
        let at = args
            .iter()
            .position(|a| a == "--max-time")
            .expect("--max-time");
        assert_eq!((args[at + 1].as_str(), CALL.as_secs()), ("55", 60));
        assert!(
            args.iter()
                .all(|a| !a.contains("Bearer ") || a.contains("{{KEY}}")),
            "{args:?}"
        );
    }
}
