//! Task rust-one-launch-agent: `dash launchd run` keeps our jobs and other repos' inside one process (done-when 1 and 2), each
//! job a stand-in that writes its name, on a clock 50 times faster (DASH_LAUNCHD_SECOND_MS); `dash launchd install` with a
//! stand-in launchctl removes every other plist of ours but the CI runners' (done-when 3).
use dash_meter::TestDir;
use dash_meter::dash_test;
use dash_meter::monorepo;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SECOND_MS: &str = "50";

struct Supervisor(Child);

impl Drop for Supervisor {
    fn drop(&mut self) {
        // TERM, as launchd's bootout: it stops each job's group before it exits
        let _ = Command::new("kill").arg(self.0.id().to_string()).status();
        let _ = self.0.wait();
    }
}

/// An executable stand-in at `path` that appends `<name> <args>` to `ran` and lives `secs`; called `launch rust-serve`, the
/// server, it lives on.
fn stand_in(path: &Path, name: &str, ran: &Path, secs: u32) {
    fs::create_dir_all(path.parent().expect("a folder")).expect("its folder");
    let body = format!(
        "#!/bin/sh\necho \"{name} $*\" >> '{}'\n[ \"$*\" = 'launch rust-serve' ] && exec sleep 600\nsleep {secs}\n",
        ran.display()
    );
    fs::write(path, body).expect("stand-in");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// A home, a host copy whose every program of OURS is a stand-in, and a checkout.
struct World {
    _dir: TestDir,
    home: PathBuf,
    host: PathBuf,
    repo: PathBuf,
    ran: PathBuf,
}

fn world(name: &str) -> World {
    let dir = TestDir::new(name);
    let (home, host, repo, ran) = (
        dir.0.join("home"),
        dir.0.join("host"),
        dir.0.join("repo"),
        dir.0.join("ran"),
    );
    fs::create_dir_all(repo.join(".claude/run")).expect("repo");
    for (script, secs) in [
        ("comment-wake", 600),
        ("crm-phone", 0),
        ("orchestrator-job", 0),
        ("sandbox-apply", 0),
    ] {
        stand_in(&host.join("bin").join(script), script, &ran, secs);
    }
    fs::write(
        host.join("Makefile"),
        format!(
            "crm/serve:\n\t@echo 'dashboard crm/serve' >> '{}'; exec sleep 600\n",
            ran.display()
        ),
    )
    .expect("mk");
    fs::create_dir_all(&home).expect("home");
    World {
        _dir: dir,
        home,
        host,
        repo,
        ran,
    }
}

fn supervise(w: &World) -> Supervisor {
    let child = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["launchd", "run", "--host"])
        .arg(&w.host)
        .arg("--repo")
        .arg(&w.repo)
        .env("HOME", &w.home)
        .env("DASH_LAUNCHD_SECOND_MS", SECOND_MS)
        .spawn()
        .expect("dash launchd run");
    Supervisor(child)
}

