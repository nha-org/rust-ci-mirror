//! `dash projects follow <name>`: his checkout of a project follows its forge's main (task nanolathe-checkout-current; @nha,
//! 2026-10-30: « better as a fork + sub-project … keep in sync »). bin/comment-wake runs it as a host job every 5 minutes
//! from ~/repo/<name> (the app's sandbox cannot write another repo's checkout), so its record on /status is green after a
//! fast-forward and red, with the line below as its detail, when the checkout is dirty, off main or ahead of origin.
use dash_process as process;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAIN: &str = "main";
/// Each git call's limit, a fetch's too: an ssh waiting on a prompt under launchd is cut, never held to the job's 20 minutes.
const LIMIT: Duration = Duration::from_secs(120);

/// One git call in `repo`, its stdout trimmed; a fetch is `fetch_healed`'s (a lost ref lock, a ref to a missing object).
fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    match args.first() {
        Some(&"fetch") => crate::fetch_healed(repo, args, LIMIT),
        _ => process::run("git", args, Some(repo), LIMIT),
    }
    .map(|s| s.trim().to_string())
}

/// Fetches origin's main and fast-forwards `repo` to it when it is clean and on main; the line to print, or why not.
/// Untracked files do not count as dirty: a fast-forward never touches them, and git refuses one it would overwrite.
pub fn follow(repo: &Path) -> Result<String, String> {
    if !repo.join(".git").exists() {
        return Err(format!("not followed: no checkout at {}", repo.display()));
    }
    // a broken repository or a missing git says so here, never as a detached HEAD below (/code-review)
    let before = git(repo, &["rev-parse", "--verify", "HEAD"])?;
    // « HEAD » when detached; a git that fails says so, never read as detached (the second review of PR 1617)
    let branch = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch != MAIN {
        let on = match branch == "HEAD" {
            true => "a detached HEAD".into(),
            false => format!("branch {branch}"),
        };
        return Err(format!(
            "not followed: {} is on {on}, not {MAIN}",
            repo.display()
        ));
    }
    // no index refresh written: a background status every 5 minutes never holds index.lock under his own git
    let status = [
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
    ];
    let dirty = git(repo, &status)?;
    if !dirty.is_empty() {
        let n = dirty.lines().count();
        let first: Vec<_> = dirty.lines().take(3).map(str::trim).collect();
        return Err(format!(
            "not followed: {} has {n} uncommitted change{}: {}",
            repo.display(),
            if n == 1 { "" } else { "s" },
            first.join(", ")
        ));
    }
    // the remote ref named: a clone with its own refspec would leave origin/main where it was (/code-review)
    let theirs_ref = format!("refs/remotes/origin/{MAIN}");
    git(
        repo,
        &[
            "fetch",
            "-q",
            "origin",
            &format!("+refs/heads/{MAIN}:{theirs_ref}"),
        ],
    )?;
    let theirs = git(repo, &["rev-parse", "--verify", &theirs_ref])?;
    if before == theirs {
        return Ok(format!(
            "{}: at origin/{MAIN} {}",
            repo.display(),
            &before[..12]
        ));
    }
    // ahead or diverged: ff-only would say « Already up to date » for an ahead main and exit 0 (/code-review)
    // counted, so a git that fails is its own error, never read as « ahead »
    let ahead = git(
        repo,
        &["rev-list", "--count", &format!("{theirs_ref}..HEAD")],
    )?;
    if ahead != "0" {
        return Err(format!(
            "not followed: {}'s {MAIN} has commits origin/{MAIN} lacks ({ahead})",
            repo.display()
        ));
    }
    // behind but refused: an untracked file the merge would overwrite, named in git's words
    git(repo, &["merge", "-q", "--ff-only", &theirs_ref]).map_err(|e| {
        format!(
            "not followed: {} refused the fast-forward ({e})",
            repo.display()
        )
    })?;
    Ok(format!(
        "{}: fast-forwarded {} to origin/{MAIN} {}",
        repo.display(),
        &before[..12],
        &theirs[..12]
    ))
}

fn project_name(name: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    !name.is_empty() && name.chars().all(word)
}

