//! `dash ship` as sessions run it (task dash-cli-survives-deploy): bin/dash execs this repo's Rust binary, which commits and
//! pushes with no call to :7788, so a front door that drops every connection (a deploy restarting it) cuts nothing; it refuses
//! the app's code from a checkout that is no task worktree; it ends on its commit or its failure, never « may have run
//! partway ». Each repo is real git: a bare origin, its main checkout holding private/live/rust/dash, a task worktree.
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

/// bin/dash, or None on the public mirror, which holds rust/ alone (its CI went red on a missing bin/dash, PR 1426).
fn bin_dash() -> Option<PathBuf> {
    dash_meter::monorepo().map(|top| top.join("private/dashboard/bin/dash"))
}

/// git with no config but the test's: his global hooks and settings never reach these repos.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = env(Command::new("git").current_dir(dir).args(args))
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn env(c: &mut Command) -> &mut Command {
    c.env(
        "GIT_CONFIG_PARAMETERS",
        "'user.name=t' 'user.email=t@t' 'init.defaultBranch=main'",
    )
    .env("SHIP_RETRY_WAIT", "0")
    .env_remove("DASH_REPO")
}

/// A bare origin and its main checkout, one commit on main pushed, the Rust binary where bin/dash looks for it.
fn repo(t: &TestDir) -> (PathBuf, PathBuf) {
    let origin = t.0.join("origin.git");
    let main = t.0.join("main");
    git(
        &t.0,
        &["init", "-q", "--bare", origin.to_str().expect("utf8")],
    );
    git(
        &t.0,
        &["clone", "-q", origin.to_str().expect("utf8"), "main"],
    );
    std::fs::write(main.join("README"), "r\n").expect("readme");
    git(&main, &["add", "README"]);
    git(&main, &["commit", "-qm", "first"]);
    git(&main, &["push", "-q", "origin", "HEAD:main"]);
    git(&main, &["branch", "-u", "origin/main"]);
    let live = main.join("private/live/rust");
    std::fs::create_dir_all(&live).expect("live");
    std::os::unix::fs::symlink(DASH, live.join("dash")).expect("the binary");
    std::fs::write(main.join(".gitignore"), "private/live/\n").expect("ignore");
    (origin, main)
}

/// A front door that takes each connection and drops it, as :7788 does when a deploy restarts it; it counts them.
fn dropping() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let seen = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            count.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (url, seen)
}

/// `<program> ship <message> <paths>` in `dir` (bin/dash, or the Rust binary itself), the front door at `url`: exit code
/// and stdout with stderr.
fn ship(program: &Path, dir: &Path, url: &str, message: &str, paths: &[&str]) -> (i32, String) {
    let out = env(Command::new(program)
        .current_dir(dir)
        .arg("ship")
        .arg(message)
        .args(paths))
    .env("DASHBOARD_URL", url)
    .env("DASH_RESTART_TRIES", "0")
    .output()
    .expect("ship");
    let said =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), said)
}

/// Part 1: bin/dash ships through the Rust binary while the front door drops every connection, and never calls it.
#[dash_test]
fn a_ship_runs_in_the_rust_binary_while_the_front_door_drops_every_connection() {
    let Some(bin) = bin_dash() else { return };
    let t = TestDir::new("ship-front-door");
    let (origin, main) = repo(&t);
    let (url, seen) = dropping();
    std::fs::write(main.join("notes.md"), "n\n").expect("notes");
    let (code, said) = ship(&bin, &main, &url, "add notes", &["notes.md", ".gitignore"]);
    assert_eq!(code, 0, "{said}");
    assert_eq!(
        git(&origin, &["log", "-1", "--format=%s", "main"]),
        "add notes"
    );
    assert!(!said.contains("did not run it"), "{said}");
    // the exec passes the binary's words and exit code through: a misuse is 2, as the binary says it
    let (code, misuse) = ship(&bin, &main, &url, "nothing named", &[]);
    assert_eq!(code, 2, "{misuse}");
    assert!(
        misuse.contains("git-ship: name the files to commit"),
        "{misuse}"
    );
    assert_eq!(
        seen.load(Ordering::SeqCst),
        0,
        "bin/dash called the front door: {said}{misuse}"
    );
}

