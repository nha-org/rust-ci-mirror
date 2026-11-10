//! Core's Launcher: `make claude` and `make codex` from the approved Makefile, kept as they are (section 20b). The line runs on the Mac, outside
//! the sandbox, so it is the approved copy's, never a working tree's, which any session may have edited. That copy's target
//! is sandbox-apply's launch: the job's profile as he approved it, the Claude account of this start.
//! ponytail: site_perso's copy only, as Rust tasks are all site_perso's; another project's reading once one moves.
pub mod brief;

use dash_core::tasks::{Launcher, Task};

pub mod mxc;

pub struct ApprovedMake {
    pub home: String,
}

/// The approved copy, outside every working tree: its Makefile and its sandbox-apply.
fn host(home: &str) -> String {
    format!("{home}/.config/site_perso/host")
}

fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Claude's own arguments on make's line: the model its pool names (RFC 20h), then a session to resume. None: no CLAUDE_ARGS.
fn claude_args(model: Option<&str>, resume: Option<&str>) -> String {
    // a pool's name is letters, digits and dashes (`TaskStore::route` refuses any other), as `--resume`'s id
    let model = model.map(|m| format!("--model {m}"));
    let resume = resume.map(|s| format!("--resume {s}"));
    let args: Vec<String> = model.into_iter().chain(resume).collect();
    match args.is_empty() {
        true => String::new(),
        false => format!(" CLAUDE_ARGS={}", quoted(&args.join(" "))),
    }
}

impl Launcher for ApprovedMake {
    // An empty PHONE_NAME: no Remote Control, as a task session has none. DASH_SESSION: our launch marker, for the load watch.
    // HERDR_DELEGATE: a delegate, as session.sh's panes: no orchestrator's hooks at its start. CLAUDE_ACCOUNT: the launch's own
    // pick otherwise, as session.sh names it. None tells Elixir's runner of a task: it reads tasks from their folder. The file
    // is not checked here: the caller's sandbox cannot see it (Operation not permitted, 2026-11-06), and a line that fails
    // shows on the pane.
    fn command(&self, task: &Task, account: Option<&str>) -> Result<String, String> {
        let session = quoted(&format!("rust-{}", task.id));
        let makefile = quoted(&format!("{}/Makefile", host(&self.home)));
        let account = account.map_or(String::new(), |a| format!("CLAUDE_ACCOUNT={} ", quoted(a)));
        let args = claude_args(task.model.as_deref(), None);
        Ok(format!(
            "HERDR_DELEGATE=1 {account}PHONE_NAME= DASH_SESSION={session} make -f {makefile} claude{args}"
        ))
    }

    // CLAUDE_ARGS on make's line, as the Makefile's own claude target hands it to the approved copy; claude-supervise takes
    // CLAUDE_ACCOUNT over the account the thread log recorded for that id. CLAUDE_MOVE_FROM_PANE, as session.sh's move: the
    // supervisor of the pane it leaves may hold the id a moment more, and its refuse_held would end this start.
    fn resume(
        &self,
        task: &Task,
        account: Option<&str>,
        session: &str,
        from: &str,
    ) -> Result<String, String> {
        // the line without its model, which the resume names beside the session
        let bare = Task {
            model: None,
            ..task.clone()
        };
        let line = self.command(&bare, account)?;
        let args = claude_args(task.model.as_deref(), Some(session));
        Ok(format!(
            "CLAUDE_MOVE_FROM_PANE={} {line}{args}",
            quoted(from)
        ))
    }
}

/// `make codex` from the approved Makefile: Codex in the same sandbox as Claude, on his one ChatGPT login, so no account.
pub struct ApprovedCodex {
    pub home: String,
    /// Codex's first words: dash-agent's `codex::OPENER`, which tells its turn from a brief's.
    pub opener: &'static str,
}

/// The model his free login runs: `codex exec` refused gpt-5.5 and his configured gpt-6.1-sol (« not supported when using Codex
/// with a ChatGPT account », 2026-11-05) and answered PONG on gpt-5.6-terra (2026-11-08).
pub const CODEX_MODEL: &str = "gpt-5.6-terra";

