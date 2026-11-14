//! The older app's landed close for a claim whose pane is gone, Rust's now (task rust-lane-landed-port): `Runner.landed/2`,
//! `branch_landed/2` and `mark_merged/4` ported. A claimed task whose pane went under a report ending its work, every PR of
//! its branch merged or closed, is done in the list and its claim released, as `Runner.run_ended/3` did. The PR lane is no
//! list task: its gone claim is released and never marked done, where the older app sent `PrLane.task()`, which names no
//! project, to `Tracker.Files.done/4` and crashed the PM loop's watch each pass from 2026-11-13 23:16 (@nha, 2026-11-10:
//! « remember we move to rust. no more elixir, so if something needs fixing port and then fix »). The older app's own copy
//! still runs until the runner is retired whole; this pass, every minute, reaches each claim first and leaves it nothing.
//! ponytail: a pane closed in the seconds before the older app's pass is still its own to judge; the runner's retirement
//! closes it.
use dash_core::tasks::{LANE, LandedRow, OlderClaim};

/// A claim this young is still starting, its pane not listed yet: `Runner`'s `@grace_s`.
pub const GRACE_S: i64 = 120;

/// A read of the task's branch PRs, `(url, state)`, or a write of the task done with its PRs.
pub type PrsOf<'a> = &'a dyn Fn(&str) -> Result<Vec<(String, String)>, String>;
pub type DoneWith<'a> = &'a dyn Fn(&str, &[String]) -> Result<(), String>;

/// What the pass reads and writes, each a closure the binary hands in and a test stands in for.
pub struct Ports<'a> {
    /// Whether an agent still runs in the pane.
    pub alive: &'a dyn Fn(&str) -> Result<bool, String>,
    /// The task's list row, none for a task the list does not hold (a reviewer, the lane).
    pub row: &'a dyn Fn(&str) -> Result<Option<LandedRow>, String>,
    /// Every PR of the task's branch, `(url, state)`.
    pub prs: PrsOf<'a>,
    /// The task done in the list with its PRs.
    pub done: DoneWith<'a>,
    /// The claim released, only as it was read: false when it moved since.
    pub release: &'a dyn Fn(&OlderClaim) -> Result<bool, String>,
}

/// `Some(urls)` when every PR is merged or closed, none included: `Runner.branch_landed/2`.
pub fn branch_landed(prs: &[(String, String)]) -> Option<Vec<String>> {
    let landed = |(_, state): &(String, String)| matches!(state.as_str(), "MERGED" | "CLOSED");
    prs.iter()
        .all(landed)
        .then(|| prs.iter().map(|(url, _)| url.clone()).collect())
}

/// The PRs to mark the task done with: its report ends its work and every PR of this period landed, none open, one at
/// least merged, as `Runner.done_run?/3` asks before `run_ended/3` (`merged_pr?/1`): a task whose PRs were all closed
/// unmerged never reached main (/code-review medium of PR 1602). Earlier periods' PRs left out (`Tasks.branch_prs/2`).
pub fn landed(row: &LandedRow, prs: &[(String, String)]) -> Option<Vec<String>> {
    let now: Vec<_> = (prs.iter())
        .filter(|(url, _)| !row.past_prs.contains(url))
        .cloned()
        .collect();
    let merged = now.iter().any(|(_, state)| state == "MERGED");
    branch_landed(&now).filter(|_| !row.work_left && merged)
}

/// One pass over the older app's claims: a line for each one it closed, or could not read, and whether every one was read.
pub fn pass(claims: &[OlderClaim], ports: &Ports) -> (Vec<String>, bool) {
    let mut read = true;
    let lines = (claims.iter())
        .filter_map(|c| {
            one(c, ports).unwrap_or_else(|e| {
                read = false;
                Some(format!("{}: {e}", c.task))
            })
        })
        .collect();
    (lines, read)
}

