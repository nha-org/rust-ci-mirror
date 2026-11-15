//! `dash review next`, the PR lane's next PR from Rust's store (task rust-review-next-port; `dash_orch::review` decides): the
//! forge read through gh, the reviewers through herdr, the PR's own worktree, then its steps. The front door answers it and the
//! lane's `review release` and `review handover`, which park its hold, so the lane never waits on the older app.
use dash_forge::checks::GREEN;
use dash_hooks::claims::{held_prs, now};
use dash_hooks::push::Read;
use dash_orch::review::{self, Lane, Next, Open};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One gh or git call's bound: the lane asks again past it.
const CALL: Duration = Duration::from_secs(30);

/// The lane's forge: site_perso's PRs, read in its checkout `root`. ponytail: replicator's are left out, none open since
/// 2026-11-01; a hold keyed by repository first if it comes back.
pub struct GitHub {
    pub root: PathBuf,
    pub data: PathBuf,
    pub host: &'static dyn dash_core::Host,
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
        let ours = |(repo, id, _): (String, String, String)| (repo == "site_perso").then_some(id);
        Ok(reviewers()?.into_iter().filter_map(ours).collect())
    }

    fn claimed(&self) -> Result<Vec<u64>, String> {
        Ok(held_prs(&now(self.host, &self.data, None, &|_| true)?))
    }
}

/// The repository and task a reviewer's session works for, and its pane, by the folders herdr lists: `…/<repo>/task-review-<id>`,
/// the older app's reviewer worktree.
pub fn reviewers() -> Result<dash_hooks::push::Reviewers, String> {
    let live = dash_terminal::Herdr::from_env().live()?;
    let of = |l: dash_terminal::Live| {
        let (repo, id) = dash_forge::folder::reviewer_of(&l.cwd)?;
        Some((repo.to_string(), id.to_string(), l.pane))
    };
    Ok(live.into_iter().filter_map(of).collect())
}

/// What `dash review push` reads of `key` (`dash_hooks::push::read`): the older app's PR mirror from its file and the reviewers
/// herdr shows. Never asks the older app.
pub fn push_read(
    host: &dyn dash_core::Host,
    data: &Path,
    repo: Option<&str>,
    rest: &str,
) -> Result<Option<Read>, String> {
    let named = dash_store::older::site_prs(&crate::tasks::older_db(), repo, rest)?;
    (named.map(|n| dash_hooks::push::read_seen(host, data, n, reviewers()))).transpose()
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
pub fn answer(
    host: &'static dyn dash_core::Host,
    data: &Path,
    cwd: Option<&Path>,
) -> (i32, String, String) {
    let said = crate::tasks::open_tasks(data).and_then(|mut tasks| {
        let lane = GitHub {
            root: crate::tasks::repo(),
            data: data.to_path_buf(),
            host,
        };
        review::next(&lane, &mut tasks)
    });
    let (held, pr) = match said {
        Ok(Next::None) => return (0, "none\n".into(), String::new()),
        Ok(Next::Held(pr)) => ("held: ", pr),
        Ok(Next::Took(pr)) => ("", pr),
        Err(e) => return (1, String::new(), format!("review next: {e}\n")),
    };
    let worktree = cwd
        .map(|cwd| dash_forge::folder::pr_worktree(pr.number, cwd))
        .transpose();
    match worktree {
        Ok(wt) => (
            0,
            format!("{held}{}", review::steps(&pr, wt.as_deref())),
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
pub fn print(
    host: &'static dyn dash_core::Host,
    data: &Path,
    cwd: Option<&Path>,
    mut out: std::fs::File,
) -> i32 {
    use std::io::Write;
    let (code, said, err) = answer(host, data, cwd);
    write!(out, "{said}").expect("stdout");
    eprint!("{err}");
    code
}

/// The lane's hold on `pr` parked after `review release` or a hand-over: what the front door adds to their answer, or an error.
pub fn park(data: &Path, pr: &str, why: &str) -> Result<bool, String> {
    // a word that names no PR (`review release <who>`) is the older app's
    let Ok(n) = dash_orch::land::pr_number(pr) else {
        return Ok(false);
    };
    review::park(&mut crate::tasks::open_tasks(data)?, n, why)
}