/// How many lines of `ran` are `line`, once at least `n` are, within 30 s.
fn wait_for(w: &World, line: &str, n: usize) -> usize {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let text = fs::read_to_string(&w.ran).unwrap_or_default();
        let seen = text.lines().filter(|l| l.trim_end() == line).count();
        if seen >= n || Instant::now() > until {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn request(w: &World, file: &str) {
    // once the supervisor reads its jobs (the orchestrator runs at its start): a file there before is no change
    assert!(
        wait_for(w, "orchestrator-job", 1) >= 1,
        "the supervisor never started"
    );
    fs::write(w.repo.join(".claude/run").join(file), "asked\n").expect("request");
}

#[dash_test]
fn rust_serve_starts_at_once_through_its_sandbox_job() {
    let w = world("launchd-rust-serve");
    let _s = supervise(&w);
    assert_eq!(wait_for(&w, "sandbox-apply launch rust-serve", 1), 1);
    assert_eq!(
        wait_for(&w, "dashboard crm/serve", 1),
        1,
        "the older app too"
    );
}

#[dash_test]
fn comment_wake_runs_inside_and_restarts_on_new_code() {
    let w = world("launchd-comment-wake");
    let _s = supervise(&w);
    assert_eq!(wait_for(&w, "comment-wake", 1), 1);
    // a sync's new copy: the same path, other bytes
    let script = w.host.join("bin/comment-wake");
    let body = fs::read_to_string(&script).expect("it") + "# new\n";
    fs::write(&script, body).expect("new code");
    assert_eq!(
        wait_for(&w, "comment-wake", 2),
        2,
        "not started again on its new code"
    );
    assert!(
        fs::read_to_string(w.home.join("Library/Logs/dash.log")).is_err(),
        "the job's log is its own"
    );
    assert!(w.home.join("Library/Logs/comment-wake.log").exists());
}

#[dash_test]
fn watcher_check_runs_each_minute() {
    let w = world("launchd-watcher-check");
    let _s = supervise(&w);
    // every=60 on a 50 ms second: 3 s apart
    assert_eq!(wait_for(&w, "crm-phone --watcher-check", 2), 2);
}

#[dash_test]
fn the_approve_bridge_runs_on_its_request() {
    let w = world("launchd-bridges");
    let _s = supervise(&w);
    request(&w, "approve.request");
    assert!(
        wait_for(&w, "sandbox-apply tap", 1) >= 1,
        "approve.request ran no tap"
    );
}

// Task rust-host-requests: each `host request` line of the host copy's sandbox/jobs.conf is served by dash launchd run itself,
// no bridge script (done-when 1: a test each for tailscale, denials and slack; done-when 2: brew).

/// An executable at `path` with this shell body.
fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("a folder")).expect("its folder");
    fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// The host copy's jobs.conf: these request lines, with a pane job beside them that is never served.
fn jobs_conf(w: &World, lines: &str) {
    let conf =
        format!("claude sandbox.sb pane claude --x\n# brew host request /bin/sh -c no\n{lines}");
    fs::create_dir_all(w.host.join("sandbox")).expect("sandbox");
    fs::write(w.host.join("sandbox/jobs.conf"), conf).expect("jobs.conf");
}

/// A request written whole in `run`, then its answer, within 30 s.
fn ask(run: &Path, name: &str, text: &str) -> String {
    fs::create_dir_all(run).expect("run");
    let out = run.join(format!("{name}.out"));
    let _ = fs::remove_file(&out);
    let tmp = run.join(format!("{name}.request.tmp"));
    fs::write(&tmp, text).expect("request");
    fs::rename(&tmp, run.join(format!("{name}.request"))).expect("request");
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        if let Ok(t) = fs::read_to_string(&out) {
            return t;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no answer to {name}: {text:?}");
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(
        ok.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
}

#[dash_test]
fn tailscale_runs_its_jobs_conf_line_and_keeps_his_down() {
    let w = world("requests-tailscale");
    let ts = w.host.join("bin/tailscale");
    script(
        &ts,
        &format!(
            "echo \"tailscale $*\" >> '{}'; echo \"tailscale said $*\"",
            w.ran.display()
        ),
    );
    jobs_conf(
        &w,
        &format!(
            "tailscale host request {} {{status|up|down|serve status}}\n",
            ts.display()
        ),
    );
    git(&w.repo, &["init", "-q", "-b", "main"]);
    git(&w.repo, &["commit", "-q", "--allow-empty", "-m", "a"]);
    let wt = w.repo.parent().expect("a folder").join("wt");
    git(
        &w.repo,
        &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "task"],
    );
    let _s = supervise(&w);
    let run = w.repo.join(".claude/run");
    // his down from a worktree is kept where comment-wake's watchdog reads it, the main checkout's run folder
    assert!(ask(&wt.join(".claude/run"), "tailscale", "down\n").ends_with("exit=0\n"));
    assert!(
        run.join("tailscale.down").exists(),
        "a worktree's down is not in the main checkout"
    );
    assert!(!wt.join(".claude/run/tailscale.down").exists());
    let up = ask(&run, "tailscale", "up\n");
    assert_eq!(up, "tailscale said up\nexit=0\n");
    assert!(!run.join("tailscale.down").exists(), "up keeps his down");
    assert!(ask(&run, "tailscale", "down\n").ends_with("exit=0\n"));
    assert!(
        run.join("tailscale.down").exists(),
        "a down that took leaves tailscale.down"
    );
    assert_eq!(
        ask(&run, "tailscale", "serve status\n"),
        "tailscale said serve status\nexit=0\n"
    );
    let refused = ask(&run, "tailscale", "status; touch /tmp/x\n");
    assert!(refused.starts_with("refused: tailscale refused « status; touch /tmp/x »: only status, up, down, serve status"), "{refused}");
    assert!(refused.ends_with("exit=2\n"), "{refused}");
    let ran = fs::read_to_string(&w.ran).unwrap_or_default();
    assert_eq!(
        ran.lines().filter(|l| l.starts_with("tailscale")).count(),
        4,
        "{ran}"
    );
}

#[dash_test]
fn denials_reads_the_log_for_a_worktree_and_refuses_what_jobs_conf_does_not_allow() {
    let w = world("requests-denials");
    git(&w.repo, &["init", "-q", "-b", "main"]);
    git(&w.repo, &["commit", "-q", "--allow-empty", "-m", "a"]);
    let wt = w.repo.parent().expect("a folder").join("wt");
    git(
        &w.repo,
        &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "task"],
    );
    let log = w.host.join("bin/log");
    script(
        &log,
        "case \"$*\" in *many*) seq 600; exit 0;; esac\necho \"Sandbox: deny(1) file-write-create /x say $*\"",
    );
    jobs_conf(
        &w,
        &format!(
            "denials host request {} show --style compact --last {{1-10080}}m --predicate 'sender == \"Sandbox\" AND eventMessage CONTAINS \"deny\"'\n",
            log.display()
        ),
    );
    let _s = supervise(&w);
    let run = wt.join(".claude/run");
    let got = ask(&run, "denials", "6h runner\nid=w1\n");
    assert_eq!(
        got,
        "Sandbox: deny(1) file-write-create /x say show --style compact --last 360m --predicate sender == \"Sandbox\" AND eventMessage CONTAINS \"deny\" AND eventMessage CONTAINS \"runner\"\nexit=0\nasked=w1\n"
    );
    // the session's client, `dash denials` (bin/denials-bridge-test's case 1, the bridge gone): its own id, the answer
    // without the asked= line, exit 0
    if let Some(client) = denials_client() {
        let o = Command::new(&client)
            .args(["6h", "runner"])
            .env("DASH_REPO", &wt)
            .output()
            .expect("sandbox-denials");
        let said = String::from_utf8_lossy(&o.stdout);
        assert!(o.status.success(), "{said}");
        assert_eq!(
            said,
            got.strip_suffix("asked=w1\n").expect("asked=").to_string()
        );
    }
    // a long read keeps its last 500 lines, a cut said first
    let many = ask(&run, "denials", "5 many\nid=w6\n");
    assert!(
        many.starts_with("truncated: the last 500 of 600 lines\n101\n"),
        "{}",
        &many[..80.min(many.len())]
    );
    assert!(many.ends_with("\n600\nexit=0\nasked=w6\n"));
    for (text, why) in [
        (
            "8d say\nid=w2\n",
            "refused: \"8d say\" (a window of 1m to 7d",
        ),
        ("tcc 2h\nid=w3\n", "refused: \"tcc 2h \""),
        ("5 a;b\nid=w4\n", "refused: \"5 a;b\""),
        ("5 say\n", "refused: no id line"),
    ] {
        let got = ask(&run, "denials", text);
        assert!(
            got.starts_with(why) && got.contains("exit=2\n"),
            "{text:?}: {got}"
        );
    }
    // a link a session planted is never written through: the request waits unserved
    let victim = w.repo.parent().expect("a folder").join("victim");
    fs::write(&victim, "KEEP").expect("victim");
    std::os::unix::fs::symlink(&victim, run.join("denials.out.tmp")).expect("link");
    fs::write(run.join("denials.request"), "5\nid=w5\n").expect("request");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(fs::read_to_string(&victim).expect("victim"), "KEEP");
    assert!(
        run.join("denials.request").exists(),
        "served through a planted link"
    );
}

/// private/dashboard/priv/commands/sandbox-denials, `dash denials` in the older app: none where the mirror holds rust/ alone.
fn denials_client() -> Option<PathBuf> {
    monorepo().map(|top| top.join("private/dashboard/priv/commands/sandbox-denials"))
}

/// The id line of the request the client wrote in `run`, once it is there; the request then removed.
fn request_id(run: &Path) -> String {
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        if let Some(id) = fs::read_to_string(run.join("denials.request"))
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("id=").map(str::to_string))
            })
        {
            fs::remove_file(run.join("denials.request")).expect("request");
            return id;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the client wrote no request in {}", run.display());
}

