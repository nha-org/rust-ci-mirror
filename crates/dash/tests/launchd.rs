//! Task rust-one-launch-agent: `dash launchd run` keeps our jobs and other repos' inside one process (done-when 1 and 2), each
//! job a stand-in that writes its name, on a clock 50 times faster (DASH_LAUNCHD_SECOND_MS); `dash launchd install` with a
//! stand-in launchctl removes every other plist of ours but the CI runners' (done-when 3).
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
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
        ("tailscale-bridge", 0),
        ("denials-bridge", 0),
        ("slack-bridge", 0),
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

#[test]
fn rust_serve_starts_at_once_through_its_sandbox_job() {
    let _budget = TestBudget::new();
    let w = world("launchd-rust-serve");
    let _s = supervise(&w);
    assert_eq!(wait_for(&w, "sandbox-apply launch rust-serve", 1), 1);
    assert_eq!(
        wait_for(&w, "dashboard crm/serve", 1),
        1,
        "the older app too"
    );
}

#[test]
fn comment_wake_runs_inside_and_restarts_on_new_code() {
    let _budget = TestBudget::new();
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

#[test]
fn watcher_check_runs_each_minute() {
    let _budget = TestBudget::new();
    let w = world("launchd-watcher-check");
    let _s = supervise(&w);
    // every=60 on a 50 ms second: 3 s apart
    assert_eq!(wait_for(&w, "crm-phone --watcher-check", 2), 2);
}

#[test]
fn the_bridges_run_on_their_request() {
    let _budget = TestBudget::new();
    let w = world("launchd-bridges");
    let _s = supervise(&w);
    assert!(
        wait_for(&w, "orchestrator-job", 1) >= 1,
        "the supervisor never started"
    );
    assert_eq!(
        wait_for(&w, "tailscale-bridge", 0),
        0,
        "a bridge never runs unasked at the start"
    );
    for (file, line) in [
        ("tailscale.request", "tailscale-bridge"),
        ("denials.request", "denials-bridge"),
        ("slack.request", "slack-bridge"),
        ("approve.request", "sandbox-apply tap"),
    ] {
        request(&w, file);
        assert!(wait_for(&w, line, 1) >= 1, "{file} ran no {line}");
    }
}

#[test]
fn the_orchestrator_runs_at_the_start_and_on_start() {
    let _budget = TestBudget::new();
    let w = world("launchd-orchestrator");
    let _s = supervise(&w);
    assert_eq!(wait_for(&w, "orchestrator-job", 1), 1, "at load");
    request(&w, "orchestrator.request");
    assert!(
        wait_for(&w, "orchestrator-job", 2) >= 2,
        "on Start's request"
    );
}

#[test]
fn freework_sweep_runs_on_its_request() {
    let _budget = TestBudget::new();
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

#[test]
fn another_repos_jobs_run_from_its_own_job_file() {
    let _budget = TestBudget::new();
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

#[test]
fn install_removes_every_other_plist_of_ours_but_the_runners() {
    let _budget = TestBudget::new();
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

#[test]
fn a_crashed_supervisors_jobs_end_before_the_next_starts_its_own() {
    let _budget = TestBudget::new();
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
