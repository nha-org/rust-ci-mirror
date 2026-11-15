//! `dash hook <guard>...` as .claude/settings.json runs it: Claude Code's PreToolUse JSON on stdin, a refusal answered as its
//! deny on stdout with exit 0, anything else silent (task rust-claude-hooks-port). The guards' cases are dash_gate::guard's.
use dash_meter::dash_test;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn hook(args: &[&str], input: &str) -> Output {
    hook_in(&home(), args, input)
}

fn hook_in(home: &std::path::Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
        .arg("hook")
        .args(args)
        .env("CLAUDE_PROJECT_DIR", env!("CARGO_MANIFEST_DIR"))
        // each refusal is a firing under HOME: a test's land under target/, never the Mac's count
        .env("HOME", home)
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

/// The tests' HOME, where their refusals' firings go: one folder under target/, so a run leaves no folder behind
/// (/code-review high: one per test process piled up in the temp dir).
fn home() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("claude-hooks-home")
}

fn bash(command: &str) -> String {
    serde_json::json!({"session_id": "s1", "cwd": "/tmp", "hook_event_name": "PreToolUse", "tool_name": "Bash",
                       "tool_input": {"command": command}})
    .to_string()
}

const GUARDS: [&str; 3] = ["no-pkill", "no-verify-commit", "cargo-test"];

