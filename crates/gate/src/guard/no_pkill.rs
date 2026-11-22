//! Refuses pkill and killall, a kill fed by `pgrep -f`, the orchestrator's watcher backgrounded with `&`, and the main
//! checkout's `herdr pane close` (or tab or workspace close) of a task's pane, unless that pane is a shell idle at its
//! prompt. Stop a process by its pid: `pgrep -lf <pattern>` to see it, then `kill <pid>`. Was .claude/hooks/no-pkill.py; the cases of close_guard_test.exs's « the hook » are the tests below.
//!
//! On 2026-10-19 at 16:57:55 UTC a task ran `pkill -f "mix phx.server" -U $(id -u) -n`. BSD pkill reads options only before
//! the pattern, so -U, 501 and -n became more patterns: every process whose command line holds « 501 » or « -n » got a
//! SIGTERM, every Claude session on the Mac among them (@nha, 2026-10-19: « It died again. I have to resume manually again.
//! Why ? This is the absolute top priority now »). A name pattern is never narrow enough here: each Claude's command line
//! carries its brief, so a word of a task's text matches a session. Reads the command bash runs (`shell::code`): a heredoc,
//! a commit message or a PR body that names pkill is text (task hook-reads-command-not-text), a `$(pkill)` in it still runs.
use super::shell::{Call, code};
use dash_core::at_prompt;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;

/// the word as a command: at the start, or after a separator, sudo, env, xargs or a subshell
static PKILL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(^|[;&|(`$]\s*|\b(sudo|env|xargs|exec|nohup|timeout)\s+)(/usr/bin/)?(pkill|killall)\b")
        .expect("pkill")
});
static PGREP_F: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bpgrep\b[^|;&)\n]*\s-[a-zA-Z]*f").expect("pgrep"));
static KILL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(^|[;&|(`$]\s*|\b(do|then|sudo|xargs|exec)\s+)(/bin/)?kill\b").expect("kill")
});
/// herdr by any name ($HERDR_BIN, ~/.local/bin/herdr), what it closes (a pane, a tab or a workspace, each holding panes) and
/// its id, the first word after `close` that is no flag
static CLOSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)herdr\w*\b[^;&|\n]*?\b(pane|tab|workspace)\s+close\s+(?:-\S+\s+)*([^\s;&|]+)")
        .expect("close")
});
const PKILL_WHY: &str = "Refused by dash hook no-pkill: pkill and killall match by pattern, and on macOS an option after the pattern becomes another pattern; on 2026-10-19 one such pkill killed every Claude session on the Mac. List with `pgrep -lf <pattern>`, check each line, then `kill <pid>`. A server you started: keep its pid ($!) and kill that.";
const PGREP_WHY: &str = "Refused by dash hook no-pkill: kill fed by pgrep -f matches every process whose command line holds the words, and each Claude's command line carries its brief or restart prompt; on 2026-10-20 one such kill stopped the orchestrator. Stop what you started by its pid ($! when you start it), by its port (lsof -ti tcp:<port>), or with private/dashboard/bin/dash task procs.";
const WAKE_WHY: &str = "Refused by dash hook no-pkill: wake-wait backgrounded with `&` inside a command ends with that command or wakes into nothing. Run private/dashboard/priv/commands/wake-wait alone, with the Bash tool's run_in_background, so its wake reaches you.";

pub fn refused(call: &Call) -> Option<String> {
    // what bash runs: a string a shell runs (bash -c '...') opens with a `;` there, a command starting after it
    let run = code(call.command);
    if PKILL.is_match(&run) {
        return Some(PKILL_WHY.into());
    }
    // kill fed by pgrep -f in the same command is the same pattern kill: on 2026-10-20 a task's
    // `for p in $(pgrep -f "make dashboard") ...; do kill $p; done` stopped the orchestrator, its restart prompt holding the
    // words. pgrep -lf alone (to look) and kill <pid> alone stay allowed.
    if PGREP_F.is_match(&run) && KILL.is_match(&run) {
        return Some(PGREP_WHY.into());
    }
    // the watcher backgrounded inside a command ends with it, or wakes into /dev/null (three times on 2026-11-10); the
    // command by its path, so a text that names it passes
    if wake_backgrounded(&run) {
        return Some(WAKE_WHY.into());
    }
    close_of_a_task(call, &run).map(|why| {
        format!(
            "Refused by dash hook no-pkill: {why}. A shell idle at its prompt closes here; close a task's other pane with `private/dashboard/bin/dash task close <id>`: it refuses a session that works or holds uncommitted or unpushed work, and says which; `--after-report` closes it once its report lands."
        )
    })
}

