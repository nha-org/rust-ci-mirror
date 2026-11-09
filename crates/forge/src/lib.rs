//! Core's Forge on git and GitHub's CLI. A task's worktree is `rust-<id>` beside the others, its branch `task/<id>`, which CI's
//! title check, the task page and the older app's PR reads all expect.
use dash_core::tasks::{Base, Checks, Forge, Pr};
use dash_process as process;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct GitHub {
    /// The checkout its worktrees hang off.
    pub repo: PathBuf,
    /// The folder they sit in.
    pub root: PathBuf,
    pub base: String,
    /// The live page, whose « Built from » line names what serves.
    pub page: String,
    /// GitHub's CLI: `gh`, or a stand-in a test names.
    pub cli: String,
}

const CALL: Duration = Duration::from_secs(30);
/// A fetch and a worktree of a large repository.
const GIT: Duration = Duration::from_secs(300);
/// A release build of the whole tool, then up to 60 s for the page to name it.
const DEPLOY: Duration = Duration::from_secs(1800);

/// What main changes most often and nothing builds or tests: a head lacking only these still holds main.
const INERT: [&str; 3] = ["explainer.d/", "EXPLAINER.md", "private/docs/pm-daily.html"];

/// What dash ci wait and the merge hook count as passed (merge-claim.py GREEN).
const GREEN: [&str; 3] = ["SUCCESS", "NEUTRAL", "SKIPPED"];

/// The real path: Claude names a transcript's folder after it, so /tmp/x or a relative path would never match. Refused under
/// a `task-<id>` folder, where the older app's Launch.task_of_folder would adopt the pane (point p-adopt).
fn real_folder(folder: &Path) -> Result<String, String> {
    let real = std::fs::canonicalize(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let named = |c: std::path::Component| c.as_os_str().to_string_lossy().starts_with("task-");
    if real.components().any(named) {
        return Err(format!(
            "{}: a task-<id> folder is the older app's to adopt; give a folder named otherwise",
            real.display()
        ));
    }
    Ok(real.to_string_lossy().into_owned())
}

/// A branch's newest PR as `gh pr list` names it, before a worktree's HEAD is judged against it (`GitHub::held`). `squash`: the
/// commit its merge left on main, empty while open.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    pr: Pr,
    squash: String,
}

/// A closed PR holds nothing: `None`, as no PR.
fn listed_in(text: &str) -> Result<Option<Listed>, String> {
    let list: Value =
        serde_json::from_str(text).map_err(|e| format!("gh's answer: {e}: {text}"))?;
    let Some(pr) = list.as_array().and_then(|l| l.first()) else {
        return Ok(None);
    };
    let word = |field: &str| pr[field].as_str().unwrap_or_default().to_string();
    let number = pr["number"]
        .as_u64()
        .ok_or(format!("no number in: {text}"))?;
    if pr["state"] == "CLOSED" {
        return Ok(None);
    }
    let squash = pr["mergeCommit"]["oid"].as_str().unwrap_or_default();
    Ok(Some(Listed {
        pr: Pr {
            number,
            url: word("url"),
            head: word("headRefOid"),
            merged: pr["state"] == "MERGED",
        },
        squash: squash.to_string(),
    }))
}

/// `git merge-tree --write-tree --name-only -z` of main and a head that lacks some of it: its exit 0 merges cleanly (`None`); 1
/// conflicts, naming each file once after the tree's id, up to the empty field before its messages, unquoted.
fn conflicts_in(code: Option<i32>, text: &str) -> Result<Option<Vec<String>>, String> {
    match code {
        Some(0) => Ok(None),
        Some(1) => {
            let files = text.split('\0').skip(1).take_while(|f| !f.is_empty());
            Ok(Some(files.map(str::to_string).collect()))
        }
        _ => Err(format!("git merge-tree: {code:?}: {text}")),
    }
}

/// The pids `pgrep -lf` lists running this very binary as the look: `<pid> <binary> orch --data <dir>`, nothing after it, so a
/// hand-run `--once` or `--sync` is never ended half way.
fn looks_in(listed: &str, binary: &str) -> Vec<String> {
    let look = |line: &str| {
        let (pid, command) = line.split_once(' ')?;
        let rest = command
            .strip_prefix(binary)?
            .strip_prefix(" orch --data ")?;
        let digits = !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit());
        let look = !rest.is_empty() && !rest.contains(" --");
        (digits && look).then(|| pid.to_string())
    };
    listed.lines().filter_map(look).collect()
}

/// An open PR as `gh pr list` names it.
#[derive(Debug, PartialEq)]
struct Open {
    number: u64,
    branch: String,
    head: String,
    base: String,
}

fn opens_in(text: &str) -> Result<Vec<Open>, String> {
    let list: Value =
        serde_json::from_str(text).map_err(|e| format!("gh's answer: {e}: {text}"))?;
    let rows = list.as_array().ok_or(format!("no list in: {text}"))?;
    let word = |pr: &Value, field: &str| pr[field].as_str().unwrap_or_default().to_string();
    let open = |pr: &Value| {
        Some(Open {
            number: pr["number"].as_u64()?,
            branch: word(pr, "headRefName"),
            head: word(pr, "headRefOid"),
            base: word(pr, "baseRefName"),
        })
    };
    Ok(rows.iter().filter_map(open).collect())
}

/// The commit a page names on its whole line « Built from <sha>. », as rust/serves matches it.
fn built_from(page: &str) -> Option<String> {
    let line = page
        .lines()
        .find_map(|l| l.strip_prefix("Built from ")?.strip_suffix('.'));
    line.map(str::to_string)
}

/// `merge-claim.py --checks`'s rows, `[name, state]`. PENDING and BILLING (GitHub refused to run it) hold no verdict; any
/// other state not green is red. No row yet is no verdict either: a push's runs list within a minute.
fn checks_in(text: &str) -> Result<Checks, String> {
    let rows = rows_in(text)?;
    let red = named(&rows, red);
    if !red.is_empty() {
        return Ok(Checks::Red(red.join(", ")));
    }
    let settled = !rows.is_empty() && rows.iter().all(|(_, s)| GREEN.contains(&s.as_str()));
    Ok(if settled {
        Checks::Green
    } else {
        Checks::Pending
    })
}

/// A head's checks, `[name, state]`, as `merge-claim.py --checks` prints them.
pub type Rows = Vec<(String, String)>;

fn rows_in(text: &str) -> Result<Rows, String> {
    serde_json::from_str(text).map_err(|e| format!("the checks: {e}: {text}"))
}

/// Neither green, nor PENDING, nor BILLING (GitHub refused to run it).
fn red(state: &str) -> bool {
    !GREEN.contains(&state) && !matches!(state, "PENDING" | "BILLING")
}

/// The names of the rows whose state `keep` takes.
fn named(rows: &Rows, keep: impl Fn(&str) -> bool) -> Vec<&str> {
    (rows.iter().filter(|(_, s)| keep(s)))
        .map(|(n, _)| n.as_str())
        .collect()
}

/// How long `dash ci wait` waits for a verdict: a longer run is a stuck queue, not a wait.
pub const CI_DEADLINE: Duration = Duration::from_secs(3 * 3600);
/// The hook's own bound is 60 s (merge-claim.py): past twice that, the read is lost and the next poll asks again.
const HOOK: Duration = Duration::from_secs(120);