/// `dash projects`: `follow <name>` here, in his home folder (exit 0 followed, 1 not, the reason on stderr); the rest is core's
/// table.
pub fn projects(words: &[&str], mut out: impl std::io::Write) -> i32 {
    let ["follow", name] = words else {
        return dash_core::projects_command(words, out);
    };
    let said = match std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        // a project's name, never a path out of ~/repo
        _ if !project_name(name) => Err(format!("not followed: {name:?} is not a project's name")),
        Some(home) => follow(&PathBuf::from(home).join(dash_core::project_folder(name))),
        None => Err("not followed: HOME is unset, so no checkout to find".into()),
    };
    match said {
        Ok(line) => writeln!(out, "{line}").map_or(1, |()| 0),
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    fn run(dir: &Path, args: &[&str]) {
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            st.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&st.stderr)
        );
    }

    /// An origin, his clone of it and a fork session's clone that pushes, each with one commit on main.
    fn repos(dir: &TestDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let (origin, his, fork) = (
            dir.0.join("origin.git"),
            dir.0.join("his"),
            dir.0.join("fork"),
        );
        run(
            &dir.0,
            &["init", "-q", "--bare", "-b", MAIN, origin.to_str().unwrap()],
        );
        for c in [&his, &fork] {
            run(
                &dir.0,
                &["clone", "-q", origin.to_str().unwrap(), c.to_str().unwrap()],
            );
            run(c, &["config", "user.email", "t@t"]);
            run(c, &["config", "user.name", "t"]);
            run(c, &["config", "commit.gpgsign", "false"]);
            run(c, &["checkout", "-q", "-B", MAIN]);
        }
        std::fs::write(fork.join("a"), "1").unwrap();
        run(&fork, &["add", "a"]);
        run(&fork, &["commit", "-q", "-m", "one"]);
        run(&fork, &["push", "-q", "origin", MAIN]);
        run(&his, &["pull", "-q", "origin", MAIN]);
        (his, fork)
    }

    fn merged(fork: &Path, n: &str) {
        std::fs::write(fork.join("a"), n).unwrap();
        run(fork, &["commit", "-q", "-am", n]);
        run(fork, &["push", "-q", "origin", MAIN]);
    }

    fn head(repo: &Path, r: &str) -> String {
        git(repo, &["rev-parse", r]).unwrap()
    }

    /// Done-when 1: after a merge on the fork, a clean checkout on main is fast-forwarded to origin's main.
    #[dash_test]
    fn a_clean_checkout_on_main_follows_a_merge() {
        let dir = TestDir::new("follow-clean");
        let (his, fork) = repos(&dir);
        merged(&fork, "2");
        std::fs::write(his.join("untracked"), "x").unwrap();
        let line = follow(&his).expect("followed");
        assert!(line.contains("fast-forwarded"), "{line}");
        assert_eq!(head(&his, "HEAD"), head(&fork, "HEAD"));
        assert!(follow(&his).expect("again").contains("at origin/main"));
    }

    /// Done-when 1: a dirty checkout, one off main, or one ahead of origin stays as it is, and the error says why.
    #[dash_test]
    fn a_dirty_off_main_or_diverged_checkout_stays_and_says_why() {
        let dir = TestDir::new("follow-refused");
        let (his, fork) = repos(&dir);
        merged(&fork, "2");
        let at = head(&his, "HEAD");

        std::fs::write(his.join("a"), "his edit").unwrap();
        let why = follow(&his).unwrap_err();
        assert!(why.contains("1 uncommitted change: M a"), "{why}");
        run(&his, &["checkout", "-q", "--", "a"]);

        run(&his, &["checkout", "-q", "-b", "play"]);
        let why = follow(&his).unwrap_err();
        assert!(why.contains("on branch play, not main"), "{why}");
        run(&his, &["checkout", "-q", MAIN]);

        std::fs::write(his.join("b"), "his").unwrap();
        run(&his, &["add", "b"]);
        run(&his, &["commit", "-q", "-m", "his own"]);
        let why = follow(&his).unwrap_err();
        assert!(why.contains("has commits origin/main lacks"), "{why}");
        assert_ne!(head(&his, "HEAD"), head(&fork, "HEAD"));
        assert_eq!(head(&his, "HEAD^"), at, "his main was left where it was");

        run(&his, &["reset", "-q", "--hard", &at]);
        std::fs::write(fork.join("c"), "theirs").unwrap();
        run(&fork, &["add", "c"]);
        run(&fork, &["commit", "-q", "-m", "c"]);
        run(&fork, &["push", "-q", "origin", MAIN]);
        std::fs::write(his.join("c"), "his untracked").unwrap();
        let why = follow(&his).unwrap_err();
        assert!(
            why.contains("refused the fast-forward") && why.contains("untracked"),
            "{why}"
        );
        assert_eq!(head(&his, "HEAD"), at);

        run(&his, &["reset", "-q", "--hard", &head(&fork, "HEAD")]);
        run(&his, &["rm", "-q", "--cached", "c"]);
        std::fs::write(his.join("d"), "ahead").unwrap();
        run(&his, &["add", "d"]);
        run(&his, &["commit", "-q", "-m", "ahead only"]);
        let why = follow(&his).unwrap_err();
        assert!(
            why.contains("has commits origin/main lacks"),
            "an ahead main is no follow: {why}"
        );

        let why = follow(&dir.0.join("none")).unwrap_err();
        assert!(why.contains("no checkout at"), "{why}");

        run(&his, &["checkout", "-q", "--detach"]);
        let why = follow(&his).unwrap_err();
        assert!(why.contains("on a detached HEAD, not main"), "{why}");
        assert!(project_name("nanolathe") && project_name("site_perso"));
        assert!(!project_name("../x") && !project_name("a/b") && !project_name(""));
        assert_eq!(projects(&["follow", "../x"], vec![]), 1);
    }
}
