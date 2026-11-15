//! `dash review next`, ported from the older app's `Dashboard.Orch.PrLane.next` (task rust-review-next-port): the PR lane's next
//! PR from Rust's store and the forge alone, so the lane never waits on the older app (it waited over an hour on 2026-11-12
//! while PRs 1460, 1480, 1484 and 1485 sat green). The lane's claim is a hold (`LANE`), as `dash pr hold`'s and a run's are, so
//! one table says who works on a PR.
use dash_core::tasks::{LANE, LANE_PARKED, TaskStore};
use std::path::Path;

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
    /// The PRs a live review claim holds, the lane's own left out, as the pre-push hook reads them (task rust-review-claims-port).
    fn claimed(&self) -> Result<Vec<u64>, String> {
        Ok(vec![])
    }
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

/// The PR the lane holds, its hold touched; else the oldest green PR no run, reviewer, hold or review claim holds, claimed for the
/// lane.
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
    let claimed = lane.claimed()?;
    let owned = |id: &str| {
        rows.iter().any(|r| r.id == id && r.status != "finished")
            || reviewed.iter().any(|r| r == id)
    };
    // a PR whose checks did not answer is passed over; with nothing taken, the first such error is the answer, never « none »
    let mut unread = None;
    for pr in open {
        let taken = holder(pr.number).is_some() || claimed.contains(&pr.number);
        if taken || pr.branch.strip_prefix("task/").is_some_and(owned) {
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

/// What `dash review next` prints for a PR: its link, where to work it, then the lane's steps (the older app's `PrLane.steps`,
/// cut to what the lane's own prompt leaves out).
pub fn steps(pr: &Open, wt: Option<&Path>) -> String {
    let url = &pr.url;
    let n = pr.number;
    let dash = "private/dashboard/bin/dash";
    let place = wt.map_or(String::new(), |wt| {
        let wt = wt.display();
        format!(
            "This PR's own worktree is {wt}, made for it by dash review next: cd there, git fetch origin, git switch --detach \
             origin/{branch}, then run every step there, never in another PR's or your own checkout, whose merges a push would \
             carry. Once the PR is merged, closed or released, from your own checkout: git worktree remove --force {wt}.\n\n",
            branch = pr.branch
        )
    });
    format!(
        "{url}\n\n{place}Review and finish {url} (branch {branch}); you hold it, nobody else works on it.\n\
         1. {dash} review rerun {url} in this worktree, then git merge --no-edit origin/main only on its yes, or when gh pr view \
         {n} --json mergeable -q .mergeable prints CONFLICTING. On its no merge none.\n\
         2. /code-review medium {url}: post its result as one comment on the PR (gh pr comment {n} --body-file <scratchpad \
         file>), fix each confirmed or unrefuted finding, answer the others with the code line that refutes them. {three_dot}\n\
         3. /simplify {url} and /ponytail:ponytail {url} on its diff; push the fixes with REVIEW_CLAIM_WHO=review-lane git push.\n\
         4. Hand it to the app's merge on green CI: {dash} review handover {url} review-lane. Or close it as superseded (gh pr \
         close {n} --comment naming what supersedes it) when main already holds its change. One that waits on his approval or \
         decision: comment what is left, then {dash} review release {url}.\n\
         5. {dash} review next again.\n",
        branch = pr.branch,
        three_dot = crate::THREE_DOT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lane's steps read the PR's diff from the merge base and review before the hand-over (task review-diff-merge-base).
    #[dash_meter::dash_test]
    fn the_lanes_steps_name_three_dot_diffs_before_the_hand_over() {
        let pr = Open {
            number: 9,
            url: "https://github.com/o/r/pull/9".into(),
            branch: "task/x".into(),
            created: String::new(),
        };
        let said = steps(&pr, None);
        let three = said.find("git diff origin/main...HEAD").expect(&said);
        assert!(three < said.find("review handover").expect(&said), "{said}");
    }
}