/// A read with no rows. `again`: it may answer at the next poll (gh did not answer, the hook past its bound, killed, crashed, or
/// not started under load); else it never will (the hook answered, but no JSON).
pub struct Unread {
    pub again: bool,
    pub why: String,
}

/// The head's checks through the merge hook's own read (`merge-claim.py --checks <pr|sha>`, `python3 -I` in `cwd`, the caller's
/// checkout): each job's latest attempt, a cancelled run superseded by a newer one dropped, a job GitHub refused for billing read
/// BILLING. The rule lives there once (task ci-wait-command). The caller checks the hook is there.
pub fn hook_rows(hook: &Path, cwd: &Path, target: &str) -> Result<Rows, Unread> {
    let hook = hook.to_string_lossy();
    let args = ["-I", &hook, "--checks", target];
    let lost = |why: String| Unread { again: true, why };
    let (status, out, err) =
        process::answer("python3", &args, Some(cwd), &[], HOOK).map_err(|no| lost(no.why))?;
    if !status.success() {
        // its exit 3 is « gh did not answer »: a gh call failed or passed its 10 s
        return Err(lost(format!("the hook: {status}: {}", err.trim())));
    }
    rows_in(&out).map_err(|why| Unread { again: false, why })
}

/// `dash ci wait <pr|sha>`, as the older app's Dashboard.Orch.CiWait: `(exit, stdout, stderr)`, 0 green, 1 red, 3 no verdict.
/// `read` asks the checks, `wait` sleeps a poll's interval and says whether the caller still listens. A lost read is asked again
/// at the next poll: no verdict comes only past `until` (task ci-wait-gh-retry: on 2026-11-01 three dash ci wait 806 exited 3 on
/// « gh did not answer » at load 120 while gh pr view answered). A red row is final at once; PENDING waits; BILLING alone is no
/// verdict, never green or red.
pub fn ci_wait(
    mut read: impl FnMut() -> Result<Rows, Unread>,
    mut wait: impl FnMut() -> bool,
    until: Instant,
) -> (i32, String, String) {
    // a head with no check after this many empty answers: no workflow covers it (a push's runs list within a minute)
    const EMPTY: u32 = 3;
    let no_verdict = |why: String| (3, String::new(), format!("ci wait: {why}\n"));
    let mut empties = 0;
    loop {
        let last = match read() {
            Err(Unread { again: false, why }) => return no_verdict(why),
            Err(Unread { why, .. }) => format!("no read answered: {why}"),
            Ok(rows) if rows.iter().any(|(_, s)| red(s)) => {
                let red = (rows.iter().filter(|(_, s)| red(s)))
                    .map(|(n, s)| format!("{n} {}", s.to_lowercase()));
                let red = red.collect::<Vec<_>>().join(", ");
                return (
                    1,
                    String::new(),
                    format!("ci wait: red on the head: {red}\n"),
                );
            }
            Ok(rows) if rows.is_empty() && empties >= EMPTY => {
                let why = format!(
                    "no verdict, no check on the head after {EMPTY} polls: no workflow covers it"
                );
                return no_verdict(why);
            }
            Ok(rows) if rows.is_empty() => {
                empties += 1;
                "no check on the head".into()
            }
            Ok(rows) => match named(&rows, |s| s == "PENDING") {
                pending if pending.is_empty() => return verdict(&rows),
                pending => format!("still pending: {}", pending.join(", ")),
            },
        };
        if Instant::now() >= until {
            return no_verdict(format!("no verdict by its deadline, {last}"));
        }
        if !wait() {
            return no_verdict("the caller left".into());
        }
    }
}

/// Settled, nothing red: green, unless GitHub ran none of some jobs (billing).
fn verdict(rows: &Rows) -> (i32, String, String) {
    let billing = named(rows, |s| s == "BILLING");
    if billing.is_empty() {
        let green = format!("ci wait: green on the head ({} checks)\n", rows.len());
        return (0, green, String::new());
    }
    let why = format!(
        "ci wait: no verdict, GitHub ran none of {} (billing). Run the local precommit; once it passes, LOCAL_GATE=passed gh pr merge ...\n",
        billing.join(", ")
    );
    (3, String::new(), why)
}

impl GitHub {
    fn git(&self, args: &[&str]) -> Result<String, String> {
        process::run("git", args, Some(&self.repo), GIT)
    }

    fn gh(&self, args: &[&str]) -> Result<String, String> {
        process::run(&self.cli, args, Some(&self.repo), CALL)
    }

    // ponytail: the merge hook's own checks, never a second copy (dash ci wait reads them too); Rust ones once Python goes. Its
    // other refusals guard one session's merge against another's work, and this path has no second session. Its file-overlap
    // stale rule is not this path's: `base` holds back any head lacking main's commits outside a few inert files, the PR's own
    // or not (task rust-merge-gate-one-rule).
    fn hook(&self, rule: &str, pr: &Pr) -> Result<String, String> {
        let hook = self.repo.join(".claude/hooks/merge-claim.py");
        let number = pr.number.to_string();
        let args = ["-I", &hook.to_string_lossy(), rule, &number];
        process::run("python3", &args, Some(&self.repo), CALL)
    }

    /// The look (`make rust/orch`) runs the binary it started with, so a merge of its own code would never run: it ends here, after
    /// each deploy, built or not (a manual make rust/deploy restarts no look), and launchd starts it again on the live binary. Its
    /// runs, each in a process group of its own, this one included, go on. Listed, each whole command line checked, then killed by
    /// pid: a pattern kill once ended every Claude session on the Mac (2026-10-19). No look running is no failure: launchd's next
    /// start takes the live binary anyway.
    fn restart_look(&self, live: &Path) -> Result<(), String> {
        let binary = live.join("dash");
        let real = std::fs::canonicalize(&binary).unwrap_or_else(|_| binary.clone());
        let args = ["-lf", "dash orch --data"];
        let (status, listed, why) =
            process::answer("pgrep", &args, None, &[], CALL).map_err(String::from)?;
        match status.code() {
            Some(0) => {}
            Some(1) => return Ok(()),
            code => return Err(format!("pgrep -lf: {code:?}: {why}")),
        }
        let binaries = [binary.to_string_lossy(), real.to_string_lossy()];
        let mut pids = looks_in(&listed, &binaries[0]);
        pids.extend(looks_in(&listed, &binaries[1]));
        pids.sort();
        pids.dedup();
        // A look that ended between the list and the kill is no failure: launchd starts it on the live binary all the same.
        for pid in pids {
            let _ = process::run("kill", &[&pid], None, CALL);
        }
        Ok(())
    }

    /// One open PR, if it is stacked on `merged_head`'s branch: it shares a commit with it that main lacks, as a PR started from
    /// another's head does (rust-skeleton-live on #1248 conflicted once that one squash-merged). Its own commits rebased onto main
    /// in a worktree of their own, never its session's, pushed with a lease on the head read, its base set to main. A rebase that
    /// conflicts is given up, its PR left to the hand-back pass. `None`: not stacked.
    /// Its pulls fetched first, with main, in one fetch (`fetch_pulls`).
    fn restack_one(&self, merged_head: &str, open: &Open) -> Result<Option<String>, String> {
        let main = format!("origin/{}", self.base);
        let shared = self.git(&["merge-base", &open.head, merged_head])?;
        let shared = shared.trim();
        let held = self.git(&["merge-base", "--is-ancestor", shared, &main]);
        if held.is_ok() {
            return Ok(None);
        }
        // The merged PR stood on this one's head, not the reverse: its squash carried all of this one into main, and a rebase
        // would replay nothing and push main to its branch.
        if shared == open.head {
            let n = open.number;
            return Ok(Some(format!(
                "#{n}: the merged PR was built on its head, so its commits landed in that squash; left as it is"
            )));
        }
        let folder = self.root.join(format!("restack-{}", open.number));
        let path = folder.to_string_lossy();
        let _ = self.git(&["worktree", "remove", "--force", &path]);
        self.git(&["worktree", "add", "-q", "--detach", &path, &open.head])?;
        let said = self.rebase_in(&path, &main, shared, open);
        // Whatever it said: a full checkout left behind fills the disk as a build does.
        let removed = self.git(&["worktree", "remove", "--force", &path]);
        let said = said?;
        removed?;
        Ok(Some(said))
    }