/// A live binary that does not know ship (one deployed before it) leaves bin/dash on git-ship from here, never the app: the
/// app's restart under a ship dropped its answer after 5 minutes, curl 52 with no word of what landed (task
/// ship-outlives-curl). The front door drops every connection, as a restart does, and is never called.
#[dash_test]
fn a_binary_that_does_not_know_ship_ships_through_git_ship_never_the_app() {
    let Some(bin) = bin_dash() else { return };
    let t = TestDir::new("ship-old-path");
    let (origin, main) = repo(&t);
    let live = main.join("private/live/rust/dash");
    std::fs::remove_file(&live).expect("rm the link");
    std::fs::write(&live, "#!/bin/sh\necho 'usage: dash serve' >&2\nexit 2\n")
        .expect("an old binary");
    std::fs::set_permissions(&live, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    std::fs::write(main.join("old.txt"), "o\n").expect("old");
    let (url, seen) = dropping();
    let (code, said) = ship(
        &bin,
        &main,
        &url,
        "the old path",
        &["old.txt", ".gitignore"],
    );
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains("dash: the dashboard did not run it; shipping from here"),
        "{said}"
    );
    assert!(said.contains("git-ship: pushed to origin/main: "), "{said}");
    assert_eq!(
        git(&origin, &["log", "-1", "--format=%s", "main"]),
        "the old path"
    );
    assert_eq!(seen.load(Ordering::SeqCst), 0, "the app was called: {said}");
}

