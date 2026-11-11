//! The comment queue through the real binary: `dash comment pass` reads the older app's comments through its CLI, lands
//! them as rows, wakes the owner session's wake file once the quiet minute is over, and asks the older app for nothing else:
//! no task is made of a comment.
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use dash_store::oplog::Replica;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

/// The older app's CLI: it logs each call's words and prints the comments file; its `watchers` and `watch wakes` answer as
/// the old app's did on 2026-11-09, one row each.
fn fake_cli(dir: &Path) -> String {
    let cli = dir.join("older-dash");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{0}/calls'\ncase \"$1\" in\n\
         watchers) printf 'Watchers: operational\\n  ok mail poll: beats every 5 min; 0 new mail(s) (2026-11-09 00:11 UTC)\\n' ;;\n\
         watch) echo '2026-11-09 3 wakes (the dashboard 3)' ;;\n\
         *) cat '{0}/comments.json' ;;\nesac\n",
        dir.display()
    );
    std::fs::write(&cli, script).expect("the fake CLI");
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    cli.display().to_string()
}

fn pass(dir: &TestDir, cli: &str) -> String {
    let output = Command::new(DASH)
        .args(["comment", "pass", "--data"])
        .arg(dir.0.join("data"))
        .env("DASH_CLI", cli)
        .env("DASH_REPO", dir.0.join("repo"))
        .output()
        .expect("dash");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("utf-8")
}

#[test]
fn a_pass_lands_his_comment_wakes_the_owner_and_makes_no_task() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("comment-queue");
    let run = dir.0.join("repo/.claude/run");
    std::fs::create_dir_all(&run).expect("run folder");
    std::fs::create_dir_all(dir.0.join("data")).expect("data folder");
    // The owner is this test's own process: alive while it runs.
    let pid = std::process::id();
    std::fs::write(run.join("owner"), format!("{pid}\n")).expect("owner");
    std::fs::write(run.join(format!("{pid}.state")), "").expect("state");
    let comments = r#"[{"id": "f99c7dcc", "file": "/d/hostman-grant-reply", "quote": "6. Call:", "text": "not before lunch",
        "at": "2026-11-08T09:51:35Z", "status": "~:open", "replies": []}]"#;
    std::fs::write(dir.0.join("comments.json"), comments).expect("comments");
    let cli = fake_cli(&dir.0);

    assert_eq!(pass(&dir, &cli), "1 open, 1 written\n");
    let wake = run.join(format!("{pid}.wake"));
    assert!(!wake.exists(), "the quiet minute first");
    let calls = std::fs::read_to_string(dir.0.join("calls")).expect("calls");
    assert_eq!(
        calls, "store text live/comments.edn\n",
        "a read, never a task"
    );

    assert_eq!(pass(&dir, &cli), "1 open, 0 written\n", "inside the minute");
    assert!(!wake.exists());

    // The quiet minute over, with no wait: the row's landing moved back two minutes in its table.
    {
        let db = dash_store::connect(&dir.0.join("data/comments.sqlite")).expect("the table");
        let mut log = Replica::new(db, "test").expect("its log");
        let kept = dash_comments::load(&log).expect("loads");
        let mut rows = kept.clone();
        rows.values_mut().for_each(|r| r.landed_ms -= 2 * 60_000);
        dash_comments::save(&mut log, &kept, &rows).expect("saves");
    }
    let said = pass(&dir, &cli);
    assert!(
        said.starts_with("1 open, 1 written, woke ") && said.ends_with(" with f99c7dcc\n"),
        "{said}"
    );
    let line = std::fs::read_to_string(&wake).expect("the wake");
    assert!(line.starts_with("Woken by the dashboard's comment queue (1): 1. comment f99c7dcc on /drafts/hostman-grant-reply"), "{line}");
    assert!(
        line.contains("(http://100.72.158.115:7788/drafts/hostman-grant-reply#c-f99c7dcc)"),
        "{line}"
    );
    assert_eq!(pass(&dir, &cli), "1 open, 0 written\n", "woken once");

    // An answer in the older app closes the row on the next pass.
    let answered = comments.replace(
        r#""replies": []"#,
        r#""replies": [{"by": "~:claude", "text": "Done"}]"#,
    );
    std::fs::write(dir.0.join("comments.json"), answered).expect("answered");
    assert_eq!(pass(&dir, &cli), "0 open, 1 written\n");
    let calls = std::fs::read_to_string(dir.0.join("calls")).expect("calls");
    assert!(
        calls.lines().all(|l| l == "store text live/comments.edn"),
        "{calls}"
    );

    // The comment watcher is Rust's on /watchers: its last pass the beat, its one wake what it woke (task rust-watchers-port).
    let output = Command::new(DASH)
        .args(["watchers", "--data"])
        .arg(dir.0.join("data"))
        .env("DASH_CLI", &cli)
        .env("DASH_REPO", dir.0.join("repo"))
        .output()
        .expect("dash");
    assert!(output.status.success(), "{output:?}");
    let page = String::from_utf8(output.stdout).expect("utf-8");
    let queue = page.split("comment queue").nth(1).expect("its row");
    assert!(queue.contains("Served by: Rust"), "{page}");
    assert!(
        queue.contains(" UTC: 0 open, 1 written"),
        "the last pass: {page}"
    );
    let woke = format!(" UTC: the orchestrator ({}) with f99c7dcc", wake.display());
    assert!(queue.contains(&woke), "{page}");
    // the old app's rows through its CLI's words, as the server asks them
    let mail = page
        .split("\nmail poll\n")
        .nth(1)
        .expect("the old app's row");
    assert!(mail.contains("Served by: the old app"), "{page}");
    assert!(
        mail.contains("Last run: ok, 2026-11-09 00:11 UTC: beats every 5 min"),
        "{page}"
    );
    assert!(
        page.contains("Woke: the orchestrator, by day: 2026-11-09 3 wakes (the dashboard 3)"),
        "{page}"
    );
    assert!(
        // Rust's count follows each watcher ported (watchers::RUST): only the old app's is this test's
        page.contains(" 2 watchers by the old app,"),
        "{page}"
    );
    let calls = std::fs::read_to_string(dir.0.join("calls")).expect("calls");
    assert!(
        calls.contains("\nwatchers\n") && calls.contains("\nwatch wakes 1\n"),
        "{calls}"
    );
}

