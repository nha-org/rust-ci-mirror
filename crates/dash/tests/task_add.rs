//! `dash task add|waits|close` end on their verdict, the real binary on a fresh store: an agent reads the last line, and the
//! old app's similar commits after « added » read as a refusal (task rust-task-add-verdict-last).
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::path::Path;
use std::process::{Command, Output};

fn task(dir: &Path, args: &[&str]) -> Output {
    let (verb, rest) = args.split_first().expect("a verb");
    Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["task", verb, "--data"])
        .arg(dir)
        .args(rest)
        // Nothing here may reach his checkout or app.
        .env("DASH_WORKTREES", dir)
        .env("DASH_REPO", dir)
        .env("DASH_CLI", "/nonexistent/dash")
        .output()
        .expect("dash task")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn last_line(out: &Output) -> String {
    stdout(out).lines().last().unwrap_or_default().to_string()
}

#[test]
fn each_task_write_prints_its_verdict_last() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("task-add-verdict");

    let first = task(&dir.0, &["add", "a-task"]);
    assert!(first.status.success(), "{first:?}");
    assert_eq!(last_line(&first), "added a-task", "{first:?}");
    assert!(
        stdout(&first).contains("status: open\n"),
        "the row sits above the verdict: {first:?}"
    );

    let again = task(&dir.0, &["add", "a-task"]);
    assert_eq!(again.status.code(), Some(1), "{again:?}");
    let line = last_line(&again);
    assert!(
        line.starts_with("refused: a-task: refused, it exists"),
        "{line}"
    );

    let bad = task(&dir.0, &["add", "not an id"]);
    assert_eq!(bad.status.code(), Some(1), "{bad:?}");
    assert_eq!(last_line(&bad), "refused: not an id: not a task id");

    let waits = task(&dir.0, &["waits", "a-task", "--on", ""]);
    assert!(waits.status.success(), "{waits:?}");
    assert_eq!(last_line(&waits), "waits set a-task", "{waits:?}");
    let none = task(&dir.0, &["waits", "no-task", "--on", ""]);
    assert_eq!(none.status.code(), Some(1), "{none:?}");
    assert_eq!(last_line(&none), "refused: no-task: refused, no such task");

    let closed = task(&dir.0, &["close", "a-task", "not needed"]);
    assert!(closed.status.success(), "{closed:?}");
    assert_eq!(last_line(&closed), "closed a-task", "{closed:?}");
}
