//! `dash task run --folded` as the orchestrator runs it (task rust-fold-port, done-when 3): bin/dash's post to the real front
//! door, the older app's database and task journal in a scratch checkout, and a stand-in older app that logs any call. The fold
//! is answered and journaled in Rust; the older app's `Dashboard.Orch.PM.fold/3` is never asked.
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The post bin/dash makes for `dash task run --folded <at> mechanism <where>`, headers included.
fn fold(port: u16, at: &str, place: &str) -> String {
    fold_as(port, at, "mechanism", place)
}

fn fold_as(port: u16, at: &str, kind: &str, place: &str) -> String {
    let mut curl = Command::new("curl");
    curl.args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"])
        .args(["--data-urlencode", "cwd=/tmp"]);
    for a in ["task", "run", "--folded", at, kind, place] {
        curl.args(["--data-urlencode", &format!("args[]={a}")]);
    }
    let out = curl
        .arg(format!("http://127.0.0.1:{port}/dashboard/cli"))
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_front_door_folds_a_lesson_without_the_older_app() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("folds-front-door");
    let d = dir.0.display().to_string();
    std::fs::create_dir_all(dir.0.join("private/dashboard")).expect("its folder");
    std::fs::create_dir_all(dir.0.join("private/live")).expect("live");
    let db = dash_store::Connection::open(dir.0.join("private/dashboard/dashboard_dev.db"))
        .expect("its db");
    db.execute_batch(
        "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, project TEXT, stage TEXT, epic TEXT, owner TEXT,
           data TEXT NOT NULL);
         INSERT INTO tasks VALUES ('tasks.edn', 0, 'rust-live-copy', 'site_perso', 'open', 'rust-poc', 'rust', '{}');
         -- a :duplicate-of that is no text never fails the read (/code-review of rust-fold-targets-live)
         INSERT INTO tasks VALUES ('tasks.edn', 1, 'odd-merge', 'site_perso', 'dropped', 'speed', '', '{\"duplicate-of\": 3}');
         CREATE TABLE feedback (id TEXT, at TEXT, source TEXT, ref TEXT, text TEXT, kind TEXT);",
    )
    .expect("the older app's rows");
    let at = "2026-11-10T23:46:24.422707Z";
    let journal = dir.0.join("private/live/task-journal.jsonl");
    let lesson = format!(
        "{{\"at\":\"{at}\",\"task\":\"rust-live-copy\",\"feedback\":\"The brief named no writable folder.\",\"verdict\":\"met\"}}\n"
    );
    std::fs::write(&journal, &lesson).expect("journal");
    let older = dir.0.join("older-dash");
    std::fs::write(
        &older,
        format!("#!/bin/sh\necho \"$*\" >> '{d}/older.log'\n"),
    )
    .expect("a stand-in");
    std::fs::set_permissions(&older, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = free.local_addr().expect("address").port();
    drop(free);
    let mut serve = Command::new(env!("CARGO_BIN_EXE_dash"));
    serve
        .args(["serve", "--data"])
        .arg(&dir.0)
        .args(["--port", &port.to_string()]);
    serve
        .env("DASH_CLI", &older)
        .env("HOME", &dir.0)
        .env("DASH_WORKTREES", &dir.0)
        .env("DASH_REPO", &dir.0);
    let mut serve = Killed(serve.stdout(Stdio::piped()).spawn().expect("serve"));
    let mut line = String::new();
    let out = serve.0.stdout.take().expect("stdout");
    BufReader::new(out).read_line(&mut line).expect("port line");

    // A kind it does not know: refused before any read, as the older app refused it.
    let kind = fold_as(port, at, "rule", "x");
    assert!(kind.contains("x-exit: 1\r\n"), "{kind}");
    assert!(
        kind.contains("pm: rule: a fold is mechanism or doc"),
        "{kind}"
    );

    // A name that exists nowhere: refused, nothing journaled.
    let refused = fold(port, at, "task no-such-task: it builds the check");
    assert!(refused.contains("x-exit: 1\r\n"), "{refused}");
    assert!(
        refused.contains("pm: task no-such-task: it builds the check: names no code path"),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&journal).expect("journal"), lesson);

    // His form of 2026-11-10: the task's id, a colon, a sentence holding « brief ». A Rust task no start could launch is dead
    // (task run-fields-at-add, done-when 3): refused, each missing field named, nothing journaled.
    let place = "task rust-live-copy part 4: the brief lists the session's writable folders from its profile";
    let dead = fold(port, at, place);
    assert!(dead.contains("x-exit: 1\r\n"), "{dead}");
    assert!(
        dead.contains("pm: task rust-live-copy: Rust's picker cannot start it (it has no layer, no run prompt)"),
        "{dead}"
    );
    assert_eq!(std::fs::read_to_string(&journal).expect("journal"), lesson);
    // its run prompt in the list, its layer in Rust's store: folded and journaled here
    db.execute_batch(
        "UPDATE tasks SET data = '{\"run\": {\"prompt\": \"Build it.\"}}' WHERE id = 'rust-live-copy'",
    )
    .expect("its prompt");
    (dash_store::tasks::open(&dir.0, &dir.0).expect("Rust's store"))
        .layer("rust-live-copy", "rfc:lessons")
        .expect("its layer");
    let folded = fold(port, at, place);
    assert!(folded.contains("x-exit: 0\r\n"), "{folded}");
    assert!(
        folded.ends_with(&format!("pm: folded {at} as mechanism into {place}\n")),
        "{folded}"
    );
    let text = std::fs::read_to_string(&journal).expect("journal");
    let last: serde_json::Value =
        serde_json::from_str(text.lines().last().expect("a line")).expect("json");
    assert_eq!(
        (&last["kind"], &last["of"], &last["as"], &last["into"]),
        (
            &"folded".into(),
            &at.into(),
            &"mechanism".into(),
            &place.into()
        )
    );
    // Closed: a second fold of it finds no open lesson.
    let again = fold(port, at, place);
    assert!(
        again.contains(&format!("pm: no open feedback at {at}")),
        "{again}"
    );
    // The older app was never asked to fold, nor for its commands: no part named « dash <command> ». The server's own pages
    // ask it for theirs (handoff, task ls) meanwhile.
    let asked = std::fs::read_to_string(dir.0.join("older.log")).unwrap_or_default();
    assert!(
        !asked.contains("--folded") && !asked.contains("help all"),
        "{asked}"
    );
}
