//! A stalled PR acted on before anyone is woken (task rust-stalled-port; @nha, 2026-11-13: « do you get waked up by the tasks
//! being stuck ? will it be part of the rust port ? »). Ports the decision of the older app's `Dashboard.Orch.Stalled`, which
//! only woke: the orchestrator then merged main or landed by hand, 6 times on 2026-11-13. Here a stall whose red is main's own
//! gets main merged in, a green one with no live run is landed, and only what neither moves wakes, with what was tried.
//! Its two rules come along: a pane waiting on its background shell is at work (task stall-reads-shells), and a PR whose task
//! waits on another open PR or on his answer is no stall (task stall-reads-blocked-by).
use crate::{at_work, shells_running};
use dash_core::tasks::{
    Base, Checks, Hold, Pr, Queued, Seen, Standing, TaskStore, Terminal, all_waits,
};
use std::time::Duration;

/// Seconds a head may sit with nobody on it: a reviewer starts within a pass or two of a task's stop.
pub const SETTLE_S: i64 = 10 * 60;
/// Seconds a head may sit whoever holds it: a turn that has not pushed in an hour is no work on its PR.
pub const STILL_S: i64 = 3600;
/// A stall still there is told again once its head has sat this much longer.
pub const AGAIN_S: i64 = 4 * 3600;

/// One open PR, as the pass reads it.
pub use dash_core::tasks::OpenPr as Open;

/// A pane and the task its folder is, as the terminal shows it.
pub struct Pane {
    pub pane: String,
    pub task: String,
    pub seen: Seen,
    /// Its footer shows a background shell, monitor or agent still running (`crate::shells_running`).
    pub shells: bool,
    /// Its dialog waits on his answer (`Terminal::question`).
    pub question: bool,
}

/// Who holds an open PR.
#[derive(Clone, Debug, PartialEq)]
pub enum On {
    Nobody,
    /// At work on it, in words: a Rust run, a session mid-turn or on its background shell.
    Working(String),
    /// Its task waits on someone else, in words: never a stall.
    Waits(String),
}

/// Whether `pane` works for `task`: its own, a phase `<task>-<phase>` of it, or its reviewer `review-<task>`.
fn of(pane: &str, task: &str) -> bool {
    pane == task
        || pane == format!("review-{task}")
        || pane.strip_prefix(task).is_some_and(|p| p.starts_with('-'))
}

/// Who holds `open`: `claimed`, the tasks a Rust run claims, then the panes of its task, then its task's waits (`waits`, its
/// blocked-by and the list's), against `open_prs`, every open PR's link. A pane asking him is a wait; one mid-turn or on its
/// background shell is at work (`at_work`, with no cap: an hour of head still tells it anyway).
pub fn on(
    open: &Open,
    claimed: &[String],
    panes: &[Pane],
    waits: &[String],
    open_prs: &[String],
) -> On {
    let Some(task) = &open.task else {
        return On::Nobody;
    };
    // a run of a phase or of its reviewer holds it as its pane would (/code-review)
    if let Some(run) = claimed.iter().find(|c| of(c, task)) {
        return On::Working(format!("Rust's run of {run}"));
    }
    let mine = panes.iter().filter(|p| of(&p.task, task));
    for p in mine {
        if p.question {
            return On::Waits(format!("pane {} asks his answer", p.pane));
        }
        let shells = p.shells.then_some(Duration::ZERO);
        if at_work(&p.seen, shells, Duration::MAX) {
            let how = if p.shells && p.seen.idle {
                "waits on its background shell"
            } else {
                "is at work"
            };
            return On::Working(format!("{} {how} in pane {}", p.task, p.pane));
        }
    }
    let other = |w: &str| {
        (open_prs.iter())
            .filter(|u| **u != open.pr.url)
            .find(|u| linked(w, u) || u.rsplit('/').next().is_some_and(|n| names(w, n)))
            .cloned()
    };
    for w in waits {
        if w.trim_start().starts_with("his:") {
            return On::Waits(format!("{task} waits on him ({})", w.trim()));
        }
        if let Some(u) = other(w) {
            return On::Waits(format!("{task} waits on {u}"));
        }
    }
    On::Nobody
}

