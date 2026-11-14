//! The binary's wiring of the dash-hooks plugin (Claude Code's hooks, task rust-hooks-rest-port): its Host from dash's
//! clock, repo and Jev client, then `dash hook <name>` and `dash outbound check|drafts` handed to it, and the outbound gate's
//! answers for the binary's own send paths (task outbound-gate). The hooks and the gate are the plugin's.
use std::fs::File;
use std::io::{Read, Write};

inventory::submit! { crate::usage::Command::usage("dash hook form-fill-guard|linkedin-budget [--post]|outbound-log [--line] (Claude Code's hooks on the send tools, its JSON on stdin)") }
inventory::submit! { crate::usage::Command::usage("dash hook outbound-gate (Claude Code's PreToolUse on every send: its JSON on stdin, a deny on stdout)") }
inventory::submit! { crate::usage::Command::usage("dash outbound check [--draft <id>] [--not-cold] [--not-first] [--long-ok] [<file>] (the outbound gate on a text, stdin by default: PASS or FAIL lines)") }
inventory::submit! { crate::usage::Command::usage("dash hook session-state start|busy|tool|idle (Claude Code's session hooks for bin/claude-supervise, its JSON on stdin)") }
inventory::submit! { crate::usage::Command::usage("dash hook statusline (Claude Code's status line: its usage figures kept, its JSON on stdin)") }
inventory::submit! { crate::usage::Command::usage("dash hook gh-meter-path (SessionStart: the gh shim first on the session's PATH)") }

/// The hooks main.rs hands here, after `dash hook`.
pub const NAMES: [&str; 7] = [
    "form-fill-guard",
    "linkedin-budget",
    "outbound-log",
    "outbound-gate",
    "session-state",
    "statusline",
    "gh-meter-path",
];

inventory::submit! { crate::usage::Command::usage("dash outbound drafts (every pending or approved draft's banned phrases and AI tells)") }

/// The outbound gate (dash_hooks::outbound) for the binary's own send paths, its Host installed first: `dash mail send`,
/// /mail's reply and new mail, `dash apply greenhouse --send`, and `dash outbound drafts` at the front door.
pub fn refusal(text: &str, flags: dash_hooks::outbound::Flags) -> Option<String> {
    install();
    dash_hooks::outbound::refusal(text, flags)
}

pub fn refusal_typed(text: &str, flags: Option<dash_hooks::outbound::Flags>) -> Option<String> {
    install();
    dash_hooks::outbound::refusal_typed(text, flags)
}

pub fn refusal_application(message: &str, answers: &[&str], id: &str) -> Option<String> {
    install();
    dash_hooks::outbound::refusal_application(message, answers, id)
}

pub fn outbound_drafts() -> (i32, String) {
    install();
    dash_hooks::outbound::drafts_command()
}

/// dash's side of the plugin, installed once.
fn install() {
    dash_hooks::install(dash_hooks::Host {
        local_at: crate::pages::local_at,
        now_ms: crate::tasks::now_ms,
        repo: crate::pages::repo,
        jev_ask: crate::jev::ask,
        jev_model: crate::jev::MODEL,
        jev_log: crate::jev::log_at,
        // bin/dash's own 10 s inside 20 s and two refused connections, not its 30: a PostToolUse hook never holds the
        // session long, and bin/dash's own timeout (exit 124, its « what holds it » line) comes first
        outbound: |line| {
            let env = [("DASH_TIMEOUT", "10"), ("DASH_RESTART_TRIES", "2")];
            let limit = std::time::Duration::from_secs(20);
            crate::tasks::older_app().outbound(line, &env, limit, &crate::pages::repo())
        },
        older: |args| crate::tasks::older_app().quick(args),
        linkedin_gate: dash_bridge::linkedin_gate,
        task_brief: |here| {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
            task_brief(here, &home.join(".claude/briefs"))
        },
    });
}

/// `dash hook <name> <args>` for a name of NAMES, or `dash outbound check|drafts <args>`.
pub fn run(args: &[&str], mut out: File) {
    install();
    use dash_hooks::{outbound, send, session};
    match args {
        ["outbound", "drafts"] => {
            let (code, said) = outbound::drafts_command();
            let _ = write!(out, "{said}");
            std::process::exit(code)
        }
        ["outbound", "check", rest @ ..] => {
            let (code, said) = outbound::check(rest);
            let _ = write!(out, "{said}");
            std::process::exit(code)
        }
        ["hook", "outbound-gate"] => {
            let mut input = String::new();
            let _ = std::io::stdin().read_to_string(&mut input);
            // every failure is a deny, never a pass
            if let Some(why) = outbound::hook_main(&input) {
                let _ = writeln!(out, "{}", dash_hooks::deny(&why));
            }
        }
        ["hook", "session-state", what] => session::session_state(what, out),
        ["hook", "statusline"] => session::statusline(out),
        ["hook", "gh-meter-path"] => session::gh_meter_path(),
        ["hook", name, rest @ ..] => send::run(name, rest, out),
        _ => {
            eprintln!("{}", crate::usage::text());
            std::process::exit(2)
        }
    }
}

/// The task whose worktree holds `here` and its brief, as herdr/session.sh names and writes them:
/// `<...>/.herdr/worktrees/<repo>/task-<id>`, `<briefs>/<repo>-<id>.md`, "" when it wrote none; none outside a task's worktree.
fn task_brief(here: &std::path::Path, briefs: &std::path::Path) -> Option<(String, String)> {
    static TASK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"/\.herdr/worktrees/([^/]+)/task-([^/]+)").expect("regex")
    });
    let here = here.to_string_lossy();
    let m = TASK.captures(&here)?;
    let brief = std::fs::read_to_string(briefs.join(format!("{}-{}.md", &m[1], &m[2])));
    Some((m[2].to_string(), brief.unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::task_brief;
    use dash_meter::dash_test;
    use std::path::Path;

    /// dash hook linkedin-budget's task: a task worktree's folder, below it too, names its task and brief; a checkout of no
    /// task names none; a task with no brief written has "" (so it calls nothing).
    #[dash_test]
    fn a_folder_names_its_task_and_brief_as_session_sh_writes_them() {
        let d = std::env::temp_dir().join(format!("briefs-{}", std::process::id()));
        std::fs::create_dir_all(&d).expect("dir");
        std::fs::write(d.join("site_perso-li-check.md"), "get_inbox").expect("brief");
        let task = Path::new("/Users/x/.herdr/worktrees/site_perso/task-li-check/private");
        assert_eq!(
            task_brief(task, &d),
            Some(("li-check".into(), "get_inbox".into()))
        );
        assert_eq!(task_brief(Path::new("/Users/x/repo/site_perso"), &d), None);
        let other = Path::new("/Users/x/.herdr/worktrees/site_perso/task-other");
        assert_eq!(task_brief(other, &d), Some(("other".into(), String::new())));
        let _ = std::fs::remove_dir_all(&d);
    }
}