/// `commands/wake-wait`, then before any of ; | a quote or a line break, a lone `&` (not `&&`).
fn wake_backgrounded(cmd: &str) -> bool {
    let c: Vec<char> = cmd.chars().collect();
    let path: Vec<char> = "commands/wake-wait".chars().collect();
    (0..c.len()).any(|at| {
        let end = at + path.len();
        c[at..].starts_with(&path)
            && !c
                .get(end)
                .is_some_and(|&ch| ch.is_alphanumeric() || ch == '_')
            && (end..c.len())
                .take_while(|&k| !matches!(c[k], ';' | '\n' | '|' | '"' | '\'' | '`'))
                .any(|k| c[k] == '&' && c[k - 1] != '&' && c.get(k + 1) != Some(&'&'))
    })
}

/// herdr pane close on a task's pane from the main checkout (.git a folder): on 2026-10-24 it queued closes of 14 task
/// sessions to regroup, one mid-build (@nha, 2026-10-24: « WAIT WHY ARE YOU WASTING WORK ? »). dash task close checks the
/// session; a bare herdr close checks nothing. Every close of a chain is read; a pane id it cannot read ($p in a loop) may
/// be any pane, so it is refused: the guard never fails open (/code-review of #473). A tab or a workspace closes each pane it
/// holds, so it is judged by them: on 2026-11-22 16 task workspaces closed past this guard, which read only `pane close`. A
/// plain shell idle at its prompt (`dash_core::at_prompt`) closes, as dash task close and the look close it (task
/// pane-close-shells; @nha, 2026-11-22: « You can. via herdr direct »).
fn close_of_a_task(call: &Call, run: &str) -> Option<String> {
    // the closes bash runs (`run`, so one a PR body or a heredoc names is none), each id read back from the raw command at
    // the same chars: `code` keeps every char's place, and a quoted id is blanked there
    let raw: Vec<char> = call.command.chars().collect();
    let closes: Vec<(String, String)> = CLOSE
        .captures_iter(run)
        .filter_map(|m| Some((m.get(1)?.as_str().to_lowercase(), m.get(2)?)))
        .map(|(what, g)| {
            let at = run[..g.start()].chars().count();
            (
                what,
                raw[at..at + g.as_str().chars().count()].iter().collect(),
            )
        })
        .collect();
    if closes.is_empty() || !(call.is_dir)(&call.project.join(".git")) {
        return None;
    }
    if let Some((what, var)) = closes.iter().find(|(_, p)| p.contains(['$', '`'])) {
        return Some(format!(
            "{what} id {var} is a shell variable, so it may run a task"
        ));
    }
    let panes = match (call.panes)().and_then(|out| folders(&out)) {
        Ok(panes) => panes,
        Err(e) => {
            return Some(format!(
                "herdr pane list failed ({e}), so a pane closed here may run a task"
            ));
        }
    };
    // the one reader of a worktree's task under this repository's worktrees, as given and real: dash task close finds its panes
    // by it, so the id named here is one it closes (task pane-lookup-one). Any other worktrees folder (another repository's, a
    // stand-in tree) is read by the same reader under its own root, refused as the older regex refused it: never fail open.
    let task_of = |cwd: &str, root: &Path| {
        let roots = [root.to_path_buf(), (call.real)(root)];
        let (_, _, task) = dash_core::worktree_task(cwd, &[&roots[0], &roots[1]])?;
        Some(task.to_string())
    };
    let name = call.project.file_name().unwrap_or_default();
    let mine = call.home.join(".herdr/worktrees").join(name);
    let task_pane = |p: &Pane| {
        let (pane, cwd) = (&p.id, &p.cwd);
        if let Some(task) = task_of(cwd, &mine) {
            return Some(format!("pane {pane} runs task {task}"));
        }
        const TAIL: &str = "/.herdr/worktrees/";
        let at = cwd.find(TAIL)? + TAIL.len();
        let root = Path::new(&cwd[..at]).join(cwd[at..].split('/').next()?);
        let task = task_of(cwd, &root)?;
        Some(format!(
            "pane {pane} runs task {task} of {}",
            root.display()
        ))
    };
    closes.iter().find_map(|(what, close)| {
        let id = close.trim_matches(['\'', '"']);
        let held = |p: &&Pane| match what.as_str() {
            "pane" => p.id == id,
            "tab" => p.tab == id,
            _ => p.workspace == id,
        };
        let why = panes
            .iter()
            .filter(held)
            .filter(|p| !p.idle)
            .find_map(task_pane)?;
        Some(match what.as_str() {
            "pane" => why,
            _ => format!("{what} {id} holds it: {why}"),
        })
    })
}

