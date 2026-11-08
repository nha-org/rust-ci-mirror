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

/// The older app's CLI: it logs each call's words and prints the comments file.
fn fake_cli(dir: &Path) -> String {
    let cli = dir.join("older-dash");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{0}/calls'\ncat '{0}/comments.json'\n",
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
    assert!(line.starts_with("Woken by the dashboard's comment queue (1): 1. comment f99c7dcc on /d/hostman-grant-reply"), "{line}");
    assert!(
        line.contains("(http://100.72.158.115:7788/d/hostman-grant-reply#c-f99c7dcc)"),
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
}