impl Launcher for ApprovedCodex {
    fn command(&self, task: &Task, _account: Option<&str>) -> Result<String, String> {
        let session = quoted(&format!("rust-{}", task.id));
        let makefile = quoted(&format!("{}/Makefile", host(&self.home)));
        let args = quoted(&format!("-m {CODEX_MODEL} {}", quoted(self.opener)));
        Ok(format!(
            "HERDR_DELEGATE=1 DASH_SESSION={session} make -f {makefile} codex CODEX_ARGS={args}"
        ))
    }

    // ponytail: one login, so no limit moves a Codex session; `codex resume <id>` once one could.
    fn resume(
        &self,
        _task: &Task,
        _account: Option<&str>,
        _session: &str,
        _from: &str,
    ) -> Result<String, String> {
        Err("Codex has one login: no session of it moves".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_approved_makefile() {
        let _budget = dash_meter::memory::TestBudget::new();
        let (id, folder, prompt) = ("it's".into(), "/work".into(), "x".into());
        let task = Task {
            id,
            folder,
            prompt,
            model: None,
        };
        let launcher = ApprovedMake { home: "/h".into() };
        assert_eq!(
            launcher.command(&task, None).expect("line"),
            r"HERDR_DELEGATE=1 PHONE_NAME= DASH_SESSION='rust-it'\''s' make -f '/h/.config/site_perso/host/Makefile' claude"
        );
        assert_eq!(
            launcher.command(&task, Some("second")).expect("line"),
            r"HERDR_DELEGATE=1 CLAUDE_ACCOUNT='second' PHONE_NAME= DASH_SESSION='rust-it'\''s' make -f '/h/.config/site_perso/host/Makefile' claude"
        );
        assert_eq!(
            launcher
                .resume(&task, Some("default"), "s-1", "w2:p1")
                .expect("line"),
            r"CLAUDE_MOVE_FROM_PANE='w2:p1' HERDR_DELEGATE=1 CLAUDE_ACCOUNT='default' PHONE_NAME= DASH_SESSION='rust-it'\''s' make -f '/h/.config/site_perso/host/Makefile' claude CLAUDE_ARGS='--resume s-1'"
        );
        // RFC 20h: the pool's model on the line, and kept when a limit moves the session to another login
        let task = Task {
            model: Some("fable".into()),
            ..task
        };
        assert_eq!(
            launcher.command(&task, Some("second")).expect("line"),
            r"HERDR_DELEGATE=1 CLAUDE_ACCOUNT='second' PHONE_NAME= DASH_SESSION='rust-it'\''s' make -f '/h/.config/site_perso/host/Makefile' claude CLAUDE_ARGS='--model fable'"
        );
        assert_eq!(
            launcher
                .resume(&task, Some("default"), "s-1", "w2:p1")
                .expect("line"),
            r"CLAUDE_MOVE_FROM_PANE='w2:p1' HERDR_DELEGATE=1 CLAUDE_ACCOUNT='default' PHONE_NAME= DASH_SESSION='rust-it'\''s' make -f '/h/.config/site_perso/host/Makefile' claude CLAUDE_ARGS='--model fable --resume s-1'"
        );
    }

    #[test]
    fn codex_s_line_names_the_approved_makefile_its_model_and_opener() {
        let _budget = dash_meter::memory::TestBudget::new();
        let (id, folder, prompt) = ("t".into(), "/work".into(), "x".into());
        let task = Task {
            id,
            folder,
            prompt,
            model: None,
        };
        let opener = "The brief of your task comes in the next message: wait for it.";
        let home = "/h".into();
        let launcher = ApprovedCodex { home, opener };
        let line = launcher.command(&task, Some("second")).expect("line");
        assert_eq!(
            line,
            r"HERDR_DELEGATE=1 DASH_SESSION='rust-t' make -f '/h/.config/site_perso/host/Makefile' codex CODEX_ARGS='-m gpt-5.6-terra '\''The brief of your task comes in the next message: wait for it.'\'''"
        );
        assert!(!line.contains("CLAUDE_ACCOUNT"), "one login");
    }
}
