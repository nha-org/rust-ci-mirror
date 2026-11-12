//! Claude Code's PreToolUse guards on Bash, `dash hook <name>...` (task rust-claude-hooks-port; @nha, 2026-11-10: « all these
//! python scripts are also very brittle. we want to move all logic to rust and have it well tested. »). Each reads the call's
//! command and answers a refusal or nothing; dash prints a refusal as Claude Code's deny. no-pkill and no-verify-commit were
//! .claude/hooks/*.py, their cases are the tests here; cargo-test is new.
use serde_json::json;

pub mod cargo_test;
pub mod download;
pub mod fly_legacy;
pub mod main_checkout;
pub mod no_pkill;
pub mod no_verify_commit;
pub mod shell;

pub use shell::Call;

pub const NAMES: [&str; 6] = [
    "no-pkill",
    "no-verify-commit",
    "cargo-test",
    "fly-legacy-guard",
    "download-guard",
    "main-checkout-code",
];

/// The refusal of guard `name`, or none; a name outside NAMES judges nothing (dash refuses it before reading the call).
pub fn refusal(name: &str, call: &Call) -> Option<String> {
    match name {
        "no-pkill" => no_pkill::refused(call),
        "no-verify-commit" => no_verify_commit::refused(call.command),
        "cargo-test" => cargo_test::refused(call),
        "fly-legacy-guard" => fly_legacy::refused(call.command),
        "download-guard" => download::refused(call),
        "main-checkout-code" => main_checkout::refused(call),
        _ => None,
    }
}

/// Claude Code's deny on exit 0, its reason shown to the session. Not exit 2: settings.json turns dash's 2 into 1, since a
/// live binary older than a guard answers usage with 2, which would refuse every Bash call; 1 is a hook error Claude Code
/// shows and lets the call through, so a guard that is off says so (/code-review of PR 1457).
pub fn deny(reason: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
           "permissionDecisionReason": reason}})
    .to_string()
}
