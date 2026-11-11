//! Claude's Bash tool on bash 5 (task claude-shell-bash5; @nha, 2026-10-29: « You fixed the mistake but didn't find the cause
//! and what could actually fix that class of errors right ? »). On /bin/bash 3.2 Claude Code refused a heredoc inside $(...)
//! and some for loops over $(...) (exit 126, « this shell (bash 3.2.57) is too old to run it reliably »), a turn lost each
//! time. The shell comes from CLAUDE_CODE_SHELL in ~/.claude/settings.json, outside any repo: this test reads the host's copy,
//! and the jobs.conf line that reinstalls that bash (`dash host brew bash`).
use dash_meter::memory::TestBudget;
use std::path::PathBuf;
use std::process::Command;

const SHELL: &str = "/opt/homebrew/bin/bash";

/// Done-when 1: settings.json names Homebrew's bash, it runs bash 5 or later, the global CLAUDE.md's quirk 3 says so, and
/// jobs.conf's brew line installs it when missing. Off the Mac (CI on Linux) or with no ~/.claude/settings.json, the test
/// stops; any other read error fails.
#[test]
fn claude_code_shell_is_homebrew_bash_5_and_quirk_3_names_it() {
    let _budget = TestBudget::new();
    let claude = PathBuf::from(std::env::var("HOME").expect("HOME")).join(".claude");
    let settings = match std::fs::read_to_string(claude.join("settings.json")) {
        Ok(text) if cfg!(target_os = "macos") => text,
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            panic!("~/.claude/settings.json: {e}")
        }
        _ => {
            println!("not his Mac (no ~/.claude/settings.json, or not macOS): this test stops");
            return;
        }
    };
    let settings: serde_json::Value =
        serde_json::from_str(&settings).expect("settings.json is JSON");
    assert_eq!(settings["env"]["CLAUDE_CODE_SHELL"], SHELL);
    // settings.local.json overrides settings.json: it may name no shell, or this one
    if let Ok(local) = std::fs::read_to_string(claude.join("settings.local.json")) {
        let local: serde_json::Value =
            serde_json::from_str(&local).expect("settings.local.json is JSON");
        let shell = &local["env"]["CLAUDE_CODE_SHELL"];
        assert!(
            shell.is_null() || shell == SHELL,
            "settings.local.json names {shell}"
        );
    }
    let out = Command::new(SHELL)
        .args(["-c", "echo \"${BASH_VERSINFO[0]}\""])
        .output()
        .expect("CLAUDE_CODE_SHELL runs");
    assert!(
        out.status.success(),
        "{SHELL}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let major: u32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("a major version");
    assert!(major >= 5, "{SHELL} is bash {major}");
    let md = std::fs::read_to_string(claude.join("CLAUDE.md")).expect("the global CLAUDE.md");
    // quirk 3 today; found by its bold lead, so a renumbering does not break it
    let quirk = md
        .lines()
        .find(|l| l.contains("**Claude's Bash tool runs"))
        .expect("the global CLAUDE.md's quirk on Claude's Bash tool");
    assert!(quirk.contains(SHELL) && quirk.contains("bash 5"), "{quirk}");
    let Some(repo) = dash_meter::monorepo() else {
        return;
    };
    let conf = std::fs::read_to_string(repo.join("sandbox/jobs.conf")).expect("jobs.conf");
    assert!(
        conf.lines()
            .any(|l| l.starts_with("brew ") && l.ends_with("/opt/homebrew/bin/brew install {bash}")),
        "jobs.conf's brew line no longer installs bash"
    );
}