/// Part 2: the app's code shipped from the main checkout is refused, nothing staged or committed; from a task worktree it ships.
#[dash_test]
fn app_code_ships_only_from_a_task_worktree() {
    let t = TestDir::new("ship-app-code");
    let (origin, main) = repo(&t);
    let (url, seen) = dropping();
    let dash = Path::new(DASH);
    let lib = "private/dashboard/lib/dashboard/launch.ex";
    std::fs::create_dir_all(main.join("private/dashboard/lib/dashboard")).expect("lib");
    std::fs::write(main.join(lib), "defmodule Launch do end\n").expect("launch.ex");
    let before = git(&main, &["rev-parse", "HEAD"]);
    let (code, said) = ship(
        dash,
        &main,
        &url,
        "scripted launch.ex",
        &[lib, ".gitignore"],
    );
    assert_eq!(code, 1, "{said}");
    assert!(
        said.contains(&format!("git-ship: {lib} is the app's code, shipped from "))
            && said.contains("a checkout that is no task worktree: nothing committed"),
        "{said}"
    );
    assert_eq!(git(&main, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        git(&main, &["diff", "--cached", "--name-only"]),
        "",
        "staged: {said}"
    );
    for code_path in [
        "private/dashboard/test/x_test.exs",
        "private/dashboard/config/runtime.exs",
        "private/dashboard/priv/commands/git-ship",
        "rust/crates/dash/src/x.rs",
        "rust/Cargo.toml",
    ] {
        let file = main.join(code_path);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(&file, "x\n").expect("code");
        let (code, said) = ship(dash, &main, &url, "code", &[code_path]);
        assert_eq!(code, 1, "{code_path}: {said}");
        assert!(
            said.contains("a checkout that is no task worktree"),
            "{said}"
        );
        std::fs::remove_file(&file).expect("rm");
    }
    // its notes are no code: a lesson in rust/CLAUDE.md ships from the main checkout
    std::fs::write(main.join("rust/CLAUDE.md"), "a lesson\n").expect("notes");
    let (code, said) = ship(
        dash,
        &main,
        &url,
        "a lesson",
        &["rust/CLAUDE.md", ".gitignore"],
    );
    assert_eq!(code, 0, "{said}");
    std::fs::remove_file(main.join(lib)).expect("rm");

    git(&main, &["worktree", "add", "-q", "-b", "task", "../task"]);
    let task = t.0.join("task");
    std::fs::create_dir_all(task.join("private/dashboard/lib/dashboard")).expect("lib");
    std::fs::write(task.join(lib), "defmodule Launch do end\n").expect("launch.ex");
    let (code, said) = ship(dash, &task, &url, "launch.ex in its task", &[lib]);
    assert_eq!(code, 0, "{said}");
    assert_eq!(
        git(&origin, &["log", "-1", "--format=%s", "task"]),
        "launch.ex in its task"
    );
    assert_eq!(seen.load(Ordering::SeqCst), 0, "the front door was called");
}

/// Part 3: it prints its commit, or its failure, and never « may have run partway », with the front door dropping everything.
#[dash_test]
fn a_ship_says_its_commit_or_its_failure() {
    let t = TestDir::new("ship-says");
    let (origin, main) = repo(&t);
    let (url, seen) = dropping();
    let dash = Path::new(DASH);
    std::fs::write(main.join("a.txt"), "a\n").expect("a");
    let (code, said) = ship(dash, &main, &url, "add a", &["a.txt", ".gitignore"]);
    assert_eq!(code, 0, "{said}");
    let sha = git(&origin, &["rev-parse", "main"]);
    assert!(
        said.contains(&format!("git-ship: pushed to origin/main: {sha} add a")),
        "{said}"
    );

    // a path that is no file and not tracked (a typo, a folder's relative path) is refused, never « pushed » with nothing new
    let (code, said) = ship(dash, &main, &url, "typo", &["no-such.txt"]);
    assert_eq!(code, 1, "{said}");
    assert!(
        said.contains("git-ship: no-such.txt is no file of "),
        "{said}"
    );
    assert!(!said.contains("pushed to"), "{said}");

    git(
        &main,
        &[
            "remote",
            "set-url",
            "origin",
            t.0.join("gone.git").to_str().expect("utf8"),
        ],
    );
    std::fs::write(main.join("b.txt"), "b\n").expect("b");
    let (code, said) = ship(dash, &main, &url, "add b", &["b.txt"]);
    assert_eq!(code, 1, "{said}");
    assert!(
        said.contains("git-ship: PUSH FAILED, main is not on origin, nothing pushed:"),
        "{said}"
    );
    // with git's own reason
    assert!(
        said.contains("does not appear to be a git repository"),
        "{said}"
    );
    assert!(!said.contains("may have run partway"), "{said}");

    let (code, said) = ship(dash, &main, &url, "nothing named", &[]);
    assert_eq!(code, 2, "{said}");
    assert!(
        said.contains("git-ship: name the files to commit"),
        "{said}"
    );
    // the binary itself never calls the front door: part 1's half the public mirror runs too
    assert_eq!(seen.load(Ordering::SeqCst), 0, "the front door was called");
}

/// A hook that writes its pid to $SHIP_MARK, then hangs: a ship held in its checks or its push, as under load.
const HANGS: &str = "#!/bin/sh\necho $$ > \"$SHIP_MARK\"\nexec sleep 60\n";

/// `dash ship` started with `hook` (pre-commit or pre-push) hanging, cut by SIGTERM once the hook runs: exit code, what it
/// said, and whether the hook's process outlived it.
fn cut_in(t: &TestDir, main: &Path, hook: &str, message: &str, file: &str) -> (i32, String, bool) {
    let hooks = t.0.join(format!("hooks-{hook}"));
    std::fs::create_dir_all(&hooks).expect("hooks");
    std::os::unix::fs::symlink(dash_meter::stand_in(HANGS), hooks.join(hook)).expect("hook");
    git(
        main,
        &["config", "core.hooksPath", hooks.to_str().expect("utf8")],
    );
    let mark = t.0.join(format!("mark-{hook}"));
    std::fs::write(main.join(file), "x\n").expect("file");
    let child = env(Command::new(DASH)
        .current_dir(main)
        .args(["ship", message, file]))
    .env("SHIP_MARK", &mark)
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .expect("ship");
    let start = std::time::Instant::now();
    while std::fs::read_to_string(&mark).map_or(true, |s| s.trim().is_empty()) {
        assert!(start.elapsed().as_secs() < 60, "the {hook} hook never ran");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let hung: i32 = std::fs::read_to_string(&mark)
        .expect("mark")
        .trim()
        .parse()
        .expect("pid");
    unsafe { libc::kill(i32::try_from(child.id()).expect("pid"), libc::SIGTERM) };
    let out = child.wait_with_output().expect("ship");
    let said =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    // a killed hook's zombie answers kill 0 until launchd reaps it: up to 10 s
    let alive = (0..200).all(|_| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        (unsafe { libc::kill(hung, 0) }) == 0
    });
    if alive {
        unsafe { libc::kill(hung, libc::SIGKILL) };
    }
    git(main, &["config", "--unset", "core.hooksPath"]);
    (out.status.code().unwrap_or(-1), said, alive)
}

/// Done-when 2 (task ship-outlives-curl): a ship cut mid-run says what landed and exits non-zero, its hung children stopped.
/// Cut in its checks: nothing committed, nothing pushed. Cut in its push: committed, its sha said, not on origin.
#[dash_test]
fn a_ship_cut_mid_run_says_what_landed_and_exits_non_zero() {
    let t = TestDir::new("ship-cut");
    let (origin, main) = repo(&t);
    let before = git(&main, &["rev-parse", "HEAD"]);
    let (code, said, alive) = cut_in(&t, &main, "pre-commit", "cut in its checks", "c.txt");
    assert_eq!(code, 128 + libc::SIGTERM, "{said}");
    assert!(
        said.contains("git-ship: stopped by signal 15 mid-run: nothing committed, nothing pushed"),
        "{said}"
    );
    assert!(!alive, "the checks outlived the ship: {said}");
    assert_eq!(git(&main, &["rev-parse", "HEAD"]), before);

    let (code, said, alive) = cut_in(&t, &main, "pre-push", "cut in its push", "p.txt");
    assert_eq!(code, 128 + libc::SIGTERM, "{said}");
    let sha = git(&main, &["rev-parse", "HEAD"]);
    assert!(
        said.contains(&format!(
            "git-ship: stopped by signal 15 mid-run: committed {sha} cut in its push\n\
             git-ship: stopped by signal 15 mid-run: not pushed: origin/main does not hold it"
        )),
        "{said}"
    );
    assert!(!alive, "the push outlived the ship: {said}");
    assert_eq!(git(&origin, &["rev-parse", "main"]), before);
}