/// Whether `w` holds the link `u` whole: `.../pull/147` is not in a wait naming `.../pull/1476` (/code-review).
fn linked(w: &str, u: &str) -> bool {
    (w.match_indices(u)).any(|(i, _)| !w[i + u.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Whether `w` names PR number `n` as « #n » or « PR n ».
fn names(w: &str, n: &str) -> bool {
    let tail = |at: &str| {
        w.match_indices(at).any(|(i, _)| {
            let rest = &w[i + at.len()..];
            rest.starts_with(n) && !rest[n.len()..].starts_with(|c: char| c.is_ascii_digit())
        })
    };
    tail("#") || tail("PR ")
}

/// Whether `open` has stalled: nobody on it 10 minutes, or an hour whoever; a wait is never one.
pub fn stalled(open: &Open, on: &On) -> bool {
    match on {
        On::Waits(_) => false,
        On::Nobody => open.head_age_s >= SETTLE_S,
        On::Working(_) => open.head_age_s >= STILL_S,
    }
}

/// What a stall can be moved by: the forge's reads, and its two acts.
pub struct Acts<'a> {
    pub base: &'a dyn Fn(&Pr) -> Result<Base, String>,
    pub checks: &'a dyn Fn(&Pr) -> Result<Checks, String>,
    /// Whether its red is main's own: each check red on its head was red too on main where it branched from main.
    pub main_red: &'a dyn Fn(&Pr) -> Result<bool, String>,
    /// Main merged into its branch and pushed, in words.
    pub remerge: &'a dyn Fn(&Open) -> Result<String, String>,
    /// `dash land`: merged, in words.
    pub land: &'a dyn Fn(&Open) -> Result<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Its CI runs: no stall yet, nothing done.
    Runs,
    /// Moved by the pass, in words: nobody woken.
    Moved(String),
    /// Nothing it can do moves it: the orchestrator is woken with what was tried.
    Wake(String),
}

/// What the pass does for a stalled `open`, held `on`: a PR someone works on is never pushed to or landed under them, only
/// woken on, as the older app did (/code-review); else main first, as `gate` reads it, then its checks.
pub fn act(open: &Open, on: &On, acts: &Acts) -> Act {
    if let On::Working(who) = on {
        return Act::Wake(format!(
            "left it alone, as {who} and its head has not moved in an hour: ask what it waits on, or close it"
        ));
    }
    let pr = &open.pr;
    let remerge = |why: &str| match (acts.remerge)(open) {
        Ok(words) => Act::Moved(format!("{why}: main merged in, {words}")),
        Err(e) => Act::Wake(format!("{why}, so merged main in, which failed: {e}")),
    };
    let base = match (acts.base)(pr) {
        Ok(base) => base,
        Err(e) => return Act::Wake(format!("read its base against main, which failed: {e}")),
    };
    if let Base::Conflicts(files) = &base {
        let files = files.join(", ");
        return Act::Wake(format!(
            "read its base: it conflicts with main in {files}, which only a session resolves"
        ));
    }
    let behind = base == Base::Behind;
    match (acts.checks)(pr) {
        Err(e) => Act::Wake(format!("read its CI, which failed: {e}")),
        Ok(Checks::Pending) => Act::Runs,
        Ok(Checks::Green) if behind => remerge("found its CI green with main moved under it"),
        Ok(Checks::Green) => match (acts.land)(open) {
            Ok(words) => Act::Moved(format!("green with no live run: landed, {words}")),
            Err(e) => Act::Wake(format!(
                "found it green with no live run, so ran dash land, which failed: {e}"
            )),
        },
        Ok(Checks::Infra(words)) => Act::Wake(format!(
            "read its CI: red before any test ({words}), no fault of its code and nothing to merge or land"
        )),
        Ok(Checks::Red(jobs)) => match (acts.main_red)(pr) {
            Ok(true) if behind => remerge(&format!("found its red main's own ({jobs})")),
            Ok(true) => Act::Wake(format!(
                "read its CI: red ({jobs}) as main was, but main has no newer commit to merge in"
            )),
            Ok(false) => Act::Wake(format!(
                "read its CI: red on its own code ({jobs}), which its task must fix"
            )),
            Err(e) => Act::Wake(format!(
                "read its CI: red ({jobs}); whether main shares the red could not be read: {e}"
            )),
        },
    }
}

/// The key of its wake: its link and head, a push is a new stall, and the count of `AGAIN_S` its head has sat, so it wakes
/// again then.
pub fn key(open: &Open) -> String {
    format!(
        "stalled {} {} {}",
        open.pr.url,
        open.pr.head,
        open.head_age_s / AGAIN_S
    )
}

/// A span of seconds as « 3 h 20 min » or « 45 min ».
pub fn span(s: i64) -> String {
    match s {
        s if s >= 3600 => format!("{} h {} min", s / 3600, s / 60 % 60),
        s => format!("{} min", s / 60),
    }
}

/// The wake's words: the PR, who holds it, and what was tried.
pub fn text(open: &Open, on: &On, tried: &str) -> String {
    let who = match on {
        On::Nobody => "nobody is on it".to_string(),
        On::Working(w) | On::Waits(w) => w.clone(),
    };
    format!(
        "Open PR {} (branch {}) has stalled: head unmoved {}; {who}. Rust's stall watcher {tried}.",
        open.pr.url,
        open.branch,
        span(open.head_age_s)
    )
}

/// Each listed agent's pane, `(pane, folder)`, its task the folder names (`rust-<id>` for Rust's, `<id>` for the older app's):
/// at work, on its background shell, or asking him, as `terminal` shows it.
pub fn panes(agents: Vec<(String, String)>, terminal: &dyn Terminal) -> Vec<Pane> {
    let mut panes = vec![];
    for (pane, folder) in agents {
        let path = std::path::Path::new(&folder);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        // dash_forge::task_folder's rule: never a half-made `.new` or a claim's `.lock` (/code-review)
        let id = name
            .strip_prefix("rust-")
            .filter(|id| !id.is_empty() && !id.contains('.'));
        let task = id.unwrap_or(name).to_string();
        let Ok(Some(seen)) = terminal.agent(&pane) else {
            continue;
        };
        let shells =
            seen.idle && (terminal.read(&pane)).is_ok_and(|screen| shells_running(&screen));
        let question = seen.asking && terminal.question(&pane, &seen) == Ok(Some(true));
        panes.push(Pane {
            pane,
            task,
            seen,
            shells,
            question,
        });
    }
    panes
}

/// A stall nothing moved: its wake's key, task, words and link.
#[derive(Debug, PartialEq)]
pub struct Wake {
    pub key: String,
    pub task: Option<String>,
    pub title: String,
    pub link: String,
    pub number: u64,
}

/// What the store holds of who is on a PR: the tasks a Rust run claims, the PRs held, each task's row and queue entry.
#[derive(Default)]
pub struct Holders {
    pub claimed: Vec<String>,
    pub holds: Vec<Hold>,
    pub rows: Vec<Standing>,
    pub queue: Vec<Queued>,
}

/// `store`'s holders, each read or none: one unread would show a held PR as nobody's, and the pass would push to it
/// (/code-review).
pub fn holders(store: &dyn TaskStore) -> Result<Holders, String> {
    Ok(Holders {
        claimed: store.claims()?.into_iter().map(|c| c.id).collect(),
        holds: store.holds()?,
        rows: store.standing()?,
        queue: store.queue()?.tasks,
    })
}

/// One pass over `listed`: each stalled PR not `told` on its head yet acted on, and each one nothing moved handed to `wake`
/// at once, never behind a later PR's land, which may wait hours on its CI (/code-review). What moved, in words, and the
/// numbers `wake` took.
pub fn pass(
    listed: &[Open],
    held: &Holders,
    panes: &[Pane],
    told: &dyn Fn(&str) -> bool,
    acts: &Acts,
    wake: &mut dyn FnMut(Wake) -> Result<bool, String>,
) -> Result<(Vec<String>, Vec<u64>), String> {
    let urls: Vec<String> = listed.iter().map(|o| o.pr.url.clone()).collect();
    let (mut moved, mut woke) = (vec![], vec![]);
    for open in listed {
        let key = key(open);
        // told on this head already: no act again until it moves or four more hours pass
        if told(&key) {
            continue;
        }
        let task = open.task.as_deref();
        let waits = task.map_or(vec![], |t| {
            all_waits(
                held.rows.iter().find(|r| r.id == t),
                held.queue.iter().find(|q| q.id == t),
            )
        });
        let on = match held.holds.iter().find(|h| h.pr == open.pr.number) {
            Some(h) => On::Working(format!("{} holds it: {}", h.holder, h.why)),
            None => on(open, &held.claimed, panes, &waits, &urls),
        };
        if !stalled(open, &on) {
            continue;
        }
        match act(open, &on, acts) {
            Act::Runs => {}
            Act::Moved(words) => moved.push(format!("#{} {words}", open.pr.number)),
            Act::Wake(tried) => {
                let number = open.pr.number;
                let w = Wake {
                    title: text(open, &on, &tried),
                    key,
                    task: open.task.clone(),
                    link: open.pr.url.clone(),
                    number,
                };
                if wake(w)? {
                    woke.push(number);
                }
            }
        }
    }
    Ok((moved, woke))
}

/// The pass's line on /watchers.
pub fn line(listed: usize, moved: &[String], woke: usize) -> String {
    let what = match moved {
        [] => String::new(),
        m => format!(": {}", m.join("; ")),
    };
    format!(
        "{listed} open PRs read, {} moved, {woke} woke the orchestrator{what}",
        moved.len()
    )
}
