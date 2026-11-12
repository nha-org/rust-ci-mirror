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

/// `dash hook outbound-gate` (task rust-hooks-rest-port): a Gmail send is denied with no app or Jev, input that is no JSON
/// is denied too (the send gates fail closed), a read passes silently.
#[test]
fn the_outbound_gate_denies_a_gmail_send_and_input_it_cannot_read() {
    let _budget = TestBudget::new();
    let send = serde_json::json!({"tool_name": "mcp__claude_ai_Gmail__send_message", "tool_input": {"body": "x"}});
    let out = hook(&["outbound-gate"], &send.to_string());
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert_eq!(said["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(
        said["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap_or_default()
            .contains("dash mail send")
    );
    let out = hook(&["outbound-gate"], "not json");
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"deny\""));
    let read =
        serde_json::json!({"tool_name": "mcp__claude_ai_Gmail__search_threads", "tool_input": {}});
    let out = hook(&["outbound-gate"], &read.to_string());
    assert!(out.stdout.is_empty() && out.status.success());
}

/// `dash outbound check` (klar-mail's and freework_apply.py's gate): a banned phrase FAILs with exit 1, from the rows a
/// stand-in dash prints; a clean text FAILs too under a test, as Jev may not be paid there (no verdict, no send).
#[test]
fn outbound_check_fails_a_banned_text_and_a_text_jev_cannot_judge() {
    let _budget = TestBudget::new();
    let repo = std::env::temp_dir().join(format!("outbound-check-{}", std::process::id()));
    let bin = repo.join("private/dashboard/bin");
    std::fs::create_dir_all(&bin).expect("dir");
    let rows = r#"[{"regex": "\\band that stays\\b", "label": "and that stays", "instead": "say it plainly"}]"#;
    std::fs::write(
        bin.join("dash"),
        format!(
            "#!/bin/sh\n[ \"$1 $2\" = 'store text' ] && printf '%s\\n' '{rows}' && exit 0\nexit 1\n"
        ),
    )
    .expect("stand-in");
    std::fs::create_dir_all(repo.join("private/ats")).expect("ats");
    let spec = r#"{"model": "jev-1.13.0", "gate": {"reads_ai_written_max": 0.5, "too_long_max": 0.6}, "questions": {}}"#;
    std::fs::write(repo.join("private/ats/draft-questions.json"), spec).expect("spec");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(bin.join("dash"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod");
    let check = |text: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
            .args(["outbound", "check", "--not-cold"])
            .env("CLAUDE_PROJECT_DIR", &repo)
            // the older app's CLI, which dash's bridge calls (Host::older)
            .env("DASH_CLI", bin.join("dash"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("dash");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(text.as_bytes())
            .expect("write");
        child.wait_with_output().expect("wait")
    };
    let out = check("Je reste en France, and that stays.");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stdout).starts_with("FAIL banned phrase « and that stays »"),
        "{:?}",
        out
    );
    let out = check("Bonjour, on en parle cette semaine ?");
    assert_eq!(out.status.code(), Some(1));
    // CI or NEXTEST, whichever is set first (jev::paid_refused): the pre-push and CI set CI
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.starts_with("FAIL Jev gave no verdict (")
            && said.contains(" is set: a check never pays Jev"),
        "{out:?}"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// The send hooks through the binary (task rust-hooks-rest-port): a form_input's text denied, the LinkedIn gate down refused,
/// a read logged by nobody; each was a .claude/hooks/*.py.
#[test]
fn the_send_hooks_answer_through_dash_hook() {
    let _budget = TestBudget::new();
    let call = |tool: &str, input: serde_json::Value| {
        serde_json::json!({"hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input, "cwd": "/tmp"})
            .to_string()
    };
    let out = hook(
        &["form-fill-guard"],
        &call(
            "mcp__claude-in-chrome__form_input",
            serde_json::json!({"value": "Kafka"}),
        ),
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Form fill refused"));
    let out = hook(
        &["form-fill-guard"],
        &call(
            "mcp__claude-in-chrome__form_input",
            serde_json::json!({"value": true}),
        ),
    );
    assert!(out.stdout.is_empty());
    let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["hook", "linkedin-budget"])
        .env("CLAUDE_PROJECT_DIR", "/tmp")
        .env(
            "LINKEDIN_GATE_URL",
            "http://127.0.0.1:9/dashboard/linkedin/",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("dash");
    let li = call("mcp__mcp-server-linkedin__get_inbox", serde_json::json!({}));
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(li.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("fails closed"),
        "{out:?}"
    );
    // input that is no JSON: the LinkedIn gate refuses, a line to log is no silent success (/code-review)
    let out = hook(&["linkedin-budget"], "not json");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("read no JSON"),
        "{out:?}"
    );
    let out = hook(&["outbound-log", "--line"], "not json");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let out = hook(
        &["outbound-log"],
        &call("mcp__mcp-server-linkedin__get_inbox", serde_json::json!({})),
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
}

/// Done-when 2 of rust-hooks-rest-port, main-checkout-code through the binary on settings.json's Edit matcher: the main
/// checkout's (.git a folder) edit of the dashboard's code is denied, a task worktree's (.git a file) passes.
#[test]
fn main_checkout_code_denies_an_edit_of_the_dashboard_in_the_main_checkout_only() {
    let _budget = TestBudget::new();
    let top = std::env::temp_dir().join(format!("main-checkout-{}", std::process::id()));
    let (main, tree) = (top.join("main"), top.join("tree"));
    std::fs::create_dir_all(main.join(".git")).expect("main");
    std::fs::create_dir_all(&tree).expect("tree");
    std::fs::write(tree.join(".git"), "gitdir: x\n").expect("worktree");
    let edit = |project: &std::path::Path| {
        let file = project.join("private/dashboard/lib/x.ex");
        let input = serde_json::json!({"hook_event_name": "PreToolUse", "tool_name": "Edit", "cwd": project,
                                       "tool_input": {"file_path": file, "old_string": "a", "new_string": "b"}});
        let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
            .args(["hook", "main-checkout-code"])
            .env("CLAUDE_PROJECT_DIR", project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("dash");
        let stdin = input.to_string();
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .expect("write");
        child.wait_with_output().expect("wait")
    };
    let out = edit(&main);
    assert_eq!(out.status.code(), Some(0));
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("\"permissionDecision\":\"deny\"")
            && said.contains("Refused by dash hook main-checkout-code:")
            && said.contains(
                "private/dashboard/lib/x.ex is the dashboard's code in the main checkout"
            ),
        "{said}"
    );
    let out = edit(&tree);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "{out:?}");
    let _ = std::fs::remove_dir_all(&top);
}