    fn rebase_in(
        &self,
        path: &str,
        main: &str,
        shared: &str,
        open: &Open,
    ) -> Result<String, String> {
        let at = |args: &[&str]| self.git(&[&["-C", path][..], args].concat());
        let who = ["-c", "user.name=dash", "-c", "user.email=dash@localhost"];
        let rebase = ["rebase", "-q", "--onto", main, shared];
        if let Err(e) = at(&[&who[..], &rebase].concat()) {
            let _ = at(&["rebase", "--abort"]);
            let n = open.number;
            return Ok(format!(
                "#{n} conflicts with main once rebased off the merged branch, left to its task: {e}"
            ));
        }
        let lease = format!(
            "--force-with-lease=refs/heads/{}:{}",
            open.branch, open.head
        );
        let to = format!("HEAD:refs/heads/{}", open.branch);
        at(&["push", "-q", &lease, "origin", &to])?;
        let n = open.number;
        if open.base != self.base {
            let number = n.to_string();
            if let Err(e) = self.gh(&["pr", "edit", &number, "--base", &self.base]) {
                return Ok(format!(
                    "#{n} rebased onto main off the merged branch, but its base stays {}: {e}",
                    open.base
                ));
            }
        }
        Ok(format!("#{n} rebased onto main off the merged branch"))
    }

    /// Main and each PR's head in one fetch: a fetch each would hold the merge's deploy for minutes.
    fn fetch_pulls(&self, numbers: &[u64]) -> Result<(), String> {
        let pulls: Vec<String> = numbers.iter().map(|n| format!("pull/{n}/head")).collect();
        let mut args = vec!["fetch", "-q", "origin", &self.base];
        args.extend(pulls.iter().map(String::as_str));
        self.git(&args).map(drop)
    }

    fn folder(&self, id: &str) -> PathBuf {
        self.root.join(format!("rust-{id}"))
    }

    fn listed(&self, id: &str) -> Result<Option<Listed>, String> {
        let branch = format!("task/{id}");
        let fields = "number,url,state,headRefOid,mergeCommit";
        let args = [
            "pr", "list", "--head", &branch, "--state", "all", "--limit", "1", "--json", fields,
        ];
        listed_in(&self.gh(&args)?)
    }

    /// `a` is `b` or behind it. A commit unknown here is a failed read, never a no.
    fn ancestor(&self, a: &str, b: &str) -> Result<bool, String> {
        let args = ["merge-base", "--is-ancestor", a, b];
        let (status, _, why) =
            process::answer("git", &args, Some(&self.repo), &[], GIT).map_err(String::from)?;
        match status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            code => Err(format!(
                "git merge-base --is-ancestor {a} {b}: {code:?}: {why}"
            )),
        }
    }

    /// `commit` holds no work `listed` lacks: its head, or behind it (a reviewer pushed since: PR 1129 read as none); merged, also
    /// main at or after its squash (rust-access-map, PR 1217, read as none for 3 PM rounds), unless `made` started task `id`'s
    /// branch after that squash: a reopen's, which holds none of its new work yet. A commit past either is work to push.
    fn held(&self, id: &str, commit: &str, listed: &Listed) -> Result<bool, String> {
        let pr = &listed.pr;
        if commit == pr.head {
            return Ok(true);
        }
        // A reviewer's push or the squash may be newer than anything fetched here.
        let known = |c: &str| {
            c.is_empty()
                || self
                    .git(&["cat-file", "-e", &format!("{c}^{{commit}}")])
                    .is_ok()
        };
        if !known(commit) || !known(&pr.head) || !known(&listed.squash) {
            self.fetch_pulls(&[pr.number])?;
        }
        // In neither main nor its head once both are here, as a newer push to its branch at the origin.
        if !known(commit) {
            return Ok(false);
        }
        if self.ancestor(commit, &pr.head)? {
            return Ok(true);
        }
        let squash = listed.squash.as_str();
        if !pr.merged || squash.is_empty() || !self.ancestor(squash, commit)? {
            return Ok(false);
        }
        let from = self.git(&["config", &format!("branch.task/{id}.dashFrom")]);
        if let Ok(from) = from
            && self.ancestor(squash, from.trim())?
        {
            return Ok(false);
        }
        let main = format!("origin/{}", self.base);
        if self.ancestor(commit, &main)? {
            return Ok(true);
        }
        // Main here may be older than the worktree's: one fetch before a no.
        self.git(&["fetch", "-q", "origin", &self.base])?;
        self.ancestor(commit, &main)
    }

    /// After its PR merged, its branch goes here and at the origin when every head of it is held by that merge, so a reopen starts
    /// from main: PR 1247 carried all of 1240's commits again (rust-design-components, 2026-11-09), and a fresh branch with the
    /// old one still at the origin is refused its push until it merges that. A head past the merge, or a dirty folder, keeps all.
    fn start_over(&self, id: &str, merged: &Listed) -> Result<(), String> {
        let (branch, folder) = (format!("task/{id}"), self.folder(id));
        let path = folder.to_string_lossy();
        let local = self.git(&[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{branch}"),
        ]);
        let checked_out = match folder.exists() {
            true => Some(self.git(&["-C", &path, "rev-parse", "HEAD"])?),
            false => None,
        };
        let at_origin = || {
            let refs = format!("refs/heads/{branch}");
            let listed = self.git(&["ls-remote", "--heads", "origin", &refs])?;
            Ok::<_, String>(listed.split_whitespace().next().map(str::to_string))
        };
        let remote = at_origin()?;
        let tracked = format!("refs/remotes/origin/{branch}");
        // Gone at the origin (GitHub deletes a merged head): what this repo still tracks of it is stale, whatever stays here.
        if remote.is_none() {
            self.git(&["update-ref", "-d", &tracked])?;
        }
        let heads = local
            .ok()
            .into_iter()
            .chain(checked_out)
            .chain(remote.clone());
        for head in heads {
            if !self.held(id, head.trim(), merged)? {
                return Ok(());
            }
        }
        if folder.exists() && self.git(&["worktree", "remove", &path]).is_err() {
            return Ok(());
        }
        let _ = self.git(&["branch", "-D", &branch]);
        // GitHub may delete it in between: only a branch still there fails the start, which a retry then finishes.
        if remote.is_some()
            && let Err(e) = self.git(&["push", "-q", "origin", "--delete", &branch])
            && at_origin()?.is_some()
        {
            return Err(e);
        }
        self.git(&["update-ref", "-d", &tracked]).map(drop)
    }

    /// The PR holding this task's work, of its newest: one its worktree's HEAD is `held` by, any while it has none.
    fn judged(&self, id: &str, listed: Option<Listed>) -> Result<Option<Pr>, String> {
        let Some(listed) = listed else {
            return Ok(None);
        };
        let folder = self.folder(id);
        let Ok(local) = self.git(&["-C", &folder.to_string_lossy(), "rev-parse", "HEAD"]) else {
            // Its worktree gone: nothing here a PR could lack.
            return Ok(Some(listed.pr));
        };
        Ok(self.held(id, local.trim(), &listed)?.then_some(listed.pr))
    }

    /// Its folder, its branch first started over if `merged` holds all of it.
    fn opened(&self, id: &str, newest: Option<&Listed>) -> Result<String, String> {
        if let Some(merged) = newest.filter(|l| l.pr.merged) {
            self.start_over(id, merged)?;
        }
        self.made(id)
    }

    /// Its folder, made once from its branch, else a new one from main.
    fn made(&self, id: &str) -> Result<String, String> {
        let folder = self.folder(id);
        if !folder.exists() {
            // Made under another name and moved: a crash half way never leaves a folder a retry would take for a whole one. No
            // session ever ran in what a crash left under that name, so it goes. The claim's lock means only one run is here.
            let new = self.root.join(format!("rust-{id}.new"));
            let path = new.to_string_lossy();
            let _ = self.git(&["worktree", "remove", "--force", &path]);
            if new.exists() {
                std::fs::remove_dir_all(&new).map_err(|e| format!("{path}: {e}"))?;
            }
            self.git(&["worktree", "prune"])?;
            let (branch, base) = (format!("task/{id}"), format!("origin/{}", self.base));
            self.git(&["fetch", "-q", "origin", &self.base])?;
            let known = self.git(&[
                "rev-parse",
                "-q",
                "--verify",
                &format!("refs/heads/{branch}"),
            ]);
            match known {
                Ok(_) => self.git(&["worktree", "add", "-q", &path, &branch])?,
                Err(_) => {
                    self.git(&["worktree", "add", "-q", "-b", &branch, &path, &base])?;
                    // What `held` reads to tell a reopen's branch from the one its PR merged; `branch -D` drops it with the branch.
                    let from = self.git(&["rev-parse", &base])?;
                    self.git(&["config", &format!("branch.{branch}.dashFrom"), from.trim()])?
                }
            };
            self.git(&["worktree", "move", &path, &folder.to_string_lossy()])?;
        }
        real_folder(&folder)
    }
}