/// An answer written whole, as the supervisor writes it (.tmp then rename).
fn answer(run: &Path, text: &str) {
    fs::write(run.join("denials.out.tmp"), text).expect("answer");
    fs::rename(run.join("denials.out.tmp"), run.join("denials.out")).expect("answer");
}

/// bin/denials-bridge-test's client cases, its bridge gone (rust-lint-port): an answer for another request is refused and
/// the client waits for its own; an answer with no asked= line is a host that predates the id, said and refused.
#[dash_test]
fn the_denials_client_waits_past_another_request_s_answer_and_refuses_one_with_no_id() {
    let Some(client) = denials_client() else {
        return;
    };
    let dir = TestDir::new("denials-client");
    let run = dir.0.join(".claude/run");
    let spawn = || {
        Command::new(&client)
            .args(["5", "say"])
            .env("DASH_REPO", &dir.0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sandbox-denials")
    };
    let c = spawn();
    let id = request_id(&run);
    answer(&run, "someone else's\nexit=0\nasked=1.1\n");
    std::thread::sleep(Duration::from_millis(1500));
    answer(&run, &format!("Sandbox: deny say\nexit=0\nasked={id}\n"));
    let o = c.wait_with_output().expect("client");
    assert!(o.status.success());
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "Sandbox: deny say\nexit=0\n"
    );
    assert!(
        String::from_utf8_lossy(&o.stderr)
            .contains("refused an answer that is not for this request"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );

    let c = spawn();
    let _ = request_id(&run);
    answer(&run, "Sandbox: deny say\nexit=0\n");
    let o = c.wait_with_output().expect("client");
    assert!(!o.status.success());
    let said = String::from_utf8_lossy(&o.stdout);
    assert!(
        said.contains("predates the id") && !said.contains("deny say"),
        "{said}"
    );
}

#[dash_test]
fn slack_quits_a_running_slack_then_opens_it_through_the_proxy() {
    let w = world("requests-slack");
    let d = w.repo.parent().expect("a folder").to_path_buf();
    let (pid, line) = (d.join("slack.pid"), d.join("slack.line"));
    // Slack is a sleep the test leaves running on its own (no parent waits on it), its launch line in slack.line
    let pgrep = d.join("pgrep");
    script(
        &pgrep,
        &format!(
            "p=$(cat '{0}' 2>/dev/null); [ -n \"$p\" ] && kill -0 \"$p\" 2>/dev/null || exit 1\n\
             case \"$1\" in -x) echo \"$p\";; -lf) echo \"$p $(cat '{1}')\";; esac",
            pid.display(),
            line.display()
        ),
    );
    let open = w.host.join("bin/open");
    script(
        &open,
        &format!(
            "echo \"open $*\" >> '{0}'; sh -c 'sleep 600 >/dev/null 2>&1 & echo $! > \"$1\"' _ '{1}'\n\
             echo \"/Applications/Slack.app/Contents/MacOS/Slack $*\" > '{2}'",
            w.ran.display(),
            pid.display(),
            line.display()
        ),
    );
    jobs_conf(
        &w,
        &format!(
            "slack host request {} -a Slack {{--args|--args --proxy-server=http://127.0.0.1:62041 --ignore-certificate-errors}}\n",
            open.display()
        ),
    );
    let status = Command::new("sh")
        .args(["-c", "sleep 600 >/dev/null 2>&1 & echo $! > \"$1\"", "_"])
        .arg(&pid)
        .status();
    assert!(status.is_ok_and(|s| s.success()));
    let first = fs::read_to_string(&pid).expect("its pid");
    let s = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["launchd", "run", "--host"])
        .arg(&w.host)
        .arg("--repo")
        .arg(&w.repo)
        .env("HOME", &w.home)
        .env("DASH_LAUNCHD_SECOND_MS", SECOND_MS)
        .env("DASH_PGREP", &pgrep)
        .spawn()
        .expect("dash launchd run");
    let _s = Supervisor(s);
    let run = w.repo.join(".claude/run");
    let got = ask(&run, "slack", "proxy\nid=s1\n");
    let now = fs::read_to_string(&pid).unwrap_or_default();
    let _ = Command::new("kill").arg(now.trim()).status();
    assert!(got.contains("Slack -a Slack --args --proxy-server=http://127.0.0.1:62041 --ignore-certificate-errors\nexit=0\nasked=s1\n"), "{got}");
    assert_ne!(first, now, "Slack was not started again");
    assert!(
        !Command::new("kill")
            .args(["-0", first.trim()])
            .status()
            .is_ok_and(|s| s.success()),
        "the running Slack was not quit first"
    );
    let got = ask(&run, "slack", "anything\nid=s2\n");
    assert_eq!(
        got,
        "refused: \"anything\" (proxy or plain)\nexit=2\nasked=s2\n"
    );
    let ran = fs::read_to_string(&w.ran).unwrap_or_default();
    assert_eq!(
        ran.lines().filter(|l| l.starts_with("open ")).count(),
        1,
        "{ran}"
    );
}

