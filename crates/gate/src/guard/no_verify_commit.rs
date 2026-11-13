//! Refuses `git commit -n` / `--no-verify` (and `--no-verify` on push or merge, a core.hooksPath override, a typed DASH_SHIP=).
//!
//! approval-first-tap committed with --no-verify and PR 801 went red on a test its task never ran (task
//! push-runs-touched-tests; @nha, 2026-10-29: « This should be second nature »). A hook that fails names a check, and a check
//! that fails is a test to fix, so the refusal names it. `git push -n` is a dry run and stays allowed. Was
//! .claude/hooks/no-verify-commit.py; its selftest cases are the tests below.
use super::shell::{LEAD, code, segments};
use regex::Regex;
use std::sync::LazyLock;

/// git, its global options (-c k=v, -C dir, --flag), then the subcommand and the rest of the segment
static GIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?s){LEAD}(?:\S*/)?git((?:\s+(?:-[cC]\s+\S+|-\S+))*)\s+([a-z-]+)(.*)$"
    ))
    .expect("git")
});
/// the assignment where a command word stands, so a grep or an echo of it goes; export too
static SHIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{LEAD}(?:export\s+(?:\w+=\S*\s+)*)?DASH_SHIP=[^\s;&|)]"
    ))
    .expect("ship")
});
static READS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"--(get|list|unset)|\s-l\b").expect("reads"));
/// git takes a unique prefix of a long option: --no-verif, --no-ve
static NO_VERIFY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^--no-ve(r(i(fy?)?)?)?$").expect("no-verify"));
/// commit's -n alone or bundled (-an, -nm); a value after -m/-F/-C/-c is blanked or not a flag
static SHORT_N: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-[a-zA-Z]*n[a-zA-Z]*$").expect("-n"));

const MESSAGE: &str = "Refused by dash hook no-verify-commit: {what} skips the repository's hooks. Fix the failing check instead: run the test the hook names (`cargo nextest run -p <crate>` for Rust), make it pass, then commit again. A hook that cannot run is a bug to fix in the hook, never a flag.";
const SHIP_MESSAGE: &str = "Refused by dash hook no-verify-commit: DASH_SHIP is git-ship's word to pre-push, never typed. A worktree pushes its own branch and opens a PR; main moves by a merged PR or dash ship.";

pub fn refused(cmd: &str) -> Option<String> {
    let segs = segments(&code(cmd));
    // git-ship's word to pre-push alone: typed, it would let a worktree's push reach main (task audit-guards, /code-review).
    // A value is required; a quoted form passes, the same ceiling as bash -c 'git commit -n' here
    if segs
        .iter()
        .any(|seg| seg.split("$(").any(|part| SHIP.is_match(part)))
    {
        return Some(SHIP_MESSAGE.into());
    }
    for seg in &segs {
        let Some(m) = GIT.captures(seg) else { continue };
        let (globals, sub, rest) = (&m[1], &m[2], &m[3]);
        let words: Vec<&str> = rest.split_whitespace().collect();
        let skips = matches!(sub, "commit" | "push" | "merge");
        // config keys are case-insensitive, and a persistent `git config core.hooksPath` skips every later commit
        let what = if skips && globals.to_lowercase().contains("core.hookspath") {
            format!("git {sub} with core.hooksPath overridden")
        } else if sub == "config"
            && rest.to_lowercase().contains("core.hookspath")
            && !READS.is_match(rest)
        {
            "git config core.hooksPath".into()
        } else if skips && words.iter().any(|w| NO_VERIFY.is_match(w)) {
            format!("git {sub} --no-verify")
        } else if sub == "commit"
            && words
                .iter()
                .any(|w| !w.starts_with("--") && SHORT_N.is_match(w))
        {
            "git commit -n".into()
        } else {
            continue;
        };
        return Some(MESSAGE.replace("{what}", &what));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// no-verify-commit.py's selftest, its cases as they were, plus the heredoc it let through
    #[dash_test]
    fn refuses_what_skips_the_hooks_and_passes_the_rest() {
        for bad in [
            "git commit --no-verify -m x",
            "git commit -n -m x",
            "git commit -an -m x",
            "cd x && git commit -nm 'x'",
            "git -C /tmp commit --no-verify",
            "git push --no-verify",
            "git -c core.hooksPath=/dev/null commit -m x",
            "git commit -m x --no-verify",
            "true;git commit -n",
            "git merge --no-verify main",
            "git commit --no-verif -m x",
            "git commit --no-ve -m x",
            "git -c core.hookspath=/dev/null commit -m x",
            "git config core.hooksPath /dev/null",
            "git config --local core.HooksPath x",
            "DASH_SHIP=1 git push origin HEAD:main",
            "export DASH_SHIP=1",
            "env DASH_SHIP=1 git push",
            "true; DASH_SHIP=1 git push",
            "export A=1 DASH_SHIP=1",
            "A=1 DASH_SHIP=1 git push",
            "x=$(DASH_SHIP=1 git push)",
            // bash ends a heredoc at its first delimiter line: the commit after it runs
            "cat <<EOF\nEOF\ngit commit --no-verify\nEOF",
        ] {
            assert!(refused(bad).is_some(), "{bad}");
        }
        for good in [
            "git commit -m 'drop -n and --no-verify'",
            "git commit -m x",
            "git push -n",
            "git push origin x",
            "git log -n 5",
            "git commit -m \"$(cat <<'EOF'\nuse --no-verify never\nEOF\n)\"",
            "echo git commit -n",
            "git commit -am 'fix'",
            "git status",
            "grep -rn 'git commit --no-verify' .",
            "git config --get core.hooksPath",
            "git config core.editor vim",
            "git commit --no-edit",
            "grep -n DASH_SHIP .githooks/pre-push",
            "git commit -m 'DASH_SHIP=1 is refused'",
            "grep -n DASH_SHIP= .githooks/pre-push",
            "git log -S DASH_SHIP= --oneline",
            "grep -rn DASH_SHIP=1 .githooks",
            "echo DASH_SHIP=1",
            "git log -S DASH_SHIP=1",
            "git commit -m 'drop --no-verify' && git push -n",
        ] {
            assert_eq!(refused(good), None, "{good}");
        }
    }

    #[dash_test]
    fn the_refusal_names_what_it_skips_and_the_fix() {
        let why = refused("git commit --no-verify -m x").expect("refused");
        assert!(
            why.starts_with("Refused by dash hook no-verify-commit: git commit --no-verify skips"),
            "{why}"
        );
        assert!(why.contains("cargo nextest run"), "{why}");
        assert_eq!(refused("export DASH_SHIP=1").as_deref(), Some(SHIP_MESSAGE));
        assert!(
            refused("git -c core.hooksPath=x push")
                .unwrap()
                .contains("git push with core.hooksPath overridden")
        );
    }
}