/// rust/CLAUDE.md rule 5, a watcher that stops says so where it is read (Elixir's wake-wait was down 5 hours, 2026-11-08,
/// and said so nowhere): the comment queue stopped an hour reads LATE on /watchers, and its first wake back names it.
#[test]
fn a_stopped_watcher_shows_on_the_watchers_page_and_in_the_next_wake() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("comment-queue-stopped");
    let (run, data) = (dir.0.join("repo/.claude/run"), dir.0.join("data"));
    std::fs::create_dir_all(&run).expect("run folder");
    std::fs::create_dir_all(&data).expect("data folder");
    let pid = std::process::id();
    std::fs::write(run.join("owner"), format!("{pid}\n")).expect("owner");
    std::fs::write(run.join(format!("{pid}.state")), "").expect("state");
    let comments = r#"[{"id": "a1b2c3d4", "file": "/tasks", "quote": "", "text": "why is this red",
        "at": "2026-11-09T09:51:35Z", "status": "~:open", "replies": []}]"#;
    std::fs::write(dir.0.join("comments.json"), comments).expect("comments");
    let cli = fake_cli(&dir.0);
    assert_eq!(pass(&dir, &cli), "1 open, 1 written\n");

    // Stopped an hour: its last beat and the comment's landing an hour back, so the comment is due on its return.
    let hour = 3_600_000;
    let beats = dash_store::connect(&data.join("watchers.sqlite")).expect("the beats");
    let aged = beats
        .execute(
            "UPDATE beat SET at_ms = at_ms - ?1 WHERE name = 'comment queue'",
            [hour],
        )
        .expect("aged");
    assert_eq!(aged, 1, "the queue's beat");
    {
        let db = dash_store::connect(&data.join("comments.sqlite")).expect("the table");
        let mut log = Replica::new(db, "test").expect("its log");
        let kept = dash_comments::load(&log).expect("loads");
        let mut rows = kept.clone();
        rows.values_mut().for_each(|r| r.landed_ms -= hour);
        dash_comments::save(&mut log, &kept, &rows).expect("saves");
    }

    let output = Command::new(DASH)
        .args(["watchers", "--data"])
        .arg(&data)
        .env("DASH_CLI", &cli)
        .env("DASH_REPO", dir.0.join("repo"))
        .output()
        .expect("dash");
    let page = String::from_utf8(output.stdout).expect("utf-8");
    let queue = page.split("comment queue").nth(1).expect("its row");
    assert!(queue.contains("Last run: LATE, "), "{page}");

    let said = pass(&dir, &cli);
    assert!(said.contains(", woke "), "{said}");
    let wake = std::fs::read_to_string(run.join(format!("{pid}.wake"))).expect("the wake");
    assert!(
        wake.contains(" Watchers that stopped: comment queue: LATE, "),
        "{wake}"
    );

    // Back and beating, the next wake names no watcher.
    std::fs::remove_file(run.join(format!("{pid}.wake"))).expect("read");
    {
        let db = dash_store::connect(&data.join("comments.sqlite")).expect("the table");
        let mut log = Replica::new(db, "test").expect("its log");
        let kept = dash_comments::load(&log).expect("loads");
        let mut rows = kept.clone();
        rows.values_mut()
            .for_each(|r| r.told_ms = r.told_ms.map(|t| t - hour));
        dash_comments::save(&mut log, &kept, &rows).expect("saves");
    }
    assert!(pass(&dir, &cli).contains(", woke "), "woken again");
    let wake = std::fs::read_to_string(run.join(format!("{pid}.wake"))).expect("the wake");
    assert!(!wake.contains("Watchers that stopped"), "{wake}");
}