#[dash_test]
fn brew_installs_only_a_formula_jobs_conf_lists_through_dash_host() {
    let w = world("requests-brew");
    git(&w.repo, &["init", "-q", "-b", "main"]);
    let brew = w.host.join("bin/brew");
    script(
        &brew,
        &format!(
            "echo \"brew $*\" >> '{0}'; echo \"==> Installing $2\"; echo exit=0; exit $(cat '{0}.code' 2>/dev/null || echo 0)",
            w.ran.display()
        ),
    );
    jobs_conf(
        &w,
        &format!("brew host request {} install {{bash}}\n", brew.display()),
    );
    // dash host knows the jobs from the host copy's jobs.conf (<SITE_PERSO_CONF>/host), never the checkout's: none here
    let conf = w.host.parent().expect("a folder").to_path_buf();
    // a request an earlier supervisor claimed and never answered: answered at the start, never run again
    let run = w.repo.join(".claude/run");
    fs::write(run.join("brew.request.serving"), "bash\nid=old\n").expect("serving");
    let _s = supervise(&w);
    let until = Instant::now() + Duration::from_secs(30);
    while !run.join("brew.out").exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    let swept = fs::read_to_string(run.join("brew.out")).expect("the interrupted request's answer");
    assert!(
        swept.starts_with("interrupted: ") && swept.ends_with("exit=75\nasked=old\n"),
        "{swept}"
    );
    assert!(!run.join("brew.request.serving").exists());
    let host = |value: &str| {
        Command::new(env!("CARGO_BIN_EXE_dash"))
            .args(["host", "brew", value])
            .env("SITE_PERSO_CONF", &conf)
            .current_dir(&w.repo)
            .output()
            .expect("dash host")
    };
    let ok = host("bash");
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&ok.stdout),
        "==> Installing bash\nexit=0\nexit=0\n"
    );
    // its own output's exit=0 never hides the job's failure: the supervisor's trailer, the last exit= line, rules
    fs::write(format!("{}.code", w.ran.display()), "1").expect("code");
    let failed = host("bash");
    assert_eq!(
        failed.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&failed.stdout)
    );
    // a job jobs.conf does not list is refused at once, never waited on
    let began = Instant::now();
    let typo = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["host", "brw", "bash"])
        .env("SITE_PERSO_CONF", &conf)
        .current_dir(&w.repo)
        .output()
        .expect("dash host");
    assert_eq!(typo.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&typo.stderr).contains("no request job brw"),
        "{}",
        String::from_utf8_lossy(&typo.stderr)
    );
    assert!(began.elapsed() < Duration::from_secs(5));
    let no = host("wget");
    assert_eq!(no.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&no.stdout),
        "refused: brew refused « wget »: only bash\nexit=2\n"
    );
    let ran = fs::read_to_string(&w.ran).unwrap_or_default();
    assert_eq!(
        ran.lines()
            .filter(|l| l.starts_with("brew"))
            .collect::<Vec<_>>(),
        ["brew install bash", "brew install bash"]
    );
}

