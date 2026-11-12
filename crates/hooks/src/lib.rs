//! Claude Code's hooks, a plugin of dash (rfc-rust-unified#self-hosting: « the core is lean », « everything is a plugin »;
//! task rust-hooks-rest-port; @nha, 2026-11-10: « all these python scripts are also very brittle. we want to move all logic
//! to rust and have it well tested. »). Each was a .claude/hooks script; dash's main.rs answers `dash hook <name>` with one
//! call here.
//! 1. `send`: form-fill-guard, linkedin-budget, outbound-log, on the tools that reach a person.
//! 2. `outbound`: the outbound gate every message passes, `dash hook outbound-gate` and `dash outbound check`.
//! 3. `session`: session-state for bin/claude-supervise, the status line's usage figures, the gh shim on PATH.
//!
//! What the plugin needs of dash is its `Host`, installed once by dash before a hook runs: no dependency on the binary.
use std::path::PathBuf;
use std::sync::OnceLock;

pub mod outbound;
pub mod send;
pub mod session;

/// dash's side of the plugin: the clock as this Mac reads it, the checkout it runs against, and its Jev client.
pub struct Host {
    /// Unix seconds as local time, `YYYY-MM-DDTHH:MM:SS`.
    pub local_at: fn(i64) -> String,
    /// Unix time in milliseconds.
    pub now_ms: fn() -> i64,
    /// The repository root dash runs against.
    pub repo: fn() -> PathBuf,
    /// One Jev call: (questions, state, agent) to its whole answer; refused under CI or a test.
    pub jev_ask:
        fn(&serde_json::Value, serde_json::Value, &str) -> Result<serde_json::Value, String>,
    /// The Jev model that call asks.
    pub jev_model: &'static str,
    /// One line of the older app's outbound log, through dash's bridge to it (section 20b): Ok once logged, else what the
    /// bridge did instead (« dashboard log unconfirmed », « outbound send kept in », « outbound send not logged nor kept »).
    pub outbound: fn(&str) -> Result<(), String>,
    /// One read of the older app's CLI through that bridge (`store text <file>`, `model drafts`): its stdout, or why not.
    pub older: fn(&[&str]) -> Result<String, String>,
    /// The older app's LinkedIn gate through that bridge: its JSON answer to an op, or none when it does not answer.
    pub linkedin_gate: fn(&str, &serde_json::Value) -> Option<serde_json::Value>,
    /// The task whose worktree holds a folder, and its brief ("" when none was written): none for a session of no task.
    pub task_brief: fn(&std::path::Path) -> Option<(String, String)>,
}

static HOST: OnceLock<Host> = OnceLock::new();

/// dash installs its host once, before the first hook; a second install keeps the first.
pub fn install(host: Host) {
    let _ = HOST.set(host);
}

pub(crate) fn host() -> &'static Host {
    #[cfg(test)]
    install(TEST_HOST);
    HOST.get()
        .expect("dash_hooks: dash installs its Host before a hook runs")
}

/// Claude Code's PreToolUse deny on exit 0, its reason shown to the session (as dash-gate's Bash guards answer).
pub fn deny(reason: &str) -> String {
    serde_json::json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                       "permissionDecisionReason": reason}})
    .to_string()
}

/// The session's folder, CLAUDE_PROJECT_DIR, an empty one unset: each hook adds its own fallback (/simplify of PR 1480).
pub(crate) fn project_dir() -> Option<String> {
    std::env::var("CLAUDE_PROJECT_DIR")
        .ok()
        .filter(|d| !d.is_empty())
}

/// A file this user may run.
pub(crate) fn is_exec(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
fn test_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A test's host: the time as its seconds, no repo and no Jev, so no test reads the live checkout or pays.
#[cfg(test)]
const TEST_HOST: Host = Host {
    local_at: |t| format!("@{t}"),
    now_ms: test_now_ms,
    repo: || {
        panic!(
            "a test reads no repo: give the outbound gate's root, CLAUDE_PROJECT_DIR, a scratch folder"
        )
    },
    jev_ask: |_, _, _| Err("a test never pays Jev".into()),
    jev_model: "jev-1.13.0",
    outbound: |_| Err("a test reaches no older app".into()),
    older: |_| Err("a test reaches no older app".into()),
    linkedin_gate: |_, _| None,
    task_brief: |_| None,
};
