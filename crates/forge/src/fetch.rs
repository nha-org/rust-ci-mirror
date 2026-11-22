//! A fetch of a checkout that gets past what stops git's own: another fetch's ref lock, and a remote ref at an object the
//! repository lacks. Below every module that fetches (the forge, `follow`), and using none of them.
use dash_process as process;
use std::path::Path;
use std::time::Duration;

/// A local git read's bound.
const GIT: Duration = Duration::from_secs(300);

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

#[cfg(test)]
mod tests {
    use super::*;
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
}
