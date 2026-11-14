//! `dash orch --inconsistencies`, the orchestrator's prompt hook: the PM pass's last inconsistencies, each « held by nobody »
//! line judged again against Rust's own store (task rust-held-by-nobody). The older app's pass reads its own holders (a task or
//! reviewer pane, the lane, its merge on green) but not Rust's: it called PRs 1387, 1391 and 1401 held by nobody while a Rust
//! run held each, and a slice's PR (PR 1435, branch task/rust-doc-core-pdf) never counted as its task's. Here a PR whose task a
//! Rust run claims, whose hand-back is pending, or which a Rust hold holds, is held; `Dashboard.Orch.View`'s line no longer
//! reaches the hook unjudged.
//! ponytail: the older pass's line is the candidate, so its holders stay its own; a full Rust read of the open PRs once the
//! panes, lane and merge on green are Rust's.
use dash_core::tasks::{Hold, Standing, handed_back};

/// What Rust's store holds, read once a call.
pub struct Rust {
    /// The tasks a Rust run claims or whose hand-back is pending.
    held: Vec<String>,
    /// The PRs a Rust hold holds (`dash pr hold`, a run's), site_perso's.
    holds: Vec<u64>,
}

impl Rust {
    pub fn of(mut claimed: Vec<String>, rows: &[Standing], holds: &[Hold]) -> Rust {
        let back = rows
            .iter()
            .filter(|r| handed_back(std::slice::from_ref(*r), &r.id).is_some());
        claimed.extend(back.map(|r| r.id.clone()));
        Rust {
            held: claimed,
            holds: holds.iter().map(|h| h.pr).collect(),
        }
    }
}

/// A « held by nobody » line's PR link, project and branch, as `Dashboard.Orch.View` writes it.
fn nobody_of(line: &str) -> Option<(&str, &str, &str)> {
    let (url, rest) = line.strip_prefix("PR ")?.split_once(" (")?;
    let (inside, _) = rest.split_once(") is held by nobody: ")?;
    let (project, branch) = inside.split_once(", branch ")?;
    Some((url, project, branch))
}

/// The task a PR title names: `[kind][<id>] …`, as CI requires of every task's PR.
fn titled(title: &str) -> Option<&str> {
    let rest = title.strip_prefix('[')?.split_once("][")?.1;
    rest.split_once(']').map(|(id, _)| id)
}

/// Whether Rust holds the PR. Rust runs site_perso's tasks only, each on its branch `task/<id>`. `title` is asked only for a
/// branch `task/<id>-<slice>` of a held task: a slice counts as its task's when its title names `[kind][<id>]`.
fn held(
    rust: &Rust,
    url: &str,
    project: &str,
    branch: &str,
    title: impl FnOnce() -> Option<String>,
) -> bool {
    if project != "site_perso" {
        return false;
    }
    let number = url.rsplit('/').next().and_then(|n| n.parse::<u64>().ok());
    if number.is_some_and(|n| rust.holds.contains(&n)) {
        return true;
    }
    let Some(id) = branch.strip_prefix("task/") else {
        return false;
    };
    let slice_of = |h: &str| id.strip_prefix(h).is_some_and(|rest| rest.starts_with('-'));
    if rust.held.iter().any(|h| h == id) {
        return true;
    }
    // GitHub asked only when a held task could own it as a slice: a free PR costs the hook no network
    if !rust.held.iter().any(|h| slice_of(h)) {
        return false;
    }
    let named = title().and_then(|t| titled(&t).map(str::to_string));
    named.is_some_and(|n| slice_of(&n) && rust.held.contains(&n))
}

/// The lines still true: each « held by nobody » line whose PR Rust holds is dropped, every other line kept as the pass wrote it.
pub fn judged(
    lines: Vec<String>,
    rust: &Rust,
    title: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let still = |line: &String| match nobody_of(line) {
        Some((url, project, branch)) => !held(rust, url, project, branch, || title(url)),
        None => true,
    };
    lines.into_iter().filter(still).collect()
}

/// `Stalled.span/1`'s words: hours and minutes past an hour.
fn span(s: i64) -> String {
    let s = s.max(0);
    match s >= 3600 {
        true => format!("{} h {} min", s / 3600, s / 60 % 60),
        false => format!("{} min", s / 60),
    }
}