#[dash_test]
fn a_refused_call_is_denied_with_the_guards_message_on_exit_0() {
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

/// settings.json's Bash chain, in its order
const CHAIN: [&str; 7] = [
    "no-pkill",
    "no-verify-commit",
    "cargo-test",
    "fly-legacy-guard",
    "download-guard",
    "main-checkout-code",
    "nextest-stress",
];

fn refusal(command: &str) -> Option<String> {
    refusal_in(&home(), command)
}

fn refusal_in(home: &std::path::Path, command: &str) -> Option<String> {
    let out = hook_in(home, &CHAIN, &bash(command));
    assert_eq!(out.status.code(), Some(0), "{command}");
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(
        said["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()?
            .into(),
    )
}

/// A hook judges the command run, never text it prints, quotes or puts in a PR body (task hook-reads-command-not-text:
/// hooks-tested-linted had to Write its probe scripts first, no-pkill refusing a Bash text naming pkill).
#[dash_test]
fn the_chain_judges_the_command_run_never_the_text_it_carries() {
    let k = ["p", "kill"].concat();
    // done-when 1: a heredoc or a cat naming pkill passes; a real one is still refused
    for text in [
        format!("cat <<'EOF' > notes.md\n{k} -f foo kills every session\nEOF"),
        format!("cat <<EOF\n{k} -f x; pgrep -f x | xargs kill\nEOF"),
        format!("git commit -m \"fix: never {k}; pgrep -f x | xargs kill\""),
        format!("grep -n 'fn \\|{k}' rust/crates/dash/tests/claude_hooks.rs"),
    ] {
        assert_eq!(refusal(&text), None, "{text}");
    }
    for run in [
        format!("{k} -f foo"),
        format!("bash -c '{k} -f foo'"),
        format!("echo \"$({k} -f foo)\""),
        format!("cat <<EOF\n$({k} -f foo)\nEOF"),
        "for p in $(pgrep -f 'make x'); do kill $p; done".into(),
    ] {
        let why = refusal(&run).unwrap_or_default();
        assert!(
            why.starts_with("Refused by dash hook no-pkill"),
            "{run}: {why}"
        );
    }
    // done-when 2: a PR body quoting a merge command, and what each guard refuses, passes the whole chain
    let body = format!(
        "Merge with gh pr merge 12 --squash --admin, never git merge --no-verify main.\n\n{k} -f x and fly orgs create y are refused."
    );
    for create in [
        format!("gh pr create --title t --body \"{body}\""),
        format!("gh pr create --title t --body \"$(cat <<'EOF'\n{body}\nEOF\n)\""),
        format!("gh pr create --title t --body-file - <<'EOF'\n{body}\nEOF"),
    ] {
        assert_eq!(refusal(&create), None, "{create}");
    }
    // a real gh pr merge is still read as code: what it chains is judged, inside a shell's string too
    for merge in [
        "gh pr merge 12 --squash && git merge --no-verify main",
        "bash -c 'gh pr merge 12 --squash; git merge --no-verify main'",
        "bash -c 'git merge --no-verify main'",
        "echo \"$(git merge --no-verify main)\"",
    ] {
        let why = refusal(merge).unwrap_or_default();
        assert!(
            why.starts_with("Refused by dash hook no-verify-commit: git merge --no-verify"),
            "{merge}: {why}"
        );
    }
}

#[dash_test]
fn a_plain_call_and_input_that_is_not_json_pass_silently() {
    for input in [bash("git status --short"), "not json".into(), String::new()] {
        let out = hook(&GUARDS, &input);
        assert_eq!(out.status.code(), Some(0), "{input}");
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{input}");
    }
}

#[dash_test]
fn an_unknown_guard_is_an_error_claude_code_lets_through() {
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
#[dash_test]
fn the_outbound_gate_denies_a_gmail_send_and_input_it_cannot_read() {
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
#[dash_test]
fn outbound_check_fails_a_banned_text_and_a_text_jev_cannot_judge() {
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
#[dash_test]
fn the_send_hooks_answer_through_dash_hook() {
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
#[dash_test]
fn main_checkout_code_denies_an_edit_of_the_dashboard_in_the_main_checkout_only() {
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
            .env("HOME", home())
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

/// `dash hook review-claim` (task review-diff-merge-base): /code-review on a PR whose push the app refuses is denied before it
/// starts, the app's reason quoted; another skill runs untouched and asks nothing.
#[dash_test]
fn a_review_pass_whose_push_is_refused_is_denied_before_it_starts() {
    let dir = std::env::temp_dir().join(format!(
        "review-claim-hook-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let app = dir.join("dash");
    std::fs::write(
        &app,
        "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/asked\"\necho 'review push: refused: merge-on-green holds the PR.'\nexit 1\n",
    )
    .expect("stand-in");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let skill = |name: &str| {
        serde_json::json!({"cwd": "/", "tool_name": "Skill",
                           "tool_input": {"skill": name, "args": "medium https://github.com/o/r/pull/5"}})
        .to_string()
    };
    let run = |input: String| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
            .args(["hook", "review-claim"])
            .env("DASH_BIN", &app)
            .env("HOME", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("dash");
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(input.as_bytes()).expect("write");
        drop(stdin);
        child.wait_with_output().expect("wait")
    };
    let out = run(skill("humanizer"));
    assert!(out.stdout.is_empty() && !dir.join("asked").exists());
    let out = run(skill("code-review"));
    assert_eq!(out.status.code(), Some(0));
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let why = said["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default();
    assert!(why.contains("merge-on-green holds the PR"), "{why}");
    let asked = std::fs::read_to_string(dir.join("asked")).expect("asked");
    assert_eq!(asked.trim(), "review push https://github.com/o/r/pull/5");
    // with no link, the checkout's branch is the key, as pre-push asks it; the default branch is never asked about
    let repo = dir.join("repo");
    let git = |args: &[&str]| {
        let ok = Command::new("git").args(args).current_dir(&repo).status();
        assert!(ok.expect("git").success(), "git {args:?}");
    };
    std::fs::create_dir_all(&repo).expect("repo");
    std::fs::write(repo.join("forge.env"), "DEFAULT_BRANCH=main\n").expect("forge.env");
    git(&["init", "-q", "-b", "main"]);
    git(&["remote", "add", "origin", "git@github.com:o/r.git"]);
    let in_repo = || {
        serde_json::json!({"cwd": repo, "tool_name": "Skill",
                           "tool_input": {"skill": "simplify", "args": ""}})
        .to_string()
    };
    std::fs::remove_file(dir.join("asked")).expect("reset");
    assert!(run(in_repo()).stdout.is_empty() && !dir.join("asked").exists());
    git(&["checkout", "-q", "-b", "task/x"]);
    assert!(!run(in_repo()).stdout.is_empty());
    let asked = std::fs::read_to_string(dir.join("asked")).expect("asked");
    assert_eq!(asked.trim(), "review push o/r:task/x");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Task stress-run-scoped, done-when 1 and 3 through the binary on settings.json's chain: a workspace stress is denied with
/// the line to run, and each refusal adds a firing under its guard's name, which dash gate firings counts; a pass adds none.
#[dash_test]
fn a_workspace_stress_is_denied_and_counted_as_a_firing() {
    // its own HOME, so the other tests' firings never reach its log
    let home = dash_meter::TestDir::new("claude-hooks-firing");
    let log = home.0.join(".claude/run/gate-firings.tsv");
    assert_eq!(
        refusal_in(
            &home.0,
            "cargo nextest run -p dash not_built_yet --stress-count 20"
        ),
        None
    );
    assert!(!log.exists(), "a pass is no firing");
    let why = refusal_in(
        &home.0,
        "cd rust && cargo nextest run --workspace --stress-count 20",
    )
    .unwrap_or_default();
    assert!(
        why.starts_with("Refused by dash hook nextest-stress:")
            && why.contains("cargo nextest run -p <crate> <test name> --stress-count 20"),
        "{why}"
    );
    let text = std::fs::read_to_string(&log).expect("the firings");
    let (rule, day) = text.trim_end().split_once('\t').expect("one line, a tab");
    assert_eq!(rule, "nextest-stress", "{text}");
    assert!(day.parse::<u64>().expect("a day number") > 20_000, "{day}");
}
