//! `dash review next`, ported from the older app's `Dashboard.Orch.PrLane.next` (task rust-review-next-port): the PR lane's next
//! PR from Rust's store and the forge alone, so the lane never waits on the older app (it waited over an hour on 2026-11-12
//! while PRs 1460, 1480, 1484 and 1485 sat green). The lane's claim is a hold (`LANE`), as `dash pr hold`'s and a run's are, so
//! one table says who works on a PR.
use dash_core::tasks::{LANE, LANE_PARKED, TaskStore};

/// An open PR of the lane's repository, as the forge lists it. `created`: ISO 8601, which sorts as time.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub number: u64,
    pub url: String,
    pub branch: String,
    pub created: String,
}

/// What the lane reads beside the store.
pub trait Lane {
    /// Its repository's open PRs, drafts left out.
    fn open(&self) -> Result<Vec<Open>, String>;
    /// Whether its head's checks are all green, as `dash ci wait` reads them. Err: no verdict.
    fn green(&self, pr: &Open) -> Result<bool, String>;
    /// The tasks whose PR a reviewer works on now (a `review-<id>` session open), by task id.
    fn reviewed(&self) -> Result<Vec<String>, String>;
}

#[derive(Debug, PartialEq)]
pub enum Next {
    /// The lane's own claim on a PR still open: a restarted lane resumes it (the older app's task review-next-held).
    Held(Open),
    /// Claimed now.
    Took(Open),
    None,
}

const WHY: &str = "dash review next";

/// The PR the lane holds, its hold touched; else the oldest green PR no run, reviewer or hold claims, claimed for the lane.
/// A PR whose task has a row of Rust's store not finished is its run's: Rust's orchestrator merges it.
pub fn next(lane: &dyn Lane, tasks: &mut dyn TaskStore) -> Result<Next, String> {
    let mut open = lane.open()?;
    open.sort_by(|a, b| a.created.cmp(&b.created));
    let holds = tasks.holds()?;
    let holder = |n: u64| holds.iter().find(|h| h.pr == n).map(|h| h.holder.as_str());
    if let Some(pr) = open.iter().find(|p| holder(p.number) == Some(LANE)) {
        tasks.hold_pr(pr.number, LANE, WHY)?;
        return Ok(Next::Held(pr.clone()));
    }
    let rows = tasks.standing()?;
    let reviewed = lane.reviewed()?;
    let owned = |id: &str| {
        rows.iter().any(|r| r.id == id && r.status != "finished")
            || reviewed.iter().any(|r| r == id)
    };
    // a PR whose checks did not answer is passed over; with nothing taken, the first such error is the answer, never « none »
    let mut unread = None;
    for pr in open {
        if holder(pr.number).is_some() || pr.branch.strip_prefix("task/").is_some_and(owned) {
            continue;
        }
        match lane.green(&pr) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                unread.get_or_insert(format!("#{}'s checks: {e}", pr.number));
                continue;
            }
        }
        match tasks.hold_pr(pr.number, LANE, WHY) {
            Ok(_) => return Ok(Next::Took(pr)),
            // taken since the read above
            Err(e) if e.starts_with(&format!("#{} is held by ", pr.number)) => continue,
            Err(e) => return Err(e),
        }
    }
    unread.map_or(Ok(Next::None), Err)
}

/// The lane is done with `pr` for now, handed to the merge on green or waiting on him: passed over for `LANE_HOLD_MS`, as the
/// older app's `PrLane.parked`. False: the lane did not hold it.
pub fn park(tasks: &mut dyn TaskStore, pr: u64, why: &str) -> Result<bool, String> {
    let ours = (tasks.holds()?.iter()).any(|h| h.pr == pr && h.holder == LANE);
    // one write, which takes over the lane's own hold and nobody else's (`hold_pr`)
    if ours {
        tasks.hold_pr(pr, LANE_PARKED, why)?;
    }
    Ok(ours)
}