impl Forge for GitHub {
    fn worktree(&self, id: &str) -> Result<String, String> {
        // An unread list is no merge: the folder as it stands, as before.
        self.opened(id, self.listed(id).ok().flatten().as_ref())
    }

    fn pr(&self, id: &str) -> Result<Option<Pr>, String> {
        self.judged(id, self.listed(id)?)
    }

    fn checks(&self, pr: &Pr) -> Result<Checks, String> {
        checks_in(&self.hook("--checks", pr)?)
    }

    // Main's commits the head lacks hold it back unless each file they change is one no build or test reads: main moves every few
    // minutes with explainer folds and the PM's page, and a PR sent back for each would never land. Anywhere, they must merge
    // cleanly. Gate tests read files outside rust/ (workflows, bin/ci, private/docs/rfc-rust-unified.html), so only these pass.
    // ponytail: main may move in the seconds from this read to the merge; a lock across runs, or branch protection's « up to date »,
    // closes that if two land in the same seconds.
    fn base(&self, pr: &Pr) -> Result<Base, String> {
        let head = format!("pull/{}/head", pr.number);
        self.git(&["fetch", "-q", "origin", &self.base, &head])?;
        let main = format!("origin/{}", self.base);
        let missing = self.git(&["rev-list", "--count", &format!("{}..{main}", pr.head)])?;
        if missing.trim() == "0" {
            return Ok(Base::Holds);
        }
        let args = [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "-z",
            &main,
            &pr.head,
        ];
        let (status, text, why) =
            process::answer("git", &args, Some(&self.repo), &[], GIT).map_err(String::from)?;
        if let Some(files) = conflicts_in(status.code(), &text).map_err(|e| format!("{e} {why}"))? {
            return Ok(Base::Conflicts(files));
        }
        let moved = self.git(&["diff", "--name-only", &format!("{}...{main}", pr.head)])?;
        let read = |f: &&str| !INERT.iter().any(|inert| f.starts_with(inert));
        Ok(if moved.lines().any(|f| read(&f)) {
            Base::Behind
        } else {
            Base::Holds
        })
    }

    fn merge(&self, pr: &Pr) -> Result<(), String> {
        let number = pr.number.to_string();
        let args = [
            "pr",
            "merge",
            &number,
            "--squash",
            "--match-head-commit",
            &pr.head,
        ];
        self.gh(&args).map(drop)
    }

    // In its session's worktree, which `pr` reads its head from: GitHub's update-branch answers before it moves the head, and
    // leaves this one behind, so the PR would read as none. Only on a met run's stop, its session idle, and only on its branch at
    // the head read. Main as `base` fetched it, the one it judged clean. A merge that stops (a file it changed and did not commit),
    // or a push the origin did not take, is undone, what it did not commit kept.
    fn update(&self, id: &str, pr: &Pr) -> Result<(), String> {
        let path = self.folder(id);
        let path = path.to_string_lossy();
        let at = |args: &[&str]| self.git(&[&["-C", &path][..], args].concat());
        let branch = format!("task/{id}");
        let was = at(&["rev-parse", "HEAD"])?;
        let on = at(&["symbolic-ref", "-q", "--short", "HEAD"]).unwrap_or_default();
        if was.trim() != pr.head || on.trim() != branch {
            let url = &pr.url;
            return Err(format!(
                "its worktree moved off {url}'s head since it was read"
            ));
        }
        let main = format!("origin/{}", self.base);
        let who = ["-c", "user.name=dash", "-c", "user.email=dash@localhost"];
        let merge = ["merge", "-q", "--no-edit", &main];
        if let Err(e) = at(&[&who[..], &merge].concat()) {
            let _ = at(&["merge", "--abort"]);
            return Err(format!("main would not merge into {}: {e}", pr.url));
        }
        let to = format!("HEAD:refs/heads/{branch}");
        if let Err(e) = at(&["push", "-q", "origin", &to]) {
            // A push that timed out may have landed: undone then, the worktree would no longer hold the PR's head.
            let now = at(&["rev-parse", "HEAD"])?;
            let theirs = at(&["ls-remote", "origin", &format!("refs/heads/{branch}")]);
            if theirs.is_ok_and(|t| t.starts_with(now.trim())) {
                return Ok(());
            }
            let _ = at(&["reset", "-q", "--keep", was.trim()]);
            return Err(format!("main merged into {} but not pushed: {e}", pr.url));
        }
        Ok(())
    }

    // Its branch goes too, here and at the origin: squashed into main, it holds nothing else, and a task reopened on it would start
    // from an old head that still reads as its merged PR's. Best effort: a reopen on a branch left behind only pushes a merge.
    fn leave(&self, id: &str) -> Result<(), String> {
        let path = self.folder(id);
        self.git(&["worktree", "remove", &path.to_string_lossy()])?;
        let branch = format!("task/{id}");
        let _ = self.git(&["branch", "-D", &branch]);
        let _ = self.git(&["push", "-q", "origin", "--delete", &branch]);
        Ok(())
    }