#[dash_test]
fn the_orchestrator_runs_at_the_start_and_on_start() {
    let w = world("launchd-orchestrator");
    let _s = supervise(&w);
    assert_eq!(wait_for(&w, "orchestrator-job", 1), 1, "at load");
    request(&w, "orchestrator.request");
    assert!(
        wait_for(&w, "orchestrator-job", 2) >= 2,
        "on Start's request"
    );
}

#[dash_test]
fn freework_sweep_runs_on_its_request() {
    let w = world("launchd-freework");
    let _s = supervise(&w);
    request(&w, "freework.request");
    assert_eq!(wait_for(&w, "sandbox-apply launch freework-sweep", 1), 1);
}

/// graal's and klar's lines, as their own job files would hold them.
fn other_repos(w: &World) {
    let graal = w.home.join(".config/graal");
    stand_in(&graal.join("bin/sandbox-approve"), "graal", &w.ran, 0);
    fs::write(graal.join("dash-jobs.conf"), "dev.graal.sandbox-tick every=5,log=graal-sandbox-tick ~/.config/graal/bin/sandbox-approve tick\n")
        .expect("graal");
    let klar = w.home.join(".config/klar");
    stand_in(&klar.join("bin/sandbox-approve"), "klar", &w.ran, 0);
    stand_in(&klar.join("bin/rc"), "klar-rc", &w.ran, 600);
    let lines = "# klar's\nim.klar.oneoff-runner every=10 ~/.config/klar/bin/sandbox-approve run-allowed\n\
                 im.klar.claude-rc keepalive ~/.config/klar/bin/rc\ncom.nicolasha.comment-wake keepalive /bin/true\n";
    fs::write(klar.join("dash-jobs.conf"), lines).expect("klar");
}

