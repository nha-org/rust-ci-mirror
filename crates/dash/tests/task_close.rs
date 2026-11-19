//! `dash task close <id>` as he runs it (rust-task-close-rust-runs): bin/dash's post to the front door, the real `dash serve` and
//! `dash task close` on a real store, a run's process holding its claim, and stand-ins for herdr and the older app. Refused while
//! its agent works, nothing touched; then its agent told to leave, its pane closed, its run's process ended, its row ended.
use dash_core::tasks::TaskStore;
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn script(path: &Path, text: &str) {
    std::fs::write(path, text).expect("a stand-in");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The post bin/dash makes for `dash task close <id>`: its words, its folder, its header.
fn close(port: u16, id: &str) -> String {
    close_with(port, &[id])
}

/// The same with `words` after `task close`: the id and a flag.
fn close_with(port: u16, words: &[&str]) -> String {
    let words = words
        .iter()
        .flat_map(|w| ["--data-urlencode".to_string(), format!("args[]={w}")]);
    let out = Command::new("curl")
        .args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"])
        .args(["--data-urlencode", "cwd=/tmp"])
        .args([
            "--data-urlencode",
            "args[]=task",
            "--data-urlencode",
            "args[]=close",
        ])
        .args(words)
        .arg(format!("http://127.0.0.1:{port}/dashboard/cli"))
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The real `dash serve` on `dir`, herdr and the older app its stand-ins: it and its port.
fn serve(dir: &Path, herdr: &Path, older: &Path) -> (Killed, u16) {
    let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = free.local_addr().expect("address").port();
    drop(free);
    let mut serve = Command::new(DASH);
    serve
        .args(["serve", "--data"])
        .arg(dir)
        .args(["--port", &port.to_string()]);
    serve
        .env("HERDR_BIN", herdr)
        .env("DASH_CLI", older)
        .env("HOME", dir);
    serve.env("DASH_WORKTREES", dir).env("DASH_REPO", dir);
    let mut serve = Killed(serve.stdout(Stdio::piped()).spawn().expect("serve"));
    let mut line = String::new();
    let out = serve.0.stdout.take().expect("stdout");
    BufReader::new(out).read_line(&mut line).expect("port line");
    (serve, port)
}

/// git in `dir`, its failure the test's.
fn git(dir: &Path, args: &[&str]) {
    let ran = Command::new("git").arg("-C").arg(dir).args(args).status();
    assert!(ran.expect("git").success(), "git {args:?}");
}

/// Task pane-lookup-one, 2026-11-18's case: no run holds rust-held-push-release-port or push-merged-pr, yet herdr lists w318:p1
/// in `rust-rust-held-push-release-port` and w32B:p1 in `rust-push-merged-pr`, panes their runs' ends left open. The no-pkill
/// hook named each pane's task; dash task close answered « no pane for <id>: nothing runs it ». Now the close finds each by the
/// one reader under this repository's worktrees (DASH_WORKTREES): a claim of the older app naming the pane leaves it to the
/// older app, one naming none refuses as still starting; refused while its agent works or its real git worktree holds work no
/// remote has; else its agent leaves and its pane closes, a shell beside it and another repository's `task-<id>` untouched, and
/// its row ends with `why`, or stays with `--panes-only`. `rust-` is never part of an id.
#[dash_test]
fn dash_task_close_finds_the_pane_the_hook_names_in_the_task_s_worktree() {
    let dir = TestDir::new("task-close-unclaimed");
    let d = dir.0.to_string_lossy().into_owned();
    let klar = dir
        .0
        .parent()
        .expect("its parent")
        .join("klar/task-push-merged-pr");
    let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("store");
    for id in ["push-merged-pr", "rust-held-push-release-port"] {
        tasks.add(id, &[]).expect("a row");
    }
    drop(tasks);
    // push-merged-pr's worktree, a real git one: its commit on a remote branch, so only what is changed is unsaved
    let tree = dir.0.join("rust-push-merged-pr");
    std::fs::create_dir_all(&tree).expect("its worktree");
    git(&tree, &["init", "-q"]);
    git(
        &tree,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    );
    git(&tree, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    // herdr: two agents in the tasks' worktrees, a shell beside one, another repository's task-push-merged-pr, the main
    // checkout's; each agent idle unless `working`, its box holding the words once pasted, gone once they are submitted with
    // Enter, as Claude's input box behaves (task close-exit-enter). Every call logged.
    let stand_in = r#"#!/bin/sh
d='@D'
echo "$*" >> "$d/herdr.log"
s=idle; [ -f "$d/working" ] && s=working
a318='"agent":"claude","agent_status":"idle",'; [ -f "$d/left-w318" ] && a318=''
a32B="\"agent\":\"claude\",\"agent_status\":\"$s\","; [ -f "$d/left-w32B" ] && a32B=''
case "$1 $2" in
'pane list') printf '{"result":{"panes":[{"pane_id":"w318:p1",%s"cwd":"@D/rust-rust-held-push-release-port"},{"pane_id":"w32B:p1",%s"cwd":"@D/rust-push-merged-pr"},{"pane_id":"w32B:p2","agent":null,"cwd":"@D/rust-push-merged-pr/rust"},{"pane_id":"w40:p1","agent":"claude","agent_status":"idle","cwd":"@K"},{"pane_id":"w8:pJN","cwd":"@D/repo/site_perso"}]}}\n' "$a318" "$a32B";;
'agent list') a=''; [ -f "$d/left-w318" ] || a='{"pane_id":"w318:p1","agent":"claude","agent_status":"idle"}'
  [ -f "$d/left-w32B" ] || a="$a${a:+,}{\"pane_id\":\"w32B:p1\",\"agent\":\"claude\",\"agent_status\":\"$s\"}"
  printf '{"result":{"agents":[%s]}}\n' "$a";;
'pane send-text') touch "$d/pasted-${3%%:*}";;
'pane read') [ -f "$d/pasted-${3%%:*}" ] && printf '│ > /exit    │\n';;
'pane send-keys') [ -f "$d/pasted-${3%%:*}" ] && [ "$4" = Enter ] && touch "$d/left-${3%%:*}";;
esac
"#;
    let herdr = dir.0.join("herdr");
    let klar_cwd = klar.to_string_lossy();
    script(&herdr, &stand_in.replace("@D", &d).replace("@K", &klar_cwd));
    let older = dir.0.join("older-dash");
    script(
        &older,
        &format!("#!/bin/sh\necho \"$*\" >> '{d}/older.log'\n"),
    );
    let (_serve, port) = serve(&dir.0, &herdr, &older);
    let closes = || {
        let log = read(&dir.0.join("herdr.log"));
        let closed = log
            .lines()
            .filter(|l| l.starts_with("pane close") || l.starts_with("pane send"));
        closed.map(String::from).collect::<Vec<_>>()
    };

    // A pane the older app's own claim names stays its to close, with its claim (/code-review of 482929e62): Rust closes nothing.
    let db = dir.0.join("private/dashboard/dashboard_dev.db");
    std::fs::create_dir_all(db.parent().expect("its folder")).expect("folder");
    let sql = |sql: &str| {
        let ran = Command::new("sqlite3").arg(&db).arg(sql).status();
        assert!(ran.expect("sqlite3").success(), "{sql}");
    };
    sql(
        "CREATE TABLE task_claims (task TEXT PRIMARY KEY, project TEXT NOT NULL, pane TEXT, session TEXT, request TEXT, \
         via TEXT, at TEXT); INSERT INTO task_claims VALUES ('push-merged-pr', 'site_perso', 'w32B:p1', NULL, NULL, 'x', 'y')",
    );
    let answer = close(port, "push-merged-pr");
    assert!(!answer.contains("no run holds it"), "{answer}");
    // Its claim names no pane, as a launch in flight: refused as still starting, as Rust's own close refuses one.
    sql("UPDATE task_claims SET pane = NULL");
    let answer = close(port, "push-merged-pr");
    assert!(
        answer.contains("refused, the older app's claim opens no pane yet: still starting"),
        "{answer}"
    );
    sql("DELETE FROM task_claims");

    std::fs::write(dir.0.join("working"), "").expect("working");
    let answer = close(port, "push-merged-pr");
    assert!(answer.contains("x-exit: 1\r\n"), "{answer}");
    assert!(
        answer.contains("push-merged-pr: refused, its agent works in pane w32B:p1"),
        "{answer}"
    );
    std::fs::remove_file(dir.0.join("working")).expect("idle");
    std::fs::write(tree.join("draft.rs"), "").expect("a change");
    let answer = close(port, "push-merged-pr");
    assert!(
        answer.contains("holds 1 files changed and not committed: push it first"),
        "{answer}"
    );
    assert!(
        closes().is_empty(),
        "a refusal sends and closes nothing: {:?}",
        closes()
    );
    std::fs::remove_file(tree.join("draft.rs")).expect("saved");

    let answer = close(port, "push-merged-pr");
    assert!(answer.contains("x-exit: 0\r\n"), "{answer}");
    assert!(answer.contains("pane w32B:p1 in its worktree"), "{answer}");
    assert!(
        answer.contains("pane w32B:p2: a shell, left open"),
        "{answer}"
    );
    assert!(
        answer.contains("closed push-merged-pr"),
        "its row too: {answer}"
    );
    // --panes-only: its pane, its row left open
    let answer = close_with(port, &["rust-held-push-release-port", "--panes-only"]);
    assert!(answer.contains("x-exit: 0\r\n"), "{answer}");
    assert!(answer.contains("pane w318:p1 in its worktree"), "{answer}");
    assert_eq!(
        closes()
            .iter()
            .filter(|l| l.starts_with("pane close"))
            .collect::<Vec<_>>(),
        ["pane close w32B:p1", "pane close w318:p1"],
        "the shell and klar's pane stay open"
    );
    let tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("store");
    assert_eq!(
        TaskStore::status(&tasks, "push-merged-pr"),
        Ok(Some("finished".into()))
    );
    let shown = tasks
        .show("push-merged-pr")
        .expect("show")
        .expect("its row");
    assert!(
        shown.contains("asked by dash task close"),
        "its why: {shown}"
    );
    assert_eq!(
        TaskStore::status(&tasks, "rust-held-push-release-port"),
        Ok(Some("open".into()))
    );
    assert!(!read(&dir.0.join("older.log")).contains("close"));

    // The folder's `rust-` is no part of the id: no pane runs `rust-rust-held-push-release-port`, so Rust closes nothing and the
    // older app is the one asked, as before (none here: refused).
    let answer = close(port, "rust-rust-held-push-release-port");
    assert!(!answer.contains("closed"), "{answer}");
}

