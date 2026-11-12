//! `dash hook <guard>...` as .claude/settings.json runs it: Claude Code's PreToolUse JSON on stdin, a refusal answered as its
//! deny on stdout with exit 0, anything else silent (task rust-claude-hooks-port). The guards' cases are dash_gate::guard's.
use dash_meter::memory::TestBudget;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn hook(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
        .arg("hook")
        .args(args)
        .env("CLAUDE_PROJECT_DIR", env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("dash");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write");
    child.wait_with_output().expect("wait")
}

fn bash(command: &str) -> String {
    serde_json::json!({"session_id": "s1", "cwd": "/tmp", "hook_event_name": "PreToolUse", "tool_name": "Bash",
                       "tool_input": {"command": command}})
    .to_string()
}

const GUARDS: [&str; 3] = ["no-pkill", "no-verify-commit", "cargo-test"];

#[test]
fn a_refused_call_is_denied_with_the_guards_message_on_exit_0() {
    let _budget = TestBudget::new();
    let out = hook(&GUARDS, &bash("git commit --no-verify -m x"));
    assert_eq!(out.status.code(), Some(0));
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let h = &said["hookSpecificOutput"];
    assert_eq!(h["permissionDecision"], "deny");
    let why = h["permissionDecisionReason"].as_str().unwrap_or_default();
    assert!(
        why.starts_with("Refused by dash hook no-verify-commit: git commit --no-verify"),
        "{why}"
    );
    let out = hook(&GUARDS, &bash("pkill -f foo"));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Refused by dash hook no-pkill"));
}

#[test]
fn a_plain_call_and_input_that_is_not_json_pass_silently() {
    let _budget = TestBudget::new();
    for input in [bash("git status --short"), "not json".into(), String::new()] {
        let out = hook(&GUARDS, &input);
        assert_eq!(out.status.code(), Some(0), "{input}");
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{input}");
    }
}

#[test]
fn an_unknown_guard_is_an_error_claude_code_lets_through() {
    let _budget = TestBudget::new();
    let out = hook(&["no-such"], &bash("pkill -f foo"));
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no guard named no-such"));
    // settings.json newer than the live dash: the guards it knows still refuse (/code-review of 79ddd91c3)
    let out = hook(&["no-such", "no-pkill"], &bash("pkill -f foo"));
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Refused by dash hook no-pkill"));
}