    fn restack(&self, merged: &Pr, busy: &[String]) -> Result<Vec<String>, String> {
        let fields = "number,headRefName,headRefOid,baseRefName";
        let args = [
            "pr", "list", "--state", "open", "--limit", "100", "--json", fields,
        ];
        // A session at work on its branch would find it rewritten under it: its PR is left to its task, as any conflict.
        let opens = opens_in(&self.gh(&args)?)?;
        let opens: Vec<Open> = opens
            .into_iter()
            .filter(|o| o.head != merged.head && !busy.contains(&o.branch))
            .collect();
        if opens.is_empty() {
            return Ok(vec![]);
        }
        // The merged head too: merged by another hand, this repo may never have fetched it.
        let mut numbers: Vec<u64> = opens.iter().map(|o| o.number).collect();
        numbers.push(merged.number);
        self.fetch_pulls(&numbers)?;
        let mut said = vec![];
        for open in opens {
            // One that fails says so and the rest go on: each is its own PR.
            match self.restack_one(&merged.head, &open) {
                Ok(Some(words)) => said.push(words),
                Ok(None) => {}
                Err(e) => said.push(format!("#{}: not restacked: {e}", open.number)),
            }
        }
        Ok(said)
    }

    // 7 to 14 GB a worktree filled the disk to 2% on 2026-11-09. No session's work is in it, so it goes even when git keeps the
    // worktree or its session may still be there.
    fn clean(&self, id: &str) -> Result<(), String> {
        let target = self.folder(id).join("rust/target");
        match std::fs::remove_dir_all(&target) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {e}", target.display()))
            }
            _ => Ok(()),
        }
    }

    // The Makefile's one step that takes a new main live, from the checkout its worktrees hang off: it builds origin/main. One at
    // a time, on a lock of its own (the build's covers the build alone): two merges seconds apart would each kill the other's new
    // binary and roll it back. Skipped, once the lock is held, when the page's « Built from » commit holds main's rust/ already: a
    // merge outside it, or one a deploy before it took live, never restarts the page. Under bin/bounded, which ends cargo with make.
    // ponytail: a manual make rust/deploy, or rust/front-door's, takes no lock; the lock moves into the Makefile once host code
    // changes for another reason.
    // Its answer says what serves: the page's commit when it holds main's rust/ already, else the one make proved the page names
    // (rust/deploy exits 0 only then), never what was meant to.
    fn deploy(&self) -> Result<String, String> {
        let live = self.repo.join("private/live/rust");
        std::fs::create_dir_all(&live).map_err(|e| format!("{}: {e}", live.display()))?;
        let lock = live.join("deploy.lock");
        let held = std::fs::File::create(&lock).map_err(|e| format!("{}: {e}", lock.display()))?;
        held.lock()
            .map_err(|e| format!("{}: {e}", lock.display()))?;
        self.git(&["fetch", "-q", "origin", &self.base])?;
        let tree = |at: &str| self.git(&["rev-parse", &format!("{at}:rust")]);
        let head = format!("origin/{}", self.base);
        let main = tree(&head)?;
        let page = process::run("curl", &["-fs", "-m", "2", &self.page], None, CALL);
        let served = page.ok().and_then(|p| built_from(&p));
        let said = match served.filter(|sha| tree(sha).is_ok_and(|t| t == main)) {
            Some(sha) => {
                let head = self.git(&["rev-parse", &head])?;
                format!("{sha} serves, no build: it holds the rust/ of main's {head}")
            }
            None => {
                let repo = self.repo.to_string_lossy();
                let bounded = self.repo.join("bin/bounded");
                let secs = DEPLOY.as_secs().to_string();
                let args = [secs.as_str(), "make", "-C", &repo, "rust/deploy"];
                process::run(&bounded.to_string_lossy(), &args, None, DEPLOY + CALL)?;
                let commit = live.join("commit");
                let sha = std::fs::read_to_string(&commit)
                    .map_err(|e| format!("{}: {e}", commit.display()))?;
                format!("{} serves", sha.trim())
            }
        };
        self.restart_look(&live)
            .map_err(|e| format!("{said}, but the orchestrator's look kept its old binary: {e}"))?;
        Ok(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_folder_the_older_app_would_adopt_is_refused_and_another_comes_back_real() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-folder");
        std::fs::create_dir_all(dir.0.join("rust-t/sub")).expect("dirs");
        std::fs::create_dir_all(dir.0.join("task-t/sub")).expect("dirs");
        let real = std::fs::canonicalize(dir.0.join("rust-t")).expect("real");
        assert_eq!(
            real_folder(&dir.0.join("rust-t/")).as_deref(),
            Ok(real.to_string_lossy().as_ref())
        );
        assert!(real_folder(&dir.0.join("task-t")).is_err_and(|e| e.contains("adopt")));
        assert!(real_folder(&dir.0.join("task-t/sub")).is_err_and(|e| e.contains("adopt")));
        assert!(real_folder(&dir.0.join("gone")).is_err_and(|e| e.contains("gone")));
    }

    #[test]
    fn gh_s_answers_read_as_its_newest_pr_a_closed_one_none() {
        let _budget = TestBudget::new();
        assert_eq!(listed_in("[]").expect("list"), None);
        let open =
            r#"[{"number":7,"url":"u","state":"OPEN","headRefOid":"abc","mergeCommit":null}]"#;
        let listed = listed_in(open).expect("list").expect("pr");
        let pr = &listed.pr;
        assert_eq!(
            (pr.number, pr.url.as_str(), pr.head.as_str(), pr.merged),
            (7, "u", "abc", false)
        );
        assert_eq!(listed.squash, "");
        let merged =
            r#"[{"number":7,"state":"MERGED","headRefOid":"abc","mergeCommit":{"oid":"def"}}]"#;
        let listed = listed_in(merged).expect("list").expect("pr");
        assert!(listed.pr.merged);
        assert_eq!(listed.squash, "def");
        let closed = r#"[{"number":7,"state":"CLOSED","headRefOid":"abc"}]"#;
        assert_eq!(listed_in(closed).expect("list"), None);
        assert!(listed_in("nope").is_err());
    }

    #[test]
    fn a_merge_tree_reads_as_behind_or_its_conflicting_files() {
        let _budget = TestBudget::new();
        let listed = "41862 /live/dash orch --data /live/data\n7 /live/dash task run --data /live/data t1\n\
                      8 /other/dash orch --data /x\n9 vim /live/dash orch --data /live/data\n\
                      10 /live/dash orch --data /live/data --sync\n";
        assert_eq!(
            looks_in(listed, "/live/dash"),
            ["41862"],
            "the look alone, never a run"
        );
        assert_eq!(built_from("x\nBuilt from abc.\n").as_deref(), Some("abc"));
        assert_eq!(built_from("a row: Built from abc.\n"), None);
        assert_eq!(conflicts_in(Some(0), "tree\0").expect("read"), None);
        let two = "tree\0a.rs\0b c.rs\0\0Auto-merging a.rs\0CONFLICT (content): a.rs\0";
        let files = conflicts_in(Some(1), two).expect("read");
        assert_eq!(files, Some(vec!["a.rs".into(), "b c.rs".into()]));
        assert!(conflicts_in(Some(128), "fatal").is_err());
        assert!(conflicts_in(None, "").is_err());
    }

    #[test]
    fn checks_read_as_the_merge_hook_says() {
        let _budget = TestBudget::new();
        let read = |t: &str| checks_in(t).expect("rows");
        assert_eq!(read(r#"[["a","SUCCESS"],["b","SKIPPED"]]"#), Checks::Green);
        assert_eq!(read("[]"), Checks::Pending);
        assert_eq!(
            read(r#"[["a","SUCCESS"],["b","PENDING"]]"#),
            Checks::Pending
        );
        assert_eq!(read(r#"[["a","BILLING"]]"#), Checks::Pending);
        assert_eq!(
            read(r#"[["a","FAILURE"],["b","PENDING"],["c","NOT_RUN"]]"#),
            Checks::Red("a, c".into())
        );
        assert!(checks_in("nope").is_err());
    }

    /// Done-when 2 of ci-wait-gh-retry: the first two reads time out, the third answers green, and the wait exits 0; reads that
    /// never will answer give no verdict at once.
    #[test]
    fn ci_wait_asks_again_after_a_read_that_timed_out() {
        let _budget = TestBudget::new();
        let rows = |r: &[(&str, &str)]| -> Rows {
            let row = |(n, s): &(&str, &str)| (n.to_string(), s.to_string());
            r.iter().map(row).collect()
        };
        let unread = |again| {
            Err::<Rows, _>(Unread {
                again,
                why: "gh did not answer".into(),
            })
        };
        let later = Instant::now() + CI_DEADLINE;
        let (mut reads, mut waits) = (0, 0);
        let read = || {
            reads += 1;
            if reads <= 2 {
                unread(true)
            } else {
                Ok(rows(&[("ci", "SUCCESS"), ("old", "SKIPPED")]))
            }
        };
        let got = ci_wait(
            read,
            || {
                waits += 1;
                true
            },
            later,
        );
        assert_eq!(
            got,
            (
                0,
                "ci wait: green on the head (2 checks)\n".into(),
                String::new()
            )
        );
        assert_eq!((reads, waits), (3, 2));

        let mut reads = 0;
        let (exit, _, why) = ci_wait(
            || {
                reads += 1;
                unread(false)
            },
            || true,
            later,
        );
        assert_eq!(
            (exit, reads, why.as_str()),
            (3, 1, "ci wait: gh did not answer\n")
        );

        // the older app's rules: red is final at once, a billing refusal is no verdict, a pending head waits
        let once = |r: Rows| ci_wait(move || Ok(r.clone()), || true, Instant::now());
        let (exit, _, why) = once(rows(&[("a", "FAILURE"), ("b", "PENDING")]));
        assert_eq!(
            (exit, why.as_str()),
            (1, "ci wait: red on the head: a failure\n")
        );
        let (exit, _, why) = once(rows(&[("a", "SUCCESS"), ("b", "BILLING")]));
        assert!(
            exit == 3 && why.contains("GitHub ran none of b (billing)"),
            "{why}"
        );
        let (exit, _, why) = once(rows(&[("a", "PENDING")]));
        assert!(exit == 3 && why.contains("still pending: a"), "{why}");
        let mut empty = 0;
        let (exit, _, why) = ci_wait(
            || Ok(rows(&[])),
            || {
                empty += 1;
                true
            },
            later,
        );
        assert!(
            exit == 3 && empty == 3 && why.contains("no workflow covers it"),
            "{why}"
        );
        let (exit, _, why) = ci_wait(|| Ok(rows(&[("a", "PENDING")])), || false, later);
        assert!(exit == 3 && why.contains("the caller left"), "{why}");
    }

    /// Done-when 2 of ci-wait-gh-retry, its second half: reads that never answer get no exit 3 before the deadline (each lost read
    /// before it is followed by another), and exit 3 once it passed.
    #[test]
    fn ci_wait_exits_3_only_once_its_deadline_passed() {
        let _budget = TestBudget::new();
        let begun = Instant::now();
        let until = begun + Duration::from_millis(50);
        let mut reads = vec![];
        let read = || {
            reads.push(Instant::now());
            Err(Unread {
                again: true,
                why: "gh did not answer".into(),
            })
        };
        let wait = || {
            std::thread::sleep(Duration::from_millis(5));
            true
        };
        let (exit, out, why) = ci_wait(read, wait, until);
        let ended = Instant::now();
        assert_eq!((exit, out.as_str()), (3, ""), "{why}");
        assert!(
            ended >= until,
            "exit 3 came {:?} before its deadline",
            until - ended
        );
        let (last, before) = reads.split_last().expect("a read");
        assert!(
            before.len() >= 5 && before.iter().all(|r| *r < until),
            "{} reads went on before it",
            before.len()
        );
        assert!(*last >= until, "the last read is the one past the deadline");
        assert_eq!(
            why,
            "ci wait: no verdict by its deadline, no read answered: gh did not answer\n"
        );
    }

    /// A real repository, a real worktree: the git half, with no forge in it.
    #[test]
    fn a_worktree_is_made_once_kept_across_a_retry_and_leaves() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-worktree");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        run(&origin, &["init", "-q", "-b", "main"]);
        run(
            &origin,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "first",
            ],
        );
        run(
            &dir.0,
            &[
                "clone",
                "-q",
                origin.to_str().expect("path"),
                repo.to_str().expect("path"),
            ],
        );
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        // What a crash half way leaves: a half-made folder, with the branch checked out there, then another with no worktree.
        let new = dir.0.join("rust-t1.new");
        run(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "task/t1",
                new.to_str().expect("path"),
                "origin/main",
            ],
        );
        std::fs::remove_file(new.join(".git")).expect("half made");
        let folder = forge.made("t1").expect("made");
        assert!(folder.ends_with("rust-t1"));
        assert!(!new.exists(), "its working name is gone");
        forge.leave("t1").expect("left");
        std::fs::create_dir_all(new.join("stray")).expect("half made");
        let folder = forge.made("t1").expect("made");
        assert!(!new.exists());
        let branch = run(Path::new(&folder), &["branch", "--show-current"]);
        assert_eq!(branch.trim(), "task/t1");
        std::fs::write(Path::new(&folder).join("left"), "x").expect("a run's work");
        assert_eq!(forge.made("t1").expect("kept"), folder);
        assert!(Path::new(&folder).join("left").exists());
        assert!(forge.leave("t1").is_err(), "git refuses a dirty one");
        std::fs::remove_file(Path::new(&folder).join("left")).expect("clean");
        forge.leave("t1").expect("left");
        assert!(!Path::new(&folder).exists());
        // A reopen: its old commit, merged and gone, is no head it starts from, so no old PR reads as holding its work.
        let again = forge.made("t1").expect("again");
        assert_eq!(again, folder);
        run(
            Path::new(&again),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "merged since",
            ],
        );
        let old = run(Path::new(&again), &["rev-parse", "HEAD"]);
        forge.leave("t1").expect("left");
        let reopened = forge.made("t1").expect("reopened");
        let head = run(Path::new(&reopened), &["rev-parse", "HEAD"]);
        assert_ne!(head, old);
        assert_eq!(head, run(&repo, &["rev-parse", "origin/main"]));
    }

    /// A real origin, PR 7's pull ref and its squash on main: a worktree's HEAD read against that PR open, then merged, then the
    /// task reopened (rust-forge-stale-worktree-head's done-when 1 to 4).
    #[test]
    fn a_head_behind_its_pr_reads_it_one_past_it_needs_a_push_and_a_reopen_starts_from_main() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-held");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| {
            let out = process::run("git", args, Some(at), GIT).expect("git");
            out.trim().to_string()
        };
        let commit = |at: &Path, message: &str| {
            let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
            let args = ["commit", "-q", "--allow-empty", "-m", message];
            run(at, &[&who[..], &args].concat());
            run(at, &["rev-parse", "HEAD"])
        };
        let push = |message: &str| {
            run(&origin, &["switch", "-q", "task/t1"]);
            let head = commit(&origin, message);
            run(&origin, &["update-ref", "refs/pull/7/head", &head]);
            run(&origin, &["switch", "-q", "main"]);
            head
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        commit(&origin, "first");
        run(&origin, &["branch", "task/t1"]);
        let a1 = push("a1");
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&repo)]);
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        let folder = PathBuf::from(forge.made("t1").expect("made"));
        run(&folder, &["reset", "-q", "--hard", &a1]);
        // A reviewer pushes on its branch: this repo has never fetched that head.
        let a2 = push("a2");
        let pr = |merged| Pr {
            number: 7,
            url: "u7".into(),
            head: a2.clone(),
            merged,
        };
        let open = Listed {
            pr: pr(false),
            squash: String::new(),
        };
        let judged = |listed: &Listed| forge.judged("t1", Some(listed.clone())).expect("read");
        assert_eq!(judged(&open), Some(pr(false)), "1: behind its head");
        commit(&folder, "b");
        assert_eq!(judged(&open), None, "2: work its head lacks");
        run(&folder, &["reset", "-q", "--hard", &a1]);
        let squash = commit(&origin, "squash of 7");
        let merged = Listed {
            pr: pr(true),
            squash,
        };
        assert_eq!(judged(&merged), Some(pr(true)), "3: behind a merged head");
        commit(&origin, "main after it");
        run(&folder, &["fetch", "-q", "origin", "main"]);
        run(&folder, &["reset", "-q", "--hard", "origin/main"]);
        assert_eq!(judged(&merged), Some(pr(true)), "3: main after its squash");
        let past = commit(&folder, "c");
        assert_eq!(judged(&merged), None, "2: work past main");
        // 4: a reopen keeps a head past the merge, and a dirty folder, whole; else starts from main, its branch gone everywhere.
        let head = |p: &Path| run(p, &["rev-parse", "HEAD"]);
        assert_eq!(
            forge.opened("t1", Some(&merged)).expect("kept"),
            at(&folder)
        );
        assert_eq!(head(&folder), past);
        run(&folder, &["reset", "-q", "--hard", &a1]);
        std::fs::write(folder.join("left"), "x").expect("a run's work");
        forge.opened("t1", Some(&merged)).expect("kept");
        assert!(folder.join("left").exists());
        assert_eq!(head(&folder), a1);
        std::fs::remove_file(folder.join("left")).expect("clean");
        let tracked = "refs/remotes/origin/task/t1";
        assert!(!run(&repo, &["for-each-ref", tracked]).is_empty());
        let fresh = PathBuf::from(forge.opened("t1", Some(&merged)).expect("reopened"));
        let main = run(&repo, &["rev-parse", "origin/main"]);
        assert_eq!(head(&fresh), main, "from main");
        assert_eq!(run(&fresh, &["branch", "--show-current"]), "task/t1");
        let at_origin = ["ls-remote", "--heads", "origin", "refs/heads/task/t1"];
        assert_eq!(run(&repo, &at_origin), "", "gone at the origin");
        assert_eq!(run(&repo, &["for-each-ref", tracked]), "", "pruned");
        assert_eq!(
            judged(&merged),
            None,
            "a reopen's branch: none of its work merged"
        );
        // GitHub deleted the branch itself: the stale ref this repo kept is pruned, the reopen's own branch kept.
        run(&repo, &["update-ref", tracked, &a1]);
        let again = PathBuf::from(forge.opened("t1", Some(&merged)).expect("reopened"));
        assert_eq!(run(&repo, &["for-each-ref", tracked]), "", "pruned");
        assert_eq!(head(&again), main, "kept");
    }

    /// A real origin and its pull refs: only a head holding main's newest commit lands; one behind it merging cleanly and one
    /// conflicting are told apart, the conflict naming its file.
    #[test]
    fn a_head_holds_main_or_lags_it_cleanly_or_conflicts_naming_the_file() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-base");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let commit = |file: &str, text: &str| {
            std::fs::write(origin.join(file), text).expect("write");
            run(&origin, &["add", file]);
            let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
            run(&origin, &[&who[..], &["commit", "-q", "-m", file]].concat());
            run(&origin, &["rev-parse", "HEAD"]).trim().to_string()
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        commit("c.rs", "1\n");
        let pull = |number: u64, branch: &str, file: &str, text: &str| {
            run(&origin, &["switch", "-q", "-c", branch, "main"]);
            let head = commit(file, text);
            run(
                &origin,
                &["update-ref", &format!("refs/pull/{number}/head"), &head],
            );
            run(&origin, &["switch", "-q", "main"]);
            Pr {
                number,
                url: format!("u{number}"),
                head,
                merged: false,
            }
        };
        let (apart, clash) = (pull(7, "a", "a.rs", "a\n"), pull(8, "b", "c.rs", "b\n"));
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&repo)]);
        let forge = GitHub {
            repo,
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        assert_eq!(forge.base(&apart).expect("read"), Base::Holds);
        commit("EXPLAINER.md", "folded\n");
        assert_eq!(
            forge.base(&apart).expect("read"),
            Base::Holds,
            "main moved in what nothing builds alone"
        );
        commit("c.rs", "main\n");
        assert_eq!(forge.base(&apart).expect("read"), Base::Behind);
        let files = vec!["c.rs".to_string()];
        assert_eq!(forge.base(&clash).expect("read"), Base::Conflicts(files));
    }

    /// A merged task's build output goes even when git keeps its worktree for a file left in it.
    #[test]
    fn a_merged_task_s_build_output_goes_even_from_a_worktree_git_keeps() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-target");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        run(&origin, &["init", "-q", "-b", "main"]);
        std::fs::write(origin.join(".gitignore"), "target/\n").expect("write");
        run(&origin, &["add", ".gitignore"]);
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        run(
            &origin,
            &[&who[..], &["commit", "-q", "-m", "first"]].concat(),
        );
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&repo)]);
        let forge = GitHub {
            repo,
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        let folder = PathBuf::from(forge.made("t1").expect("made"));
        let target = folder.join("rust/target/release");
        std::fs::create_dir_all(&target).expect("a build");
        std::fs::write(target.join("dash"), "x").expect("a build");
        std::fs::write(folder.join("left"), "x").expect("a run's work");
        forge.clean("t1").expect("cleaned");
        assert!(
            !folder.join("rust/target").exists(),
            "its build output went"
        );
        assert!(forge.leave("t1").is_err(), "git refuses a dirty one");
        assert!(folder.join("left").exists(), "its work stays");
        forge.clean("t1").expect("nothing left to clean");
        std::fs::remove_file(folder.join("left")).expect("clean");
        forge.leave("t1").expect("left");
        assert!(!folder.exists());
    }

    /// A page whose « Built from » commit holds main's rust/ already is never built again: it stays up for a merge outside rust/.
    /// One that lags, or no page, goes to bin/bounded, which this stand-in checkout lacks, so its failure names it.
    #[test]
    fn a_deploy_builds_only_when_main_s_rust_is_not_live() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-deploy");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(origin.join("rust")).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = |file: &str| {
            std::fs::write(origin.join(file), file).expect("write");
            run(&origin, &["add", file]);
            run(&origin, &[&who[..], &["commit", "-q", "-m", file]].concat());
            run(&origin, &["rev-parse", "HEAD"]).trim().to_string()
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        let live = commit("rust/lib.rs");
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&repo)]);
        // The page as dash serve gives it, to every look.
        let server = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = server.local_addr().expect("its port").port();
        let named = live.clone();
        std::thread::spawn(move || {
            for stream in server.incoming() {
                use std::io::{Read, Write};
                let mut stream = stream.expect("a look");
                let _ = stream.read(&mut [0; 1024]);
                let body = format!("Tasks\nBuilt from {named}.\n");
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                let _ = stream.write_all(format!("{head}{body}").as_bytes());
            }
        });
        let mut forge = GitHub {
            repo,
            root: dir.0.clone(),
            base: "main".into(),
            page: format!("http://127.0.0.1:{port}/"),
            cli: "gh".into(),
        };
        commit("EXPLAINER.md");
        let served = forge.deploy().expect("nothing to build");
        let head = run(&forge.repo, &["rev-parse", "origin/main"]);
        let said = format!("{live} serves, no build: it holds the rust/ of main's {head}");
        assert_eq!(served, said, "the sha the page names, and main's");
        let lock = forge.repo.join("private/live/rust/deploy.lock");
        assert!(lock.exists(), "its folder made");
        commit("rust/main.rs");
        let e = forge.deploy().expect_err("a build");
        assert!(e.contains("bin/bounded"), "{e}");
        forge.page = "http://127.0.0.1:9/".into();
        let e = forge.deploy().expect_err("no page, a build");
        assert!(e.contains("bin/bounded"), "{e}");
    }

    /// A met PR behind main: main merged into its branch in its worktree and pushed, that worktree's head the pushed one, so `pr`
    /// still finds it. A merge a file it did not commit stops is undone, that file kept.
    #[test]
    fn an_update_merges_main_into_the_task_branch_where_its_session_works_and_pushes() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-update");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = |at: &Path, file: &str| {
            std::fs::write(at.join(file), file).expect("a file");
            run(at, &["add", file]);
            run(at, &[&who[..], &["commit", "-q", "-m", file]].concat());
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        commit(&origin, "first");
        let (o, r) = (origin.to_str().expect("path"), repo.to_str().expect("path"));
        run(&dir.0, &["clone", "-q", o, r]);
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        let folder = forge.worktree("t1").expect("made");
        let work = Path::new(&folder);
        commit(work, "work");
        run(work, &["push", "-q", "origin", "task/t1"]);
        commit(&origin, "main");
        run(&repo, &["fetch", "-q", "origin", "main"]);
        let head = run(work, &["rev-parse", "HEAD"]);
        let pr = Pr {
            number: 7,
            url: "https://pr/7".into(),
            head: head.trim().into(),
            merged: false,
        };
        let moved = Pr {
            head: "0000".into(),
            ..pr.clone()
        };
        let off = forge.update("t1", &moved).expect_err("not the head read");
        assert!(off.contains("moved off"), "{off}");
        std::fs::write(work.join("main"), "his").expect("not committed");
        let refused = forge
            .update("t1", &pr)
            .expect_err("the merge would overwrite it");
        assert!(
            refused.contains("main would not merge into https://pr/7"),
            "{refused}"
        );
        assert_eq!(run(work, &["rev-parse", "HEAD"]), head, "undone");
        assert_eq!(
            std::fs::read_to_string(work.join("main")).expect("kept"),
            "his"
        );
        std::fs::remove_file(work.join("main")).expect("clean");
        forge.update("t1", &pr).expect("updated");
        let pushed = run(&origin, &["rev-parse", "task/t1"]);
        assert_eq!(run(work, &["rev-parse", "HEAD"]), pushed);
        run(&origin, &["merge-base", "--is-ancestor", "main", "task/t1"]);
        run(
            &origin,
            &["merge-base", "--is-ancestor", head.trim(), "task/t1"],
        );
    }

    /// rust-skeleton-live's case: a PR stacked on another's head conflicts once that one squash-merges. Its own commits move onto
    /// main; one not stacked is left alone; one whose own commits conflict with main is given up, its branch untouched.
    #[test]
    fn a_pr_stacked_on_a_squash_merged_branch_is_rebased_onto_main() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("forge-restack");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = |file: &str, text: &str| {
            std::fs::write(origin.join(file), text).expect("write");
            run(&origin, &["add", file]);
            run(&origin, &[&who[..], &["commit", "-q", "-m", file]].concat());
            run(&origin, &["rev-parse", "HEAD"]).trim().to_string()
        };
        let on = |branch: &str, from: &str| run(&origin, &["switch", "-q", "-C", branch, from]);
        let pull = |number: u64, branch: &str, head: &str| {
            run(
                &origin,
                &["update-ref", &format!("refs/pull/{number}/head"), head],
            );
            Open {
                number,
                branch: branch.into(),
                head: head.into(),
                base: "main".into(),
            }
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        commit("m.rs", "m\n");
        on("a", "main");
        let merged = commit("a.rs", "a\n");
        on("b", "a");
        let b = pull(2, "b", &commit("b.rs", "b\n"));
        on("d", "a");
        let d = pull(4, "d", &commit("a.rs", "d\n"));
        on("z", "a");
        let z = commit("z.rs", "z\n");
        let y = pull(6, "a", &merged);
        on("c", "main");
        let c = pull(3, "c", &commit("c.rs", "c\n"));
        // a squash-merged, then main changes what d changes too.
        on("main", "main");
        run(&origin, &["checkout", "-q", "a", "--", "a.rs"]);
        run(
            &origin,
            &[&who[..], &["commit", "-q", "-m", "a squashed (#1)"]].concat(),
        );
        commit("a.rs", "main\n");
        on("idle", "main");
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&repo)]);
        let forge = GitHub {
            repo,
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        forge.fetch_pulls(&[2, 3, 4, 6]).expect("fetched");
        let said = forge.restack_one(&merged, &b).expect("restacked");
        assert_eq!(
            said.as_deref(),
            Some("#2 rebased onto main off the merged branch")
        );
        let main = run(&origin, &["rev-parse", "main"]);
        assert_eq!(
            run(&origin, &["rev-parse", "b~1"]),
            main,
            "its own commit, on main"
        );
        assert_eq!(
            forge.restack_one(&merged, &c).expect("read"),
            None,
            "not stacked"
        );
        let said = forge
            .restack_one(&merged, &d)
            .expect("given up")
            .expect("stacked");
        assert!(said.contains("#4 conflicts with main"), "{said}");
        assert_eq!(
            run(&origin, &["rev-parse", "d"]).trim(),
            d.head,
            "untouched"
        );
        // z, built on y's head, merged first: y's commits landed in its squash, so y is left as it is, never pushed to main.
        let said = forge.restack_one(&z, &y).expect("read").expect("said");
        assert!(said.contains("landed in that squash"), "{said}");
        assert_eq!(
            run(&origin, &["rev-parse", "a"]).trim(),
            merged,
            "untouched"
        );
        assert!(!dir.0.join("restack-4").exists() && !dir.0.join("restack-2").exists());
        let opens = r#"[{"number":2,"headRefName":"b","headRefOid":"x","baseRefName":"a"}]"#;
        let read = opens_in(opens).expect("read");
        assert_eq!((read[0].number, read[0].base.as_str()), (2, "a"));
    }
}
