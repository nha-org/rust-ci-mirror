//! `dash land`'s run of dash-gate's tests on a PR merged with main, before its merge (task rust-land-gates-merged; @nha,
//! 2026-11-13: « Each PR alone was green… seems bad design. »): #1578's port-map rule and #1563's surfaces.rs were green alone,
//! red together. The tree GitHub's merge makes, written by `merge-tree`, is checked out detached in one worktree beside the
//! tasks' (`land-gate`), whose build stays warm from land to land, and tested there on this Mac: no CI job is added.
use crate::{GATE_TESTS_LIMIT as TESTS, GIT, GitHub};
use dash_core::tasks::{MERGED_RED, MergedGate};
use dash_process as process;
use std::time::{Duration, Instant};

/// dash-gate's tests, as the pre-commit's rust-gate step runs them.
pub const GATE_TESTS: [&str; 6] = ["cargo", "nextest", "run", "--locked", "-p", "dash-gate"];
/// The PRs named as crossing it, newest first.
const CROSSES: usize = 5;

impl GitHub {
    /// PR `pr` (`dash land`'s word: a number, `#n`, its URL or its branch) at `head` merged with main, and `tests`
    /// (`GATE_TESTS`; a stand-in in a test) run on that tree in its rust/: passed, with its time; red, naming each failing
    /// test and the PRs main took since the head's base; or the files the merge conflicts in. A head that holds main is the
    /// tree its CI tested, passed with no run. Err: not run (a fetch, a checkout, another land's run past `TESTS`, its own
    /// run past `TESTS`). One land at a time holds the worktree; its build is removed past `SLOT_CAP`.
    pub fn gate_merged(&self, pr: &str, head: &str, tests: &[&str]) -> Result<MergedGate, String> {
        // by its sha: a branch name, which gh and `dash land` take, has no pull/<n>/head to fetch (/code-review)
        self.git(&["fetch", "-q", "origin", &self.base, head])?;
        let main = self.git(&["rev-parse", &format!("origin/{}", self.base)])?;
        let main = main.trim();
        let short = &main[..main.len().min(9)];
        let on = format!("{} merged with main at {short}", named(pr));
        let ancestor = ["merge-base", "--is-ancestor", main, head];
        let (holds, _, _) =
            process::answer("git", &ancestor, Some(&self.repo), &[], GIT).map_err(String::from)?;
        if holds.success() {
            return Ok(MergedGate::Passed(format!(
                "dash-gate not run again on {on}: its head holds main, the tree its checks tested"
            )));
        }
        let tree = match self.merged_tree(main, head)? {
            Ok(tree) => tree,
            Err(files) => return Ok(MergedGate::Conflicts(files)),
        };
        let message = format!("dash land: {} merged with main", named(pr));
        let merged = self.commit_tree(&tree, &[main, head], &message, "dash land")?;
        let at = self.root.join("land-gate");
        let fault = |e: std::io::Error| format!("{}: {e}", at.display());
        // the lock is the open file's: the OS ends it with this process, a crash included
        std::fs::create_dir_all(&self.root).map_err(fault)?;
        let lock = std::fs::File::create(self.root.join("land-gate.lock")).map_err(fault)?;
        let waited = Instant::now();
        while let Err(e) = lock.try_lock() {
            if !matches!(e, std::fs::TryLockError::WouldBlock) || waited.elapsed() > TESTS {
                return Err(format!(
                    "{}: another land's run holds it: {e}",
                    at.display()
                ));
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        if !at.join(".git").exists() {
            // a folder a crashed `worktree add` left, or a build with no checkout: `worktree add` refuses one that exists
            if at.exists() {
                std::fs::remove_dir_all(&at).map_err(fault)?;
            }
            self.git(&["worktree", "prune"])?;
            let at = at.to_string_lossy();
            self.git(&["worktree", "add", "-q", "--detach", &at, &merged])?;
        } else {
            let args = ["checkout", "-q", "--detach", "--force", &merged];
            process::run("git", &args, Some(&at), GIT)?;
        }
        let (bin, args) = tests.split_first().ok_or("no test command")?;
        let start = Instant::now();
        // its own process group: past its limit, rustc and the tests end with cargo, never left writing the next land's target
        let ran = process::answer_grouped(bin, args, Some(&at.join("rust")), TESTS);
        let took = start.elapsed().as_secs();
        // outside every task's cap (rust-disk-caps): held to a slot's, the next run is cold once past it
        let target = at.join("rust/target");
        if crate::bytes(&target, crate::SLOT_CAP) > crate::SLOT_CAP {
            let _ = crate::remove_build(&target);
        }
        let (status, out, err) = ran.map_err(String::from)?;
        if status.success() {
            return Ok(MergedGate::Passed(format!(
                "dash-gate passed on {on}, in {took} s"
            )));
        }
        let failed = failing(&format!("{out}\n{err}"));
        // the run happened: a log not read leaves the red standing, its PRs unnamed
        let crossed = match self.git(&["log", "--format=%s", &format!("{head}..{main}")]) {
            Ok(log) => crosses(&log),
            Err(e) => format!("the PRs main took since its base, unread: {e}"),
        };
        Ok(MergedGate::Red(format!(
            "{MERGED_RED}{on}, in {took} s: {failed}; it crosses {crossed}"
        )))
    }
}

/// `dash land`'s word as a PR: `#n` from a number, `#n` or its URL, else the word (a branch).
fn named(pr: &str) -> String {
    let n = match pr.split_once("/pull/") {
        Some((_, rest)) => rest.split('/').next().unwrap_or(rest),
        None => pr.trim_start_matches('#'),
    };
    match n.parse::<u64>() {
        Ok(n) => format!("#{n}"),
        Err(_) => pr.to_string(),
    }
}

/// The tests nextest names failed (FAIL, TIMEOUT or a signal, each once), else the build's first error: a merge that does
/// not compile is red too.
fn failing(out: &str) -> String {
    let mut names: Vec<&str> = vec![];
    for line in out.lines().map(str::trim_start) {
        let ends = ["FAIL [", "TIMEOUT [", "SIGSEGV [", "SIGABRT [", "SIGKILL ["];
        let Some(rest) = ends.iter().find_map(|w| line.strip_prefix(w)) else {
            continue;
        };
        let Some((_, name)) = rest.split_once("] ") else {
            continue;
        };
        // nextest's « (3/120) » before the name
        let name = match name.strip_prefix('(') {
            Some(n) => n.split_once(") ").map_or(name, |(_, n)| n),
            None => name,
        };
        if !names.contains(&name.trim()) {
            names.push(name.trim());
        }
    }
    if names.is_empty() {
        let error = out.lines().find(|l| l.starts_with("error"));
        return format!(
            "its build: {}",
            error.unwrap_or("no test named in its output").trim()
        );
    }
    names.join(", ")
}

/// The squash-merged PRs in these subjects of main's commits since the head's base, each `(#n)` at a subject's end.
fn crosses(log: &str) -> String {
    let pr = |s: &str| {
        let (_, n) = s.trim_end().strip_suffix(')')?.rsplit_once("(#")?;
        n.parse::<u64>().ok().map(|n| format!("#{n}"))
    };
    let prs: Vec<String> = log.lines().filter_map(pr).collect();
    match prs.len() {
        0 => "no PR: main took none since its base".into(),
        n if n <= CROSSES => prs.join(", "),
        n => format!("{} and {} more", prs[..CROSSES].join(", "), n - CROSSES),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};
    use std::path::Path;

    /// Done-when 1 and 2, the crossing pair of #1578 and #1563 in small: a rule on main and a file in the PR, each green alone,
    /// red together. The PR merged with main before the rule passes, its time said; once main takes the rule, the same head
    /// merged with it is red, naming the failing test and the PRs it crosses. A head holding main runs nothing again; one that
    /// conflicts with main is no run but its files. A folder a crashed checkout left is replaced, never a land refused on it.
    #[dash_test]
    fn a_pr_green_alone_is_red_merged_with_main_naming_the_test_and_the_pr_it_crosses() {
        let dir = TestDir::new("forge-merged-gate");
        let (origin, repo) = (dir.0.join("origin"), dir.0.join("repo"));
        std::fs::create_dir_all(origin.join("rust")).expect("dirs");
        let run = |at: &Path, args: &[&str]| process::run("git", args, Some(at), GIT).expect("git");
        let who = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = |file: &str, text: &str, message: &str| {
            std::fs::write(origin.join(file), text).expect("write");
            run(&origin, &["add", file]);
            run(
                &origin,
                &[&who[..], &["commit", "-q", "-m", message]].concat(),
            );
            run(&origin, &["rev-parse", "HEAD"]).trim().to_string()
        };
        // a PR's head at GitHub's refs/pull/<n>/head, main left where it was
        let pull = |n: u64, file: &str, text: &str| {
            run(&origin, &["switch", "-q", "--detach", "main"]);
            let head = commit(file, text, file);
            run(
                &origin,
                &["update-ref", &format!("refs/pull/{n}/head"), &head],
            );
            run(&origin, &["switch", "-q", "main"]);
            head
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        // GitHub serves any commit by its sha
        run(
            &origin,
            &["config", "uploadpack.allowAnySHA1InWant", "true"],
        );
        commit("rust/lib.rs", "1\n", "base");
        let at = |p: &Path| p.to_str().expect("path").to_string();
        let url = format!("file://{}", at(&origin));
        run(&dir.0, &["clone", "-q", &url, &at(&repo)]);
        let forge = GitHub {
            repo: repo.clone(),
            root: dir.0.join("worktrees"),
            base: "main".into(),
            page: "http://127.0.0.1:9/".into(),
            cli: "gh".into(),
        };
        // dash-gate in small: the rule fails once the file it reads is there too
        let tests = dash_meter::stand_in(
            "#!/bin/sh\nif [ -e rule ] && [ -e surfaces.rs ]; then\n  echo '        FAIL [   0.010s] (1/1) dash-gate::workspace_rules port_map_names_each_module' >&2\n  exit 100\nfi\n",
        );
        let tests = [tests.to_str().expect("path")];
        let pr = pull(7, "rust/surfaces.rs", "pub mod surfaces;\n");
        commit("rust/a.rs", "a\n", "[feat][a] another (#1570)");
        let leftover = dir.0.join("worktrees/land-gate/rust/target");
        std::fs::create_dir_all(&leftover).expect("a crashed checkout's folder");
        let alone = forge.gate_merged("7", &pr, &tests).expect("run");
        let MergedGate::Passed(words) = alone else {
            panic!("green alone: {alone:?}")
        };
        assert!(
            words.starts_with("dash-gate passed on #7 merged with main at "),
            "{words}"
        );
        assert!(words.ends_with(" s"), "its time: {words}");
        commit(
            "rust/rule",
            "every module named\n",
            "[feat][rust-port-map] a port-map rule (#1578)",
        );
        commit("rust/lib.rs", "2\n", "[fix][other] another (#1580)");
        let main = run(&origin, &["rev-parse", "HEAD"]);
        let crossed = forge
            .gate_merged("https://github.com/o/r/pull/7", &pr, &tests)
            .expect("run");
        let MergedGate::Red(words) = crossed else {
            panic!("red merged: {crossed:?}")
        };
        let at_main = format!("{MERGED_RED}#7 merged with main at {}, in ", &main[..9]);
        assert!(words.starts_with(&at_main), "{words}");
        assert!(
            words.ends_with(
                ": dash-gate::workspace_rules port_map_names_each_module; it crosses #1580, #1578, #1570"
            ),
            "{words}"
        );
        // a head holding main is the tree its checks tested: nothing runs again
        let other = pull(8, "rust/other.rs", "x\n");
        let holds = forge
            .gate_merged("task/other", &other, &tests)
            .expect("read");
        let not_run = format!(
            "dash-gate not run again on task/other merged with main at {}: its head holds main, the tree its checks tested",
            &main[..9]
        );
        assert_eq!(holds, MergedGate::Passed(not_run));
        run(&origin, &["switch", "-q", "--detach", "main~2"]);
        let clash = commit("rust/lib.rs", "3\n", "clash");
        run(&origin, &["update-ref", "refs/pull/9/head", &clash]);
        run(&origin, &["switch", "-q", "main"]);
        let conflicts = forge.gate_merged("9", &clash, &tests).expect("run");
        assert_eq!(conflicts, MergedGate::Conflicts(vec!["rust/lib.rs".into()]));
    }

    #[dash_test]
    fn nextest_s_failures_are_named_once_else_the_build_s_error() {
        let out = "        PASS [   0.010s] (1/3) dash-gate::workspace_rules fine\n        FAIL [   0.020s] (2/3) dash-gate::workspace_rules port_map\n------------\n     Summary [   0.1s] 3 tests run: 2 passed, 1 failed\n        FAIL [   0.020s] (2/3) dash-gate::workspace_rules port_map\n     TIMEOUT [  60.000s] dash-gate binary::slow\n";
        assert_eq!(
            failing(out),
            "dash-gate::workspace_rules port_map, dash-gate binary::slow"
        );
        let build = "   Compiling dash-gate\nerror[E0425]: cannot find value `x` in this scope\nerror: could not compile\n";
        assert_eq!(
            failing(build),
            "its build: error[E0425]: cannot find value `x` in this scope"
        );
    }

    #[dash_test]
    fn the_prs_crossed_are_main_s_squash_merges_newest_first_five_at_most() {
        let log = "a (#7)\nb, no PR\nc (#6)\n";
        assert_eq!(crosses(log), "#7, #6");
        let many: String = (1..=7).rev().map(|n| format!("x (#{n})\n")).collect();
        assert_eq!(crosses(&many), "#7, #6, #5, #4, #3 and 2 more");
        assert_eq!(crosses(""), "no PR: main took none since its base");
    }
}
