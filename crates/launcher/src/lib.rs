//! Core's Launcher: `make claude` from the approved Makefile, kept as it is (section 20b). The line runs on the Mac, outside
//! the sandbox, so it is the approved copy's, never a working tree's, which any session may have edited. That copy's target
//! is sandbox-apply's launch: the job's profile as he approved it, the Claude account of this start.
//! ponytail: site_perso's copy only, as Rust tasks are all site_perso's; another project's reading once one moves.
use dash_core::tasks::{Launcher, Task};

pub mod mxc;

pub struct ApprovedMake {
    pub home: String,
}

fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

impl Launcher for ApprovedMake {
    // An empty PHONE_NAME: no Remote Control, as a task session has none. DASH_SESSION: our launch marker, for the load watch.
    // HERDR_DELEGATE: a delegate, as session.sh's panes: no orchestrator's hooks at its start. CLAUDE_ACCOUNT: the launch's own
    // pick otherwise, as session.sh names it. None tells Elixir's runner of a task: it reads tasks from their folder. The file
    // is not checked here: the caller's sandbox cannot see it (Operation not permitted, 2026-11-06), and a line that fails
    // shows on the pane.
    fn command(&self, task: &Task, account: Option<&str>) -> Result<String, String> {
        let session = quoted(&format!("rust-{}", task.id));
        let makefile = quoted(&format!("{}/.config/site_perso/host/Makefile", self.home));
        let account = account.map_or(String::new(), |a| format!("CLAUDE_ACCOUNT={} ", quoted(a)));
        Ok(format!(
            "HERDR_DELEGATE=1 {account}PHONE_NAME= DASH_SESSION={session} make -f {makefile} claude"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_approved_makefile() {
        let _budget = dash_meter::memory::TestBudget::new();
        let (id, folder, prompt) = ("it's".into(), "/work".into(), "x".into());
        let task = Task { id, folder, prompt };
        let launcher = ApprovedMake { home: "/h".into() };
        assert_eq!(
            launcher.command(&task, None).expect("line"),
            r"HERDR_DELEGATE=1 PHONE_NAME= DASH_SESSION='rust-it'\''s' make -f '/h/.config/site_perso/host/Makefile' claude"
        );
        assert_eq!(
            launcher.command(&task, Some("second")).expect("line"),
            r"HERDR_DELEGATE=1 CLAUDE_ACCOUNT='second' PHONE_NAME= DASH_SESSION='rust-it'\''s' make -f '/h/.config/site_perso/host/Makefile' claude"
        );
    }
}
