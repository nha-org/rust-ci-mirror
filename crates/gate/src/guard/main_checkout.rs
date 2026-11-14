//! Refuses the main checkout's edit of the dashboard's code, on Edit, Write, MultiEdit, NotebookEdit and Bash. A change goes
//! through a task's worktree and a merge (private/dashboard/CLAUDE.md rule 0): the orchestrator's edits of linkedin.ex and
//! runner.ex in the main checkout reloaded the app under it, down twice on 2026-10-22 (@nha, 2026-10-26: « The dashboard is
//! more responsive too, I guess because edits are not made in-place in the main checkout. We should keep this habit. One for
//! you, the orchestrator. »). A task worktree (.git a file) edits freely; docs (*.md) in private/dashboard stay editable.
//! Was .claude/hooks/main-checkout-code.py; its fixture cases are the tests below.
use super::shell::Call;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

const TARGET: &str = r"([\w./~-]*private/dashboard/[\w./-]+)";
static WRITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r#"(?:>|\btee\s+(?:-a\s+)?)\s*['"]?{TARGET}"#)).expect("write")
});
static PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(TARGET).expect("path"));
static SEG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[;&|\n]+").expect("seg"));
static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""[^"]*"|'[^']*'"#).expect("quoted"));
static IN_PLACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(sed|perl)\s+-\w*i").expect("in place"));

pub fn refused(call: &Call) -> Option<String> {
    if !(call.is_dir)(&call.project.join(".git")) {
        return None;
    }
    let code = (call.real)(call.project).join("private/dashboard");
    let mut paths = vec![call.file.to_string()];
    if call.tool == "Bash" {
        // ponytail: an in-place edit (sed -i, perl -pi) or a redirect to a dashboard path; a script that writes by itself passes
        paths = WRITE
            .captures_iter(call.command)
            .map(|m| m[1].to_string())
            .collect();
        // only the in-place segment's own paths, and a commit message that says « sed -i » is text: quotes blanked to test
        for seg in SEG.split(call.command) {
            if IN_PLACE.is_match(&QUOTED.replace_all(seg, "''")) {
                paths.extend(PATH.find_iter(seg).map(|m| m.as_str().to_string()));
            }
        }
    }
    let is_code = |p: &str| {
        let p = match p.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                call.home.join(rest.trim_start_matches('/'))
            }
            _ => Path::new(p).to_path_buf(),
        };
        let full = (call.real)(&call.cwd.join(p));
        full.starts_with(&code) && full.extension().is_none_or(|e| e != "md")
    };
    let hit = paths.iter().find(|p| !p.is_empty() && is_code(p))?;
    Some(format!(
        "Refused by dash hook main-checkout-code: {hit} is the dashboard's code in the main checkout, which the orchestrator never edits: the app reloads under it. File the change as a task (`private/dashboard/bin/dash task add`) and let its worktree make it, or edit the task's own worktree under ~/.herdr/worktrees."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Component, PathBuf};

    use dash_meter::dash_test;

    /// The guard on one call from the main checkout /m (or a worktree), paths folded as realpath would.
    fn judge(tool: &str, command: &str, file: &str, main: bool) -> Option<String> {
        let list = || Ok(String::new());
        let fold = |p: &Path| {
            let mut out = PathBuf::new();
            for part in p.components() {
                match part {
                    Component::CurDir => {}
                    Component::ParentDir => {
                        out.pop();
                    }
                    other => out.push(other),
                }
            }
            out
        };
        let call = Call {
            tool,
            command,
            file,
            cwd: Path::new("/m"),
            project: Path::new("/m"),
            home: Path::new("/home"),
            panes: &list,
            is_file: &|_| false,
            is_dir: &|_| main,
            read: &|_| None,
            real: &fold,
        };
        refused(&call)
    }

    #[dash_test]
    fn refuses_the_main_checkouts_edit_of_dashboard_code() {
        let why = judge("Edit", "", "/m/private/dashboard/lib/x.ex", true).expect("refused");
        assert!(
            why.starts_with(
                "Refused by dash hook main-checkout-code: /m/private/dashboard/lib/x.ex"
            ),
            "{why}"
        );
        assert!(judge("Write", "", "private/dashboard/lib/a/../b.ex", true).is_some());
        for cmd in [
            "echo x > private/dashboard/lib/x.ex",
            "printf y | tee -a private/dashboard/lib/x.ex",
            "sed -i '' 's/a/b/' private/dashboard/lib/x.ex",
            "true; perl -pi -e 's/a/b/' /m/private/dashboard/test/x_test.exs",
        ] {
            assert!(judge("Bash", cmd, "", true).is_some(), "{cmd}");
        }
    }

    #[dash_test]
    fn passes_docs_a_worktree_and_text_that_names_sed() {
        assert_eq!(
            judge("Edit", "", "/m/private/dashboard/CLAUDE.md", true),
            None
        );
        assert_eq!(
            judge("Edit", "", "/m/private/dashboard/lib/x.ex", false),
            None
        );
        assert_eq!(judge("Edit", "", "/m/private/notes/x.ex", true), None);
        for cmd in [
            "git commit -m \"sed -i private/dashboard/lib/x.ex\"",
            "dash pane pick 1; sed -i '' s/a/b/ to_process/x.md",
            "cat private/dashboard/lib/x.ex",
            "echo x > private/dashboard/README.md",
        ] {
            assert_eq!(judge("Bash", cmd, "", true), None, "{cmd}");
        }
    }
}
