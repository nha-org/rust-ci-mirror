//! `dash task close <id>` as he runs it (rust-task-close-rust-runs): bin/dash's post to the front door, the real `dash serve` and
//! `dash task close` on a real store, a run's process holding its claim, and stand-ins for herdr and the older app. Refused while
//! its agent works, nothing touched; then its agent told to leave, its pane closed, its run's process ended, its row ended.
use dash_core::tasks::TaskStore;
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
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
    let out = Command::new("curl")
        .args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"])
        .args(["--data-urlencode", "cwd=/tmp"])
        .args([
            "--data-urlencode",
            "args[]=task",
            "--data-urlencode",
            "args[]=close",
        ])
        .args(["--data-urlencode", &format!("args[]={id}")])
        .arg(format!("http://127.0.0.1:{port}/dashboard/cli"))
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn dash_task_close_through_the_front_door_ends_a_rust_run_and_closes_its_pane() {
    let _budget = TestBudget::new();
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

    let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = free.local_addr().expect("address").port();
    drop(free);
    let mut serve = Command::new(DASH);
    serve
        .args(["serve", "--data"])
        .arg(&dir.0)
        .args(["--port", &port.to_string()]);
    serve
        .env("HERDR_BIN", &herdr)
        .env("DASH_CLI", &older)
        .env("HOME", &dir.0);
    serve.env("DASH_WORKTREES", &dir.0).env("DASH_REPO", &dir.0);
    let mut serve = Killed(serve.stdout(Stdio::piped()).spawn().expect("serve"));
    let mut line = String::new();
    let out = serve.0.stdout.take().expect("stdout");
    BufReader::new(out).read_line(&mut line).expect("port line");

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