fn one(c: &OlderClaim, ports: &Ports) -> Result<Option<String>, String> {
    if c.age_s < GRACE_S {
        return Ok(None);
    }
    if let Some(pane) = &c.pane
        && (ports.alive)(pane)?
    {
        return Ok(None);
    }
    let pane = c.pane.as_deref().unwrap_or("none");
    if c.task == LANE {
        return Ok(((ports.release)(c)?).then(|| {
            format!("{LANE}'s pane {pane} is gone: claim closed, never marked done, the lane being no list task")
        }));
    }
    // the older app's intake and reviewers stay its own: another repository's branch, or no list row of their own
    if c.project != "site_perso" {
        return Ok(None);
    }
    let Some(row) = (ports.row)(&c.task)? else {
        return Ok(None);
    };
    // done or dropped since, this pass's mark among them when its release failed: the claim alone is left to close
    // (/code-review medium of PR 1602)
    if !row.open && row.runs_here && !row.review_of {
        return Ok(((ports.release)(c)?).then(|| {
            format!(
                "{}'s pane {pane} is gone, the task closed: claim closed",
                c.task
            )
        }));
    }
    if !row.open || !row.runs_here || row.review_of || row.work_left {
        return Ok(None);
    }
    let Some(urls) = landed(&row, &(ports.prs)(&c.task)?) else {
        return Ok(None);
    };
    // mark_merged: done first, so a release never leaves an open task with no claim and every PR landed
    (ports.done)(&c.task, &urls)?;
    let released = (ports.release)(c)?;
    let closed = if released {
        "claim closed"
    } else {
        "its claim moved since"
    };
    Ok(Some(format!(
        "{} marked done ({}), its pane {pane} gone under its report: {closed}",
        c.task,
        urls.join(" ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::RefCell;

    fn claim(task: &str, pane: Option<&str>, age_s: i64) -> OlderClaim {
        OlderClaim {
            task: task.into(),
            project: "site_perso".into(),
            pane: pane.map(Into::into),
            at: "2026-11-12T16:46:10.280667".into(),
            age_s,
        }
    }

    fn pr(n: u32, state: &str) -> (String, String) {
        (
            format!("https://github.com/nha-org/site_perso/pull/{n}"),
            state.into(),
        )
    }

    #[dash_test]
    fn a_gone_claim_whose_prs_landed_is_marked_done_and_the_lane_is_released_never_marked() {
        let done = RefCell::new(vec![]);
        let released = RefCell::new(vec![]);
        let row = |id: &str| {
            Ok(match id {
                // a report ending the work, its PR merged and an earlier period's left out
                "merged" => Some(LandedRow {
                    open: true,
                    runs_here: true,
                    past_prs: vec![pr(1, "MERGED").0],
                    ..LandedRow::default()
                }),
                "work-left" => Some(LandedRow {
                    open: true,
                    runs_here: true,
                    work_left: true,
                    ..LandedRow::default()
                }),
                "closed" => Some(LandedRow {
                    runs_here: true,
                    ..LandedRow::default()
                }),
                "rusts" | "open-pr" | "no-pr" | "unmerged" => Some(LandedRow {
                    open: true,
                    runs_here: id != "rusts",
                    ..LandedRow::default()
                }),
                _ => None,
            })
        };
        let prs = |id: &str| {
            Ok(match id {
                "merged" | "rusts" | "work-left" => {
                    vec![pr(1, "MERGED"), pr(2, "CLOSED"), pr(3, "MERGED")]
                }
                "open-pr" => vec![pr(4, "MERGED"), pr(5, "OPEN")],
                "unmerged" => vec![pr(7, "CLOSED")],
                // the lane's branch: a PR the older app's run_ended found, which this pass never reads
                LANE => vec![pr(6, "MERGED")],
                _ => vec![],
            })
        };
        let ports = Ports {
            alive: &|pane| Ok(pane == "w1:p1"),
            row: &row,
            prs: &prs,
            done: &|id, urls| {
                done.borrow_mut().push((id.to_string(), urls.to_vec()));
                Ok(())
            },
            release: &|c| {
                released.borrow_mut().push(c.task.clone());
                Ok(true)
            },
        };
        let claims = [
            // the live one: the lane's claim with no pane since 2026-11-12, the PM loop's crash
            claim(LANE, None, 86_400),
            claim("merged", Some("w9:p9"), 600),
            claim("alive", Some("w1:p1"), 600),
            claim("young", None, 30),
            claim("work-left", None, 600),
            claim("rusts", None, 600),
            claim("open-pr", None, 600),
            claim("no-pr", None, 600),
            claim("review-merged", None, 600),
            // every PR closed unmerged: its work never reached main
            claim("unmerged", None, 600),
            // marked done by an earlier pass whose release failed
            claim("closed", None, 600),
        ];
        let (lines, read) = pass(&claims, &ports);
        assert!(read);
        assert_eq!(
            *done.borrow(),
            [("merged".to_string(), vec![pr(2, "").0, pr(3, "").0])],
            "only the open task whose report ended its work and whose PRs all landed is marked done, the lane never"
        );
        assert_eq!(*released.borrow(), [LANE, "merged", "closed"]);
        assert_eq!(
            lines,
            [
                "review-lane's pane none is gone: claim closed, never marked done, the lane being no list task",
                "merged marked done (https://github.com/nha-org/site_perso/pull/2 \
                 https://github.com/nha-org/site_perso/pull/3), its pane w9:p9 gone under its report: claim closed",
                "closed's pane none is gone, the task closed: claim closed",
            ]
        );
    }

    #[dash_test]
    fn a_read_that_fails_names_its_claim_and_the_rest_still_pass() {
        let released = RefCell::new(vec![]);
        let ports = Ports {
            alive: &|pane| {
                if pane == "w2:p2" {
                    Err("herdr down".into())
                } else {
                    Ok(false)
                }
            },
            row: &|_| Ok(None),
            prs: &|_| Ok(vec![]),
            done: &|_, _| Ok(()),
            release: &|c| {
                released.borrow_mut().push(c.task.clone());
                Ok(true)
            },
        };
        let (lines, read) = pass(
            &[
                claim("x", Some("w2:p2"), 600),
                claim(LANE, Some("w3:p3"), 600),
            ],
            &ports,
        );
        assert_eq!(lines[0], "x: herdr down");
        assert!(!read, "a claim unread makes the beat red");
        assert_eq!(
            *released.borrow(),
            [LANE],
            "a pane unread is never read as gone"
        );
    }

    #[dash_test]
    fn branch_landed_needs_every_pr_merged_or_closed() {
        assert_eq!(branch_landed(&[]), Some(vec![]));
        assert_eq!(branch_landed(&[pr(1, "MERGED"), pr(2, "OPEN")]), None);
        let row = LandedRow::default();
        assert_eq!(landed(&row, &[]), None, "no PR is no landing");
        assert_eq!(
            landed(&row, &[pr(1, "CLOSED")]),
            None,
            "closed unmerged is no landing"
        );
    }
}
