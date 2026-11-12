//! `dash help`, the real binary: the usage lines each command registers beside its code (usage.rs, task rust-usage-registered).
use dash_meter::memory::TestBudget;
use std::process::Command;

/// main.rs's USAGE as it was before each command registered its own line, verbatim.
const OLD_USAGE: &str = "usage: dash serve|mcp|extensions --data <dir>\n       dash mcp-proxy --cache <file> -- <server> [<argument>...]\n       dash serve [--data <dir>] --port <n> [--older <n>]\n       dash ui [--data <dir>] [--shot <page> <png>]|--design|--check <png> [--stop-front-door]\n       dash ingest --data <dir> --mime <key> <file>\n       dash doc extract|import <file>\n       dash task add --data <dir> <id> [--waits-on <ids, a comma between>]\n       dash task waits --data <dir> <id> --on <ids, a comma between>\n       dash task close --data <dir> <id> <why>\n       dash task show|run --data <dir> <id> [--account <name>|--adopt]\n       dash orch --data <dir> [--once|--sync|--reap|--deploy]\n       dash orch next [--data <dir>]\n       dash roadmap show [--data <dir>]\n       dash roadmap set [--data <dir>] <id> [--start <day>] [--target <day>] [--after <ids>] [--milestone '<day> <name>']...\n       dash linear roadmap [--data <dir>]\n       dash pr hold [--data <dir>] <n> <why>\n       dash pr release [--data <dir>] <n>\n       dash claude [--data <dir>]\n       dash limits\n       dash folds\n       dash dmarc <file of domains, one a line>\n       dash data copy <dir>\n       dash menu\n       dash parity <rust-path|rust-url> [<elixir-path>]|menu\n       dash scripts count [--check [--base <rev> [--changed]]]\n       dash ci list [--repo <owner/name>] [--from <day>] [--days <n>]\n       dash ci affected <base> [--pushed <sha>] [-- <cargo arguments>]\n       dash ci no-elixir <base> <head>\n       dash hook pre-commit|pre-push <remote> <url> (git's hooks)\n       dash hook pipe-cut (Claude's PreToolUse hook on Bash: its input on stdin)\n       dash hook <guard>... (Claude Code's Bash guards, its JSON on stdin: no-pkill, no-verify-commit, cargo-test, fly-legacy-guard, download-guard, main-checkout-code)\n       dash telemetry [on|off] --data <dir>\n       dash fallback [on|off] --data <dir>\n       dash alerts --data <dir>\n       dash draft import --data <dir> --from <export>\n       dash draft new --data <dir> --file <labelled.md> [--from <export>]\n       dash draft show --data <dir> <id>\n       dash draft rates --data <dir>\n       dash comment pass|work --data <dir>\n       dash inbox pass|ls [<state>]|rules --data <dir>\n       dash inbox raise --data <dir> <id> --asker <who> --what <line> [--why <reason>] [--pick <pick>] [--link <url>] [--approval --sent-by <tool>] [--kind|--pr|--task|--draft|--supersedes|--not-before|--deadline <value>]\n       dash inbox answer --data <dir> <id> <words>\n       dash inbox gate lift --data <dir> <gate> <his words>\n       dash agents [--data <dir>]\n       dash agents key add <kind> <secret name> [--data <dir>] [--cap <usd>]\n       dash agents key rm|run <kind> [<prompt>] [--data <dir>]\n       dash login add <name> [--dry-run]|sign-in <name>|status|remove <name>\n       dash access\n       dash sandbox denied (a hook: its input on stdin)\n       dash ship \"<message>\" <file>...\n       dash pane say [--his] <pane|task id> <text>...";

fn dash(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(args)
        .output()
        .expect("dash")
}

fn commands(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = (text.lines())
        .map(|l| l.trim_start_matches("usage:").trim().to_string())
        .collect();
    lines.sort();
    lines
}

/// Done-when 2: main.rs holds no USAGE, and `dash help` prints every line it had, sorted, plus its own; an unknown command
/// gets the same text on stderr, exit 2.
#[test]
fn help_keeps_every_line_the_old_usage_had() {
    let _budget = TestBudget::new();
    assert!(
        !include_str!("../src/main.rs").contains("const USAGE"),
        "main.rs still holds a USAGE"
    );
    let help = dash(&["help"]);
    assert_eq!(help.status.code(), Some(0), "{help:?}");
    let said = String::from_utf8(help.stdout).expect("utf-8");
    let lines = commands(&said);
    let missing: Vec<String> = (commands(OLD_USAGE).into_iter())
        .filter(|l| !lines.contains(l))
        .collect();
    assert!(missing.is_empty(), "lines dash help lost: {missing:#?}");
    let printed: Vec<String> = (said.lines())
        .map(|l| l.trim_start_matches("usage:").trim().to_string())
        .collect();
    assert_eq!(printed, lines, "sorted");
    assert!(lines.contains(&"dash help".to_string()));
    let unknown = dash(&["no-such-command"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&unknown.stderr), said);
}

/// A command registered with its `run` is answered with no arm in main.rs: `dash pane say` with no words gets its own usage.
#[test]
fn a_registered_command_is_dispatched() {
    let _budget = TestBudget::new();
    let said = dash(&["pane", "say"]);
    assert_eq!(said.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&said.stderr).starts_with("usage: dash pane say [--his]"),
        "{said:?}"
    );
}