#[dash_test]
fn another_repos_jobs_run_from_its_own_job_file() {
    let w = world("launchd-other-repos");
    other_repos(&w);
    let _s = supervise(&w);
    assert!(
        wait_for(&w, "graal tick", 2) >= 2,
        "graal's tick, every 5 s"
    );
    assert!(
        wait_for(&w, "klar run-allowed", 1) >= 1,
        "klar's one-off runner"
    );
    assert_eq!(
        wait_for(&w, "klar-rc", 1),
        1,
        "klar's claude-rc, kept alive"
    );
    assert!(w.home.join("Library/Logs/graal-sandbox-tick.log").exists());
    // another repo never takes one of our labels: comment-wake stays ours, started once
    assert_eq!(wait_for(&w, "comment-wake", 1), 1);
}

#[dash_test]
fn install_removes_every_other_plist_of_ours_but_the_runners() {
    let w = world("launchd-install");
    other_repos(&w);
    let agents = w.home.join("Library/LaunchAgents");
    fs::create_dir_all(&agents).expect("agents");
    for label in [
        "com.nicolasha.comment-wake",
        "com.nicolasha.dashboard",
        "com.nicolasha.rust-serve",
        "com.nicolasha.rust-orch",
        "com.nicolasha.ci-runner",
        "com.nicolasha.ci-runner-gate",
        "dev.graal.sandbox-tick",
        "im.klar.oneoff-runner",
        "actions.runner.klar-im-internal.macbookpro",
        "com.google.keystone.agent",
    ] {
        fs::write(agents.join(format!("{label}.plist")), "<plist/>").expect("plist");
    }
    // launchctl's stand-in: each call a line; print answers once a bootstrap loaded the agent
    let calls = w.home.join("launchctl.calls");
    let launchctl = w.home.join("launchctl");
    let loaded = w.home.join("loaded");
    let body = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$1\" in print) [ -e '{l}' ];; bootstrap) [ ! -e '{l}.refuse' ] && touch '{l}';; esac\n",
        calls.display(),
        l = loaded.display()
    );
    fs::write(&launchctl, body).expect("launchctl");
    fs::set_permissions(&launchctl, fs::Permissions::from_mode(0o755)).expect("chmod");
    let install = || {
        Command::new(env!("CARGO_BIN_EXE_dash"))
            .args(["launchd", "install", "--host"])
            .arg(&w.host)
            .arg("--repo")
            .arg(&w.repo)
            .args(["--bin", "/c/dash/dash"])
            .env("HOME", &w.home)
            .env("DASH_LAUNCHCTL", &launchctl)
            .output()
            .expect("dash launchd install")
    };
    // a bootstrap launchd refuses touches no other agent: nothing stops running
    fs::write(w.home.join("loaded.refuse"), "").expect("refuse");
    let out = install();
    assert!(!out.status.success(), "a refused bootstrap exits 0");
    let count = fs::read_dir(&agents).expect("agents").count();
    assert_eq!(
        count, 11,
        "a plist went while the one agent is not loaded (10 and its own)"
    );
    fs::remove_file(w.home.join("loaded.refuse")).expect("refused no more");
    fs::remove_file(&calls).expect("calls");
    let out = install();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut left: Vec<String> = fs::read_dir(&agents)
        .expect("agents")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            "actions.runner.klar-im-internal.macbookpro.plist",
            "com.google.keystone.agent.plist",
            "com.nicolasha.ci-runner-gate.plist",
            "com.nicolasha.ci-runner.plist",
            "com.nicolasha.dash.plist",
        ]
    );
    let said = fs::read_to_string(&calls).expect("calls");
    let uid =
        String::from_utf8(Command::new("id").arg("-u").output().expect("id").stdout).expect("uid");
    let domain = format!("gui/{}", uid.trim());
    for gone in ["comment-wake", "dashboard", "rust-serve", "rust-orch"] {
        assert!(
            said.contains(&format!("bootout {domain}/com.nicolasha.{gone}\n")),
            "{gone} not booted out: {said}"
        );
    }
    for gone in ["dev.graal.sandbox-tick", "im.klar.oneoff-runner"] {
        assert!(
            said.contains(&format!("bootout {domain}/{gone}\n")),
            "{gone} not booted out: {said}"
        );
    }
    let plist = agents.join("com.nicolasha.dash.plist");
    // the one agent loaded first, the others only after
    let first = said
        .lines()
        .position(|l| l.starts_with("bootout"))
        .expect("a bootout");
    let loaded = said
        .lines()
        .position(|l| l == format!("bootstrap {domain} {}", plist.display()));
    assert!(loaded.is_some_and(|at| at < first), "{said}");
    let text = fs::read_to_string(&plist).expect("the plist");
    let host = w.host.display();
    assert!(text.contains(&format!("<string>/c/dash/dash</string><string>launchd</string><string>run</string><string>--host</string><string>{host}</string>")), "{text}");
    assert!(
        text.contains("<key>KeepAlive</key><true/>")
            && text.contains("<key>RunAtLoad</key><true/>")
    );
    // again: current, nothing booted out or loaded
    let before = said.lines().count();
    let out = install();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        format!("{stdout}{stderr}").contains("installed and current"),
        "{stdout}{stderr}"
    );
    let after = fs::read_to_string(&calls).expect("calls");
    assert_eq!(
        after.lines().skip(before).collect::<Vec<_>>(),
        [format!("print {domain}/com.nicolasha.dash")]
    );
}

