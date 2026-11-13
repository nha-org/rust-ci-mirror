//! The comment queue through the real binary: `dash comment pass` reads the older app's comments through its CLI, lands
//! them as rows, wakes the owner session's wake file once the quiet minute is over, and asks the older app for nothing else:
//! no task is made of a comment.
use dash_meter::TestDir;
use dash_meter::dash_test;
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

#[dash_test]
fn a_pass_lands_his_comment_wakes_the_owner_and_makes_no_task() {
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
        // his inbox's check pass rides the beat, after the comments, and reads the older app's /inbox (task rust-inbox-triage)
        calls,
        "store text live/comments.edn\nhandoff\n",
        "reads, never a task"
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
        calls
            .lines()
            .all(|l| l == "store text live/comments.edn" || l == "handoff"),
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
#[dash_test]
fn a_stopped_watcher_shows_on_the_watchers_page_and_in_the_next_wake() {
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

/// `dash hook chat` as Claude Code runs it on his message: its JSON on stdin, what it prints going into the turn.
fn hook_chat(dir: &TestDir, prompt: &str) -> String {
    use std::io::Write;
    let mut child = Command::new(DASH)
        .args(["hook", "chat", "--data"])
        .arg(dir.0.join("data"))
        .env("DASH_REPO", dir.0.join("repo"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("dash");
    let input = serde_json::json!({"session_id": "s1", "hook_event_name": "UserPromptSubmit", "prompt": prompt});
    (child.stdin.take().expect("stdin"))
        .write_all(input.to_string().as_bytes())
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("utf-8")
}

fn answer(dir: &TestDir, id: &str, words: &str) -> std::process::Output {
    (Command::new(DASH).args(["comment", "answer", "--data"]))
        .arg(dir.0.join("data"))
        .args([id, words])
        .env("DASH_REPO", dir.0.join("repo"))
        .output()
        .expect("dash")
}

/// The open question's id, as the hook says it: « - chat-…: ».
fn id_of(said: &str, question: &str) -> String {
    let line = (said.lines())
        .find(|l| l.contains(question))
        .unwrap_or_else(|| panic!("{question}: {said}"));
    line.trim_start_matches("- ")
        .split(' ')
        .next()
        .expect("an id")
        .into()
}

/// Done-when 1 and 2 of rust-chat-questions-tracked: his two questions land as two chat rows, the hook heads every turn with
/// them, a comment pass (whose export never holds them) closes neither, and only `dash comment answer` closes one.
#[dash_test]
fn his_chat_questions_head_every_turn_until_dash_comment_answer_closes_each() {
    let dir = TestDir::new("chat-questions");
    std::fs::create_dir_all(dir.0.join("repo/.claude/run")).expect("run folder");
    std::fs::create_dir_all(dir.0.join("data")).expect("data folder");

    let said = hook_chat(
        &dir,
        "You asked me to check some meeting. Where is the link? Also see http://x/projects?issue=a and tell me: did it go out ?",
    );
    assert!(
        said.starts_with("His questions still open (2), oldest first."),
        "{said}"
    );
    let (link, out) = (
        id_of(&said, "« Where is the link? »"),
        id_of(&said, "tell me: did it go out ? »"),
    );
    assert!(
        link.starts_with("chat-") && out.starts_with("chat-") && link != out,
        "{said}"
    );

    // a pass reads the older app's comments, where no chat row is: both stay open
    std::fs::write(dir.0.join("comments.json"), "[]").expect("comments");
    let cli = fake_cli(&dir.0);
    assert_eq!(pass(&dir, &cli), "0 open, 0 written\n");

    // the next turn, a message with no question, then an agent's signed one: both still head it, and no third lands
    let said = hook_chat(&dir, "ok. carry on");
    assert!(said.contains(&link) && said.contains(&out), "{said}");
    let said = hook_chat(
        &dir,
        "From the task review-x: PR 1470 is green, merge it now?",
    );
    assert!(said.starts_with("His questions still open (2)"), "{said}");

    let done = answer(&dir, &link, "The link is http://100.72.158.115:7788/inbox");
    assert!(done.status.success(), "{done:?}");
    assert_eq!(
        String::from_utf8_lossy(&done.stdout),
        format!("{link}: answered, closed, on /inbox\n")
    );
    let said = hook_chat(&dir, "thanks");
    assert!(
        said.starts_with("His questions still open (1)")
            && !said.contains(&link)
            && said.contains(&out),
        "{said}"
    );
    // its words as several arguments, as the older app's answer takes them
    let several = (Command::new(DASH).args(["comment", "answer", "--data"]))
        .arg(dir.0.join("data"))
        .args([out.as_str(), "Yes,", "sent", "at", "10:00."])
        .env("DASH_REPO", dir.0.join("repo"))
        .output()
        .expect("dash");
    assert!(several.status.success(), "{several:?}");
    assert_eq!(hook_chat(&dir, "good"), "");

    // no other command closes it: an unknown id or empty words are refused
    let none = answer(&dir, "chat-nope", "x");
    assert!(
        !none.status.success()
            && String::from_utf8_lossy(&none.stderr).contains("no comment chat-nope")
    );
    assert!(!answer(&dir, &out, "  ").status.success());
}
