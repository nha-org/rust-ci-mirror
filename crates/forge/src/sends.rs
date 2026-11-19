//! The paths where a change sends something in his name (task rust-land-waits-review): a PR touching one lands only after a
//! reviewer's verdict (`dash_core::tasks::review_needed`).

/// Where a change sends something in his name, or decides whether it goes (the outbound gate): mail, LinkedIn, a form, the
/// hooks that gate them. A PR touching one lands only after a reviewer's verdict (@nha, 2026-11-11: « I approved so this is up to
/// you to send automatically. »; PR 1490, the outbound gate, merged on green mid-review, and its review then found a double
/// send and three fail-open paths). A prefix: a folder holds every file under it.
pub const SEND_PATHS: [&str; 20] = [
    // this list: a change to it is reviewed as any send path
    "rust/crates/forge/src/sends.rs",
    "rust/crates/mail/src/",
    "rust/crates/drafts/src/send.rs",
    "rust/crates/drafts/src/outbox.rs",
    "rust/crates/drafts/src/posting.rs",
    "rust/crates/hooks/src/outbound.rs",
    "rust/crates/hooks/src/send.rs",
    // wires the outbound log
    "rust/crates/hooks/src/lib.rs",
    "rust/crates/dash/src/mail_send.rs",
    "rust/crates/dash/src/mail.rs",
    "rust/crates/dash/src/apply.rs",
    "rust/crates/dash/src/claude_hooks.rs",
    // names the task of a LinkedIn call, so its brief's limit (send-path review of PR 1574)
    "rust/crates/forge/src/folder.rs",
    "private/dashboard/priv/commands/klar-mail",
    "private/dashboard/priv/commands/mcp-linkedin",
    // the Elixir app's senders, archived and frozen (RFC-0025): a delete is the one change left, and it waits too
    "archive/elixir-dashboard/lib/dashboard/linkedin",
    "archive/elixir-dashboard/lib/dashboard/easy_apply.ex",
    "archive/elixir-dashboard/lib/dashboard/freework.ex",
    ".claude/skills/nicolas-daily/freework_apply.py",
    // the hooks it registers, the outbound gate's among them
    ".claude/settings.json",
];

/// The files of `files` on a send path (`SEND_PATHS`), in order.
pub fn sends<'a>(files: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let sent = |f: &&str| SEND_PATHS.iter().any(|p| f.starts_with(p));
    files.into_iter().filter(sent).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task rust-land-waits-review: a send path is named, under a folder or as a file; any other file is not.
    #[dash_test]
    fn a_send_path_is_a_file_or_a_folder_of_the_list() {
        let files = [
            "README.md",
            "rust/crates/mail/src/jmap.rs",
            ".claude/settings.json",
            "rust/crates/mail/tests/a.rs",
        ];
        assert_eq!(
            sends(files),
            ["rust/crates/mail/src/jmap.rs", ".claude/settings.json"]
        );
    }

    /// PR 1631 (dash restart) changed the hooks plugin's lib.rs, a send path: it merged on green with no reviewer's pass.
    #[dash_test]
    fn pr_1631_s_files_reach_a_send_path_through_the_hooks_lib() {
        let files = [
            "explainer.d/rust-self-restart.md",
            "private/dashboard/bin/dash",
            "rust/crates/hooks/Cargo.toml",
            "rust/crates/hooks/src/lib.rs",
            "rust/crates/hooks/src/restart.rs",
        ];
        assert_eq!(sends(files), ["rust/crates/hooks/src/lib.rs"]);
    }

    /// RFC-0025: the Elixir senders moved to the archive, so a delete of one there still waits for a reviewer.
    #[dash_test]
    fn an_archived_sender_is_a_send_path() {
        // the senders' own entries, and a path the archive does not hold: dash-gate's nothing_runs_from_archive passes this
        // file only for those entries
        let files = [
            "archive/elixir-dashboard/lib/dashboard/easy_apply.ex",
            "archive/elixir-dashboard/lib/dashboard/freework.ex",
            "archive/elixir-dashboard/lib/dashboard/not_a_sender.ex",
        ];
        assert_eq!(sends(files), files[..2]);
    }
}