#[dash_test]
fn a_crashed_supervisors_jobs_end_before_the_next_starts_its_own() {
    let w = world("launchd-orphans");
    let mut first = supervise(&w);
    assert_eq!(wait_for(&w, "comment-wake", 1), 1);
    let groups = w.home.join(".config/site_perso/dash/launchd.groups");
    let until = Instant::now() + Duration::from_secs(30);
    while fs::read_to_string(&groups)
        .unwrap_or_default()
        .lines()
        .count()
        < 3
        && Instant::now() < until
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    let recorded = fs::read_to_string(&groups).expect("the groups recorded");
    let pids: Vec<String> = recorded
        .lines()
        .filter_map(|l| l.split(' ').next())
        .map(str::to_string)
        .collect();
    // a crash: KILL, so nothing stops its jobs, which live on in their own groups
    let _ = first.0.kill();
    let _ = first.0.wait();
    let alive = |pid: &str| {
        Command::new("kill")
            .args(["-0", pid])
            .status()
            .is_ok_and(|s| s.success())
    };
    let pids: Vec<String> = pids.into_iter().filter(|p| alive(p)).collect();
    assert!(
        pids.len() >= 3,
        "rust-serve, the dashboard and comment-wake outlive it: {recorded}"
    );
    let _second = supervise(&w);
    assert_eq!(wait_for(&w, "comment-wake", 2), 2, "started again");
    assert!(
        pids.iter().all(|p| !alive(p)),
        "an earlier run's job still runs beside the new one"
    );
}