#[dash_test]
fn dash_task_close_through_the_front_door_ends_a_rust_run_and_closes_its_pane() {
    let dir = TestDir::new("task-close-front-door");
    let d = dir.0.to_string_lossy().into_owned();

    // A run's claim on the real store: its pane p1, its session s1.
    let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("store");
    tasks.claim("t1").expect("claim");
    tasks.opened("t1", "p1").expect("opened");
    tasks.adopted("t1", "s1").expect("adopted");
    drop(tasks);
    // Its run, another process leading its group as `dash orch` starts one: its claim lock held, its pid written.
    let hold = "import fcntl,os,sys,time\nf=open(sys.argv[1],'r+')\nfcntl.flock(f,fcntl.LOCK_EX)\n\
                f.truncate(0)\nf.write(str(os.getpid()))\nf.flush()\nprint('held',flush=True)\ntime.sleep(120)";
    let mut run = Command::new("python3");
    run.args(["-I", "-c", hold]).arg(dir.0.join("rust-t1.lock"));
    let mut run = Killed(
        run.stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .expect("python3"),
    );
    let mut held = String::new();
    let out = run.0.stdout.take().expect("stdout");
    BufReader::new(out).read_line(&mut held).expect("held");
    assert_eq!(held.trim(), "held");

    // herdr: p1 lists s1, idle unless `working` exists, gone once told to leave. Every call logged.
    let herdr = dir.0.join("herdr");
    script(
        &herdr,
        &format!(
            "#!/bin/sh\nd='{d}'\necho \"$*\" >> \"$d/herdr.log\"\ncase \"$1 $2\" in\n\
             'agent list') if [ -f \"$d/left\" ]; then echo '{{\"result\":{{\"agents\":[]}}}}'; else s=idle; \
             [ -f \"$d/working\" ] && s=working; printf '{{\"result\":{{\"agents\":[{{\"pane_id\":\"p1\",\"agent\":\"claude\",\
             \"agent_status\":\"%s\",\"agent_session\":{{\"value\":\"s1\"}}}}]}}}}\\n' \"$s\"; fi;;\n\
             'pane send-text') touch \"$d/left\";;\nesac\n"
        ),
    );
    // The older app: its calls logged, nothing answered (its kind reads as claude).
    let older = dir.0.join("older-dash");
    script(
        &older,
        &format!("#!/bin/sh\necho \"$*\" >> '{d}/older.log'\n"),
    );

    let (_serve, port) = serve(&dir.0, &herdr, &older);

    // At work: refused, its run and pane as they were.
    std::fs::write(dir.0.join("working"), "").expect("working");
    let answer = close(port, "t1");
    assert!(answer.contains("x-exit: 1\r\n"), "{answer}");
    assert!(
        answer.contains("refused: t1: refused, its agent works in pane p1"),
        "{answer}"
    );
    assert!(
        run.0.try_wait().expect("look").is_none(),
        "its run still lives"
    );
    assert!(!read(&dir.0.join("herdr.log")).contains("pane close"));

    // Idle: its run ended, its agent told to leave, its pane closed, the list told.
    std::fs::remove_file(dir.0.join("working")).expect("idle");
    let answer = close(port, "t1");
    assert!(answer.contains("x-exit: 0\r\n"), "{answer}");
    assert!(
        answer.ends_with("t1: its run ended, its pane p1 closed\n"),
        "{answer}"
    );
    let status = run.0.wait().expect("its run ended");
    assert!(status.code().is_none(), "ended by a signal: {status:?}");
    let herdr_log = read(&dir.0.join("herdr.log"));
    assert!(
        herdr_log.contains("pane send-text p1 \u{1b}[200~/exit"),
        "{herdr_log}"
    );
    assert!(herdr_log.contains("pane close p1"), "{herdr_log}");
    let older_log = read(&dir.0.join("older.log"));
    assert!(
        older_log.contains("task ended t1 closed by dash task close: asked by dash task close"),
        "{older_log}"
    );
    let tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("store");
    assert_eq!(TaskStore::status(&tasks, "t1"), Ok(Some("finished".into())));
    let shown = tasks.show("t1").expect("show").expect("its row");
    assert!(shown.contains("ended"), "{shown}");

    // Ended: no run of Rust's holds it, so the older app is asked as before (none here: refused).
    let answer = close(port, "t1");
    assert!(!answer.contains("its run ended"), "{answer}");
}