/// The hook's text, as `View.inconsistencies_text/1` printed it: nothing when none, else a dated head and a line each.
pub fn text(at: &str, age_s: i64, lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let read = at.get(..16).unwrap_or(at).replacen('T', " ", 1);
    let head = format!(
        "Inconsistencies ({}, as read {read}Z, {} ago):",
        lines.len(),
        span(age_s)
    );
    let body: String = lines.iter().map(|l| format!("  {l}\n")).collect();
    format!("{head}\n{body}")
}

/// A PR's title from GitHub, `None` when gh cannot say within 2 s: the hook's whole answer has 5 (/code-review).
pub fn gh_title(url: &str) -> Option<String> {
    let args = ["pr", "view", url, "--json", "title", "-q", ".title"];
    let said = dash_process::run("gh", &args, None, std::time::Duration::from_secs(2)).ok()?;
    Some(said.trim().to_string()).filter(|t| !t.is_empty())
}

/// Rust's store as `held` reads it.
fn read(tasks: &dash_store::tasks::SqliteTasks) -> Result<Rust, String> {
    use dash_core::tasks::TaskStore;
    let claimed = tasks.claims()?.into_iter().map(|c| c.id).collect();
    Ok(Rust::of(claimed, &tasks.standing()?, &tasks.holds()?))
}

/// Each lease, claim, hold or pane a finished task holds (task rust-nothing-done-holds), then each part of the store or the list
/// unread. herdr is asked for a second at most, the hook's whole answer having 5; unread, its panes go unnamed here, as the
/// hourly reap says it and releases nothing then (/code-review of 9a61ee07f: a line at every prompt outside herdr).
fn done_holds(tasks: &dash_store::tasks::SqliteTasks) -> Vec<String> {
    use crate::tasks::{older_db, worktrees};
    let herdr = dash_terminal::Herdr::from_env().limited(std::time::Duration::from_secs(1));
    let panes = Ok(herdr.live().unwrap_or_default());
    let (held, mut unread) =
        crate::done_holds::read(tasks, false, &worktrees(), &older_db(), panes);
    let mut lines = held.lines();
    lines.append(&mut unread);
    lines
}

inventory::submit! { crate::usage::Command::usage("dash orch --inconsistencies [--data <dir>]") }

