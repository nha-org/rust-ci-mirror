//! Core's Forge on git and GitHub's CLI. A task's worktree is `rust-<id>` beside the others, its branch `task/<id>`, which CI's
//! title check, the task page and the older app's PR reads all expect.
pub mod checks;
pub mod live;
pub use checks::{GREEN, Rows};
use live::built_from;
pub mod mirror;
pub mod queue;
pub mod red;
pub mod runs;
pub mod sends;
pub use sends::{SEND_PATHS, sends};
pub mod watchdog;

use dash_core::tasks::{Base, Checks, Forge, Pr, Review, Serving, verdict_of};
use dash_process as process;
use serde_json::Value;
use std::collections::HashSet;
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

/// `GitHub::remerge`'s answer, as the Elixir `merge_main` gave it: its merge pushed, the branch moved off the head read, the
/// head already holding main, or the files main conflicts in.
#[derive(Debug, PartialEq)]
pub enum Remerge {
    Pushed(String),
    Moved,
    Holds,
    Conflicts(Vec<String>),
}

/// One PR of a merge train's batch, in its order.
pub struct TrainPr {
    pub number: u64,
    pub head: String,
    pub title: String,
    pub url: String,
}

/// A built `train/<n>`: the main it starts from, its tip, the PRs squashed on it in order, and each left out with why.
#[derive(Debug)]
pub struct Train {
    pub base: String,
    pub tip: String,
    pub took: Vec<String>,
    pub out: Vec<(String, String)>,
}

const CALL: Duration = Duration::from_secs(30);
/// A fetch and a worktree of a large repository.
const GIT: Duration = Duration::from_secs(300);
/// A release build of the whole tool, then up to 60 s for the page to name it.
const DEPLOY: Duration = Duration::from_secs(1800);
/// How long a worktree stays untouched before `reap` judges it.
const SETTLED: Duration = Duration::from_secs(3600);
/// How long a task's worktree that holds nothing a remote lacks stays untouched before `reap` releases it, its PR open or not.
const UNHELD: Duration = Duration::from_secs(24 * 3600);

/// The folder the pool's slots sit in, beside the tasks' worktrees. Its `.noindex` suffix keeps Spotlight out: mds read the idle
/// slots' builds at 13 MB/s (task reap-kept-worktrees).
const POOL: &str = "pool.noindex";

/// How many idle worktrees the pool keeps, as `pool.noindex/pool-<n>` (rfc-rust-unified#worktrees): past it a worktree that
/// leaves is removed, so the slots' builds stay under SLOTS times `SLOT_CAP`, 20 GB; every build's total is `BUILDS_CAP`.
const SLOTS: usize = 4;
/// What a slot's build may hold when it joins the pool; no session builds in it after, so it never grows there.
const SLOT_CAP: u64 = 5 << 30;
/// What every task worktree's and slot's build may total once the reap passed: past it the idle ones go, oldest built first. 14 idle
/// builds of 2 to 6 GB held 57 GB on 2026-11-13, none past `SLOT_CAP` (task rust-idle-builds-total).
const BUILDS_CAP: u64 = 30 << 30;
/// How long a test run's folder stays in TMPDIR before `prune_test_runs` takes it.
const TEST_RUN_KEPT: Duration = Duration::from_secs(24 * 3600);
/// How many of them one hourly pass takes.
const PRUNED_A_PASS: usize = 50;

/// TMPDIR's `dash-test-run-*` folders untouched for a day, gone: a test run killed half way leaves its own, up to 320 MB each
/// (262 of them on 2026-11-11, task rust-worktree-pool). What went, in words; one that will not go is said and the rest go on.
/// `PRUNED_A_PASS` at most, so a backlog never holds the look for minutes: the next hour takes the rest.
/// ponytail: its age is the folder's own, which a write deeper down does not move; no test run lasts a day.
pub fn prune_test_runs(tmp: &Path) -> Vec<String> {
    let (mut gone, mut said) = (0, vec![]);
    for entry in std::fs::read_dir(tmp).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if gone == PRUNED_A_PASS {
            break;
        }
        if !name.starts_with("dash-test-run-") {
            continue;
        }
        // a symbolic link is never followed: its own metadata is no folder's
        let old = entry.metadata().is_ok_and(|m| {
            let age = m.modified().ok().and_then(|t| t.elapsed().ok());
            m.is_dir() && age.is_some_and(|a| a >= TEST_RUN_KEPT)
        });
        if !old {
            continue;
        }
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => gone += 1,
            Err(e) => said.push(format!("{name}: a test folder past a day stays: {e}")),
        }
    }
    if gone > 0 {
        let tmp = tmp.display();
        said.push(format!("{gone} test folders past a day pruned from {tmp}"));
    }
    said
}

/// What main changes most often and nothing builds or tests: a head lacking only these still holds main.
const INERT: [&str; 3] = ["explainer.d/", "EXPLAINER.md", "private/docs/pm-daily.html"];

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

/// The bytes a folder takes on disk, as du counts them (a file cargo hard-links into its profile's folder once), up to the first
/// count past `past`: a look walks no more of a 30 GB build than its cap. A folder not there is 0.
fn bytes(dir: &Path, past: u64) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let (mut total, mut todo, mut seen) = (0, vec![dir.to_path_buf()], HashSet::new());
    while let Some(at) = todo.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            // a symbolic link is counted, never followed
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                todo.push(entry.path());
            } else if meta.nlink() < 2 || seen.insert((meta.dev(), meta.ino())) {
                total += meta.blocks() * 512;
                if total > past {
                    return total;
                }
            }
        }
    }
    total
}

/// When cargo last wrote in a build: the newest change of the folder, its profiles' (`debug`, a triple) and their folders'
/// (`deps`, `.fingerprint`), where a compile adds or swaps files. Never read: the oldest.
fn last_built(target: &Path) -> std::time::SystemTime {
    let changed = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let all = std::iter::once(target.to_path_buf()).chain(profiles(target));
    all.filter_map(|p| changed(&p))
        .max()
        .unwrap_or(std::time::UNIX_EPOCH)
}

fn subfolders(at: &Path) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(at).into_iter().flatten().flatten();
    entries.map(|e| e.path()).filter(|p| p.is_dir()).collect()
}

/// A build's folders two levels down: its profiles (`debug`, a triple) and theirs (`deps`, `.fingerprint`, a triple's `release`),
/// where cargo keeps its locks.
fn profiles(target: &Path) -> Vec<PathBuf> {
    let top = subfolders(target);
    let under = top.iter().flat_map(|d| subfolders(d));
    top.clone().into_iter().chain(under).collect()
}

/// A build folder gone, never while a cargo builds there: each lock cargo takes in it (`.cargo-lock`, `.cargo-build-lock`,
/// `.cargo-artifact-lock`, in a profile's folder or a target triple's) is held while the folder moves aside to `<target>.trash`,
/// then that goes: a cargo starting meanwhile builds a new folder, never into one half gone (task rust-disk-caps; /code-review of
/// 487680671). A trash an earlier delete left goes first.
fn remove_build(target: &Path) -> Result<(), String> {
    let locks = cargo_locks(target)?;
    let trash = target.with_extension("trash");
    let gone = |at: &Path| match std::fs::remove_dir_all(at) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", at.display())),
        _ => Ok(()),
    };
    gone(&trash)?;
    match std::fs::rename(target, &trash) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {e}", target.display()));
        }
        _ => drop(locks),
    }
    gone(&trash)
}

/// Every lock cargo takes in a build folder, held: no cargo builds there while they live. Err: one does.
fn cargo_locks(target: &Path) -> Result<Vec<std::fs::File>, String> {
    let mut locks = vec![];
    for profile in profiles(target) {
        for name in [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"] {
            let at = profile.join(name);
            let lock = match std::fs::File::open(&at) {
                Ok(lock) => lock,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("{}: {e}", at.display())),
            };
            match lock.try_lock() {
                Ok(()) => locks.push(lock),
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(format!("a cargo runs in {}", target.display()));
                }
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(format!("{}: {e}", at.display()));
                }
            }
        }
    }
    Ok(locks)
}

/// A head's checks (`checks::checks`), `[name, state]`. PENDING and BILLING (GitHub refused to run it) hold no verdict; any
/// other state not green is red. No row yet is no verdict either: a push's runs list within a minute.
#[cfg(test)]
fn checks_in(text: &str) -> Result<Checks, String> {
    Ok(checks_of(&rows_in(text)?))
}

fn checks_of(rows: &Rows) -> Checks {
    let red = named(rows, red);
    if !red.is_empty() {
        return Checks::Red(red.join(", "));
    }
    let settled = !rows.is_empty() && rows.iter().all(|(_, s)| GREEN.contains(&s.as_str()));
    if settled {
        Checks::Green
    } else {
        Checks::Pending
    }
}

#[cfg(test)]
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
/// The head's checks (`checks::checks` in `cwd`, the caller's checkout, rust/ci-mirror.txt read under `root`): each job's latest
/// attempt, a cancelled run superseded by a newer one dropped, a job GitHub refused for billing read BILLING (task
/// ci-wait-command). A red `ci-mirror/rust` is read again from the mirror's own runs (`mirror::reread`): a rerun there is no
/// verdict yet, never the old red. An Err may answer at the next poll (gh did not answer, under load): `ci_wait` asks again.
pub fn read_checks(root: &Path, cwd: &Path, target: &str) -> Result<Rows, String> {
    read_rows(root, cwd, target, "gh")
}

/// `read_checks` through `gh`.
fn read_rows(root: &Path, cwd: &Path, target: &str, gh: &str) -> Result<Rows, String> {
    let ask = checks::gh_in(gh, cwd);
    let rows = checks::checks(&ask, target, None, cwd, root).ok_or("gh did not answer")?;
    let mut gh = |args: &[&str]| process::run(gh, args, Some(cwd), CALL);
    reread_mirror(rows, target, &mut gh)
}

/// `rows` with a red `ci-mirror/rust` read again from the mirror's own runs (`mirror::reread`); any other row costs no call.
fn reread_mirror(mut rows: Rows, target: &str, gh: mirror::Gh) -> Result<Rows, String> {
    let Some(row) = (rows.iter_mut()).find(|(n, s)| n == mirror::CONTEXT && red(s)) else {
        return Ok(rows);
    };
    match mirror::reread(target, gh)? {
        mirror::Reread::Running => row.1 = "PENDING".into(),
        mirror::Reread::Green => row.1 = "SUCCESS".into(),
        mirror::Reread::Red(None) => {}
        mirror::Reread::Red(Some(why)) => {
            // its first line, cut: a gh error may run over many (/code-review of 0906afff9)
            let why: String = why
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect();
            row.0 = format!("{} (its log not posted on the PR: {why})", mirror::CONTEXT)
        }
    }
    Ok(rows)
}

/// The reds of `names` on `target` (a PR or a sha) read through gh in `cwd`, the caller's checkout; Err: a gh call failed.
pub fn reds_in(cwd: &Path, target: &str, names: &[&str]) -> Result<Vec<red::Red>, String> {
    let gh = |args: &[&str]| process::run("gh", args, Some(cwd), CALL);
    red::reds(&gh, target, names)
}

