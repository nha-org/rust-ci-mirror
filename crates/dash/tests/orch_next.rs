//! `dash orch next` as the orchestrator runs it, on a real store holding PRs (task rust-pr-one-owner, done-when 2): the real binary,
//! stand-ins for herdr (no session) and the older app (no list), so the pick reads only the store.
use dash_core::tasks::{ORCHESTRATOR, TaskStore};
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

fn script(path: &Path, text: &str) {
    std::fs::write(path, text).expect("a stand-in");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// Done-when 2: `dash orch next` names each held PR and its holder, the orchestrator's and a run's, and none released.
#[dash_test]
fn dash_orch_next_names_each_held_pr_and_its_holder() {
    let dir = TestDir::new("orch-next-holds");
    {
        let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("the store");
        // a sync's seats: the list, as the older app would file it, holds nothing to pick
        let list = dash_core::tasks::Queue {
            seats: 3,
            seats_why: String::new(),
            tasks: vec![],
        };
        tasks.sync(&list).expect("a sync");
        tasks.claim("t1").expect("a run at work");
        let urgent = "an urgent fix";
        TaskStore::hold_pr(&mut tasks, 1400, ORCHESTRATOR, urgent).expect("held");
        TaskStore::hold_pr(&mut tasks, 1401, "run t1", "its run works on it").expect("held");
        TaskStore::hold_pr(&mut tasks, 1402, ORCHESTRATOR, "dash land").expect("held");
        TaskStore::unhold(&mut tasks, 1402).expect("released");
    }
    let herdr = dir.0.join("herdr");
    script(&herdr, "#!/bin/sh\necho '{\"result\":{\"agents\":[]}}'\n");
    let older = dir.0.join("older-dash");
    script(&older, "#!/bin/sh\nexit 0\n");
    let ran = Command::new(DASH)
        .args(["orch", "next", "--data"])
        .arg(&dir.0)
        .env("HERDR_BIN", &herdr)
        .env("DASH_CLI", &older)
        .env("HOME", &dir.0)
        .env("DASH_REPO", &dir.0)
        .env("DASH_WORKTREES", &dir.0)
        .output()
        .expect("dash orch next");
    let out = String::from_utf8_lossy(&ran.stdout);
    let err = String::from_utf8_lossy(&ran.stderr);
    assert!(ran.status.success(), "{out}{err}");
    let held = out.split_once("Held PRs:\n").expect("its held PRs").1;
    let lines: Vec<&str> = held.lines().take(2).collect();
    assert!(lines[0].starts_with("#1400\torchestrator, since "), "{out}");
    assert!(lines[0].ends_with(": an urgent fix"), "{out}");
    assert!(lines[1].starts_with("#1401\trun t1, since "), "{out}");
    assert!(lines[1].ends_with(": its run works on it"), "{out}");
    assert!(!out.contains("#1402"), "released: {out}");
    println!("{out}");
}