/// A pane as `herdr pane list` lists it: what holds it, and whether it is a plain shell idle at its prompt.
struct Pane {
    id: String,
    cwd: String,
    tab: String,
    workspace: String,
    idle: bool,
}

/// `herdr pane list`'s panes.
fn folders(out: &str) -> Result<Vec<Pane>, String> {
    // as the .py read it: no `result` is no pane, a `result` or answer of another shape a failure (/code-review of PR 1457)
    let v: Value = serde_json::from_str(out).map_err(|e| format!("not JSON: {e}"))?;
    let v = v.as_object().ok_or("its answer is no object")?;
    let Some(result) = v.get("result") else {
        return Ok(vec![]);
    };
    let result = result.as_object().ok_or("its result is no object")?;
    let Some(panes) = result.get("panes") else {
        return Ok(vec![]);
    };
    let panes = panes.as_array().ok_or("result.panes is no list")?;
    panes
        .iter()
        .map(|p| {
            let p = p.as_object().ok_or("a pane is no object")?;
            let field = |k: &str| {
                p.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let shell = p.get("agent").is_none_or(Value::is_null);
            Ok(Pane {
                idle: shell && at_prompt(&field("terminal_title"), &field("cwd")),
                id: field("pane_id"),
                cwd: field("cwd"),
                tab: field("tab_id"),
                workspace: field("workspace_id"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::path::Path;

    const PANES: &str = r#"{"result": {"panes": [
        {"pane_id": "w1:p2", "cwd": "/Users/x/.herdr/worktrees/site_perso/task-cg8/private"},
        {"pane_id": "w1:p3", "cwd": "/Users/x/repo/klar"},
        {"pane_id": "w1:p4", "agent": null, "cwd": "/Users/x/.herdr/worktrees/replicator/task-prs-today"},
        {"pane_id": "w1:p5", "cwd": "/Users/x/.herdr/worktrees/site_perso/rust-rust-skeleton/rust"},
        {"pane_id": "w1:p6", "cwd": "/Users/x/.herdr/worktrees/site_perso/rust-rust-skeleton/t/.herdr/worktrees/site_perso/task-z"},
        {"pane_id": "w1:p7", "cwd": "/tmp/t/.herdr/worktrees/site_perso/task-z"}]}}"#;

    /// The guard on `cmd`, run from the main checkout (`main`) or a worktree, herdr answering `panes`.
    fn judge(cmd: &str, main: bool, panes: Result<&str, &str>) -> Option<String> {
        let list = || panes.map(String::from).map_err(String::from);
        let call = Call {
            tool: "Bash",
            command: cmd,
            file: "",
            cwd: Path::new("/Users/x/repo/site_perso"),
            project: Path::new("/Users/x/repo/site_perso"),
            home: Path::new("/Users/x"),
            panes: &list,
            is_file: &|_| false,
            is_dir: &|_| main,
            read: &|_| None,
            real: &|p| p.to_path_buf(),
            set: &|_| false,
        };
        refused(&call)
    }

    fn main(cmd: &str) -> Option<String> {
        judge(cmd, true, Ok(PANES))
    }

    #[dash_test]
    fn refuses_pattern_kills_and_passes_a_pid_kill() {
        for bad in [
            "pkill -f foo",
            "sudo killall Finder",
            "true; /usr/bin/pkill x",
            "for p in $(pgrep -f 'make dashboard'); do kill $p; done",
            "pgrep -lf x | xargs kill",
            "private/dashboard/priv/commands/wake-wait --x &",
            // what bash runs inside a quote or an unquoted heredoc (task hook-reads-command-not-text)
            "bash -c 'pkill -f foo'",
            "sh -lc \"killall Finder\"",
            "echo \"$(pkill -f foo)\"",
            "cat <<EOF\n`pkill x`\nEOF",
            "watch -n1 'pkill -f foo'",
            "bash -o pipefail -c 'pkill -f foo'",
            "ssh host 'killall x'",
            "bash -c \\\n'pkill x'",
            "grep -n '<<EOF' f\npkill -f foo\nEOF",
            "bash -c 'private/dashboard/priv/commands/wake-wait &'",
        ] {
            assert!(
                main(bad).is_some_and(|w| w.starts_with("Refused by dash hook no-pkill")),
                "{bad}"
            );
        }
        for good in [
            "pgrep -lf foo",
            "kill 123",
            "echo use pkill never",
            "git status",
            "private/dashboard/priv/commands/wake-wait",
            "private/dashboard/priv/commands/wake-wait && echo ok",
            "grep -n commands/wake-wait f && echo y",
            // text that only names them: a heredoc, a message, a PR body, a grep pattern
            "cat <<'EOF' > n.md\npkill -f x\nEOF",
            "git commit -m \"x; pkill -f y; pgrep -f z | xargs kill\"",
            "git commit -m \"$(printf \"%s\" \"pkill: never use it\")\"",
            "cat <<'EOF' > n.md\nnever run private/dashboard/priv/commands/wake-wait & in a chain\nEOF",
            "gh pr create --body 'never pkill; herdr pane close w1:p2'",
            "grep -n \"fn \\|pkill\" f",
        ] {
            assert_eq!(main(good), None, "{good}");
        }
    }

    #[dash_test]
    fn refuses_the_main_checkouts_close_of_a_task_pane_and_names_dash_task_close() {
        let why = main("herdr pane close w1:p2").expect("refused");
        assert!(why.contains("pane w1:p2 runs task cg8"), "{why}");
        assert!(why.contains("dash task close <id>"), "{why}");
        for bad in [
            "~/.local/bin/herdr pane close 'w1:p2' && echo ok",
            "herdr pane close --force w1:p2",
            "\"$HERDR_BIN\" pane close w1:p2",
        ] {
            assert!(main(bad).is_some(), "{bad}");
        }
        // every close of a chain, not the first only
        assert!(
            main("herdr pane close w1:p3 && herdr pane close w1:p2")
                .unwrap()
                .contains("pane w1:p2 runs task cg8")
        );
        // another repository's delegate: refused, its worktrees named, since this repository's close never finds it (task
        // pane-lookup-one), and a Rust run's pane by the store's id, the folder's mark dropped
        assert!(
            main("herdr pane close w1:p4")
                .unwrap()
                .contains("pane w1:p4 runs task prs-today of /Users/x/.herdr/worktrees/replicator")
        );
        assert!(
            main("herdr pane close w1:p5")
                .unwrap()
                .contains("pane w1:p5 runs task rust-skeleton")
        );
        // a worktrees folder nested in a worktree is that worktree's; a stand-in tree's is refused as the older regex refused it
        assert!(
            main("herdr pane close w1:p6")
                .unwrap()
                .contains("pane w1:p6 runs task rust-skeleton")
        );
        assert!(
            main("herdr pane close w1:p7")
                .unwrap()
                .contains("pane w1:p7 runs task z of /tmp/t/.herdr/worktrees/site_perso")
        );
    }

    #[dash_test]
    fn refuses_a_pane_id_it_cannot_read_and_a_herdr_that_cannot_say() {
        let why = main("for p in $(cat ids); do herdr pane close $p; done").expect("refused");
        assert!(why.contains("shell variable"), "{why}");
        assert!(main("herdr pane close \"$pane\"").is_some());
        let why = judge(
            "herdr pane close w1:p2",
            true,
            Err("/nonexistent/herdr: not found"),
        )
        .expect("refused");
        assert!(why.contains("herdr pane list failed"), "{why}");
        // an answer of another shape fails closed, as the .py's AttributeError did (/code-review of PR 1457)
        for odd in [
            "oops",
            "[]",
            r#"{"result": null}"#,
            r#"{"result": {"panes": {}}}"#,
        ] {
            let why = judge("herdr pane close w1:p2", true, Ok(odd)).expect(odd);
            assert!(why.contains("herdr pane list failed"), "{odd}: {why}");
        }
        assert_eq!(judge("herdr pane close w1:p2", true, Ok("{}")), None);
    }

    /// Task pane-close-shells: one rule for a pane, a tab and a workspace, as dash task close and the look apply it. A plain
    /// shell idle at its prompt in a task's worktree closes; an agent's pane, or a shell running a command, refuses each route
    /// that would close it (16 task workspaces closed past `pane close`'s guard on 2026-11-22); a pane of no task closes.
    #[dash_test]
    fn one_rule_for_a_pane_a_tab_and_a_workspace_an_idle_shell_closes_a_busy_one_refuses() {
        let t = "/Users/x/.herdr/worktrees/site_perso";
        let panes = r#"{"result": {"panes": [
            {"pane_id": "w32M:p1", "workspace_id": "w32M", "tab_id": "w32M:t1", "cwd": "@T/rust-untangle-core-sheds", "terminal_title": "x@Mac:~/.herdr/worktrees/site_perso/rust-untangle-core-sheds"},
            {"pane_id": "w32N:p1", "workspace_id": "w32N", "tab_id": "w32N:t1", "cwd": "@T/rust-untangle-core-sheds", "terminal_title": "cargo nextest run"},
            {"pane_id": "w32N:p2", "workspace_id": "w32N", "tab_id": "w32N:t1", "cwd": "@T/rust-untangle-core-sheds", "agent": null, "terminal_title": "x@Mac:@T/rust-untangle-core-sheds"},
            {"pane_id": "w336:p1", "workspace_id": "w336", "tab_id": "w336:t1", "cwd": "@T/rust-done-when-frozen-file", "agent": "claude", "agent_status": "idle", "terminal_title": "x@Mac:@T/rust-done-when-frozen-file"},
            {"pane_id": "w25J:p1", "workspace_id": "w25J", "tab_id": "w25J:t1", "cwd": "/Users/x/repo/klar", "terminal_title": "cargo build"}]}}"#
            .replace("@T", t);
        let judged = |cmd: &str| judge(cmd, true, Ok(&panes));
        for idle in [
            "herdr pane close w32M:p1",
            "herdr tab close w32M:t1",
            "herdr workspace close w32M",
            "herdr pane close w32N:p2",
            "herdr workspace close w25J",
        ] {
            assert_eq!(judged(idle), None, "{idle}");
        }
        for (busy, why) in [
            (
                "herdr pane close w32N:p1",
                "pane w32N:p1 runs task untangle-core-sheds",
            ),
            (
                "herdr workspace close w32N",
                "workspace w32N holds it: pane w32N:p1 runs task untangle-core-sheds",
            ),
            (
                "herdr tab close w32N:t1",
                "tab w32N:t1 holds it: pane w32N:p1",
            ),
            (
                "herdr pane close w336:p1",
                "pane w336:p1 runs task done-when-frozen-file",
            ),
            (
                "herdr workspace close w336",
                "workspace w336 holds it: pane w336:p1",
            ),
            (
                "herdr workspace close w32M && herdr workspace close w336",
                "workspace w336",
            ),
            (
                "for w in $(cat ws); do herdr workspace close $w; done",
                "workspace id $w is a shell variable",
            ),
        ] {
            let said = judged(busy).expect(busy);
            assert!(said.contains(why), "{busy}: {said}");
        }
    }

    #[dash_test]
    fn lets_a_pane_of_no_task_close_and_a_worktree_close_its_own() {
        assert_eq!(main("herdr pane close w1:p3"), None);
        assert_eq!(judge("herdr pane close w1:p2", false, Ok(PANES)), None);
        assert_eq!(main("herdr pane list"), None);
        // a close text names beside a real one is not judged (/code-review of this task)
        assert_eq!(
            main("herdr pane close w1:p3 && gh pr create --body 'loop: herdr pane close $p'"),
            None
        );
        assert!(
            main("bash -c 'herdr pane close w1:p2'")
                .unwrap()
                .contains("pane w1:p2 runs task cg8")
        );
    }
}
