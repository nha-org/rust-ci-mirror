//! The main checkout brought up to its base once a PR merges (task rust-land-pulls-main): his superapp window is built from
//! that checkout's dash/, so a checkout left behind origin kept a build without what had just landed, and on 2026-11-20 he
//! looked for #1855's Connect Chrome button where it was not. Run by the forge's merge itself (`GitHub::merge`), so by `dash
//! land`, the stall watcher's lands and a run's own merge alike, as soon as GitHub merged and whatever the deploy then does.
//! The fast-forward is `follow`'s, his other checkouts' (`crate::follow`), with his uncommitted edits left where they are:
//! never a reset, a stash, a checkout on another branch or a merge commit. What it could not do it says.
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The lookup's bound: one local git read.
const GIT: Duration = Duration::from_secs(60);

/// The main checkout of the repository `any` is a checkout of: the folder holding its common git dir, so a merge run from
/// a worktree pulls the main one.
fn main_of(any: &Path) -> Result<PathBuf, String> {
    let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
    let common = dash_process::run("git", &args, Some(any), GIT)?;
    let common = PathBuf::from(common.trim());
    match common.file_name().is_some_and(|n| n == ".git") {
        true => Ok(common.parent().map(Path::to_path_buf).unwrap_or(common)),
        false => Err(format!(
            "{} is a bare repository, no checkout",
            common.display()
        )),
    }
}

