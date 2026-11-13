//! `dash review next`, the PR lane's next PR from Rust's store (task rust-review-next-port; `dash_orch::review` decides): the
//! forge read through gh, the reviewers through herdr, the PR's own worktree, then its steps. The front door answers it and the
//! lane's `review release` and `review handover`, which park its hold, so the lane never waits on the older app.
use dash_forge::checks::GREEN;
use dash_orch::review::{self, Lane, Next, Open};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One gh or git call's bound: the lane asks again past it.
const CALL: Duration = Duration::from_secs(30);

/// The lane's forge: site_perso's PRs, read in its checkout `root`. ponytail: replicator's are left out, none open since
/// 2026-11-01; a hold keyed by repository first if it comes back.
pub struct GitHub {
    pub root: PathBuf,
}

impl Lane for GitHub {
    fn open(&self) -> Result<Vec<Open>, String> {
        let fields = "number,url,headRefName,createdAt,isDraft";
        let args = [
            "pr", "list", "--state", "open", "--limit", "200", "--json", fields,
        ];
        let out = run("gh", &args, &self.root)?;
        let listed: Vec<serde_json::Value> =
            serde_json::from_str(&out).map_err(|e| format!("gh pr list: {e}"))?;
        let said = |p: &serde_json::Value, k: &str| p[k].as_str().unwrap_or_default().to_string();
        Ok((listed.iter())
            .filter(|p| p["isDraft"] != true)
            .filter_map(|p| {
                Some(Open {
                    number: p["number"].as_u64()?,
                    url: said(p, "url"),
                    branch: said(p, "headRefName"),
                    created: said(p, "createdAt"),
                })
            })
            .collect())
    }

    fn green(&self, pr: &Open) -> Result<bool, String> {
        let rows = dash_forge::read_checks(&self.root, &self.root, &pr.url)?;
        Ok(!rows.is_empty() && rows.iter().all(|(_, s)| GREEN.contains(&s.as_str())))
    }

    fn reviewed(&self) -> Result<Vec<String>, String> {
        let live = dash_terminal::Herdr::from_env().live()?;
        Ok(reviewers(live.iter().map(|l| l.cwd.as_str())))
    }
}

/// The tasks a reviewer's session works for, by the folders herdr lists: `…/task-review-<id>`, the older app's reviewer worktree.
fn reviewers<'a>(cwds: impl Iterator<Item = &'a str>) -> Vec<String> {
    cwds.filter_map(|cwd| {
        let folder = cwd.split('/').find(|s| s.starts_with("task-review-"))?;
        Some(folder["task-review-".len()..].to_string())
    })
    .collect()
}

fn run(bin: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
    let said = dash_process::answer(bin, args, Some(cwd), &[("GH_URGENT", "1")], CALL);
    let (status, out, err) = said.map_err(|e| format!("{bin} {}: {e:?}", args.join(" ")))?;
    match status.success() {
        true => Ok(out),
        false => Err(format!("{bin} {}: {}", args.join(" "), err.trim())),
    }
}

/// `dash review next` in `cwd`, the lane's checkout (`None`: no worktree made): the exit code, stdout and stderr.
pub fn answer(data: &Path, cwd: Option<&Path>) -> (i32, String, String) {
    let said = crate::tasks::open_tasks(data).and_then(|mut tasks| {
        let lane = GitHub {
            root: crate::tasks::repo(),
        };
        review::next(&lane, &mut tasks)
    });
    let (held, pr) = match said {
        Ok(Next::None) => return (0, "none\n".into(), String::new()),
        Ok(Next::Held(pr)) => ("held: ", pr),
        Ok(Next::Took(pr)) => ("", pr),
        Err(e) => return (1, String::new(), format!("review next: {e}\n")),
    };
    let worktree = cwd.map(|cwd| worktree(pr.number, cwd)).transpose();
    match worktree {
        Ok(wt) => (
            0,
            format!("{held}{}", steps(&pr, wt.as_deref())),
            String::new(),
        ),
        // claimed all the same: the next ask resumes it as held
        Err(e) => (
            1,
            String::new(),
            format!("review next: {held}{}: {e}\n", pr.url),
        ),
    }
}

inventory::submit! { crate::usage::Command::usage("dash review next --data <dir> [--cwd <the lane's checkout>]") }
/// `dash review next --data <dir>`: `answer` printed, its exit code returned. bin/dash's `dash review next` reaches `answer` through
/// the front door, which writes the live store a session's sandbox keeps read-only.
pub fn print(data: &Path, cwd: Option<&Path>, mut out: std::fs::File) -> i32 {
    use std::io::Write;
    let (code, said, err) = answer(data, cwd);
    write!(out, "{said}").expect("stdout");
    eprint!("{err}");
    code
}

/// The lane's hold on `pr` parked after `review release` or a hand-over: what the front door adds to their answer, or an error.
pub fn park(data: &Path, pr: &str, why: &str) -> Result<bool, String> {
    // a word that names no PR (`review release <who>`) is the older app's
    let Ok(n) = crate::tasks::pr_number(pr) else {
        return Ok(false);
    };
    review::park(&mut crate::tasks::open_tasks(data)?, n, why)
}

/// The PR's own worktree, `<the lane's checkout>/.claude/run/pr-lane/site_perso-<n>`: one checkout per PR, so a push never
/// carries another PR's merge (the older app's `PrLane.worktree`, task review-next-held). One there already is kept.
fn worktree(n: u64, cwd: &Path) -> Result<PathBuf, String> {
    let top = run("git", &["rev-parse", "--show-toplevel"], cwd)?;
    // run from inside a PR's worktree, the folder still hangs off the lane's checkout, never nested
    let lane = top
        .trim()
        .split("/.claude/run/pr-lane/")
        .next()
        .unwrap_or_default();
    let wt = Path::new(lane).join(format!(".claude/run/pr-lane/site_perso-{n}"));
    let at = wt.to_string_lossy();
    // a folder removed by hand leaves git's registration, which refuses the add
    let _ = run("git", &["worktree", "prune"], cwd);
    let listed = run("git", &["worktree", "list", "--porcelain"], cwd)?;
    if listed
        .lines()
        .any(|l| l.strip_prefix("worktree ") == Some(&*at))
    {
        return Ok(wt);
    }
    if wt.exists() {
        return Err(format!(
            "{at} is no registered worktree: remove it, then run review next again"
        ));
    }
    // no hooks: the checkout's post-checkout hook is the session's, not this folder's
    let add = [
        "-c",
        "core.hooksPath=/dev/null",
        "worktree",
        "add",
        "--detach",
        &at,
        "origin/main",
    ];
    run("git", &add, cwd).map(|_| wt)
}

/// What `dash review next` prints for a PR: its link, where to work it, then the lane's steps (the older app's `PrLane.steps`,
/// cut to what the lane's own prompt leaves out).
fn steps(pr: &Open, wt: Option<&Path>) -> String {
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
        three_dot = dash_orch::THREE_DOT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dash_meter::dash_test]
    fn a_reviewer_is_read_from_its_folder() {
        let cwds = [
            "/u/.herdr/worktrees/site_perso/task-review-rust-x/rust",
            "/u/.herdr/worktrees/site_perso/task-rust-y",
            "/u/repo/site_perso",
        ];
        assert_eq!(reviewers(cwds.into_iter()), ["rust-x"]);
    }

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