/// `dash orch --inconsistencies --data <dir>`: the pass's last view judged against Rust's store in `dir`, then each lease,
/// claim, hold or pane a finished task still holds (`done_holds`, task rust-nothing-done-holds). A store that does not
/// open keeps every line, and says so when one is « held by nobody »; a view not read exits 1, which the hook prints nothing for.
pub fn run(dir: &std::path::Path, mut out: std::fs::File) {
    use std::io::Write;
    let view = dash_store::older::orch_view(&crate::tasks::older_db());
    // no pass's view yet: the finished tasks' holds are read now, so said as of now
    let now = || {
        dash_core::iso(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64),
        )
    };
    let (at, age, lines) = match view {
        Ok(Some(view)) => view,
        Ok(None) => (now(), 0, vec![]),
        Err(e) => {
            eprintln!("orch --inconsistencies: {e}");
            std::process::exit(1);
        }
    };
    // opened once for both reads (/code-review of 9a61ee07f)
    let tasks = crate::tasks::open_tasks(dir);
    let (mut lines, unread) = match tasks.as_ref().map_err(String::clone).and_then(read) {
        Ok(rust) => (judged(lines, &rust, gh_title), None),
        Err(e) => (lines, Some(e)),
    };
    if let Some(e) = unread.filter(|_| lines.iter().any(|l| nobody_of(l).is_some())) {
        lines.push(format!(
            "held-by-nobody lines unjudged: Rust's store unread: {e}"
        ));
    }
    match &tasks {
        Ok(tasks) => lines.extend(done_holds(tasks)),
        Err(e) => lines.push(format!("finished tasks' holds unread: Rust's store: {e}")),
    }
    write!(out, "{}", text(&at, age, &lines)).expect("stdout");
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/nha-org/site_perso/pull/";

    fn line(n: u64, project: &str, branch: &str) -> String {
        format!(
            "PR {URL}{n} ({project}, branch {branch}) is held by nobody: no task, reviewer, lane or merge on it"
        )
    }

    fn claiming(ids: &[&str]) -> Rust {
        Rust::of(ids.iter().map(|i| i.to_string()).collect(), &[], &[])
    }

    fn no_title(_: &str) -> Option<String> {
        panic!("no title asked")
    }

    /// Done-when 1: a PR whose task a Rust run claims is never listed as held by nobody (PRs 1387, 1391 and 1401).
    #[dash_meter::dash_test]
    fn a_pr_whose_task_a_rust_run_claims_is_not_held_by_nobody() {
        let held = claiming(&["rust-terminal-no-hc"]);
        let claimed = line(1387, "site_perso", "task/rust-terminal-no-hc");
        let free = line(1449, "site_perso", "task/rust-window-compass");
        let other = "task t has no pane: its claim's pane 9 is gone (last status none)".to_string();
        let lines = vec![claimed, free.clone(), other.clone()];
        // a free PR no held task could own as a slice asks GitHub nothing
        let kept = judged(lines, &held, no_title);
        assert_eq!(kept, vec![free, other]);
    }

    /// Done-when 1: a PR whose task's hand-back is pending is never listed as held by nobody; one whose run since finished is.
    #[dash_meter::dash_test]
    fn a_pr_whose_hand_back_is_pending_is_not_held_by_nobody() {
        let row = |id: &str, back: bool| Standing {
            id: id.into(),
            status: "open".into(),
            handed: Some("abc123 1".into()),
            back,
            ..Default::default()
        };
        let held = Rust::of(vec![], &[row("rust-a", true), row("rust-b", false)], &[]);
        let pending = line(1391, "site_perso", "task/rust-a");
        let finished = line(1392, "site_perso", "task/rust-b");
        let kept = judged(vec![pending, finished.clone()], &held, |_| None);
        assert_eq!(kept, vec![finished]);
    }

    /// Done-when 1: a PR a Rust hold holds (`dash pr hold`, a run's) is held, by number on site_perso only.
    #[dash_meter::dash_test]
    fn a_pr_a_rust_hold_holds_is_not_held_by_nobody() {
        let hold = Hold {
            pr: 1401,
            holder: "run rust-x".into(),
            why: "its run works on it".into(),
            touched_ms: 0,
        };
        let held = Rust::of(vec![], &[], &[hold]);
        let site = line(1401, "site_perso", "fix-something");
        let elsewhere = line(1401, "replicator", "task/rust-x");
        let kept = judged(vec![site, elsewhere.clone()], &held, |_| None);
        assert_eq!(kept, vec![elsewhere]);
    }

    /// Done-when 2: PR 1435's shape, branch task/rust-doc-core-pdf titled « [feat][rust-doc-core] … », counts as rust-doc-core's.
    #[dash_meter::dash_test]
    fn a_slice_s_pr_counts_as_its_task_s() {
        let held = claiming(&["rust-doc-core"]);
        let slice = line(1435, "site_perso", "task/rust-doc-core-pdf");
        let title = |url: &str| {
            assert_eq!(url, format!("{URL}1435"));
            Some(
                "[feat][rust-doc-core] The PDF text-layer reader plugs into the extraction core"
                    .into(),
            )
        };
        assert_eq!(
            judged(vec![slice.clone()], &held, title),
            Vec::<String>::new()
        );
        // a title naming another task, or a branch that is no slice of the one named, stays nobody's
        let other = |_: &str| Some("[feat][rust-doc-other] x".into());
        assert_eq!(judged(vec![slice.clone()], &held, other), vec![slice]);
        let unrelated = line(1436, "site_perso", "task/rust-doc-corepdf");
        let named = |_: &str| Some("[feat][rust-doc-core] x".into());
        assert_eq!(
            judged(vec![unrelated.clone()], &held, named),
            vec![unrelated]
        );
        // its own task's claim needs no title
        let held = claiming(&["rust-doc-core", "rust-doc-core-pdf"]);
        let own = line(1437, "site_perso", "task/rust-doc-core-pdf");
        assert!(judged(vec![own], &held, no_title).is_empty());
    }

    /// Done-when 3: the hook's text, nothing when every line went, else the pass's head over the lines kept.
    #[dash_meter::dash_test]
    fn the_hook_s_text_counts_the_lines_kept() {
        assert_eq!(text("2026-11-11T19:32:51.436221", 300, &[]), "");
        let lines = vec![line(1449, "site_perso", "task/rust-window-compass")];
        let said = text("2026-11-11T19:32:51.436221", 3900, &lines);
        assert_eq!(
            said,
            format!(
                "Inconsistencies (1, as read 2026-11-11 19:32Z, 1 h 5 min ago):\n  {}\n",
                lines[0]
            )
        );
    }
}