/// The main checkout of `any`'s repository fast-forwarded to `origin/<base>`, as one line to say: done, already there, or
/// why not, in `follow`'s words.
pub fn fast_forward(any: &Path, base: &str) -> String {
    let said = main_of(any)
        .map_err(|e| format!("was not found, not followed: {e}"))
        .and_then(|top| crate::follow::follow_with_edits(&top, base))
        .unwrap_or_else(|why| why);
    // one line, git's own several joined
    let said: Vec<&str> = said.split_whitespace().collect();
    format!("land: the main checkout {}", said.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    fn run(dir: &Path, args: &[&str]) -> String {
        let mut all = vec!["-c", "user.name=t", "-c", "user.email=t@t"];
        all.extend_from_slice(args);
        dash_process::run("git", &all, Some(dir), GIT)
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"))
    }

    /// Done-when of rust-land-pulls-main, on scratch repositories: an origin, the main checkout behind it with an uncommitted
    /// edit, a worktree, and a second clone that lands a commit. Run from the worktree, it fast-forwards the main checkout,
    /// keeping the edit (the forge's merge runs it: `a_merge_fast_forwards_the_main_checkout`); a landing on the file he
    /// edits moves nothing and stashes nothing; a checkout on another branch, or with a commit origin lacks, is left as it is and said so.
    #[dash_test]
    fn a_landing_fast_forwards_the_main_checkout_and_says_when_it_cannot() {
        let dir = TestDir::new("land-pulls-main");
        let origin = dir.0.join("origin.git");
        let (main, other) = (dir.0.join("main"), dir.0.join("other"));
        let at = |path: &Path| path.to_string_lossy().into_owned();
        run(
            &dir.0,
            &["init", "-q", "--bare", "-b", "main", &at(&origin)],
        );
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&main)]);
        std::fs::write(main.join("a.txt"), "one\n").expect("a");
        std::fs::write(main.join("b.txt"), "one\n").expect("b");
        run(&main, &["add", "-A"]);
        run(&main, &["commit", "-qm", "base"]);
        run(&main, &["push", "-q", "origin", "main"]);
        run(&main, &["worktree", "add", "-q", "-b", "task/x", "../task"]);
        // an identity, as his checkout has: a merge commit could be made, so only --ff-only refuses one. And his git set to
        // autostash: no edit of his is stashed all the same
        run(&main, &["config", "user.name", "t"]);
        run(&main, &["config", "user.email", "t@t"]);
        run(&main, &["config", "merge.autostash", "true"]);
        run(&dir.0, &["clone", "-q", &at(&origin), &at(&other)]);
        std::fs::write(other.join("a.txt"), "landed\n").expect("a");
        run(&other, &["commit", "-qam", "landed"]);
        run(&other, &["push", "-q", "origin", "main"]);
        let landed = run(&other, &["rev-parse", "HEAD"]);
        // his edit in the main checkout, on a file the landing did not touch
        std::fs::write(main.join("b.txt"), "his edit\n").expect("b");

        let said = fast_forward(&dir.0.join("task"), "main");
        assert!(
            said.starts_with("land: the main checkout ")
                && said.contains(": fast-forwarded ")
                && said.ends_with(&landed[..12]),
            "{said}"
        );
        assert_eq!(run(&main, &["rev-parse", "HEAD"]), landed);
        assert_eq!(
            std::fs::read_to_string(main.join("a.txt")).expect("a"),
            "landed\n"
        );
        assert_eq!(
            std::fs::read_to_string(main.join("b.txt")).expect("b"),
            "his edit\n",
            "his edit kept"
        );
        assert_eq!(run(&main, &["status", "--porcelain"]), " M b.txt\n");
        let said = fast_forward(&dir.0.join("task"), "main");
        assert!(
            said.starts_with("land: the main checkout ")
                && said.ends_with(&format!(": at origin/main {}", &landed[..12])),
            "{said}"
        );

        // a landing that changes the file he is editing: nothing moves, his edit stays as he left it, unstaged, with no
        // conflict marker and no stash, and the one line names the file
        std::fs::write(main.join("staged.txt"), "his staged file\n").expect("staged");
        run(&main, &["add", "staged.txt"]);
        std::fs::write(other.join("b.txt"), "landed\n").expect("b");
        run(&other, &["commit", "-qam", "landed on his file"]);
        run(&other, &["push", "-q", "origin", "main"]);
        let said = fast_forward(&main, "main");
        assert!(
            said.contains("refused the fast-forward") && said.contains("b.txt"),
            "{said}"
        );
        assert_eq!(said.lines().count(), 1, "one line: {said}");
        assert_eq!(run(&main, &["rev-parse", "HEAD"]), landed);
        assert_eq!(
            std::fs::read_to_string(main.join("b.txt")).expect("b"),
            "his edit\n"
        );
        assert_eq!(
            run(&main, &["status", "--porcelain"]),
            " M b.txt\nA  staged.txt\n",
            "his edit unstaged, his staged file staged"
        );
        assert_eq!(run(&main, &["stash", "list"]), "", "no stash");
        // his edit put aside by him: the same landing now fast-forwards, his staged file kept
        std::fs::write(main.join("b.txt"), "one\n").expect("b");
        let said = fast_forward(&main, "main");
        assert!(said.contains(": fast-forwarded "), "{said}");
        assert_eq!(run(&main, &["status", "--porcelain"]), "A  staged.txt\n");
        std::fs::write(main.join("b.txt"), "his edit\n").expect("b");

        run(&main, &["checkout", "-q", "-b", "elsewhere"]);
        let said = fast_forward(&main, "main");
        assert!(said.ends_with("is on branch elsewhere, not main"), "{said}");
        run(&main, &["checkout", "-q", "main"]);

        // a commit of the main checkout's own that origin lacks, beside a newer landing: no merge commit, no reset
        run(&main, &["commit", "-qam", "his own commit"]);
        let own = run(&main, &["rev-parse", "HEAD"]);
        std::fs::write(other.join("a.txt"), "landed again\n").expect("a");
        run(&other, &["commit", "-qam", "landed again"]);
        run(&other, &["push", "-q", "origin", "main"]);
        let said = fast_forward(&main, "main");
        assert!(
            said.ends_with("main has commits origin/main lacks (1)"),
            "{said}"
        );
        assert_eq!(run(&main, &["rev-parse", "HEAD"]), own, "left where it was");
    }
}