/// `dash ci wait <pr|sha>`, as the older app's Dashboard.Orch.CiWait: `(exit, stdout, stderr)`, 0 green, 1 red, 3 no verdict.
/// `read` asks the checks, `wait` sleeps a poll's interval and says whether the caller still listens. A lost read is asked again
/// at the next poll: no verdict comes only past `until` (task ci-wait-gh-retry: on 2026-11-01 three dash ci wait 806 exited 3 on
/// « gh did not answer » at load 120 while gh pr view answered). A red row is final at once; PENDING waits; BILLING alone is no
/// verdict, never green or red.
pub fn ci_wait(
    mut read: impl FnMut() -> Result<Rows, String>,
    mut wait: impl FnMut() -> bool,
    until: Instant,
    mut explain: impl FnMut(&[&str]) -> Result<Vec<red::Red>, String>,
) -> (i32, String, String) {
    // a head with no check after this many empty answers: no workflow covers it (a push's runs list within a minute)
    const EMPTY: u32 = 3;
    let no_verdict = |why: String| (3, String::new(), format!("ci wait: {why}\n"));
    let mut empties = 0;
    loop {
        let last = match read() {
            Err(why) => format!("no read answered: {why}"),
            Ok(rows) if rows.iter().any(|(_, s)| red(s)) => {
                // Each red's failed step named; one before any test is no verdict on the code (task rust-infra-red-reruns).
                // A failed read asks again, as GitHub::checks waits on it: the outage that turns a step red fails reads too
                // (/code-review of PR 1460). None to judge (Ok of none) is the rows' red.
                match explain(&named(&rows, red)) {
                    Err(e) => format!("its reds not read: {e}"),
                    Ok(reds) if red::all_infra(&reds) => {
                        // Others still run: one of them may be a code red, the verdict (/code-review of 79af9f8d5).
                        let pending = named(&rows, |s| s == "PENDING");
                        if pending.is_empty() {
                            let why = format!(
                                "ci wait: no verdict, an infra red: {}. Not the code's: a task's orchestrator reruns it once a later run gets past that step; a PR no task holds gets a new run only at its next push or a rerun from its run's page\n",
                                red::words(&reds)
                            );
                            return (3, String::new(), why);
                        }
                        let last = format!(
                            "an infra red ({}), still pending: {}",
                            red::words(&reds),
                            pending.join(", ")
                        );
                        if Instant::now() >= until {
                            return no_verdict(format!("no verdict by its deadline, {last}"));
                        }
                        if !wait() {
                            return no_verdict("the caller left".into());
                        }
                        continue;
                    }
                    Ok(reds) if !reds.is_empty() => {
                        let why = format!("ci wait: red on the head: {}\n", red::words(&reds));
                        return (1, String::new(), why);
                    }
                    Ok(_) => {
                        let red = (rows.iter().filter(|(_, s)| red(s)))
                            .map(|(n, s)| format!("{n} {}", s.to_lowercase()));
                        let red = red.collect::<Vec<_>>().join(", ");
                        return (
                            1,
                            String::new(),
                            format!("ci wait: red on the head: {red}\n"),
                        );
                    }
                }
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

/// A reflog line's « what moved HEAD » that made work here: a commit (a conflict's resolved merge included), a cherry-pick or
/// revert, a rebase's picks, `pull --rebase`'s too. A clean merge (`land`'s update from main, a session's), a pull, a reset, a
/// checkout or a fast-forward only brought what someone else made (/code-review of a76563791: an update counted as a push).
fn made_here(what: &str) -> bool {
    const PICKS: [&str; 5] = ["(pick)", "(squash)", "(fixup)", "(reword)", "(continue)"];
    let picked = PICKS.iter().any(|w| what.contains(w));
    picked
        || ["commit", "cherry-pick", "revert"]
            .iter()
            .any(|w| what.starts_with(w))
}

/// Tries past the first at a fetch another fetch beat to a ref's lock: that one ends in seconds.
const LOCK_RETRIES: u32 = 3;
const LOCK_PAUSE: Duration = Duration::from_secs(2);

/// `fetch` again, `pause` apart, while git says another fetch holds a lock (« cannot lock ref », or a ref or packed-refs
/// `.lock` that « File exists »), up to `LOCK_RETRIES` times; any other answer is the answer.
fn fetched(
    mut fetch: impl FnMut() -> Result<String, String>,
    pause: Duration,
) -> Result<String, String> {
    let mut retries = 0;
    loop {
        match fetch() {
            Err(e) if locked(&e) && retries < LOCK_RETRIES => {
                retries += 1;
                std::thread::sleep(pause);
            }
            done => return done,
        }
    }
}

/// `git <args>` in `repo`, a fetch: tried again past another fetch's ref lock (`fetched`), then, failed otherwise, once more
/// after `drop_missing` dropped a ref, each dropped ref one stderr line. Every Rust fetch of a checkout takes it, so a ref at a
/// missing object never blocks one (task rust-fetch-heals-missing-ref).
pub fn fetch_healed(repo: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let run = || process::run("git", args, Some(repo), limit);
    fetched(run, LOCK_PAUSE).or_else(|e| match drop_missing(repo) {
        Ok(dropped) if !dropped.is_empty() => {
            for name in &dropped {
                eprintln!("dash forge: {name} dropped, its object missing; a fetch of its branch writes it again");
            }
            fetched(run, LOCK_PAUSE)
        }
        Ok(_) => Err(e),
        Err(why) => Err(format!("{e} (its missing refs unread: {why})")),
    })
}

/// The refs under refs/remotes of `repo` whose object it lacks, each deleted at the sha read: git's fetch reads every ref as a
/// « have » and fails on one (« fatal: bad object refs/remotes/origin/task/x »), so one stale copy stopped every fetch, the
/// git CLI's too. The Elixir app's merge on green pushed from an object overlay to origin's URL, and git wrote that branch's
/// remote ref at a commit only the overlay held (task rust-fetch-heals-missing-ref). A remote ref is origin's copy: a fetch
/// naming its branch, or a whole one, writes it again; meanwhile a read of `--remotes` sees that branch as unpushed, which
/// keeps a worktree, never drops one. A store HEAD's commit does not read in (an unread pack) drops none. Symbolic refs are
/// left: their target is a line of its own. Local branches are work, never dropped. `fetch_healed` and the look, each
/// minute on the main checkout, run it.
pub fn drop_missing(repo: &Path) -> Result<Vec<String>, String> {
    let git = |args: &[&str]| process::run("git", args, Some(repo), GIT);
    git(&["cat-file", "-e", "HEAD^{commit}"])
        .map_err(|e| format!("its HEAD does not read, so no ref is judged: {e}"))?;
    let format = "--format=%(objectname) %(refname) %(symref)";
    let listed = git(&["for-each-ref", format, "refs/remotes"])?;
    let refs: Vec<(&str, &str)> = (listed.lines())
        .filter_map(|l| match l.split(' ').collect::<Vec<_>>()[..] {
            [sha, name, ""] => Some((sha, name)),
            _ => None,
        })
        .collect();
    let shas: String = refs.iter().map(|(sha, _)| format!("{sha}\n")).collect();
    let check = ["cat-file", "--batch-check=%(objectname)"];
    let (status, read, why) = process::answer_fed("git", &check, Some(repo), &[], Some(&shas), GIT)
        .map_err(String::from)?;
    if !status.success() {
        return Err(format!("git cat-file: {status}: {why}"));
    }
    let missing: Vec<&str> = (read.lines())
        .filter_map(|l| l.strip_suffix(" missing"))
        .collect();
    let dropped = (refs.iter())
        .filter(|(sha, _)| missing.contains(sha))
        .filter(|(sha, name)| git(&["update-ref", "-d", name, sha]).is_ok())
        .map(|(_, name)| name.to_string())
        .collect();
    Ok(dropped)
}

fn locked(e: &str) -> bool {
    e.contains("cannot lock ref") || e.contains(".lock': File exists")
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
    /// `pr`'s checks (`read_checks`), a red mirror read again (task rust-pr-waits-mirror).
    fn head_rows(&self, pr: &Pr) -> Result<Rows, String> {
        read_rows(&self.repo, &self.repo, &pr.number.to_string(), &self.cli)
    }

    /// The red rows of `rows` read down to their failed steps.
    fn reds(&self, pr: &Pr, rows: &Rows) -> Result<Vec<red::Red>, String> {
        let names = named(rows, red);
        red::reds(&|args| self.gh(args), &pr.number.to_string(), &names)
    }

    /// A fetch that lost a ref's lock to another fetch of the same checkout (a look's, a run's, a session's) is tried again: it
    /// left rust-fold-port merged at bd243645c and undeployed, nothing retrying it (task rust-deploy-fetch-retry). A fetch is
    /// `fetch_healed`'s.
    fn git(&self, args: &[&str]) -> Result<String, String> {
        match args.first() {
            Some(&"fetch") => fetch_healed(&self.repo, args, GIT),
            _ => process::run("git", args, Some(&self.repo), GIT),
        }
    }

    /// Port of the Elixir `Handover.merge_main` (task rust-fetch-heals-missing-ref): main merged into PR `url`'s `branch` as git
    /// does alone, `merge-tree` then `commit-tree` on `head` and main, pushed as a fast forward, so a branch moved meanwhile
    /// refuses it. All in this repository's own objects, pushed to the named origin: the Elixir one wrote from an object
    /// overlay to origin's URL, and git left `refs/remotes/origin/<branch>` at a commit the repository lacked.
    pub fn remerge(&self, url: &str, branch: &str, head: &str) -> Result<Remerge, String> {
        self.origin_is(url)?;
        // a branch gone at the origin (its PR merged) moved off the head read
        match self.git(&["fetch", "-q", "origin", &self.base, branch]) {
            Err(e) if e.contains("couldn't find remote ref") => return Ok(Remerge::Moved),
            fetched => fetched?,
        };
        let tip = self.git(&["rev-parse", &format!("refs/remotes/origin/{branch}")])?;
        if tip.trim() != head {
            return Ok(Remerge::Moved);
        }
        let main = self.git(&["rev-parse", &format!("refs/remotes/origin/{}", self.base)])?;
        let main = main.trim();
        if self.ancestor(main, head)? {
            return Ok(Remerge::Holds);
        }
        let tree = match self.merged_tree(head, main)? {
            Ok(tree) => tree,
            Err(files) => return Ok(Remerge::Conflicts(files)),
        };
        let base = &self.base;
        let message = format!("Merge branch '{base}' into {branch} (merge on green, {url})");
        let sha = self.commit_tree(&tree, &[head, main], &message, "dash merge on green")?;
        // Leased on the head read: a branch pushed or deleted since (its PR merged) refuses it, never recreated.
        let lease = format!("--force-with-lease=refs/heads/{branch}:{head}");
        match self.push(&[&lease, &format!("{sha}:refs/heads/{branch}")]) {
            Ok(_) => Ok(Remerge::Pushed(sha)),
            Err(e)
                if ["stale info", "non-fast-forward", "fetch first"]
                    .iter()
                    .any(|w| e.contains(w)) =>
            {
                Ok(Remerge::Moved)
            }
            Err(e) => Err(e),
        }
    }

    /// Port of the Elixir `MergeTrain` build's push (task rust-fetch-heals-missing-ref): each of `prs` (number, head, title,
    /// url) squashed in order on the one before, from main, then `train/<n>` force-pushed: a build that failed after its push
    /// left the branch, and main moved since. A PR whose head moved or that conflicts is left out, named. The repository's own
    /// objects, the named origin, as `remerge`.
    pub fn train_build(&self, n: u64, prs: &[TrainPr]) -> Result<Train, String> {
        if let Some(pr) = prs.first() {
            self.origin_is(&pr.url)?;
        }
        let numbers: Vec<u64> = prs.iter().map(|p| p.number).collect();
        self.fetch_pulls(&numbers)?;
        // Each head as this one fetch brought it, never read again: a head read later may be a commit not fetched here.
        let fetched = self.git(&["rev-parse", "--git-path", "FETCH_HEAD"])?;
        let fetched = std::fs::read_to_string(self.repo.join(fetched.trim()))
            .map_err(|e| format!("FETCH_HEAD: {e}"))?;
        let main = self.git(&["rev-parse", &format!("refs/remotes/origin/{}", self.base)])?;
        let mut train = Train {
            base: main.trim().to_string(),
            tip: main.trim().to_string(),
            took: vec![],
            out: vec![],
        };
        for pr in prs {
            let pull = format!("pull/{}/head'", pr.number);
            let line = fetched.lines().find(|l| l.contains(&pull));
            let now = line.and_then(|l| l.split('\t').next()).unwrap_or("none");
            if now != pr.head {
                let why = format!("its head moved to {now}");
                train.out.push((pr.url.clone(), why));
                continue;
            }
            let tree = match self.merged_tree(&train.tip, &pr.head)? {
                Ok(tree) => tree,
                Err(files) => {
                    let why = format!("conflicts in {}", files.join(", "));
                    train.out.push((pr.url.clone(), why));
                    continue;
                }
            };
            let message = format!("{} (#{})\n\n{}", pr.title, pr.number, pr.url);
            train.tip = self.commit_tree(&tree, &[&train.tip], &message, "dash merge train")?;
            train.took.push(pr.url.clone());
        }
        if !train.took.is_empty() {
            let to = format!("+{}:refs/heads/train/{n}", train.tip);
            self.push(&[&to])?;
        }
        Ok(train)
    }

    /// `train/<n>` deleted at the origin once its batch ended (the Elixir `MergeTrain` drop's push).
    /// Gone already is dropped: the Elixir drop was best effort.
    pub fn train_drop(&self, n: u64) -> Result<(), String> {
        match self.push(&[&format!(":refs/heads/train/{n}")]) {
            Err(e) if !e.contains("remote ref does not exist") => Err(e),
            _ => Ok(()),
        }
    }

    /// The app's push to the named origin of commits git made alone, `--no-verify` as the Elixir pushes and `dash ship`'s
    /// commit: the PR's or the train's CI run is the check, and the pre-push hook's tests would hold the merge pass for
    /// minutes, or refuse a merge of main for a check of main's.
    fn push(&self, args: &[&str]) -> Result<String, String> {
        let at = args.len() - 1;
        let args = [
            &["push", "-q", "--no-verify"],
            &args[..at],
            &["origin"],
            &args[at..],
        ]
        .concat();
        self.git(&args)
    }

    /// The checkout's origin is PR `url`'s repository (`https://github.com/<owner>/<name>/pull/<n>`): replicator and site_perso
    /// share task/<id> branch names, so a merge in the wrong checkout would push to another repository's branch.
    fn origin_is(&self, url: &str) -> Result<(), String> {
        let repo = url.split("/pull/").next().unwrap_or_default();
        let repo: Vec<&str> = repo.rsplit('/').take(2).collect();
        let [name, owner] = repo[..] else {
            return Err(format!("{url}: no PR link"));
        };
        let origin = self.git(&["remote", "get-url", "--push", "origin"])?;
        let origin = origin.trim().to_lowercase();
        let origin = origin.strip_suffix(".git").unwrap_or(&origin);
        let wanted = format!("{owner}/{name}").to_lowercase();
        match [':', '/']
            .iter()
            .any(|c| origin.ends_with(&format!("{c}{wanted}")))
        {
            true => Ok(()),
            false => Err(format!(
                "{}'s origin is {origin}, not {url}'s",
                self.repo.display()
            )),
        }
    }

    /// `ours` and `theirs` merged by git's `merge-tree`, written to no worktree: its tree, or the files that conflict.
    fn merged_tree(&self, ours: &str, theirs: &str) -> Result<Result<String, Vec<String>>, String> {
        let args = [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "-z",
            ours,
            theirs,
        ];
        let (status, text, why) =
            process::answer("git", &args, Some(&self.repo), &[], GIT).map_err(String::from)?;
        if let Some(files) = conflicts_in(status.code(), &text).map_err(|e| format!("{e} {why}"))? {
            let unnamed = || vec!["(files git did not name)".to_string()];
            return Ok(Err(if files.is_empty() { unnamed() } else { files }));
        }
        Ok(Ok(text.split('\0').next().unwrap_or_default().to_string()))
    }

    /// A commit of `tree` on `parents`, signed as `who` where git has no identity (a CI runner, a fresh VM refused
    /// « Author identity unknown » to the Elixir merge).
    fn commit_tree(
        &self,
        tree: &str,
        parents: &[&str],
        message: &str,
        who: &str,
    ) -> Result<String, String> {
        let name = format!("user.name={who}");
        let mut args = match self.git(&["var", "GIT_COMMITTER_IDENT"]) {
            Ok(_) => vec![],
            Err(_) => vec!["-c", &name, "-c", "user.email=dash@localhost"],
        };
        args.extend(["commit-tree", tree, "-m", message]);
        for p in parents {
            args.extend(["-p", p]);
        }
        Ok(self.git(&args)?.trim().to_string())
    }

    fn gh(&self, args: &[&str]) -> Result<String, String> {
        process::run(&self.cli, args, Some(&self.repo), CALL)
    }

    /// The look (`dash orch`, the child of `dash serve` with DASH_ORCH set) runs the binary it started with, so a merge of its
    /// own code would never run: it ends here, after each deploy, built or not (a manual dash deploy restarts no look), and
    /// serve starts it again on the live binary. Its runs, each in a process group of its own, this one included, go on.
    /// Listed, each whole command line checked, then killed by pid: a pattern kill once ended every Claude session on the Mac
    /// (2026-10-19). No look running is no failure: serve's next start takes the live binary anyway.
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

    fn slot(&self, n: usize) -> PathBuf {
        self.root.join(POOL).join(format!("pool-{n}"))
    }

    /// A slot made before `POOL`, at `<root>/pool-<n>`, moved under it where that slot is free; Spotlight indexed it there.
    /// One git refuses (locked, mid-move) stays and is said; the next pass tries again.
    fn slots_to_noindex(&self) -> Vec<String> {
        let _ = std::fs::create_dir_all(self.root.join(POOL));
        let mut said = vec![];
        for n in 1..=SLOTS {
            let old = self.root.join(format!("pool-{n}"));
            if !old.join(".git").exists() || self.slot(n).exists() {
                continue;
            }
            let (from, to) = (old.to_string_lossy(), self.slot(n));
            if let Err(e) = self.git(&["worktree", "move", &from, &to.to_string_lossy()]) {
                said.push(format!("pool-{n}: stays out of {POOL}: {e}"));
            }
        }
        said
    }

    /// A worktree that leaves: back to the pool while a slot is free, its HEAD detached (its branch is the caller's to keep or
    /// drop) and its rust/target kept, warm for the next task; else removed. Only a clean one is pooled, and git's own remove
    /// refuses any other, as before the pool.
    /// ponytail: two leaving at once may pick the same slot; git refuses the second move and that worktree is removed.
    fn release(&self, path: &str) -> Result<(), String> {
        self.pooled(path, SLOT_CAP)
    }

    /// `release` of a worktree whose merged PR holds every commit of it: what git neither tracks nor ignores goes first, as the
    /// next lease's clean drops it anyway. On 2026-11-12, 10 merged runs' worktrees stayed held, none pooled, each for a data
    /// copy or a scratch folder alone (7 MB to 1.3 GB; task rust-merge-leaves-lease). A tracked change keeps it all, as before;
    /// an ignored file (his live words, its build) is never cleaned here (no `-x`). `-ff`: a clone in a scratch folder goes too.
    fn release_merged(&self, path: &str) -> Result<(), String> {
        let at = |args: &[&str]| self.git(&[&["-C", path][..], args].concat());
        // a cargo there refuses the release (`pooled`): nothing is cleaned for a worktree that then stays (/code-review)
        drop(cargo_locks(&Path::new(path).join("rust/target"))?);
        if at(&["status", "--porcelain", "--untracked-files=no"])?
            .trim()
            .is_empty()
        {
            at(&["clean", "-ffdq"])?;
        }
        self.release(path)
    }

    /// `release` with its cap named. Never while a cargo runs there, whatever its build's size: Err then, and the worktree stays
    /// as it is for the next pass (/code-review of c5a721cc1: a cargo would follow the folder into the next task). A dirty one
    /// keeps its build, as git keeps it; a build past `cap` goes only from one that is pooled. Detached once in its slot, so a
    /// move git refuses leaves it on its branch.
    /// ponytail: a shell or watcher whose cwd is the worktree follows the rename into the slot, where the delete before the pool
    /// failed its writes; its run's session is stopped before `leave` and the reap waits an hour of git silence. A check of
    /// open cwds (lsof) if one ever writes into another task.
    fn pooled(&self, path: &str, cap: u64) -> Result<(), String> {
        let at = |args: &[&str]| self.git(&[&["-C", path][..], args].concat());
        let target = Path::new(path).join("rust/target");
        drop(cargo_locks(&target)?);
        let free = (1..=SLOTS).map(|n| self.slot(n)).find(|s| !s.exists());
        let clean = || at(&["status", "--porcelain"]).is_ok_and(|s| s.trim().is_empty());
        let Some(slot) = free.filter(|_| clean()) else {
            return self.git(&["worktree", "remove", path]).map(drop);
        };
        if bytes(&target, cap) > cap {
            remove_build(&target)?;
        }
        let slot = slot.to_string_lossy();
        // a repository's first leave makes its pool's folder: git moves into no missing one, and the worktree went instead
        let _ = std::fs::create_dir_all(self.root.join(POOL));
        if self.git(&["worktree", "move", path, &slot]).is_err() {
            return self.git(&["worktree", "remove", path]).map(drop);
        }
        // A slot still on its branch would hold that branch: it goes rather than stay so.
        match self.git(&["-C", &slot, "checkout", "-q", "--detach"]) {
            Ok(_) => Ok(()),
            Err(_) => self.git(&["worktree", "remove", &slot]).map(drop),
        }
    }

    /// A pooled worktree moved to `path` and put on `branch` (new from `base` unless `known`), its tracked files as the branch
    /// has them (`-f`: an edit made in an idle slot never rides), every untracked and ignored file dropped but its rust/target:
    /// nothing of the last task rides into this one. `false`: no slot free, nothing moved. A step that fails puts the slot
    /// back, detached, for the next run, so a failed start never costs a warm build.
    fn lease(&self, path: &str, branch: &str, base: &str, known: bool) -> Result<bool, String> {
        let taken = |slot: &PathBuf| {
            slot.is_dir()
                && self
                    .git(&["worktree", "move", &slot.to_string_lossy(), path])
                    .is_ok()
        };
        let Some(slot) = (1..=SLOTS).map(|n| self.slot(n)).find(taken) else {
            return Ok(false);
        };
        let at = |args: &[&str]| self.git(&[&["-C", path][..], args].concat());
        let start = || {
            match known {
                true => at(&["checkout", "-q", "-f", branch])?,
                false => at(&["checkout", "-q", "-f", "-b", branch, base])?,
            };
            // `-x` with `-e`: git's ignore rules are not read, the one pattern given is. A nested repository stays (git wants
            // -ff).
            at(&["clean", "-fdxq", "-e", "/rust/target"])
        };
        if let Err(e) = start() {
            let _ = at(&["checkout", "-q", "-f", "--detach"]);
            let _ = self.git(&["worktree", "move", path, &slot.to_string_lossy()]);
            return Err(e);
        }
        Ok(true)
    }

    /// Each slot's folder that is no worktree (no `.git` in it: a crash half way, a folder made by hand): it would hold its slot
    /// for good, unseen, so the reap says it each hour.
    fn lost_slots(&self) -> Vec<String> {
        let lost = |n: &usize| {
            let slot = self.slot(*n);
            slot.exists() && !slot.join(".git").exists()
        };
        let said = |n| format!("pool-{n}: no worktree, its slot is lost until the folder goes");
        (1..=SLOTS).filter(lost).map(said).collect()
    }

    /// Every task worktree's and slot's build counted; while they total `cap` or more, an idle one's goes, the oldest
    /// built first, a task worktree's before a slot's (any next run leases a slot; sort key `(slot, last_built)` ascending),
    /// never one a cargo uses: that one is said and the next goes. Idle: a slot, or a task worktree no claim names whose git and build are an hour still, as a session no
    /// claim names (herdr's, one by hand) builds or moves git within it (/code-review of 46c7db0eb); a git unread is still. A
    /// run claiming one after the reap read the claims moves its git within minutes. A claimed one counts and
    /// stays, so claimed builds past `cap` leave no idle one. What went, in words.
    /// ponytail: a lease moving a slot between the look just before the removal and the removal itself reads as dropped; the
    /// window is milliseconds against an hourly pass.
    /// ponytail: each build is walked whole, about 20 000 files a 3.3 GB one, a few seconds an hour for 14; bound the walk if it
    /// shows in the look's time.
    fn builds_under(&self, busy: &[String], cap: u64) -> Vec<String> {
        let tasks = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten();
        let still = |at: &Path, built: std::time::SystemTime| {
            let git = self
                .idle_for(&at.to_string_lossy())
                .unwrap_or(Duration::MAX);
            let build = built.elapsed().unwrap_or(Duration::ZERO);
            git >= SETTLED && build >= SETTLED
        };
        let tasks = tasks.filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = task_folder(&name)?;
            Some((false, name.clone(), e.path(), busy.iter().any(|b| b == id)))
        });
        let slots = (1..=SLOTS).map(|n| (true, format!("pool-{n}"), self.slot(n), false));
        let mut builds: Vec<_> = (tasks.chain(slots))
            .filter(|(.., at, _)| at.join("rust/target").is_dir())
            .map(|(slot, name, at, claimed)| {
                let target = at.join("rust/target");
                let (held, built) = (bytes(&target, u64::MAX), last_built(&target));
                let kept = claimed || !(slot || still(&at, built));
                (slot, built, name, target, kept, held)
            })
            .collect();
        let mut total: u64 = builds.iter().map(|b| b.5).sum();
        builds.sort_by_key(|(slot, built, ..)| (*slot, *built));
        let gb = |b: u64| format!("{:.1} GB", b as f64 / (1u64 << 30) as f64);
        let mut said = vec![];
        for (.., name, target, _, held) in builds.into_iter().filter(|b| !b.4) {
            if total < cap {
                break;
            }
            if !target.is_dir() {
                continue;
            }
            match remove_build(&target) {
                Ok(()) => {
                    said.push(format!(
                        "{name}: idle build of {} dropped, builds totalled {} past {}",
                        gb(held),
                        gb(total),
                        gb(cap)
                    ));
                    total -= held;
                }
                Err(e) => said.push(format!("{name}: idle build kept: {e}")),
            }
        }
        if total >= cap {
            said.push(format!(
                "builds still total {}, past {}: the rest are claimed, in use within the hour, or a cargo's",
                gb(total),
                gb(cap)
            ));
        }
        said
    }

    /// A forge on any repository's checkout, a worktree of it included, its pool `<home>/.herdr/worktrees/<main checkout's name>`
    /// beside herdr's worktrees: site_perso's is its runs' pool, klar's its own (task reap-kept-worktrees).
    pub fn on(repo: &Path, home: &Path) -> Result<GitHub, String> {
        let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
        let common = process::run("git", &args, Some(repo), CALL)?;
        // `<main>/.git`, or a bare repository's own folder, `klar.git` named klar (/code-review: its parent was taken for all)
        let common = Path::new(common.trim());
        let main = match common.file_name().is_some_and(|n| n == ".git") {
            true => common.parent().map(Path::to_path_buf),
            false => Some(common.to_path_buf()),
        };
        let name = main.as_deref().and_then(Path::file_name).map(|n| {
            let n = n.to_string_lossy();
            n.strip_suffix(".git").unwrap_or(&n).to_string()
        });
        let (Some(main), Some(name)) = (main, name) else {
            return Err(format!("{}: no main checkout", repo.display()));
        };
        let root = home.join(".herdr/worktrees").join(name);
        let (base, page, cli) = ("main".into(), String::new(), "gh".into());
        Ok(GitHub {
            repo: main,
            root,
            base,
            page,
            cli,
        })
    }

    /// `dash worktree lease`: a pooled slot moved to `path`, on `branch`, new from `base` unless it exists. `false`: none free,
    /// nothing made, the caller makes its own (herdr's `worktree create`, klar's workz).
    pub fn lease_at(&self, path: &str, branch: &str, base: &str) -> Result<bool, String> {
        let known = self.git(&[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{branch}"),
        ]);
        self.lease(path, branch, base, known.is_ok())
    }

    /// `dash worktree leave`: back to the pool, else removed, its branch kept. A dirty one, one a cargo builds in, or one holding
    /// his live words in an ignored file (klar's workshop comments.live.json, which the next lease's clean would delete) stays.
    pub fn leave_at(&self, path: &str) -> Result<(), String> {
        if live_data(Path::new(path)) {
            return Err(format!(
                "{path} holds his live words in an ignored file: kept"
            ));
        }
        self.release(path)
    }

    /// Every worktree but the main checkout, as `dash worktree list` prints them.
    pub fn rows(&self) -> Result<Vec<Row>, String> {
        let listed = self.git(&["worktree", "list", "--porcelain"])?;
        let pool = self.root.join(POOL);
        let root = std::fs::canonicalize(&pool).unwrap_or(pool);
        let mut rows = vec![];
        for block in listed.split("\n\n").skip(1) {
            let field = |key: &str| block.lines().find_map(|l| l.strip_prefix(key));
            let Some(path) = field("worktree ") else {
                continue;
            };
            let at = Path::new(path);
            let branch = field("branch refs/heads/").unwrap_or_default().to_string();
            let pooled = at.parent().and_then(|p| std::fs::canonicalize(p).ok())
                == Some(root.clone())
                && at
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("pool-"));
            let gone = field("prunable").is_some() || !at.exists();
            // a worktree git cannot read is said as such, never the end of the list (/code-review: the reap lost every agent's)
            let rated = || -> Result<(Duration, &'static str), String> {
                let idle = self.idle_for(path)?;
                let unheld = idle >= UNHELD && !self.holds(path)?;
                Ok((idle, if unheld { "unheld" } else { "held" }))
            };
            let (idle, state) = match (pooled, gone, field("locked").is_some()) {
                (true, _, _) => (Duration::ZERO, "pool"),
                (_, true, _) => (Duration::ZERO, "prunable"),
                // `git worktree lock`: its owner holds it, and git refuses to move or remove it
                (_, _, true) => (Duration::ZERO, "held"),
                _ => rated().unwrap_or((Duration::ZERO, "unread")),
            };
            let path = path.to_string();
            rows.push(Row {
                path,
                branch,
                idle,
                state,
            });
        }
        Ok(rows)
    }

    /// How long since its index, HEAD or reflog last moved (`settled`'s read).
    fn idle_for(&self, path: &str) -> Result<Duration, String> {
        let args = [
            "-C",
            path,
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
        ];
        Ok(untouched(Path::new(self.git(&args)?.trim())))
    }

    /// It holds what no remote has: an uncommitted file, untracked included, his words in an ignored file (`live_data`), or a
    /// HEAD on no remote ref. A ref line, never any output: git's « ignoring broken ref » warning would read as pushed (the older
    /// app's PR 1039).
    fn holds(&self, path: &str) -> Result<bool, String> {
        let at =
            |args: &[&str]| self.git(&[&["-C", path, "--no-optional-locks"][..], args].concat());
        if !at(&["status", "--porcelain"])?.trim().is_empty() || live_data(Path::new(path)) {
            return Ok(true);
        }
        let refs = [
            "for-each-ref",
            "--contains",
            "HEAD",
            "--format=%(refname)",
            "refs/remotes",
        ];
        Ok(!at(&refs)?.lines().any(|l| l.starts_with("refs/remotes/")))
    }

    /// A worktree untouched for a day that holds nothing a remote lacks leaves as a run's does, its branch kept for a reopen: a
    /// Claude subagent's (`.claude/worktrees/agent-*`, which Claude Code keeps for good once it holds a commit) or a task's no run
    /// claims, its PR open or not (task reap-kept-worktrees). A day, never an hour: a subagent that only builds or reads moves no
    /// git file (/code-review). `None` kept.
    fn release_unheld(&self, path: &str) -> Result<Option<String>, String> {
        if self.idle_for(path)? < UNHELD || self.holds(path)? {
            return Ok(None);
        }
        self.release(path)?;
        Ok(Some(
            "untouched, its HEAD pushed: worktree released, branch kept".into(),
        ))
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
    /// main at or after its squash (rust-access-map, PR 1217, read as none for 3 PM rounds). When `made` started task `id`'s
    /// branch after that squash, a reopen's, only its start or before: a commit past it is the reopen's new work, which the
    /// squash lacks, while one at it holds nothing new (1511 and 1513, reopened for a report alone, looped on « no PR holds
    /// it »). A commit past either is work to push.
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
            return self.ancestor(commit, from.trim());
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
        if folder.exists() && self.release_merged(&path).is_err() {
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
            // From the pool first, a fresh one only when no slot is free (rfc-rust-unified#worktrees).
            let pooled = self.lease(&path, &branch, &base, known.is_ok())?;
            if !pooled && known.is_ok() {
                self.git(&["worktree", "add", "-q", &path, &branch])?;
            } else if !pooled {
                self.git(&["worktree", "add", "-q", "-b", &branch, &path, &base])?;
            }
            if known.is_err() {
                // What `held` reads to tell a reopen's branch from the one its PR merged; `branch -D` drops it with the branch.
                let from = self.git(&["rev-parse", &base])?;
                self.git(&["config", &format!("branch.{branch}.dashFrom"), from.trim()])?;
            }
            self.git(&["worktree", "move", &path, &folder.to_string_lossy()])?;
        }
        real_folder(&folder)
    }

    /// One worktree for `reap`: `None` kept. Untouched an hour first, as a session restarted in it has no claim for a moment, and
    /// no tracked file changed, before any GitHub read. Merged, it leaves as its run's `leave` does, branch, scratch and all;
    /// closed, only the folder goes, its branch kept for a reopen, and only with no untracked file (`holds`). `waits` false: a
    /// finished task's that the caller saw no pane or claim in (`release_done`), so its scratch waits an hour untouched, not a
    /// day of git silence.
    fn reap_one(&self, id: &str, waits: bool) -> Result<Option<String>, String> {
        let path = self.folder(id);
        let path = path.to_string_lossy();
        // `--no-optional-locks`: a status that refreshed the index would read as a touch at the next pass (/code-review).
        let at =
            |args: &[&str]| self.git(&[&["-C", &path, "--no-optional-locks"][..], args].concat());
        let dir = at(&["rev-parse", "--path-format=absolute", "--git-dir"])?;
        // Untracked files are a merged run's scratch (`release_merged`); the closed path reads them in `holds`.
        let idle = || -> Result<bool, String> {
            let status = ["status", "--porcelain", "--untracked-files=no"];
            Ok(settled(Path::new(dir.trim())) && at(&status)?.trim().is_empty())
        };
        if !idle()? {
            return Ok(None);
        }
        // The branch it has checked out, never only task/<id>: a session that moved to a branch of its own merged that one (PR
        // 1278 from task/rust-forge-stale-worktree-head-main-fix). A detached HEAD names no PR: kept.
        let branch = at(&["branch", "--show-current"])?.trim().to_string();
        if branch.is_empty() {
            return Ok(None);
        }
        let fields = "number,url,state,headRefOid,mergeCommit";
        let args = [
            "pr", "list", "--head", &branch, "--state", "all", "--limit", "1", "--json", fields,
        ];
        let text = self.gh(&args)?;
        let head = at(&["rev-parse", "HEAD"])?;
        let head = head.trim();
        if let Some(listed) = listed_in(&text)? {
            if !listed.pr.merged
                || !(self.held(id, head, &listed)? || self.merges_only(head, &listed.pr.head)?)
            {
                return Ok(None);
            }
            // Again right before: a run claimed during the GitHub read may have started in it (/code-review).
            if !idle()? {
                return Ok(None);
            }
            // Its untracked files go with it: a day of git silence first, as `release_unheld` waits, since a session no claim
            // names may write new files for an hour without a git move (/code-review of a488ecfaf).
            // A finished task's no pane or claim names (`waits` false): its scratch untouched an hour, as git is, is enough.
            let scratch = !at(&["status", "--porcelain"])?.trim().is_empty();
            let young = match waits {
                true => self.idle_for(&path)? < UNHELD,
                false => self.scratch_age(&path)? < SETTLED,
            };
            if scratch && young {
                return Ok(None);
            }
            // As its run's `leave`, for the branch it holds: here, then at the origin, best effort.
            self.release_merged(&path)?;
            let _ = self.git(&["branch", "-D", &branch]);
            let _ = self.git(&["push", "-q", "origin", "--delete", &branch]);
            return Ok(Some(format!(
                "its PR {} merged: worktree and branch gone",
                listed.pr.url
            )));
        }
        let list: Value = serde_json::from_str(&text).map_err(|e| format!("gh's answer: {e}"))?;
        let closed = list
            .as_array()
            .and_then(|l| l.first())
            .is_some_and(|pr| pr["state"] == "CLOSED");
        // its HEAD on a remote, his live words in none of its ignored files (`holds`)
        if !closed || self.holds(&path)? || !idle()? {
            return Ok(None);
        }
        self.release(&path)?;
        let url = list[0]["url"].as_str().unwrap_or_default().to_string();
        Ok(Some(format!(
            "its PR {url} closed, its HEAD pushed: worktree gone, branch kept"
        )))
    }

    /// How long since any file git neither tracks nor ignores was last written: a session outside herdr writing scratch there,
    /// which no pane or claim names (/code-review of 9a61ee07f).
    fn scratch_age(&self, path: &str) -> Result<Duration, String> {
        let listed = self.git(&["-C", path, "ls-files", "-o", "--exclude-standard", "-z"])?;
        let newest = (listed.split('\0').filter(|f| !f.is_empty()))
            .filter_map(|f| {
                std::fs::metadata(Path::new(path).join(f))
                    .ok()?
                    .modified()
                    .ok()
            })
            .max();
        Ok(newest
            .and_then(|t| t.elapsed().ok())
            .unwrap_or(Duration::MAX))
    }

    /// `head` adds only merge commits to main and `pr_head`: a merge of main made after its PR merged holds no work of its own
    /// (rust-merge-deploy's worktree, 2026-11-09). A conflict resolved in such a merge is lost with it, as its PR is merged.
    fn merges_only(&self, head: &str, pr_head: &str) -> Result<bool, String> {
        let main = format!("^origin/{}", self.base);
        let not_pr = format!("^{pr_head}");
        let args = ["rev-list", "--no-merges", "--count", head, &main, &not_pr];
        Ok(self.git(&args)?.trim() == "0")
    }
}

/// The task whose worktree a folder of the root named `name` is: `rust-<id>`, never a half-made `.new`, which `made` clears, or
/// a claim's `.lock` file. What the reap, the builds and the finished tasks' holds all read (`dash` `done_holds`).
pub fn task_folder(name: &str) -> Option<&str> {
    let id = name.strip_prefix("rust-")?;
    (!id.is_empty() && !id.contains('.')).then_some(id)
}

/// An hour since its index, HEAD or reflog last moved: a commit, a checkout, an add.
fn settled(git_dir: &Path) -> bool {
    untouched(git_dir) >= SETTLED
}

/// How long since its index, HEAD or reflog last moved. None readable is touched now.
fn untouched(git_dir: &Path) -> Duration {
    let touched = ["index", "HEAD", "logs/HEAD"]
        .iter()
        .filter_map(|f| {
            std::fs::metadata(git_dir.join(f))
                .and_then(|m| m.modified())
                .ok()
        })
        .max();
    touched
        .and_then(|t| t.elapsed().ok())
        .unwrap_or(Duration::ZERO)
}

/// His words in a file git ignores, which `git status` never shows: a non-empty file under `private/live/` (site_perso's live
/// data) or a `comments.live.json` under `website/workshop/` (klar's workshop), herdr/session.sh's `live_data` rule (klar's
/// request of 2026-10-17). A link is never followed. ponytail: session.sh's exception for a clean, pushed clone under
/// private/live is not ported, so such a worktree reads held; port it if one is ever kept for that alone.
fn live_data(worktree: &Path) -> bool {
    fn any(dir: &Path, hit: &dyn Fn(&Path) -> bool) -> bool {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                let Ok(kind) = e.file_type() else {
                    return false;
                };
                let path = e.path();
                (kind.is_dir() && any(&path, hit))
                    || (kind.is_file() && hit(&path) && e.metadata().is_ok_and(|m| m.len() > 0))
            })
    }
    any(&worktree.join("private/live"), &|_| true)
        || any(&worktree.join("website/workshop"), &|p| {
            p.ends_with("comments.live.json")
        })
}

