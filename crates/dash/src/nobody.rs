//! `dash orch --inconsistencies`: the PM pass's last view read, then judged by `dash_hooks::nobody` against Rust's store
//! (task rust-held-by-nobody), each finished task's holds after it.
use dash_hooks::nobody::{Rust, gh_title, judged, nobody_of, text};

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

dash_core::submit! { crate::usage::Command::usage("dash orch --inconsistencies [--data <dir>]") }

/// `dash orch --inconsistencies --data <dir>`: the pass's last view judged against Rust's store in `dir`, then each lease,
/// claim, hold or pane a finished task still holds (`done_holds`, task rust-nothing-done-holds). A store that does not
/// open keeps every line, and says so when one is « held by nobody »; a view not read exits 1, which the hook prints nothing for.
pub fn run(dir: &std::path::Path, mut out: std::fs::File) {
    use std::io::Write;
    let view = dash_store::older::orch_view(&crate::tasks::older_db());
    // no pass's view yet: the finished tasks' holds are read now, so said as of now
    let now = || dash_core::iso(dash_process::now_s() as i64);
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