/// One worktree as `dash worktree list` prints it (task reap-kept-worktrees). `state`: `pool` a free slot, `prunable` its
/// folder gone, `unheld` untouched for `UNHELD` and holding nothing a remote lacks (the reap's to release), `unread` git failed
/// in it, else `held`.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub path: String,
    pub branch: String,
    pub idle: Duration,
    pub state: &'static str,
}

impl Forge for GitHub {
    fn worktree(&self, id: &str) -> Result<String, String> {
        // An unread list is no merge: the folder as it stands, as before.
        self.opened(id, self.listed(id).ok().flatten().as_ref())
    }

    fn worktree_as_is(&self, id: &str) -> Result<String, String> {
        self.made(id)
    }

    fn pr(&self, id: &str) -> Result<Option<Pr>, String> {
        self.judged(id, self.listed(id)?)
    }

    // The checks dash ci wait reads too (`checks`, merge-claim.py's rules in Rust, task rust-hooks-rest-port). The .py's file-overlap
    // stale rule is not this path's: `base` holds back any head lacking main's commits outside a few inert files, the PR's own
    // or not (task rust-merge-gate-one-rule). A red mirror is read again (task rust-pr-waits-mirror).
    fn checks(&self, pr: &Pr) -> Result<Checks, String> {
        let rows = self.head_rows(pr)?;
        let checks = checks_of(&rows);
        let Checks::Red(names) = checks else {
            return Ok(checks);
        };
        // A red whose steps a gh call failed to read is no verdict yet: the outage that turns a step red fails reads too, and a
        // code red then would reopen a met task (/code-review of PR 1460). Read with none to judge, it is the code's.
        Ok(match self.reds(pr, &rows) {
            Ok(reds) if red::all_infra(&reds) => Checks::Infra(red::words(&reds)),
            Ok(reds) if !reds.is_empty() => Checks::Red(red::words(&reds)),
            Ok(_) => Checks::Red(names),
            Err(_) => Checks::Pending,
        })
    }

    fn rerun(&self, pr: &Pr) -> Result<Vec<String>, String> {
        let reds = self.reds(pr, &self.head_rows(pr)?)?;
        if !red::all_infra(&reds) {
            return Ok(vec![]);
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        let probe = dash_core::iso(now.map_or(0, |d| d.as_secs() as i64) - red::PROBE_SECS);
        red::rerun(&|args| self.gh(args), &reds, &probe)
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

    // `leave_at`: his live words in an ignored file keep it, which the next lease's clean would drop (/code-review).
    fn give_back(&self, id: &str) -> Result<(), String> {
        let path = self.folder(id);
        match path.exists() {
            true => self.leave_at(&path.to_string_lossy()),
            false => Ok(()),
        }
    }

    // Its branch goes too, here and at the origin: squashed into main, it holds nothing else, and a task reopened on it would start
    // from an old head that still reads as its merged PR's. Best effort: a reopen on a branch left behind only pushes a merge.
    fn leave(&self, id: &str) -> Result<(), String> {
        let path = self.folder(id);
        self.release_merged(&path.to_string_lossy())?;
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

    // What a push has not saved: a remote branch holds every commit pushed, its PR's or not, so `--not --remotes` names the rest. A
    // head its merged PR holds is saved too: a squash leaves its commits on no branch once the branch is pruned (/code-review of
    // baa197583).
    fn unsaved(&self, id: &str) -> Result<Option<String>, String> {
        let path = self.folder(id);
        if !path.exists() {
            return Ok(None);
        }
        let path = path.to_string_lossy();
        let at = |args: &[&str]| self.git(&[&["-C", &path][..], args].concat());
        let changed = at(&["status", "--porcelain"])?;
        if !changed.trim().is_empty() {
            let files = changed.lines().count();
            return Ok(Some(format!("{files} files changed and not committed")));
        }
        let ahead = at(&["rev-list", "--count", "HEAD", "--not", "--remotes"])?;
        if ahead.trim() == "0" {
            return Ok(None);
        }
        let head = at(&["rev-parse", "HEAD"])?;
        // An unread list proves no merge: the commits stand as unsaved.
        let listed = self.listed(id).ok().flatten();
        let merged = listed.is_some_and(|l| l.pr.merged && l.pr.head == head.trim());
        Ok((!merged).then(|| format!("{} commits on no remote branch", ahead.trim())))
    }

    // HEAD's reflog is this worktree's own: a commit, merge or rebase made here, never what a pull or a reset brought (task
    // rust-pr-one-owner: two reruns found the fix pushed and pushed nothing). Pushed: a remote branch holds it.
    fn own_pushes(&self, id: &str, since_ms: i64) -> Result<bool, String> {
        let path = self.folder(id);
        if !path.exists() {
            return Ok(false);
        }
        let path = path.to_string_lossy();
        let at = |args: &[&str]| self.git(&[&["-C", &path][..], args].concat());
        let log = at(&["reflog", "--date=unix", "--format=%H %gd %gs", "HEAD"])?;
        // newest first, each « <sha> HEAD@{<seconds>} <what moved it> »
        for line in log.lines() {
            let mut parts = line.splitn(3, ' ');
            let (Some(sha), Some(when), Some(what)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let when = when.trim_start_matches("HEAD@{").trim_end_matches('}');
            let Ok(secs) = when.parse::<i64>() else {
                continue;
            };
            if secs * 1000 < since_ms {
                break;
            }
            if made_here(what)
                && at(&["rev-list", "--count", sha, "--not", "--remotes"])?.trim() == "0"
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // Its PR merged or closed as `reap_one` reads it, else whatever no remote lacks. The day `reap` waits stands for a session
    // no claim names; the caller ruled out herdr's, and an hour of git silence and of untouched scratch stands for the rest, a
    // session outside herdr (task rust-nothing-done-holds: 11 merged tasks' worktrees held for a scratch file alone, 2026-11-12;
    // /code-review of 9a61ee07f).
    fn release_done(&self, id: &str) -> Result<String, String> {
        let path = self.folder(id);
        if !path.exists() {
            return Ok("no worktree".into());
        }
        if self.idle_for(&path.to_string_lossy())? < SETTLED {
            return Ok("kept: git moved there within the hour".into());
        }
        if let Some(words) = self.reap_one(id, false)? {
            return Ok(words);
        }
        let path = path.to_string_lossy();
        if self.holds(&path)? {
            return Ok("kept: it holds what no remote has (a change, an untracked file, his live words or a commit)".into());
        }
        self.release(&path)?;
        Ok("nothing a remote lacks: worktree released, branch kept".into())
    }

    fn reap(&self, busy: &[String]) -> Result<Vec<String>, String> {
        let entries =
            std::fs::read_dir(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        let mut said = vec![];
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = task_folder(&name) else {
                continue;
            };
            if !entry.path().is_dir() || busy.iter().any(|b| b == id) {
                continue;
            }
            let path = entry.path().to_string_lossy().into_owned();
            // A merged or closed PR's first, its branch handled; else one untouched a day. ponytail: a run claiming it during
            // reap_one's GitHub read is not seen here, only a git touch in the milliseconds before the release, as reap_one's own
            // last idle check; the claims read again here if a run ever loses its folder so.
            let reaped = self.reap_one(id, true);
            match reaped.and_then(|r| r.map_or_else(|| self.release_unheld(&path), |w| Ok(Some(w))))
            {
                Ok(Some(words)) => said.push(format!("{id}: {words}")),
                Ok(None) => {}
                Err(e) => said.push(format!("{id}: kept, unread: {e}")),
            }
        }
        // A Claude session's own: its subagents' (`.claude/worktrees/agent-*`, wherever the session ran) and those it made in its
        // scratchpad (`<tmp>/claude-<uid>/.../scratchpad/`), gone with the session (two untouched 35 and 42 hours on 2026-11-12).
        let agent = |p: &Path| {
            let named = |n: &std::ffi::OsStr| n.to_string_lossy().starts_with("agent-");
            let subagent = p.parent().is_some_and(|d| d.ends_with(".claude/worktrees"))
                && p.file_name().is_some_and(named);
            subagent || p.components().any(|c| c.as_os_str() == "scratchpad")
        };
        let rows = self.rows().unwrap_or_else(|e| {
            said.push(format!("worktree list: {e}"));
            vec![]
        });
        for row in rows
            .iter()
            .filter(|r| r.state == "unheld" && agent(Path::new(&r.path)))
        {
            match self.release_unheld(&row.path) {
                Ok(Some(words)) => said.push(format!("{}: {words}", row.path)),
                Ok(None) => {}
                Err(e) => said.push(format!("{}: kept, unread: {e}", row.path)),
            }
        }
        said.extend(self.slots_to_noindex());
        said.extend(self.lost_slots());
        // last, once the merged ones are pooled warm or gone: `busy` is minutes old here, which the hour of stillness covers
        said.extend(self.builds_under(busy, BUILDS_CAP));
        Ok(said)
    }

    fn built(&self) -> Result<Vec<String>, String> {
        let entries =
            std::fs::read_dir(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        let ids = entries.flatten().filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = task_folder(&name)?.to_string();
            let built = self.folder(&id).join("rust/target").is_dir();
            built.then_some(id)
        });
        Ok(ids.collect())
    }

    // 7 to 14 GB a worktree filled the disk to 2% on 2026-11-09. No session's work is in it, so it goes even when git keeps the
    // worktree or its session may still be there.
    fn clean(&self, id: &str) -> Result<(), String> {
        remove_build(&self.folder(id).join("rust/target"))
    }

    // The repository's own rust/target held 32.8 GB on 2026-11-10, an open task's no-one ran past 5 (task rust-disk-caps).
    fn trim(&self, id: Option<&str>, cap: u64) -> Result<Option<u64>, String> {
        let target = id
            .map_or(self.repo.clone(), |id| self.folder(id))
            .join("rust/target");
        let held = bytes(&target, cap);
        if held <= cap {
            return Ok(None);
        }
        remove_build(&target).map(|()| Some(held))
    }

    // Paged through the API: `gh pr view --json files` stops at 100, and a send path past them would land unreviewed.
    fn sends(&self, pr: &Pr) -> Result<Vec<String>, String> {
        let path = format!("repos/{{owner}}/{{repo}}/pulls/{}/files", pr.number);
        // a file moved out of a send path is named by its old name too
        let names = ".[] | .filename, (.previous_filename // empty)";
        let listed = self.gh(&["api", &path, "--paginate", "-q", names])?;
        Ok(sends(listed.lines()))
    }

    fn title(&self, pr: &Pr) -> Result<String, String> {
        // by its URL: a number alone is read against this forge's repository, whoever's PR it is (/code-review of PR 1546)
        Ok(self
            .gh(&["pr", "view", &pr.url, "--json", "title", "-q", ".title"])?
            .trim()
            .to_string())
    }

    // The newest verdict comment, whoever posted it, covering the head when no commit of the PR's own (one main lacks, merges
    // left out) follows the one it names: the run's update merges main in after a pass. ponytail: a merge's own resolution
    // reads as no change; a conflict's is the session's, and its run hands it back rather than land it.
    fn verdict(&self, pr: &Pr) -> Result<Option<Review>, String> {
        let path = format!("repos/{{owner}}/{{repo}}/issues/{}/comments", pr.number);
        let bodies = self.gh(&["api", &path, "--paginate", "-q", ".[].body | @json"])?;
        let last = (bodies.lines().rev())
            .filter_map(|line| serde_json::from_str::<String>(line).ok())
            .find_map(|body| verdict_of(&body));
        let Some((review, named)) = last else {
            return Ok(None);
        };
        if named == pr.head {
            return Ok(Some(review));
        }
        let head = format!("pull/{}/head", pr.number);
        self.git(&["fetch", "-q", "origin", &self.base, &head])?;
        // a sha this repository lacks, or one the head left behind (a force push), covers nothing
        if !self.ancestor(&named, &pr.head).unwrap_or(false) {
            return Ok(None);
        }
        let main = format!("^origin/{}", self.base);
        let range = format!("{named}..{}", pr.head);
        let own = self.git(&["rev-list", "--no-merges", "--count", &range, &main])?;
        Ok((own.trim() == "0").then_some(review))
    }

    /// Its local branch against where it left main, its worktree's commits included before any push: three dots, so main's
    /// own moves since are not its change. Each changed file whole as context, so the judge's read knows a test module from
    /// a page (`page_local_lines`).
    fn diff(&self, id: &str) -> Result<String, String> {
        let range = format!("origin/{}...task/{id}", self.base);
        self.git(&[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--unified=1000000",
            &range,
        ])
    }

    // The Makefile's rust/build, from the checkout its worktrees hang off (it builds origin/main), then `live::go_live` here, in
    // this process's own binary, the one known to run: the new server waited on with no clock, rolled back only when it exits or
    // answers 5xx (task rust-deploy-keeps-new-build). One at a time, on a lock of its own (the build's covers the build alone): two
    // merges seconds apart would each kill the other's new binary. Skipped, once the lock is held, when the page's « Built from »
    // commit holds main's rust/ already: a merge outside it, or one a deploy before it took live, never restarts the page. The
    // build under bin/bounded, which ends cargo with make.
    // ponytail: a manual make rust/deploy, or rust/front-door's, takes no lock.
    // Its answer says what serves: the page's commit when it holds main's rust/ already, else the one go_live saw the page name,
    // never what was meant to.
    fn deploy(&self) -> Result<String, String> {
        let live = self.repo.join("private/live/rust");
        std::fs::create_dir_all(&live).map_err(|e| format!("{}: {e}", live.display()))?;
        let lock = live.join("deploy.lock");
        let held = std::fs::File::create(&lock).map_err(|e| format!("{}: {e}", lock.display()))?;
        let locked = |e: std::io::Error| format!("{}: {e}", lock.display());
        // Another deploy held it, a run's while the look's child waited: that one restarted the look, and main serves since.
        let waited = match held.try_lock() {
            Ok(()) => false,
            Err(std::fs::TryLockError::WouldBlock) => held.lock().map_err(locked).map(|()| true)?,
            Err(std::fs::TryLockError::Error(e)) => return Err(locked(e)),
        };
        let mut built = false;
        let said = match self.serving()? {
            Serving::Main(said) if waited => return Ok(said),
            Serving::Main(said) => said,
            Serving::Behind(_) | Serving::Other(_) | Serving::Unnamed(_) => {
                let repo = self.repo.to_string_lossy();
                let bounded = self.repo.join("bin/bounded");
                let secs = DEPLOY.as_secs().to_string();
                let args = [secs.as_str(), "make", "-C", &repo, "rust/build"];
                process::run(&bounded.to_string_lossy(), &args, None, DEPLOY + CALL)?;
                built = true;
                live::go_live(&live, &self.page)?
            }
        };
        self.restart_look(&live)
            .map_err(|e| format!("{said}, but the orchestrator's look kept its old binary: {e}"))?;
        // after the lock: the next merge's deploy never waits on rust-analyzer's index
        drop(held);
        Ok(match built {
            true => format!("{said}; {}", live::code_map(&live)),
            false => said,
        })
    }

    /// From the page's « Built from » commit: what `deploy` skips its build on, the look's deploy check
    /// (`dash_orch::deploy_due`) and `dash orch next`'s « Live: » line.
    fn serving(&self) -> Result<Serving, String> {
        self.git(&["fetch", "-q", "origin", &self.base])?;
        let tree = |at: &str| self.git(&["rev-parse", &format!("{at}:rust")]);
        let head = format!("origin/{}", self.base);
        let main = tree(&head)?;
        let page = process::run("curl", &["-fs", "-m", "2", &self.page], None, CALL);
        let served = page.ok().and_then(|p| built_from(&p));
        let head = self.git(&["rev-parse", &head])?;
        let head = head.trim();
        Ok(match served {
            Some(sha) if tree(&sha).is_ok_and(|t| t == main) => Serving::Main(format!(
                "{sha} serves, no build: it holds the rust/ of main's {head}"
            )),
            Some(sha)
                if self
                    .git(&["merge-base", "--is-ancestor", &sha, head])
                    .is_ok() =>
            {
                Serving::Behind(format!("{sha} serves, not the rust/ of main's {head}"))
            }
            Some(sha) => Serving::Other(format!(
                "{sha} serves, a commit main does not hold; main is at {head}"
            )),
            None => Serving::Unnamed(format!("the page names no commit; main is at {head}")),
        })
    }

    /// `git diff --quiet` on the monitors' data from the commit last saved (`monitors-saved`, beside the live binary) to the one
    /// `make rust/build` wrote live (`commit`), never main's ref, which a later fetch moves: a deploy that went live but failed
    /// after, or a run that died before its save, is saved at the next deploy (/code-review of 4e5fbfe26). No mark yet, or a
    /// git that cannot tell, saves them: harmless twice, a save PUTs in place.
    fn save_monitors(&self) -> Result<Option<String>, String> {
        let live = self.repo.join("private/live/rust");
        let (commit, mark) = (live.join("commit"), live.join("monitors-saved"));
        let read = |f: &Path| std::fs::read_to_string(f).map(|s| s.trim().to_string());
        let deployed = read(&commit).map_err(|e| format!("{}: {e}", commit.display()))?;
        let same = read(&mark).is_ok_and(|saved| {
            let args = ["diff", "--quiet", &saved, &deployed, "--", MONITORS_DATA];
            self.git(&args).is_ok()
        });
        if same {
            return Ok(None);
        }
        // the binary just deployed, never this one: it holds the merge's monitors
        let dash = live.join("dash");
        let limit = Duration::from_secs(120);
        let saved = process::run(&dash.to_string_lossy(), &["monitors", "save"], None, limit)?;
        // a mark not written saves again at the next deploy, harmless
        let _ = std::fs::write(&mark, &deployed);
        Ok(Some(saved))
    }
}

/// The monitors' data, `dash monitors save`'s source, as git names it: a merge that changes it has its deploy save them. A
/// path, not a `use` of monitors.rs, which reaches back to the tasks through alerts.rs.
pub const MONITORS_DATA: &str = "rust/crates/dash/src/monitors.rs";

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    /// Done-when 1 of rust-deploy-fetch-retry: a stand-in git whose fetch loses the ref lock is run again, three times at most.
    #[dash_test]
    fn a_fetch_that_lost_a_ref_lock_is_tried_three_times_more_then_fails() {
        let locked = "git fetch -q origin main: exit status: 1: error: cannot lock ref 'refs/remotes/origin/main'";
        let git = |fails: u32| {
            let calls = std::cell::Cell::new(0);
            let said = fetched(
                || {
                    calls.set(calls.get() + 1);
                    match calls.get() <= fails {
                        true => Err(locked.to_string()),
                        false => Ok(String::new()),
                    }
                },
                Duration::ZERO,
            );
            (said, calls.get())
        };
        assert_eq!(git(0), (Ok(String::new()), 1));
        assert_eq!(git(3), (Ok(String::new()), 4), "three retries take it");
        assert_eq!(
            git(4),
            (Err(locked.to_string()), 4),
            "the fourth lock is the answer"
        );
        let calls = std::cell::Cell::new(0);
        let other = fetched(
            || {
                calls.set(calls.get() + 1);
                Err("fatal: couldn't find remote ref".to_string())
            },
            Duration::ZERO,
        );
        assert_eq!(
            (other.is_err(), calls.get()),
            (true, 1),
            "no retry on another error"
        );
        let packed = "fatal: Unable to create '/r/.git/packed-refs.lock': File exists.";
        assert!(super::locked(packed), "another fetch's lock file");
    }

    #[dash_test]
    fn a_folder_the_older_app_would_adopt_is_refused_and_another_comes_back_real() {
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

    #[dash_test]
    fn gh_s_answers_read_as_its_newest_pr_a_closed_one_none() {
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

    #[dash_test]
    fn a_merge_tree_reads_as_behind_or_its_conflicting_files() {
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

    /// The merge on green's read (`Forge::checks`, which the look's gate turns into Ready, Wait or Fix) through the real
    /// `checks` under `repo`, a stand-in gh on PATH answering `gh pr view` with `view.json`, the forge's own gh calls too.
    fn gate_reads(repo: &Path, dir: &TestDir, view: &str) -> Checks {
        let gh = dir.0.join("gh");
        let script = format!("#!/bin/sh\ncat \"{}/view.json\"\n", dir.0.display());
        std::fs::write(&gh, script).expect("gh");
        std::fs::write(dir.0.join("view.json"), view).expect("view");
        let made = process::run("chmod", &["+x", &gh.to_string_lossy()], None, CALL);
        made.expect("chmod");
        let path = format!(
            "{}:{}",
            dir.0.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        // one process a test (nextest): no other thread reads the environment; withdrawn statuses not read from the live folder
        unsafe {
            std::env::set_var("PATH", path);
            std::env::set_var("MERGE_CLAIM_SELFTEST", "1");
        }
        let forge = GitHub {
            repo: repo.to_path_buf(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: gh.to_string_lossy().into_owned(),
        };
        let pr = Pr {
            number: 1500,
            url: String::new(),
            head: String::new(),
            merged: false,
        };
        forge.checks(&pr).expect("the checks")
    }

    fn pr_view(file: &str, mirror: Option<&str>) -> String {
        let mirror = mirror.map_or(String::new(), |s| {
            format!(r#",{{"context":"ci-mirror/rust","state":"{s}"}}"#)
        });
        format!(
            r#"{{"url":"https://github.com/nha-org/site_perso/pull/1500","changedFiles":1,"files":[{{"path":"{file}"}}],
                "statusCheckRollup":[{{"name":"gate","status":"COMPLETED","conclusion":"SUCCESS"}}{mirror}]}}"#
        )
    }

    /// Done-when 1 of rust-pr-waits-mirror. The public mirror holds rust/ alone, no hook: there the test stops.
    #[dash_test]
    fn merge_on_green_holds_a_rust_pr_until_ci_mirror_rust_is_green() {
        let Some(repo) = dash_meter::monorepo() else {
            return;
        };
        let dir = TestDir::new("forge-mirror-gate");
        let rust = "rust/crates/forge/src/lib.rs";
        assert_eq!(
            gate_reads(&repo, &dir, &pr_view(rust, None)),
            Checks::Pending,
            "no mirror status yet: held"
        );
        assert_eq!(
            gate_reads(&repo, &dir, &pr_view(rust, Some("PENDING"))),
            Checks::Pending,
            "the mirror runs: held"
        );
        assert_eq!(
            gate_reads(&repo, &dir, &pr_view(rust, Some("SUCCESS"))),
            Checks::Green,
            "the mirror green: it merges"
        );
    }

    /// Done-when 2 of rust-pr-waits-mirror; on the mirror it stops, as the one above.
    #[dash_test]
    fn a_pr_changing_nothing_in_rust_merges_as_before() {
        let Some(repo) = dash_meter::monorepo() else {
            return;
        };
        let dir = TestDir::new("forge-mirror-none");
        let view = pr_view("private/docs/rfc-rust-unified.html", None);
        assert_eq!(
            gate_reads(&repo, &dir, &view),
            Checks::Green,
            "no mirror status owed: green on its own checks"
        );
    }

    #[dash_test]
    /// Done-when 3 of rust-pr-waits-mirror: PR 1444's head read red while mirror run 43041329651 ran its rerun (2026-11-11).
    fn dash_ci_wait_reads_a_mirror_rerun_in_progress_as_no_verdict_yet() {
        use mirror::CONTEXT;
        use mirror::tests::{COMMIT, HEAD, stand_in, status};
        let head = format!("1444 {HEAD}");
        // the rerun's job, started after the red was written
        let runs = r#"[{"id":1,"name":"test","status":"in_progress","conclusion":null,"started_at":"2026-11-11T16:10:00Z"},
                       {"id":2,"name":"site-musl","status":"completed","conclusion":"success"}]"#;
        let rules: [(&[&str], &str); 3] = [
            (&["pr", "view", "1444"], &head),
            (&[&format!("commits/{HEAD}/status")], &status()),
            (
                &[&format!("rust-ci-mirror/commits/{COMMIT}/check-runs")],
                runs,
            ),
        ];
        let mut calls = vec![];
        let mut gh = stand_in(&rules, &mut calls);
        let mut read = || {
            let red = vec![
                ("gate".into(), "SUCCESS".into()),
                (CONTEXT.into(), "FAILURE".into()),
            ];
            reread_mirror(red, "1444", &mut gh)
        };
        let (code, out, err) = ci_wait(
            &mut read,
            || false,
            std::time::Instant::now(),
            |_| Ok(vec![]),
        );
        println!("exit {code}: {out}{err}");
        assert_eq!(
            code, 3,
            "a rerun in progress is no verdict yet, never the old red: {err}"
        );
        assert!(err.contains(&format!("still pending: {CONTEXT}")), "{err}");
        drop(gh);
        // nothing written while it runs
        assert!(
            !calls.iter().any(|c| c.contains(&"POST".to_string())),
            "{calls:?}"
        );
    }

    #[dash_test]
    fn a_mirror_row_not_red_costs_no_call() {
        let mut calls = vec![];
        let mut gh = mirror::tests::stand_in(&[], &mut calls);
        let rows: Rows = vec![
            ("gate".into(), "FAILURE".into()),
            (mirror::CONTEXT.into(), "PENDING".into()),
        ];
        assert_eq!(reread_mirror(rows.clone(), "1444", &mut gh), Ok(rows));
        drop(gh);
        assert!(calls.is_empty(), "{calls:?}");
    }

    #[dash_test]
    fn checks_read_as_the_merge_hook_says() {
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

    /// Done-when 2 of ci-wait-gh-retry: the first two reads time out, the third answers green, and the wait exits 0.
    #[dash_test]
    fn ci_wait_asks_again_after_a_read_that_timed_out() {
        let rows = |r: &[(&str, &str)]| -> Rows {
            let row = |(n, s): &(&str, &str)| (n.to_string(), s.to_string());
            r.iter().map(row).collect()
        };
        let later = Instant::now() + CI_DEADLINE;
        let (mut reads, mut waits) = (0, 0);
        let read = || {
            reads += 1;
            if reads <= 2 {
                Err("gh did not answer".into())
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
            |_| Ok(vec![]),
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

        // the older app's rules: red is final at once, a billing refusal is no verdict, a pending head waits
        let once = |r: Rows| {
            ci_wait(
                move || Ok(r.clone()),
                || true,
                Instant::now(),
                |_| Ok(vec![]),
            )
        };
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
            |_| Ok(vec![]),
        );
        assert!(
            exit == 3 && empty == 3 && why.contains("no workflow covers it"),
            "{why}"
        );
        let (exit, _, why) = ci_wait(
            || Ok(rows(&[("a", "PENDING")])),
            || false,
            later,
            |_| Ok(vec![]),
        );
        assert!(exit == 3 && why.contains("the caller left"), "{why}");
    }

    /// Task rust-infra-red-reruns, done-when 1: `dash ci wait` names each red's failed step, read from the mirror's recorded jobs;
    /// one before any test is no verdict (exit 3), one in a test is red (exit 1), its failing test named.
    #[dash_test]
    fn ci_wait_names_the_failed_step_infra_or_code() {
        let fixture = |name: &str| {
            let at = format!("{}/tests/fixtures/red/{name}", env!("CARGO_MANIFEST_DIR"));
            std::fs::read_to_string(&at).expect("a recorded mirror job")
        };
        let red_of = |job: &str, log: &str| {
            let job: Value = serde_json::from_str(&fixture(job)).expect("JSON");
            red::red_of("ci-mirror/rust", "nha-org/rust-ci-mirror", &job, || {
                Some(fixture(log))
            })
        };
        let wait = |red: red::Red| {
            let rows: Rows = vec![("ci-mirror/rust".into(), "FAILURE".into())];
            ci_wait(
                move || Ok(rows.clone()),
                || true,
                Instant::now(),
                |names| {
                    assert_eq!(names, ["ci-mirror/rust"]);
                    Ok(vec![red.clone()])
                },
            )
        };
        let (exit, _, why) = wait(red_of("apt-job.json", "apt.log"));
        assert_eq!(exit, 3, "{why}");
        assert!(why.starts_with("ci wait: no verdict, an infra red: ci-mirror/rust failed at step « Run sudo apt-get update"), "{why}");
        assert!(
            why.contains("(infra, before any test)") && why.contains("reruns it"),
            "{why}"
        );
        let (exit, _, why) = wait(red_of("nextest-job.json", "nextest.log"));
        assert_eq!(exit, 1, "{why}");
        assert!(
            why.contains("nextest run --locked »")
                && why.contains("failing: dash::compass a_page_comment_keeps_its_images_and_pdfs"),
            "{why}"
        );
    }

    /// An infra red while another check still runs is no verdict yet: the wait goes on, and a code red there is the verdict
    /// (/code-review of 79af9f8d5).
    #[dash_test]
    fn ci_wait_waits_past_an_infra_red_while_others_run() {
        let infra = red::red_of(
            "a",
            "o/r",
            &serde_json::json!({"id": 1, "steps": []}),
            || None,
        );
        let code = serde_json::json!({"id": 2, "steps": [{"name": "Run cargo nextest run", "conclusion": "failure"}]});
        let code = red::red_of("b", "o/r", &code, || {
            Some("2026-11-11T17:07:42.9Z FAIL [ 0.1s] t::x".into())
        });
        let mut reads = 0;
        let read = || {
            reads += 1;
            let b = if reads == 1 { "PENDING" } else { "FAILURE" };
            Ok(vec![("a".into(), "FAILURE".into()), ("b".into(), b.into())])
        };
        let explain = |names: &[&str]| {
            let all = [infra.clone(), code.clone()];
            Ok(all
                .into_iter()
                .filter(|r| names.contains(&r.check.as_str()))
                .collect())
        };
        let (exit, _, why) = ci_wait(
            read,
            || true,
            Instant::now() + Duration::from_secs(5),
            explain,
        );
        assert_eq!(exit, 1, "{why}");
        assert!(
            why.contains("b failed at step « Run cargo nextest run », failing: t::x"),
            "{why}"
        );
    }

    /// A red whose steps a gh call failed to read is no verdict, as GitHub::checks says Pending: asked again, exit 3 only past its
    /// deadline, never a code red (/code-review of PR 1460).
    #[dash_test]
    fn ci_wait_asks_again_when_a_red_s_steps_are_not_read() {
        let rows: Rows = vec![("a".into(), "FAILURE".into())];
        let mut asked = 0;
        let (exit, _, why) = ci_wait(
            move || Ok(rows.clone()),
            || true,
            Instant::now(),
            |_| {
                asked += 1;
                Err("HTTP 502".into())
            },
        );
        assert_eq!((exit, asked), (3, 1), "{why}");
        assert!(why.contains("its reds not read: HTTP 502"), "{why}");
    }

    /// Done-when 2 of ci-wait-gh-retry, its second half: reads that never answer get no exit 3 before the deadline (each lost read
    /// before it is followed by another), and exit 3 once it passed.
    #[dash_test]
    fn ci_wait_exits_3_only_once_its_deadline_passed() {
        let begun = Instant::now();
        let until = begun + Duration::from_millis(50);
        let mut reads = vec![];
        let read = || {
            reads.push(Instant::now());
            Err("gh did not answer".into())
        };
        let wait = || {
            std::thread::sleep(Duration::from_millis(5));
            true
        };
        let (exit, out, why) = ci_wait(read, wait, until, |_| Ok(vec![]));
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
    #[dash_test]
    fn a_worktree_is_made_once_kept_across_a_retry_and_leaves() {
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
        assert_eq!(
            forge.unsaved("t1"),
            Ok(None),
            "main's commits are on a remote"
        );
        std::fs::write(Path::new(&folder).join("left"), "x").expect("a run's work");
        let unsaved = forge.unsaved("t1").expect("a look").expect("a change");
        assert_eq!(unsaved, "1 files changed and not committed");
        assert_eq!(forge.made("t1").expect("kept"), folder);
        assert!(Path::new(&folder).join("left").exists());
        // tracked, it is work; untracked, a merged run's scratch, which `leave` drops (task rust-merge-leaves-lease)
        run(Path::new(&folder), &["add", "left"]);
        assert!(forge.leave("t1").is_err(), "git refuses a dirty one");
        run(Path::new(&folder), &["rm", "-q", "-f", "left"]);
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
        let unsaved = forge.unsaved("t1").expect("a look").expect("a commit");
        assert_eq!(unsaved, "1 commits on no remote branch");
        let old = run(Path::new(&again), &["rev-parse", "HEAD"]);
        forge.leave("t1").expect("left");
        let reopened = forge.made("t1").expect("reopened");
        let head = run(Path::new(&reopened), &["rev-parse", "HEAD"]);
        assert_ne!(head, old);
        assert_eq!(head, run(&repo, &["rev-parse", "origin/main"]));
    }

    /// A real origin, PR 7's pull ref and its squash on main: a worktree's HEAD read against that PR open, then merged, then the
    /// task reopened (rust-forge-stale-worktree-head's done-when 1 to 4).
    #[dash_test]
    fn a_head_behind_its_pr_reads_it_one_past_it_needs_a_push_and_a_reopen_starts_from_main() {
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
        // a tracked change: untracked files are a merged run's scratch (task rust-merge-leaves-lease)
        std::fs::write(folder.join("left"), "x").expect("a run's work");
        run(&folder, &["add", "left"]);
        forge.opened("t1", Some(&merged)).expect("kept");
        assert!(folder.join("left").exists());
        assert_eq!(head(&folder), a1);
        run(&folder, &["rm", "-q", "-f", "left"]);
        let tracked = "refs/remotes/origin/task/t1";
        assert!(!run(&repo, &["for-each-ref", tracked]).is_empty());
        let fresh = PathBuf::from(forge.opened("t1", Some(&merged)).expect("reopened"));
        let main = run(&repo, &["rev-parse", "origin/main"]);
        assert_eq!(head(&fresh), main, "from main");
        assert_eq!(run(&fresh, &["branch", "--show-current"]), "task/t1");
        let at_origin = ["ls-remote", "--heads", "origin", "refs/heads/task/t1"];
        assert_eq!(run(&repo, &at_origin), "", "gone at the origin");
        assert_eq!(run(&repo, &["for-each-ref", tracked]), "", "pruned");
        // Task rust-rejudge-merged-closes, done-when 2: nothing new on a reopen's branch, the squash holds all of it; a commit past
        // its start is the reopen's own work, which no merged PR holds.
        assert_eq!(
            judged(&merged),
            Some(pr(true)),
            "a reopen's branch with nothing new: the squash holds its work"
        );
        commit(&fresh, "the reopen's work");
        assert_eq!(judged(&merged), None, "a reopen's new work: none merged");
        run(&fresh, &["reset", "-q", "--hard", &main]);
        // GitHub deleted the branch itself: the stale ref this repo kept is pruned, the reopen's own branch kept.
        run(&repo, &["update-ref", tracked, &a1]);
        let again = PathBuf::from(forge.opened("t1", Some(&merged)).expect("reopened"));
        assert_eq!(run(&repo, &["for-each-ref", tracked]), "", "pruned");
        assert_eq!(head(&again), main, "kept");
    }

    /// A real origin and its pull refs: only a head holding main's newest commit lands; one behind it merging cleanly and one
    /// conflicting are told apart, the conflict naming its file.
    #[dash_test]
    fn a_head_holds_main_or_lags_it_cleanly_or_conflicts_naming_the_file() {
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

    /// Done-when 1 of rust-fetch-heals-missing-ref: a remote ref planted at a commit only an object overlay holds, as the Elixir
    /// merge on green's push left one, fails a plain fetch; the forge's fetch drops it, then fetches, and the PR lands.
    #[dash_test]
    fn a_remote_ref_to_a_missing_object_is_dropped_and_the_pr_lands() {
        let dir = TestDir::new("forge-missing-ref");
        let (origin, repo, overlay) = (dir.0.join("origin"), dir.0.join("repo"), dir.0.join("ov"));
        for d in [&origin, &overlay] {
            std::fs::create_dir_all(d).expect("dirs");
        }
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = |m: &str| {
            run(
                &origin,
                &[&who[..], &["commit", "-q", "--allow-empty", "-m", m]].concat(),
            );
            run(&origin, &["rev-parse", "HEAD"]).trim().to_string()
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        commit("first");
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(
            &dir.0,
            &[
                "clone",
                "-q",
                &format!("file://{}", at(&origin)),
                &at(&repo),
            ],
        );
        let objects = at(&repo.join(".git/objects"));
        let env = [
            ("GIT_OBJECT_DIRECTORY", at(&overlay)),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects),
        ];
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let overlaid = |args: &[&str]| {
            let (status, out, why) =
                process::answer("git", args, Some(&repo), &env, GIT).expect("git");
            assert!(status.success(), "git {args:?}: {why}");
            out.trim().to_string()
        };
        let tree = run(&repo, &["rev-parse", "HEAD^{tree}"]);
        let lost = overlaid(
            &[
                &who[..],
                &["commit-tree", tree.trim(), "-p", "HEAD", "-m", "m"],
            ]
            .concat(),
        );
        overlaid(&["update-ref", "refs/remotes/origin/task/x", &lost]);
        commit("main moved");
        let plain = process::run("git", &["fetch", "-q", "origin", "main"], Some(&repo), GIT);
        let said = plain.expect_err("a plain fetch fails on the planted ref");
        assert!(said.contains("refs/remotes/origin/task/x"), "{said}");

        let head = commit("its work");
        run(&origin, &["update-ref", "refs/pull/9/head", &head]);
        let gh = dir.0.join("gh");
        let script = format!("#!/bin/sh\necho \"$*\" >> \"{}/gh.log\"\n", dir.0.display());
        std::fs::write(&gh, script).expect("gh");
        process::run("chmod", &["+x", &at(&gh)], None, CALL).expect("chmod");
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: at(&gh),
        };
        let pr = Pr {
            number: 9,
            url: "u9".into(),
            head: head.clone(),
            merged: false,
        };
        assert_eq!(forge.base(&pr).expect("fetched"), Base::Holds);
        forge.merge(&pr).expect("landed");
        let asked = std::fs::read_to_string(dir.0.join("gh.log")).expect("gh called");
        assert_eq!(
            asked.trim(),
            format!("pr merge 9 --squash --match-head-commit {head}")
        );
        let refs = run(
            &repo,
            &["for-each-ref", "--format=%(refname)", "refs/remotes"],
        );
        assert!(!refs.contains("task/x"), "dropped: {refs}");
        let check = run(&repo, &["fsck", "--connectivity-only", "--no-dangling"]);
        assert_eq!(check.trim(), "", "no ref names a missing object");

        // The git CLI's own fetch, a session's, heals at the look's minute: `drop_missing` on the checkout, then it fetches.
        overlaid(&["update-ref", "refs/remotes/origin/task/y", &lost]);
        commit("main moved again");
        let fetch = ["fetch", "-q", "origin", "main"];
        assert!(
            process::run("git", &fetch, Some(&repo), GIT).is_err(),
            "planted again"
        );
        let dropped = drop_missing(&repo).expect("read");
        assert_eq!(dropped, ["refs/remotes/origin/task/y"]);
        process::run("git", &fetch, Some(&repo), GIT).expect("a plain fetch passes");
        assert_eq!(
            drop_missing(&repo).expect("read"),
            Vec::<String>::new(),
            "none left"
        );
    }

    /// Done-when 2 of rust-fetch-heals-missing-ref: the stale-head merge (`Handover.merge_main` ported) and the merge train's
    /// build and drop push from Rust with the repository's own objects, so no remote ref is left at a missing object.
    #[dash_test]
    fn the_remerge_and_the_train_push_from_the_repository_s_own_objects() {
        let dir = TestDir::new("forge-remerge-train");
        // its path ends in the PR links' repository, which `origin_is` matches
        let (origin, repo) = (dir.0.join("nha-org/site_perso"), dir.0.join("repo"));
        let link = |n: u64| format!("https://github.com/nha-org/site_perso/pull/{n}");
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = |file: &str, text: &str| {
            std::fs::write(origin.join(file), text).expect("write");
            run(&origin, &["add", file]);
            run(&origin, &[&who[..], &["commit", "-q", "-m", file]].concat());
            run(&origin, &["rev-parse", "HEAD"]).trim().to_string()
        };
        let branch = |name: &str, file: &str, text: &str| {
            run(&origin, &["switch", "-q", "-c", name, "main"]);
            let head = commit(file, text);
            run(&origin, &["switch", "-q", "main"]);
            head
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        commit("c.rs", "1\n");
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(
            &dir.0,
            &[
                "clone",
                "-q",
                &format!("file://{}", at(&origin)),
                &at(&repo),
            ],
        );
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        let sound = || {
            let refs = run(
                &repo,
                &["for-each-ref", "--format=%(objectname)", "refs/remotes"],
            );
            for sha in refs.lines() {
                run(&repo, &["cat-file", "-e", sha]);
            }
        };
        let a = branch("task/a", "a.rs", "a\n");
        let clash = branch("task/b", "c.rs", "b\n");
        let main = commit("c.rs", "main\n");

        let other = forge.remerge("https://github.com/nha-org/replicator/pull/1", "task/a", &a);
        let why = other.expect_err("another repository's PR");
        assert!(
            why.contains("not https://github.com/nha-org/replicator/pull/1's"),
            "{why}"
        );
        let pushed = forge.remerge(&link(1), "task/a", &a).expect("remerge");
        let Remerge::Pushed(sha) = pushed else {
            panic!("{pushed:?}")
        };
        assert_eq!(
            run(&origin, &["rev-parse", "task/a"]).trim(),
            sha,
            "the PR's new head"
        );
        let parents = run(&repo, &["rev-list", "--parents", "-n", "1", &sha]);
        assert_eq!(parents.trim(), format!("{sha} {a} {main}"));
        sound();
        assert_eq!(
            forge.remerge(&link(1), "task/a", &a),
            Ok(Remerge::Moved),
            "the head read is gone"
        );
        assert_eq!(forge.remerge(&link(1), "task/a", &sha), Ok(Remerge::Holds));
        let files = vec!["c.rs".to_string()];
        assert_eq!(
            forge.remerge(&link(2), "task/b", &clash),
            Ok(Remerge::Conflicts(files))
        );

        let pull = |n: u64, head: &str| {
            run(
                &origin,
                &["update-ref", &format!("refs/pull/{n}/head"), head],
            );
            TrainPr {
                number: n,
                head: head.to_string(),
                title: format!("pr {n}"),
                url: link(n),
            }
        };
        let seven = pull(7, &branch("task/c", "d.rs", "d\n"));
        let eight = pull(8, &clash);
        let mut nine = pull(9, &branch("task/e", "e.rs", "e\n"));
        nine.head = a.clone();
        let train = forge.train_build(57, &[seven, eight, nine]).expect("built");
        assert_eq!(train.took, [link(7)]);
        let out: Vec<(&str, &str)> = (train.out.iter())
            .map(|(u, why)| (u.as_str(), why.split(' ').next().unwrap_or_default()))
            .collect();
        let (eight, nine) = (link(8), link(9));
        let want = [(eight.as_str(), "conflicts"), (nine.as_str(), "its")];
        assert_eq!(
            out, want,
            "a conflict and a moved head left out: {:?}",
            train.out
        );
        assert_eq!(run(&origin, &["rev-parse", "train/57"]).trim(), train.tip);
        let parent = run(&repo, &["rev-parse", &format!("{}^", train.tip)]);
        assert_eq!(parent.trim(), main, "squashed on main");
        run(&repo, &["cat-file", "-e", &train.tip]);
        sound();
        forge.train_drop(57).expect("dropped");
        let left = run(&origin, &["branch", "--list", "train/*"]);
        assert_eq!(left.trim(), "", "train/57 gone at the origin");
        forge.train_drop(57).expect("gone already is dropped");
        let gone = branch("task/g", "g.rs", "g\n");
        run(&origin, &["branch", "-D", "task/g"]);
        let deleted = forge.remerge(&link(3), "task/g", &gone);
        assert_eq!(
            deleted,
            Ok(Remerge::Moved),
            "a deleted branch is never recreated"
        );
        let check = run(&repo, &["fsck", "--connectivity-only", "--no-dangling"]);
        assert_eq!(check.trim(), "");
    }

    /// Task rust-pr-one-owner, done-when 3: a run's own push is a commit its worktree made that a remote branch holds; what a pull
    /// brought, a commit not pushed, or one before the run began is none.
    #[dash_test]
    fn own_pushes_are_commits_made_in_its_worktree_and_pushed() {
        let dir = TestDir::new("forge-own");
        let (origin, folder) = (dir.0.join("origin"), dir.0.join("rust-t1"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = |at: &Path, m: &str| {
            let args = ["commit", "-q", "--allow-empty", "-m", m];
            run(at, &[&who[..], &args].concat());
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        commit(&origin, "first");
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&folder)]);
        run(&folder, &["switch", "-q", "-c", "task/t1"]);
        let forge = GitHub {
            repo: folder.clone(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        let own = || forge.own_pushes("t1", 0).expect("read");
        assert!(!own(), "a clone and a checkout made nothing");
        // another's fix, pulled
        run(&origin, &["switch", "-q", "-c", "task/t1"]);
        commit(&origin, "the orchestrator's fix");
        run(&origin, &["switch", "-q", "main"]);
        run(
            &folder,
            &[&who[..], &["pull", "-q", "--ff-only", "origin", "task/t1"]].concat(),
        );
        assert!(!own(), "a pull brought it");
        commit(&folder, "mine");
        assert!(!own(), "not pushed yet");
        run(&folder, &["push", "-q", "origin", "task/t1:task/t1-mine"]);
        assert!(own(), "pushed");
        assert!(
            !made_here("merge origin/main: Merge made by the 'ort' strategy."),
            "an update"
        );
        assert!(!made_here("pull: Merge made by the 'ort' strategy."));
        assert!(
            made_here("commit (merge): Merge origin/main"),
            "a conflict resolved"
        );
        assert!(made_here("pull --rebase (pick): mine"));
        assert!(!made_here(
            "rebase (finish): returning to refs/heads/task/t1"
        ));
        let later = (std::time::SystemTime::now() + Duration::from_secs(5))
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_millis() as i64;
        assert!(
            !forge.own_pushes("t1", later).expect("read"),
            "before the run"
        );
        assert!(!forge.own_pushes("t2", 0).expect("read"), "no worktree");
    }

    /// A real repository, a stand-in gh answering per branch: an idle, clean worktree whose PR merged leaves with its branch, one
    /// whose PR closed leaves its branch, and an open PR, a changed tree, a claimed task or a fresh touch each keep theirs
    /// (task worktrees-under-5g, done-when 1). A merged one holding only a data copy git does not track goes, the copy with it,
    /// once git is silent a day there; a tracked change keeps it (task rust-merge-leaves-lease, done-when 2: 10 such stayed held
    /// on 2026-11-12).
    #[dash_test]
    fn a_reap_takes_the_idle_clean_worktrees_whose_pr_merged_or_closed() {
        let dir = TestDir::new("forge-reap");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| {
            let out = process::run("git", args, Some(at), GIT).expect("git");
            out.trim().to_string()
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let first = ["commit", "-q", "--allow-empty", "-m", "first"];
        run(&origin, &[&who[..], &first].concat());
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&repo)]);
        let gh = dir.0.join("gh");
        let answers = dir.0.join("answers");
        std::fs::create_dir_all(&answers).expect("dirs");
        let script = format!(
            "#!/bin/sh\ncat \"{}/$(echo \"$4\" | tr / -)\"\n",
            at(&answers)
        );
        std::fs::write(&gh, script).expect("gh");
        std::process::Command::new("chmod")
            .args(["+x", &at(&gh)])
            .status()
            .expect("chmod");
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: at(&gh),
        };
        // Its PR's head: the worktree's HEAD as made, from main.
        let answer = |id: &str, state: &str, main: &str| {
            let pr = format!(
                r#"[{{"number":7,"url":"u-{id}","state":"{state}","headRefOid":"{main}","mergeCommit":null}}]"#
            );
            std::fs::write(answers.join(format!("task-{id}")), pr).expect("answer");
        };
        // Untouched two hours: its git dir's index, HEAD and reflog set back.
        let idle = |folder: &Path| {
            let git_dir = run(
                folder,
                &["rev-parse", "--path-format=absolute", "--git-dir"],
            );
            let then = std::time::SystemTime::now() - Duration::from_secs(7200);
            for f in ["index", "HEAD", "logs/HEAD"] {
                let file = std::fs::File::options()
                    .write(true)
                    .open(Path::new(&git_dir).join(f));
                file.expect("git file").set_modified(then).expect("mtime");
            }
        };
        let mut folders = vec![];
        for (id, state) in [
            ("merged", "MERGED"),
            ("moved", "MERGED"),
            ("merges", "MERGED"),
            ("closed", "CLOSED"),
            ("open", "OPEN"),
            ("dirty", "MERGED"),
            ("scratch", "MERGED"),
            ("young", "MERGED"),
            ("busy", "MERGED"),
            ("fresh", "MERGED"),
        ] {
            let folder = PathBuf::from(forge.made(id).expect("made"));
            let made = run(&folder, &["rev-parse", "HEAD"]);
            answer(id, state, &made);
            // A session that moved to a branch of its own: that branch's PR is the one read.
            if id == "moved" {
                run(&folder, &["switch", "-q", "-c", "task/moved-own"]);
                answer("moved-own", state, &made);
                answer(id, "OPEN", &made);
            }
            // Main moves past its PR's head and is merged in: no commit of its own.
            if id == "merges" {
                let moved = ["commit", "-q", "--allow-empty", "-m", "main moves"];
                run(&origin, &[&who[..], &moved].concat());
                run(&folder, &["fetch", "-q", "origin", "main"]);
                let merge = ["merge", "-q", "--no-ff", "-m", "main in", "origin/main"];
                run(&folder, &[&who[..], &merge].concat());
            }
            if id != "fresh" {
                idle(&folder);
            }
            folders.push((id, folder));
        }
        let path = |id: &str| {
            folders
                .iter()
                .find(|(i, _)| *i == id)
                .expect("one")
                .1
                .clone()
        };
        std::fs::write(path("dirty").join("left"), "x").expect("a run's work");
        run(&path("dirty"), &["add", "left"]);
        idle(&path("dirty"));
        // A data copy, git silent two days; the same two hours old may be a session's new files, kept for a day.
        for id in ["scratch", "young"] {
            std::fs::create_dir_all(path(id).join("live-copy")).expect("a copy");
            std::fs::write(path(id).join("live-copy/dash.sqlite"), "x").expect("a copy");
        }
        aged(&path("scratch"), Duration::from_secs(2 * 86_400));
        let said = forge.reap(&["busy".to_string()]).expect("a pass");
        let mut said: Vec<&str> = said
            .iter()
            .map(|s| s.split(':').next().unwrap_or(""))
            .collect();
        said.sort_unstable();
        assert_eq!(
            said,
            ["closed", "merged", "merges", "moved", "scratch"],
            "what went"
        );
        for id in ["merged", "closed", "merges", "moved", "scratch"] {
            assert!(!path(id).exists(), "{id} gone");
        }
        for n in 1..=SLOTS {
            assert!(!forge.slot(n).join("live-copy").exists(), "no copy rides");
        }
        // A pass never reads as a touch: the open one, judged, stays idle for the next pass.
        let git_dir = run(
            &path("open"),
            &["rev-parse", "--path-format=absolute", "--git-dir"],
        );
        assert!(settled(Path::new(&git_dir)), "a pass left it untouched");
        for id in ["open", "dirty", "young", "busy", "fresh"] {
            assert!(path(id).exists(), "{id} kept");
        }
        let branch = |id: &str| run(&repo, &["branch", "--list", &format!("task/{id}")]);
        assert_eq!(branch("merged"), "", "a merged one's branch goes");
        assert_eq!(branch("moved-own"), "", "the branch it held goes");
        assert_ne!(
            branch("closed"),
            "",
            "a closed one's branch stays for a reopen"
        );
        // A closed PR whose worktree holds a commit no remote has keeps it.
        let ahead = PathBuf::from(forge.made("ahead").expect("made"));
        let local = ["commit", "-q", "--allow-empty", "-m", "unpushed"];
        run(&ahead, &[&who[..], &local].concat());
        let made = run(&repo, &["rev-parse", "origin/main"]);
        answer("ahead", "CLOSED", &made);
        idle(&ahead);
        assert_eq!(
            forge.reap(&["busy".into()]).expect("a pass"),
            Vec::<String>::new()
        );
        assert!(ahead.exists(), "unpushed work stays");
    }

    /// Task rust-nothing-done-holds, done-when 2 (lease): a finished task's worktree the caller saw nothing work in goes at once,
    /// once git and its scratch are an hour untouched, where `reap` waits a day: a merged one with its scratch and its branch, a
    /// dropped one with no PR and its HEAD pushed, its branch kept. One holding a tracked change or a commit no remote has stays,
    /// and so does one whose scratch or git moved within the hour (a session outside herdr), each said.
    #[dash_test]
    fn a_finished_task_s_worktree_goes_after_an_hour_when_nothing_works_in_it() {
        let dir = TestDir::new("forge-release-done");
        let mut forge = pool_repo(&dir);
        let run = |at: &Path, args: &[&str]| {
            let out = process::run("git", args, Some(at), GIT).expect("git");
            out.trim().to_string()
        };
        let at = |p: &Path| p.to_str().expect("path").to_string();
        let (gh, answers) = (dir.0.join("gh"), dir.0.join("answers"));
        std::fs::create_dir_all(&answers).expect("dirs");
        let script = format!(
            "#!/bin/sh\ncat \"{}/$(echo \"$4\" | tr / -)\"\n",
            at(&answers)
        );
        std::fs::write(&gh, script).expect("gh");
        let chmod = std::process::Command::new("chmod")
            .args(["+x", &at(&gh)])
            .status();
        assert!(chmod.expect("chmod").success());
        forge.cli = at(&gh);
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let mut folders = vec![];
        for (id, merged) in [
            ("merged", true),
            ("dropped", false),
            ("dirty", true),
            ("ahead", false),
            ("young", true),
            ("touched", true),
        ] {
            let folder = PathBuf::from(forge.made(id).expect("made"));
            let head = run(&folder, &["rev-parse", "HEAD"]);
            let pr = format!(
                r#"[{{"number":7,"url":"u-{id}","state":"MERGED","headRefOid":"{head}","mergeCommit":null}}]"#
            );
            let answer = if merged { pr } else { "[]".to_string() };
            std::fs::write(answers.join(format!("task-{id}")), answer).expect("answer");
            // its session's scratch, two hours old: `reap` would wait a day for it
            std::fs::write(folder.join("scratch.md"), "notes").expect("scratch");
            let two_hours = std::time::SystemTime::now() - Duration::from_secs(7200);
            let scratch = std::fs::File::options()
                .write(true)
                .open(folder.join("scratch.md"));
            scratch
                .expect("scratch")
                .set_modified(two_hours)
                .expect("mtime");
            folders.push(folder);
        }
        std::fs::remove_file(folders[1].join("scratch.md"))
            .expect("a dropped one's scratch is its own to keep");
        std::fs::write(folders[2].join(".gitignore"), "changed").expect("a tracked change");
        run(
            &folders[3],
            &[
                &who[..],
                &["commit", "-q", "--allow-empty", "-m", "unpushed"],
            ]
            .concat(),
        );
        for folder in &folders[..5] {
            aged(folder, Duration::from_secs(7200));
        }
        // a session outside herdr wrote scratch a moment ago in one, and moved git in the other
        std::fs::write(folders[4].join("scratch.md"), "new notes").expect("scratch");
        assert_eq!(
            forge.reap(&[]).expect("a pass"),
            Vec::<String>::new(),
            "the reap waits its day"
        );
        let said = ["merged", "dropped", "dirty", "ahead", "young", "touched"]
            .map(|id| forge.release_done(id).expect("released or kept"));
        assert_eq!(said[0], "its PR u-merged merged: worktree and branch gone");
        assert_eq!(
            said[1],
            "nothing a remote lacks: worktree released, branch kept"
        );
        assert_eq!(said[5], "kept: git moved there within the hour");
        for kept in &said[2..5] {
            assert!(
                kept.starts_with("kept: it holds what no remote has"),
                "{kept}"
            );
        }
        assert!(!folders[0].exists() && !folders[1].exists(), "both gone");
        assert!(folders[2..].iter().all(|f| f.exists()), "the rest kept");
        let branch = |id: &str| run(&forge.repo, &["branch", "--list", &format!("task/{id}")]);
        assert_eq!(branch("merged"), "", "a merged one's branch goes");
        assert_ne!(branch("dropped"), "", "a dropped one's stays for a reopen");
        assert_eq!(forge.release_done("never").expect("none"), "no worktree");
    }

    /// A real repository whose main ignores `target/` and `live/`, cloned, and a forge on the clone (task rust-worktree-pool).
    fn pool_repo(dir: &TestDir) -> GitHub {
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(&origin).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        run(&origin, &["init", "-q", "-b", "main"]);
        std::fs::write(origin.join(".gitignore"), "target/\nlive/\n").expect("write");
        run(&origin, &["add", ".gitignore"]);
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        run(
            &origin,
            &[&who[..], &["commit", "-q", "-m", "first"]].concat(),
        );
        let at = |p: &Path| p.to_str().expect("path").to_string();
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&repo)]);
        GitHub {
            repo,
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        }
    }

    /// A worktree with a build and a file git ignores, as a task's run leaves them.
    fn used(forge: &GitHub, id: &str) -> PathBuf {
        let folder = PathBuf::from(forge.made(id).expect("made"));
        std::fs::create_dir_all(folder.join("rust/target/debug")).expect("a build");
        std::fs::write(folder.join("rust/target/debug/dash"), id).expect("a build");
        std::fs::create_dir_all(folder.join("live")).expect("its data");
        std::fs::write(folder.join("live/store"), id).expect("its data");
        folder
    }

    /// Task rust-worktree-pool, done-when 1: a run takes a pooled worktree, its rust/target kept, its untracked files dropped.
    #[dash_test]
    fn a_run_takes_a_pooled_worktree_its_build_kept_its_untracked_files_dropped() {
        let dir = TestDir::new("forge-pool-lease");
        let forge = pool_repo(&dir);
        let first = used(&forge, "t1");
        forge.leave("t1").expect("left");
        let slot = dir.0.join("pool.noindex/pool-1");
        assert!(!first.exists() && slot.join(".git").exists(), "pooled");
        // Spotlight skips a folder whose name ends .noindex, and all under it (task reap-kept-worktrees, done-when 4)
        let pool = slot
            .parent()
            .expect("its pool")
            .file_name()
            .expect("a name");
        assert!(pool.to_string_lossy().ends_with(".noindex"), "{pool:?}");
        // A start that fails (its branch is checked out elsewhere) puts the slot back, its build with it.
        process::run("git", &["branch", "task/t9"], Some(&slot), GIT).expect("a branch");
        let held = ["checkout", "-q", "task/t9"];
        process::run("git", &held, Some(&forge.repo), GIT).expect("checked out");
        assert!(forge.made("t9").is_err(), "git refuses a branch twice");
        assert!(
            slot.join("rust/target/debug/dash").exists(),
            "back in its slot"
        );
        // An edit to a tracked file made in the idle slot never rides either.
        std::fs::write(slot.join(".gitignore"), "edited\n").expect("an edit");
        let folder = PathBuf::from(forge.made("t2").expect("made"));
        assert!(
            folder.ends_with("rust-t2") && !slot.exists(),
            "the slot is its worktree now"
        );
        let built = std::fs::read_to_string(folder.join("rust/target/debug/dash"));
        assert_eq!(built.expect("kept"), "t1", "the last task's build is kept");
        assert!(
            !folder.join("live").exists(),
            "its untracked files are dropped"
        );
        let git = |args: &[&str]| process::run("git", args, Some(&folder), GIT).expect("git");
        assert_eq!(git(&["branch", "--show-current"]).trim(), "task/t2");
        assert_eq!(
            git(&["rev-parse", "HEAD"]),
            git(&["rev-parse", "origin/main"])
        );
        assert_eq!(git(&["status", "--porcelain"]), "", "a clean start");
        let listed = git(&["worktree", "list", "--porcelain"]);
        assert_eq!(listed.matches("worktree ").count(), 2, "{listed}");
        // A slot's folder that is no worktree is said at the reap, never silently held.
        std::fs::create_dir_all(dir.0.join("pool.noindex/pool-2")).expect("a stray folder");
        let said = forge.reap(&[]).expect("a pass");
        let lost = "pool-2: no worktree, its slot is lost until the folder goes";
        assert_eq!(said, vec![lost.to_string()]);
    }

    /// Task rust-worktree-pool, done-when 1: a fresh worktree only when no slot is free.
    #[dash_test]
    fn a_fresh_worktree_is_made_only_when_no_slot_is_free() {
        let dir = TestDir::new("forge-pool-fresh");
        let forge = pool_repo(&dir);
        used(&forge, "t1");
        let fresh = PathBuf::from(forge.made("t2").expect("made"));
        assert!(!fresh.join("rust/target").exists(), "an empty pool: fresh");
        forge.leave("t1").expect("left");
        let pooled = PathBuf::from(forge.made("t3").expect("made"));
        assert!(
            pooled.join("rust/target/debug/dash").exists(),
            "the one free slot is taken"
        );
        let fresh = PathBuf::from(forge.made("t4").expect("made"));
        assert!(
            !fresh.join("rust/target").exists(),
            "none free again: fresh"
        );
        // Past SLOTS idle ones, a worktree that leaves is removed: the pool never grows.
        for n in 0..=SLOTS {
            used(&forge, &format!("more{n}"));
        }
        for n in 0..=SLOTS {
            forge.leave(&format!("more{n}")).expect("left");
        }
        let slots = std::fs::read_dir(dir.0.join(POOL)).expect("read").flatten();
        let slots = slots.filter(|e| e.file_name().to_string_lossy().starts_with("pool-"));
        assert_eq!(slots.count(), SLOTS);
        assert!(
            !dir.0.join(format!("rust-more{SLOTS}")).exists(),
            "the last one is removed"
        );
    }

    /// Task rust-worktree-pool, done-when 2: an idle slot keeps no build past its cap, never while a cargo runs there.
    #[dash_test]
    fn a_worktree_joins_the_pool_without_a_build_past_its_cap_never_while_a_cargo_runs() {
        let dir = TestDir::new("forge-pool-cap");
        let forge = pool_repo(&dir);
        let folder = used(&forge, "t1");
        let path = folder.to_string_lossy().into_owned();
        let lock = folder.join("rust/target/debug/.cargo-lock");
        std::fs::write(&lock, "").expect("a lock");
        let cargo = std::fs::File::open(&lock).expect("cargo's lock");
        cargo.lock().expect("a cargo runs");
        let said = forge.pooled(&path, 0).expect_err("a cargo runs there");
        assert_eq!(said, format!("a cargo runs in {path}/rust/target"));
        assert!(folder.join("rust/target/debug/dash").exists(), "all stays");
        let under = forge.pooled(&path, SLOT_CAP);
        assert_eq!(
            under,
            Err(said),
            "nor under its cap: a cargo would follow it"
        );
        assert!(forge.leave("t1").is_err() && folder.exists());
        drop(cargo);
        // A dirty one stays, git's refusal, and keeps its build.
        std::fs::write(folder.join("left"), "x").expect("a run's work");
        assert!(forge.pooled(&path, 0).is_err(), "git refuses a dirty one");
        assert!(
            folder.join("rust/target/debug/dash").exists(),
            "its build too"
        );
        std::fs::remove_file(folder.join("left")).expect("clean");
        forge.pooled(&path, 0).expect("pooled");
        let slot = dir.0.join("pool.noindex/pool-1");
        assert!(
            slot.join(".git").exists() && !slot.join("rust/target").exists(),
            "no build past its cap"
        );
        // under its cap, the build rides into the pool
        let folder = used(&forge, "t2");
        forge
            .pooled(&folder.to_string_lossy(), SLOT_CAP)
            .expect("pooled");
        assert!(
            dir.0
                .join("pool.noindex/pool-1/rust/target/debug/dash")
                .exists()
        );
    }

    /// Its index, HEAD and reflog moved `ago` back, as a worktree no one touched since.
    fn aged(folder: &Path, ago: Duration) {
        let args = ["rev-parse", "--path-format=absolute", "--git-dir"];
        let git_dir = process::run("git", &args, Some(folder), GIT).expect("git dir");
        let then = std::time::SystemTime::now() - ago;
        for f in ["index", "HEAD", "logs/HEAD"] {
            let file = std::fs::File::options()
                .write(true)
                .open(Path::new(git_dir.trim()).join(f));
            file.expect("git file").set_modified(then).expect("mtime");
        }
    }

    /// Task reap-kept-worktrees, done-when 2: a finished `.claude/worktrees/agent-*` worktree, its branch pushed, is released at
    /// the reap, its branch kept; one holding a commit no remote has, or touched within the hour, stays. And a task's worktree
    /// untouched a day, pushed, is released whatever its PR, as `dash worktree list` reads it unheld.
    #[dash_test]
    fn a_finished_agent_worktree_with_a_pushed_branch_is_released_at_the_reap() {
        let dir = TestDir::new("forge-reap-agents");
        let forge = pool_repo(&dir);
        let repo = std::fs::canonicalize(&forge.repo).expect("real"); // git lists the real path
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let made_at = |path: PathBuf, name: &str, push: bool| {
            let branch = format!("worktree-{name}");
            let at = path.to_string_lossy().into_owned();
            run(
                &repo,
                &["worktree", "add", "-q", "-b", &branch, &at, "origin/main"],
            );
            run(
                &path,
                &[&who[..], &["commit", "-q", "--allow-empty", "-m", name]].concat(),
            );
            if push {
                run(&path, &["push", "-q", "origin", &branch]);
            }
            aged(&path, UNHELD);
            path
        };
        let agent =
            |name: &str, push: bool| made_at(repo.join(".claude/worktrees").join(name), name, push);
        let (pushed, unpushed) = (agent("agent-a1", true), agent("agent-a2", false));
        // a worktree a session made in its scratchpad, as two were left untouched 35 and 42 hours on 2026-11-12
        let scratch = std::fs::canonicalize(&dir.0)
            .expect("real")
            .join("claude-501/s/scratchpad/wt");
        let scratch = made_at(scratch, "scratch", true);
        let fresh = agent("agent-a3", true);
        aged(&fresh, Duration::from_secs(7200));
        // his words in an ignored file (`live_data`), and a worktree its owner locked, stay
        let live = agent("agent-a4", true);
        std::fs::create_dir_all(live.join("private/live")).expect("dirs");
        std::fs::write(live.join("private/live/comments.edn"), "his words").expect("live data");
        let locked = agent("agent-a5", true);
        run(&repo, &["worktree", "lock", &locked.to_string_lossy()]);
        let said = forge.reap(&[]).expect("a pass");
        let gone = "untouched, its HEAD pushed: worktree released, branch kept";
        let mut said = said;
        said.sort();
        let mut want = [&pushed, &scratch].map(|p| format!("{}: {gone}", p.display()));
        want.sort();
        assert_eq!(said, want);
        assert!(
            !pushed.exists()
                && !scratch.exists()
                && dir.0.join("pool.noindex/pool-2/.git").exists(),
            "both released to the pool"
        );
        assert_ne!(
            run(&repo, &["branch", "--list", "worktree-agent-a1"]),
            "",
            "its branch kept"
        );
        assert!(
            unpushed.exists() && fresh.exists(),
            "unpushed work and a running agent stay"
        );
        assert!(
            live.exists() && locked.exists(),
            "live data and a lock stay"
        );
        // A task's worktree untouched a day, its HEAD pushed, reads unheld and goes at the next pass, PR or none: no gh asked.
        let task = PathBuf::from(forge.made("t1").expect("made"));
        let list = |forge: &GitHub| {
            forge
                .rows()
                .expect("rows")
                .into_iter()
                .map(|r| (r.path, r.state))
        };
        let at = |p: &Path| p.to_string_lossy().into_owned();
        aged(&task, Duration::from_secs(3600));
        assert!(
            list(&forge).any(|(p, s)| p == at(&task) && s == "held"),
            "an hour: held"
        );
        aged(&task, UNHELD);
        assert!(
            list(&forge).any(|(p, s)| p == at(&task) && s == "unheld"),
            "a day: unheld"
        );
        let stand_in = dir.0.join("gh");
        std::fs::write(&stand_in, "#!/bin/sh\necho '[]'\n").expect("a stand-in gh");
        std::fs::set_permissions(
            &stand_in,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("chmod");
        let forge = GitHub {
            cli: at(&stand_in),
            ..forge
        };
        let said = forge.reap(&[]).expect("a pass");
        assert_eq!(said, vec![format!("t1: {gone}")]);
        assert!(
            !task.exists() && dir.0.join("pool.noindex/pool-1/.git").exists(),
            "the slot it took, back"
        );
        assert!(
            !list(&forge).any(|(_, s)| s == "unheld"),
            "none unheld left"
        );
    }

    /// Task reap-kept-worktrees, done-when 4, workz against klar's model work (klar's .workz.toml): a worktree whose model data
    /// and env come in by symlink, as workz's `symlink_add` makes them, leaves and is leased again. The primary's shared data
    /// stays whole (git clean never follows a link); the links do not ride into the next task (the gap: klar runs `workz sync`
    /// after a lease); a worktree holding his workshop comments is refused at leave.
    #[dash_test]
    fn klar_s_model_data_stays_whole_its_links_need_a_sync_and_live_comments_refuse_a_leave() {
        let dir = TestDir::new("forge-klar-needs");
        let forge = pool_repo(&dir);
        // klar ignores what workz links in
        let exclude = forge.repo.join(".git/info/exclude");
        std::fs::write(&exclude, "model-lab/data/\n.env\nwebsite/workshop/\n").expect("ignored");
        let corpus = dir.0.join("primary-corpus");
        std::fs::create_dir_all(&corpus).expect("the shared corpus");
        std::fs::write(corpus.join("a.eml"), "a training mail").expect("model data");
        std::fs::write(dir.0.join("primary.env"), "KEY=1").expect("the shared env");
        let link = |folder: &Path| {
            std::fs::create_dir_all(folder.join("model-lab/data")).expect("dirs");
            std::os::unix::fs::symlink(&corpus, folder.join("model-lab/data/corpus"))
                .expect("a link");
            std::os::unix::fs::symlink(dir.0.join("primary.env"), folder.join(".env"))
                .expect("a link");
        };
        let first = PathBuf::from(forge.made("k1").expect("made"));
        link(&first);
        let comments = first.join("website/workshop/comments.live.json");
        std::fs::create_dir_all(comments.parent().expect("dir")).expect("dirs");
        std::fs::write(&comments, "[\"his words\"]").expect("his comments");
        let path = first.to_string_lossy().into_owned();
        let refused = forge.leave_at(&path).expect_err("his live words");
        assert_eq!(
            refused,
            format!("{path} holds his live words in an ignored file: kept")
        );
        std::fs::remove_file(&comments).expect("answered and gone");
        forge.leave_at(&path).expect("left");
        let next = PathBuf::from(forge.made("k2").expect("made"));
        assert!(
            !dir.0.join("pool.noindex/pool-1").exists(),
            "k2 took the slot"
        );
        assert_eq!(
            std::fs::read_to_string(corpus.join("a.eml")).expect("kept"),
            "a training mail",
            "the primary's model data stays whole"
        );
        assert!(
            dir.0.join("primary.env").exists(),
            "the primary's env stays"
        );
        let gone = |p: &str| std::fs::symlink_metadata(next.join(p)).is_err();
        assert!(
            gone("model-lab/data/corpus") && gone(".env"),
            "links do not ride: workz sync after a lease"
        );
    }

    /// Task reap-kept-worktrees, done-when 4: a slot left at `<root>/pool-<n>` before `POOL` moves under it at the reap, out of
    /// Spotlight's reach, still a worktree the next run leases.
    #[dash_test]
    fn a_slot_made_before_the_noindex_folder_moves_under_it_at_the_reap() {
        let dir = TestDir::new("forge-pool-noindex");
        let forge = pool_repo(&dir);
        let old = dir.0.join("pool-1");
        let args = [
            "worktree",
            "add",
            "-q",
            "--detach",
            &old.to_string_lossy(),
            "origin/main",
        ];
        process::run("git", &args, Some(&forge.repo), GIT).expect("an old slot");
        assert_eq!(forge.reap(&[]).expect("a pass"), Vec::<String>::new());
        assert!(
            !old.exists() && dir.0.join("pool.noindex/pool-1/.git").exists(),
            "moved"
        );
        let folder = PathBuf::from(forge.made("t1").expect("made"));
        assert!(
            !dir.0.join("pool.noindex/pool-1").exists() && folder.ends_with("rust-t1"),
            "leased from there"
        );
    }

    /// /code-review of reap-kept-worktrees: a bare repository's pool is named after it, never after its parent folder.
    #[dash_test]
    fn a_bare_repository_names_its_own_pool() {
        let dir = TestDir::new("forge-bare");
        let bare = dir.0.join("klar.git");
        let init = ["init", "-q", "--bare", &bare.to_string_lossy()];
        process::run("git", &init, Some(&dir.0), GIT).expect("a bare repository");
        let forge = GitHub::on(&bare, Path::new("/h")).expect("a forge");
        assert_eq!(forge.root, Path::new("/h/.herdr/worktrees/klar"));
    }

    /// Task rust-worktree-pool, done-when 3: TMPDIR's dash-test-run-* folders past a day are pruned, no other.
    #[dash_test]
    fn test_run_folders_past_a_day_are_pruned() {
        let dir = TestDir::new("forge-prune");
        let aged = |name: &str, age: Duration| {
            let at = dir.0.join(name);
            std::fs::create_dir_all(at.join("store")).expect("a folder");
            std::fs::write(at.join("store/db"), "x").expect("a file");
            let then = std::time::SystemTime::now() - age;
            let folder = std::fs::File::open(&at).expect("open");
            folder.set_modified(then).expect("aged");
            at
        };
        let day = Duration::from_secs(24 * 3600);
        let old = aged("dash-test-run-1-1", day + Duration::from_secs(60));
        let new = aged("dash-test-run-2-2", day - Duration::from_secs(60));
        let other = aged("dash-forge-3", day * 3);
        let said = prune_test_runs(&dir.0);
        let words = format!("1 test folders past a day pruned from {}", dir.0.display());
        assert_eq!(said, vec![words]);
        assert!(!old.exists(), "past a day: gone");
        assert!(
            new.exists() && other.exists(),
            "a newer one and another's stay"
        );
        assert_eq!(
            prune_test_runs(&dir.0),
            Vec::<String>::new(),
            "nothing left to say"
        );
    }

    /// A merged task's build output goes even when git keeps its worktree for a file left in it.
    #[dash_test]
    fn a_merged_task_s_build_output_goes_even_from_a_worktree_git_keeps() {
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
        // a tracked change: a merged worktree's untracked files are scratch (task rust-merge-leaves-lease)
        std::fs::write(folder.join("left"), "x").expect("a run's work");
        run(&folder, &["add", "left"]);
        assert_eq!(
            forge.built(),
            Ok(vec!["t1".to_string()]),
            "a build to clean"
        );
        forge.clean("t1").expect("cleaned");
        assert_eq!(forge.built(), Ok(vec![]), "none left");
        assert!(
            !folder.join("rust/target").exists(),
            "its build output went"
        );
        assert!(forge.leave("t1").is_err(), "git refuses a dirty one");
        assert!(folder.join("left").exists(), "its work stays");
        forge.clean("t1").expect("nothing left to clean");
        run(&folder, &["rm", "-q", "-f", "left"]);
        forge.leave("t1").expect("left");
        assert!(!folder.exists());
    }

    /// Task rust-idle-builds-total, done-when 1: past the total, the reap drops idle builds oldest built first, a task
    /// worktree's before a slot's, never a claimed one, one a cargo uses, nor one whose git or build moved within the hour, and
    /// stops once under it.
    #[dash_test]
    fn idle_builds_go_oldest_built_first_until_all_total_under_30_gb() {
        let dir = TestDir::new("forge-builds-total");
        let forge = pool_repo(&dir);
        let ago = |secs: u64| std::time::SystemTime::now() - Duration::from_secs(secs);
        let dated = |p: &Path, when| {
            let file = std::fs::File::open(p).expect("there");
            file.set_modified(when).expect("dated");
        };
        let day = 24 * 3600;
        // (folder, its build's age, its git's age): 64 KB each
        let builds = [
            ("rust-claimed", 4 * day, 4 * day),
            ("pool.noindex/pool-1", 5 * day, 5 * day),
            ("rust-old", 3 * day, 3 * day),
            ("rust-building", 2 * day, 2 * day),
            ("rust-late", day, day),
            ("rust-git-moved", 3 * day, 60),
            ("rust-just-built", 600, 3 * day),
        ];
        for (at, built, git) in builds {
            let folder = dir.0.join(at);
            if let Some(id) = at.strip_prefix("rust-") {
                forge.made(id).expect("a worktree");
                let args = ["rev-parse", "--path-format=absolute", "--git-dir"];
                let gd = process::run("git", &args, Some(&folder), GIT).expect("its git");
                for f in ["index", "HEAD", "logs/HEAD"] {
                    let f = Path::new(gd.trim()).join(f);
                    if f.exists() {
                        dated(&f, ago(git));
                    }
                }
            }
            let debug = folder.join("rust/target/debug");
            std::fs::create_dir_all(&debug).expect("a build");
            std::fs::write(debug.join("dash"), vec![1u8; 64 << 10]).expect("a build");
            std::fs::write(debug.join(".cargo-lock"), "").expect("a lock");
            for d in [debug.parent().expect("target"), &debug] {
                dated(d, ago(built));
            }
        }
        let target = |at: &str| dir.0.join(at).join("rust/target");
        let total = || {
            builds
                .map(|(at, ..)| bytes(&target(at), u64::MAX))
                .iter()
                .sum::<u64>()
        };
        // the reap passes BUILDS_CAP (`builds_under(busy, BUILDS_CAP)`); this test passes smaller caps to stay small
        assert_eq!(BUILDS_CAP, 30 << 30, "all builds total under 30 GB");
        let busy = ["claimed".to_string()];
        assert_eq!(
            forge.builds_under(&busy, 1 << 30),
            Vec::<String>::new(),
            "under the cap, all stay"
        );

        let cargo = std::fs::File::open(target("rust-building").join("debug/.cargo-lock"))
            .expect("cargo's lock");
        cargo.lock().expect("a cargo runs");
        // 448 KB held, under 360 KB once two idle tasks' went: the slot, older but leased by any next run, stays
        let said = forge.builds_under(&busy, 360 << 10);
        let names: Vec<_> = said
            .iter()
            .map(|l| l.split(':').next().unwrap_or(""))
            .collect();
        // built 3 days, 2 days, then 1 day ago: oldest built first
        assert_eq!(
            names,
            ["rust-old", "rust-building", "rust-late"],
            "{said:?}"
        );
        assert!(
            total() < 360 << 10,
            "the total is under the cap: {}",
            total()
        );
        assert!(
            said[0].contains("idle build of") && said[0].contains("dropped"),
            "{said:?}"
        );
        assert!(
            said[1].starts_with("rust-building: idle build kept: a cargo runs in"),
            "{said:?}"
        );
        assert!(
            !target("rust-old").exists() && !target("rust-late").exists(),
            "the oldest built idle ones went"
        );
        assert!(target("pool.noindex/pool-1").exists(), "the slot stays");

        // Past it with only the slot idle: it goes; each in use within the hour, the claimed and the cargo's stay, and it says so.
        let said = forge.builds_under(&busy, 0);
        assert_eq!(said.len(), 3, "{said:?}");
        assert!(
            said[0].starts_with("rust-building: idle build kept"),
            "{said:?}"
        );
        assert!(said[1].starts_with("pool-1: idle build of"), "{said:?}");
        assert!(said[2].starts_with("builds still total"), "{said:?}");
        assert!(!target("pool.noindex/pool-1").exists(), "the slot went");
        for kept in [
            "rust-claimed",
            "rust-building",
            "rust-git-moved",
            "rust-just-built",
        ] {
            assert!(target(kept).join("debug/dash").exists(), "{kept} stays");
        }
        drop(cargo);
    }

    /// Task rust-disk-caps, done-when 1 and 2's guard: a build past its cap goes, never while a cargo runs there. The stand-in cargo
    /// holds the lock a real one takes, in a profile's folder (the repository's own checkout) and a target triple's (a task's).
    #[dash_test]
    fn a_build_past_its_cap_goes_but_never_while_a_cargo_runs_there() {
        use std::io::BufRead;
        let dir = TestDir::new("forge-trim");
        let forge = GitHub {
            repo: dir.0.join("repo"),
            root: dir.0.clone(),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        let own = dir.0.join("repo/rust/target");
        let task = dir.0.join("rust-t1/rust/target");
        let locks = [
            own.join("debug/.cargo-build-lock"),
            task.join("x86_64-unknown-linux-musl/release/.cargo-lock"),
        ];
        for lock in &locks {
            std::fs::create_dir_all(lock.parent().expect("its folder")).expect("a build");
            std::fs::write(lock, "").expect("a lock");
            std::fs::write(lock.with_file_name("dash"), vec![1u8; 64 << 10]).expect("a build");
        }
        let cargo = dir.0.join("cargo");
        let flock = "use Fcntl ':flock'; open(my $f, '<', $ARGV[0]) or die; flock($f, LOCK_EX) or die; $| = 1; print \"locked\\n\"; sleep 60;";
        std::fs::write(&cargo, format!("#!/usr/bin/perl\n{flock}\n")).expect("a stand-in cargo");
        std::fs::set_permissions(&cargo, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("runnable");
        let building = |lock: &Path| {
            let mut child = std::process::Command::new(&cargo)
                .arg(lock)
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("a cargo");
            let mut said = String::new();
            let out = child.stdout.take().expect("its output");
            std::io::BufReader::new(out)
                .read_line(&mut said)
                .expect("read");
            assert_eq!(said, "locked\n", "the stand-in holds cargo's lock");
            child
        };

        assert_eq!(
            forge.trim(None, 1 << 30),
            Ok(None),
            "under its cap, it stays"
        );
        let mut running = building(&locks[0]);
        let said = forge.trim(None, 0).expect_err("a cargo runs there");
        assert_eq!(said, format!("a cargo runs in {}", own.display()));
        assert!(locks[0].with_file_name("dash").exists(), "its build stays");
        running.kill().expect("killed");
        running.wait().expect("ended");
        let held = forge.trim(None, 0).expect("cleaned").expect("past its cap");
        assert!(held >= 64 << 10, "the bytes it held: {held}");
        assert!(!own.exists(), "the repository's own build went");

        let mut running = building(&locks[1]);
        assert!(
            forge.trim(Some("t1"), 0).is_err(),
            "a cargo runs under a triple"
        );
        assert!(forge.clean("t1").is_err(), "a close's clean waits too");
        assert!(task.exists());
        running.kill().expect("killed");
        running.wait().expect("ended");
        assert!(forge.trim(Some("t1"), 0).expect("cleaned").is_some());
        assert!(!task.exists(), "the task's build went");
    }

    /// A page whose « Built from » commit holds main's rust/ already is never built again: it stays up for a merge outside rust/.
    /// One that lags, or no page, goes to bin/bounded, which this stand-in checkout lacks, so its failure names it.
    #[dash_test]
    fn a_deploy_builds_only_when_main_s_rust_is_not_live() {
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
        let named = std::sync::Arc::new(std::sync::Mutex::new(live.clone()));
        let page_names = named.clone();
        std::thread::spawn(move || {
            for stream in server.incoming() {
                use std::io::{Read, Write};
                let mut stream = stream.expect("a look");
                let _ = stream.read(&mut [0; 1024]);
                let named = page_names.lock().expect("its commit").clone();
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
        let head = head.trim();
        let said = format!("{live} serves, no build: it holds the rust/ of main's {head}");
        assert_eq!(served, said, "the sha the page names, and main's");
        assert_eq!(
            forge.serving(),
            Ok(Serving::Main(said)),
            "what `dash orch next` says"
        );
        let lock = forge.repo.join("private/live/rust/deploy.lock");
        assert!(lock.exists(), "its folder made");
        let main = commit("rust/main.rs");
        let behind = format!("{live} serves, not the rust/ of main's {main}");
        assert_eq!(forge.serving(), Ok(Serving::Behind(behind)));
        // A commit main lacks, a branch deployed by hand: the look leaves it (/code-review of dc971a27c).
        let branch = "0123456789abcdef0123456789abcdef01234567";
        *named.lock().expect("its commit") = branch.into();
        let other = format!("{branch} serves, a commit main does not hold; main is at {main}");
        assert_eq!(forge.serving(), Ok(Serving::Other(other)));
        let e = forge.deploy().expect_err("a build");
        assert!(e.contains("bin/bounded"), "{e}");
        forge.page = "http://127.0.0.1:9/".into();
        let e = forge.deploy().expect_err("no page, a build");
        assert!(e.contains("bin/bounded"), "{e}");
        // The monitors saved only when the commit live changes them since the last save; the live binary a script that says so.
        let rust = forge.repo.join("private/live/rust");
        let saves = rust.join("saves");
        let script = format!(
            "#!/bin/sh\necho saved >> '{}'\necho 'monitor 1: OK'\n",
            saves.display()
        );
        std::fs::write(rust.join("dash"), script).expect("the live dash");
        use std::os::unix::fs::PermissionsExt;
        let runs = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(rust.join("dash"), runs).expect("chmod");
        std::fs::write(rust.join("commit"), &main).expect("live");
        let saved = forge.save_monitors();
        assert_eq!(
            saved,
            Ok(Some("monitor 1: OK\n".into())),
            "no mark yet: saved"
        );
        assert_eq!(forge.save_monitors(), Ok(None), "saved at this commit");
        std::fs::create_dir_all(origin.join("rust/crates/dash/src")).expect("dirs");
        let changed = commit(MONITORS_DATA);
        forge.serving().expect("main fetched");
        assert_eq!(forge.save_monitors(), Ok(None), "main moved, not live yet");
        std::fs::write(rust.join("commit"), &changed).expect("live");
        assert!(
            forge.save_monitors().is_ok_and(|s| s.is_some()),
            "live: saved"
        );
        let read = std::fs::read_to_string(&saves).expect("its saves");
        assert_eq!(read, "saved\nsaved\n", "twice");
    }

    /// A met PR behind main: main merged into its branch in its worktree and pushed, that worktree's head the pushed one, so `pr`
    /// still finds it. A merge a file it did not commit stops is undone, that file kept.
    #[dash_test]
    fn an_update_merges_main_into_the_task_branch_where_its_session_works_and_pushes() {
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
    #[dash_test]
    fn a_pr_stacked_on_a_squash_merged_branch_is_rebased_onto_main() {
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
