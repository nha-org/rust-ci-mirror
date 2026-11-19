//! `dash hook pre-commit`: each step in order, the first refusal ends the commit with its reason. Every step reads what is
//! staged, never the working tree (task rust-git-hooks-port): the git ones read the index, and the tools that read files
//! (cargo, python, mix) run in `staged_root`, the top when it holds nothing else, else a checkout of the index.
use super::{Ctx, ran};
use std::fs;
use std::path::{Path, PathBuf};

/// Every refusal names its step, even when its tool printed nothing (a refused commit printed « commit failed » alone,
/// commit-failure-says-why, 2026-10-19).
fn refuse(step: &str) -> String {
    format!("pre-commit: {step} refused this commit (its own words, if any, above)")
}

/// The steps in order: no-elixir and rust-gate first (the task's done-when 1), the staged .rs formatted and the staged
/// checkout rust-gate builds in between them, then the rest as the shell hook ran them. crates/dash/tests/hooks.rs reads
/// the order from hook-times.log.
pub const STEPS: [&str; 7] = [
    "no-elixir",
    "rust-fmt",
    "staged-checkout",
    "rust-gate",
    "conflict-markers",
    "hooks-tests",
    "scripts-count",
];

/// `tells`: the AI tells table's reader, the outbound gate's (dash_hooks), which this crate cannot take; the binary hands it.
pub fn run(ctx: &Ctx, tells: Tells) -> Result<(), String> {
    ctx.timed("no-elixir", || super::no_elixir::staged(ctx))?;
    // read once a commit, for the two steps that read them: rust-fmt formats none of their files, so none moves between
    // (review of PR 1780)
    // timed with rust-fmt, its first reader, so no step's time goes uncounted (/code-review)
    let mut markers = vec![];
    ctx.timed("rust-fmt", || {
        markers = added_markers(ctx);
        rust_fmt(ctx, &markers)
    })?;
    let root = ctx.timed("staged-checkout", || staged_root(ctx))?;
    ctx.timed("rust-gate", || rust_gate(ctx, &root))?;
    ctx.timed("conflict-markers", || conflict_markers(&markers))?;
    ctx.timed("hooks-tests", || hooks_tests(ctx, &root))?;
    ctx.timed("scripts-count", || scripts_count(ctx))?;
    let only_rust = rust_only(ctx)?;
    // The three checks once in Python, each a Rust function now (task rust-git-hooks-port). A sent CV is immutable
    // (private/cv/sent.lock, @nha, 2026-09-26); CI runs the same check in mix cv.check. cv-locks and comments-anchored read
    // the index themselves, at the top beside the live state they read (private/live, the app); offline-check reads the
    // files, in `root`. Each refusal names its step.
    if !only_rust {
        ctx.timed("cv-locks", || step("cv-locks", super::cv_locks::check(ctx)))?;
    }
    // An edit must not strand an open comment of his.
    ctx.timed("comments-anchored", || {
        step(
            "comments-anchored",
            super::anchored::check(ctx).map(|ok| ctx.say(&ok)),
        )
    })?;
    // The CRM works offline: nothing it serves is fetched from the internet. Its words only on a refusal.
    ctx.timed("offline-check", || {
        step("offline-check", super::offline::check(&root).map(|_| ()))
    })?;
    // every commit: its rules read the Rust too (raw colors, « Nicolas, <date> »), and on a Rust-only commit no tool runs
    ctx.timed("lint", || lint(ctx, &root, tells))?;
    clojure_growth(ctx)?;
    Ok(())
}

/// The staged paths, git's own words (`--name-only` and the given flags)
/// In a merge of main, read against main's side ([`Ctx::merged_in`]): a file main already holds is not the branch's
/// to check (task merge-commit-precommit).
fn staged(ctx: &Ctx, args: &[&str]) -> Result<Vec<String>, String> {
    let mut a = vec!["diff", "--cached", "--name-only"];
    let mh = ctx.merged_in();
    a.extend(mh.as_deref());
    a.extend_from_slice(args);
    Ok(ctx.git(&a)?.lines().map(String::from).collect())
}

/// The staged .rs files formatted and restaged, so a commit never lands unformatted and the next scripted edit meets the
/// text cargo fmt would leave (task rust-scripted-edit-fmt: a sed written against the unformatted text matched nothing once
/// fmt ran, 3 tasks on 2026-11-09). Each staged blob goes through rustfmt on stdin: a path would make rustfmt read the
/// modules it declares too, staged or not (/code-review). A file it changes that is also changed unstaged is refused,
/// named: a restage would take the unstaged part, or leave the work tree behind. So is one in `git commit <paths>`, whose
/// temporary index a restage leaves the real one behind. A file as origin/<main> holds it is skipped: main is formatted
/// (workspace rule rust_is_rustfmt_clean), so a merge of main runs rustfmt on the branch's files alone.
pub fn rust_fmt(ctx: &Ctx, markers: &[(String, usize)]) -> Result<(), String> {
    let files: Vec<String> = staged(ctx, &["--diff-filter=ACMR"])?
        .into_iter()
        // the two workspaces, each with its rustfmt.toml: a fixture or a reference file elsewhere keeps its text, and so does
        // a vendored crate, as rust_is_rustfmt_clean reads it (/code-review of PR 1649)
        .filter(|p| p.ends_with(".rs") && (p.starts_with("rust/") || p.starts_with("dash/")))
        .filter(|p| !p.strip_prefix("rust/").is_some_and(crate::vendored))
        .collect();
    if files.is_empty() {
        return Ok(());
    }
    let mut args = vec!["diff", "--cached", "--name-only", "--no-renames"];
    let main = format!("origin/{}", ctx.main);
    args.extend([main.as_str(), "--"]);
    args.extend(files.iter().map(String::as_str));
    let files: Vec<String> = match ctx.git(&args) {
        Ok(differ) => files
            .into_iter()
            .filter(|f| differ.lines().any(|l| l == f))
            .collect(),
        Err(_) => files,
    };
    let mut changed = vec![];
    // a file whose staged lines add a conflict marker is the conflict-markers step's, which names the line; rustfmt would
    // only fail to parse it. A marker already committed (a fixture's) never skips it (/code-review of PR 1649 and its review)
    let conflicted = |f: &&String| markers.iter().any(|(p, _)| p == *f);
    for file in files.iter().filter(|f| !conflicted(f)) {
        let path = ctx.top.join(file);
        // a symlink's blob is its target's path, no Rust: rustfmt would refuse it, a write would go through the link
        if path.is_symlink() {
            continue;
        }
        let blob = ctx.git(&["show", &format!(":{file}")])?;
        // in the file's workspace, rust/ or dash/: its rustfmt.toml and its toolchain, which rustup reads from there
        let dir = ctx.top.join(file.split('/').next().unwrap_or_default());
        let out = rustfmt_stdin(ctx, &dir, blob.as_bytes()).map_err(|e| {
            format!(
                "{e}\npre-commit: rust-fmt: rustfmt failed on the staged {file} (its words above)"
            )
        })?;
        if out != blob.as_bytes() {
            changed.push((file.as_str(), path, out));
        }
    }
    if changed.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = changed.iter().map(|(f, ..)| *f).collect();
    let only = std::env::var_os("GIT_INDEX_FILE").is_some_and(|i| {
        Path::new(&i)
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("next-index"))
    });
    if only {
        return Err(format!(
            "pre-commit: rust-fmt would format {}, and `git commit <paths>` commits a temporary index a restage leaves \
             the real one behind: run `git add` on them and commit with no paths",
            names.join(", ")
        ));
    }
    let mut args = vec!["diff", "--name-only", "--"];
    args.extend(&names);
    let unstaged = ctx.git(&args)?;
    if !unstaged.trim().is_empty() {
        return Err(format!(
            "pre-commit: rust-fmt would format these staged .rs files, also changed unstaged: {}. Stage each whole \
             (git add <file>), or commit the unstaged part apart, then commit",
            unstaged.lines().collect::<Vec<_>>().join(", ")
        ));
    }
    for (file, path, out) in &changed {
        fs::write(path, out).map_err(|e| format!("pre-commit: rust-fmt: {file}: {e}"))?;
    }
    let mut add = vec!["add", "--"];
    add.extend(&names);
    ctx.git(&add)?;
    ctx.say(&format!(
        "pre-commit: rust-fmt formatted and restaged {}",
        names.join(", ")
    ));
    Ok(())
}

/// `text` through rustfmt on stdin, in `dir` (its rustfmt.toml and toolchain).
fn rustfmt_stdin(ctx: &Ctx, dir: &Path, text: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = ctx
        .cmd(dir, "rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("rustfmt did not start in {}: {e}", dir.display()))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(text)
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

/// Each staged line a commit adds that opens or closes a conflict, as `path` and line: git's own check, its whitespace
/// complaints and a bare ======= left out, each line read in the staged file. The one reading of the conflict-markers and
/// rust-fmt steps.
fn added_markers(ctx: &Ctx) -> Vec<(String, usize)> {
    // exit 2 when it finds a problem: its words are the answer
    let said = ran(ctx.cmd(&ctx.top, "git").args([
        "-c",
        "core.quotePath=false",
        "diff",
        "--cached",
        "--check",
    ]))
    .said;
    let mut found = vec![];
    for line in said.lines() {
        let Some(head) = line.strip_suffix(": leftover conflict marker") else {
            continue;
        };
        let Some((path, n)) = head.rsplit_once(':') else {
            continue;
        };
        let Ok(n) = n.parse::<usize>() else { continue };
        let blob = ctx.git(&["show", &format!(":{path}")]).unwrap_or_default();
        let text = blob.lines().nth(n.saturating_sub(1)).unwrap_or_default();
        if crate::guard::shell::conflict_marker(text).is_some() {
            found.push((path.to_string(), n));
        }
    }
    found
}

/// No conflict marker lands: git takes a file once it is added, markers and all (a refused Write left sandbox/approval.md
/// conflicted, and the merge commit of dcg-hook-other-repos held its <<<<<<< lines, 2026-11-01). git's own check names
/// path:line; its whitespace complaints are left out, and so is a bare ======= (a setext heading, a table rule), read in the
/// staged file: a real conflict always holds a <<<<<<< and a >>>>>>> line too (/code-review of PR 861).
pub fn conflict_markers(markers: &[(String, usize)]) -> Result<(), String> {
    let found: Vec<String> = (markers.iter())
        .map(|(path, n)| format!("{path}:{n}: leftover conflict marker"))
        .collect();
    if found.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{}\npre-commit: conflict markers staged (above): resolve them, then commit",
        found.join("\n")
    ))
}

/// The folder whose files are the index being committed. The top when its work tree holds nothing else: no unstaged change
/// and no untracked file. Else `<git dir>/pre-commit-staged`, a checkout of HEAD given this index's tree (read-tree -u
/// rewrites only the files that moved), kept from commit to commit with its builds: a push no longer drops it, since the
/// next commit then built cold (/code-review); it goes with its worktree, or with `bin/prune-paths`. A step that read the
/// working tree passed or refused a commit on what was never staged (task rust-git-hooks-port). Commits in one
/// checkout never share it at once: git holds the index's lock for the whole hook.
pub fn staged_root(ctx: &Ctx) -> Result<PathBuf, String> {
    let untracked = ctx.git(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "--directory",
        "--no-empty-directory",
    ])?;
    if ctx.git_ok(&["diff", "--quiet"]) && untracked.trim().is_empty() {
        return Ok(ctx.top.clone());
    }
    let why = |w: &str| {
        format!(
            "pre-commit: the work tree holds what is not staged, and {w}: commit from a clean tree, or stage it all"
        )
    };
    let tree = ctx
        .git(&["write-tree"])
        .map_err(|e| why(&format!("its index makes no tree ({e})")))?;
    let Some(head) = ctx.rev("HEAD") else {
        return Err(why("there is no HEAD to check the staged files out from"));
    };
    let wt = ctx.checkout("pre-commit-staged", &head).ok_or_else(|| {
        why(&format!(
            "no checkout of the staged files could be made in {}",
            ctx.git_dir.join("pre-commit-staged").display()
        ))
    })?;
    let read = ran(ctx
        .cmd(&wt, "git")
        .args(["read-tree", "--reset", "-u", tree.trim()]));
    if !read.ok() {
        return Err(why(&format!(
            "git read-tree in {} failed: {}",
            wt.display(),
            read.said.trim()
        )));
    }
    Ok(wt)
}

/// The Rust gate runs at each commit that touches rust/ (@nha, 2026-11-10: « these seem easier as code gates than prompts »):
/// budgets, workspace rules and the dependency graph live in dash-gate, and tasks met them only at push time. It reads the
/// source as text, so a warm run takes seconds. On the index being committed: in `root`, its GIT_INDEX_FILE handed to the
/// tests as `crate::HOOK_INDEX`, since #[dash_test] removes every GIT_* variable (review of PR 1596).
pub fn rust_gate(ctx: &Ctx, root: &Path) -> Result<(), String> {
    // rust/, else what outside it the gate's tests read (crate::lex::reads_past_rust, task rust-gate-reads-trigger): a port map
    // edit alone in private/docs shipped red. The gate's own sources, as staged; the scratch hooks tests' gate is rust/gate.
    let base = ctx.merged_in();
    let base = base.as_deref().unwrap_or("--cached");
    if ctx.git_ok(&["diff", "--cached", "--quiet", base, "--", "rust"]) {
        let sources: Vec<String> = ctx
            .git(&[
                "ls-files",
                "--",
                ":(glob)rust/**/gate/src/**/*.rs",
                ":(glob)rust/**/gate/tests/**/*.rs",
            ])?
            .lines()
            .map(String::from)
            .collect();
        // the staged names, a delete and both sides of a rename among them, matched against the list: no probe per literal,
        // which started a git cat-file for each of some 1.5k missing ones, about 90 s a commit (/simplify of #1624). A folder
        // on the list is read for its own entries (.github/workflows' files), never every file below it: private/dashboard
        // whole made half the tree a trigger (/code-review of #1624). ponytail: a test walking a folder deep is missed there
        let staged = staged(ctx, &["--no-renames"])?;
        let past: std::collections::BTreeSet<String> =
            crate::lex::reads_past_rust(root, &sources, &|_| true)
                .into_iter()
                .collect();
        let read = |f: &str| {
            past.contains(f)
                || f.rsplit_once('/')
                    .is_some_and(|(dir, _)| past.contains(dir))
        };
        if !staged.iter().any(|f| read(f)) {
            return Ok(());
        }
    }
    // PRECOMMIT_RUST_GATE_BOUND: the hooks test's short bound for a run cut at its shutdown (task mac-runner-shutdown-cause)
    let bound = super::env_secs("PRECOMMIT_RUST_GATE_BOUND", 300);
    let gate = || {
        let mut gate = ctx.bounded(&root.join("rust"), bound, "cargo");
        if let Some(index) = std::env::var_os("GIT_INDEX_FILE") {
            let [name, top] = crate::HOOK_INDEX;
            gate.env(name, index).env(top, &ctx.top);
        }
        gate
    };
    // --no-fail-fast: every ratchet a tree breaks is named in one refusal, never one per commit (extension-login-form
    // found three, Makefile task name, binary size, SDK reach, a commit each, 2026-11-15); dash_forge::GATE_TESTS too
    // both runs, the branch's and the merged-in head's, measure the same thing (/code-review of PR 1677)
    const GATE: [&str; 6] = [
        "nextest",
        "run",
        "--locked",
        "--no-fail-fast",
        "-p",
        "dash-gate",
    ];
    // Without the tests that read site_perso's .github/workflows: rust/ is frozen and its rules on those files with it
    // (@nha, 2026-11-16: « take the gate's workflow tests out of the pre-commit, since rust/ is frozen anyway »); the
    // superapp's CI (superapp.yml) and the frozen old ones are no longer its to judge.
    const NOT_WORKFLOWS: &str = "not (test(=a_hosted_job_checks_out_code_only_and_the_lint_job_runs_dash_lint) \
         | test(=site_perso_ci_steps_are_one_line) | test(=the_active_workflows_run_on_the_hostman_runner_and_build_nothing_there) \
         | test(=the_precommit_and_the_pr_gate_run_the_script_count) | test(=a_gate_step_runs_a_command_mains_dash_answers))";
    let mut cmd = |more: &[&str]| ran(gate().args(GATE).args(more).args(["-E", NOT_WORKFLOWS]));
    let r = nextest_again(ctx, "dash-gate", &mut cmd);
    if r.passed() {
        cut_at_shutdown(ctx, &r, "dash-gate");
        return Ok(());
    }
    // a merge fails only on tests the merged-in head fails too: main's own red is not the branch's (rust-stalled-port,
    // 2026-11-14). Only a complete run's failures, every one named, are compared: a run cut at its bound never is.
    if let (Some(mh), Some(failed)) = (ctx.merged_in(), failed_tests(&r.said)) {
        // exact names: a bare filter is a substring and ran tests that share it (/code-review of PR 1677)
        let only = failed
            .iter()
            .filter_map(|t| t.rsplit(' ').next())
            .map(|t| format!("test(={t})"))
            .collect::<Vec<_>>()
            .join(" | ");
        if !failed.is_empty()
            && ctx.checkout("pre-commit-merged", &mh).is_some_and(|wt| {
                let mut head = ctx.bounded(&wt.join("rust"), bound, "cargo");
                let h = ran(head
                    .env_remove("GIT_INDEX_FILE")
                    .args(GATE)
                    .args(["-E", &only]));
                // every failed test also failed there (a run that built nothing names none: the refusal stands)
                failed_tests(&h.said).is_some_and(|red| failed.iter().all(|t| red.contains(t)))
            })
        {
            ctx.say("pre-commit: dash-gate fails only on tests the merged-in head fails too: counted as main's");
            return Ok(());
        }
    }
    Err(format!(
        "{}\npre-commit: dash-gate fails on this tree (above): fix it, then commit (rust/CLAUDE.md rule 2)",
        failing(
            &r,
            &["FAIL", "panicked", "past its budget", "Found:", "error"]
        )
    ))
}

/// A nextest run, `run(extra args)`, and once more under the long profile (no global limit, no per-test end) when time
/// alone cut it, no test failing: nine commits were refused at load 50 to 131 (push-merged-pr, 2026-11-19), never on an
/// assertion. The rerun has a bound of its own, so a step waits two bounds at worst; a failure still refuses, a hang at
/// the bound.
fn nextest_again(ctx: &Ctx, what: &str, run: &mut dyn FnMut(&[&str]) -> super::Ran) -> super::Ran {
    let r = run(&[]);
    if r.passed() || !cut_by_time(&r.said) {
        return r;
    }
    ctx.say(&format!(
        "pre-commit: {what} cut by time alone, no test failed: run again with no time limit but its bound"
    ));
    run(&["--profile", "long"])
}

/// The status words of a test nextest ended by time
const CUTS: [&str; 3] = ["TIMEOUT", "SIGTERM", "SIGKILL"];

/// A nextest status line's first word (`PASS`, `SIGSEGV`), never a test's captured output (/code-review)
fn status(l: &str) -> Option<&str> {
    l.contains(']')
        .then(|| l.split_whitespace().next())
        .flatten()
}

/// A nextest run that only time cut: a TIMEOUT, a SIGTERM or SIGKILL (its global limit's, its terminate-after's) or its
/// global limit, and no FAIL, ABORT, LEAK-FAIL or other signal line (a crash: SIGSEGV, SIGABRT).
fn cut_by_time(said: &str) -> bool {
    let words: Vec<&str> = said.lines().filter_map(status).collect();
    let failed = words.iter().any(|w| {
        ["FAIL", "ABORT", "LEAK-FAIL"].contains(w) || w.starts_with("SIG") && !CUTS.contains(w)
    });
    let cut = words.iter().any(|w| CUTS.contains(w)) || said.contains("due to global timeout");
    cut && !failed
}

/// The tests a nextest run names failing: its « FAIL [ 0.1s] <binary> <test> » lines (a timeout, a signal, an abort or a
/// leak failure too), each once, as « <binary> <test> ». None unless the run is complete and they are all of its failures:
/// its « Summary [..] N tests run: P passed.. » (never « N/M », a run cut short) counts N - P, so a status word not known
/// here, or a FAIL-looking line of a test's output, refuses rather than passes (/code-review of PR 1677).
/// ponytail: names, not reasons: a test red on main that the branch breaks otherwise passes
fn failed_tests(said: &str) -> Option<std::collections::BTreeSet<String>> {
    const BAD: [&str; 4] = ["FAIL", "TIMEOUT", "ABORT", "LEAK-FAIL"];
    let failed: std::collections::BTreeSet<String> = said
        .lines()
        .filter(|l| {
            l.contains(']')
                && l.split_whitespace()
                    .next()
                    .is_some_and(|w| BAD.contains(&w) || w.starts_with("SIG"))
        })
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            (w.len() >= 2).then(|| w[w.len() - 2..].join(" "))
        })
        .collect();
    let (before, after) = said.lines().rev().find_map(|l| l.split_once(" run: "))?;
    let n = before.rsplit(']').next()?.trim();
    let n: usize = n
        .strip_suffix(" tests")
        .or(n.strip_suffix(" test"))?
        .parse()
        .ok()?;
    let passed: usize = after.split(" passed").next()?.trim().parse().ok()?;
    (n.checked_sub(passed)? == failed.len()).then_some(failed)
}

/// The hooks tests (rust/crates/dash/tests/hooks.rs: each hook through git on scratch repositories), when a commit touches
/// what they test: the hooks, this module, the scripts they call, or the tests. Here, on the Mac: no CI machine can (task
/// rust-ci-off-mac). Without GIT_INDEX_FILE: they build their own repositories.
pub fn hooks_tests(ctx: &Ctx, root: &Path) -> Result<(), String> {
    let base = ctx.merged_in();
    let base = base.as_deref().unwrap_or("--cached");
    if ctx.git_ok(&[
        "diff",
        "--cached",
        "--quiet",
        base,
        "--",
        ".githooks",
        "bin/bounded",
        ":(glob)rust/**/gate/src/hook.rs",
        ":(glob)rust/**/gate/src/hook/**",
        ":(glob)rust/**/tests/hooks.rs",
        ":(glob)rust/**/tests/bounded.rs",
    ]) {
        return Ok(());
    }
    // bounded.rs: bin/bounded's process group and pty read (task bounded-test-under-load)
    let mut cmd = |more: &[&str]| {
        ran(ctx
            .bounded(&root.join("rust"), 600, "cargo")
            .env_remove("GIT_INDEX_FILE")
            .args(["nextest", "run", "--locked"])
            .args(more)
            .args(["-p", "dash", "--test", "hooks", "--test", "bounded"]))
    };
    let r = nextest_again(ctx, "the hooks tests", &mut cmd);
    if r.passed() {
        cut_at_shutdown(ctx, &r, "the hooks tests");
        return Ok(());
    }
    Err(format!(
        "{}\npre-commit: the hooks tests fail on this tree (above): fix them, then commit",
        failing(&r, &["FAIL", "panicked", "error"])
    ))
}

/// A pass its bound cut in the shutdown (124 after its summary) is said, so a slow exit stays visible.
fn cut_at_shutdown(ctx: &Ctx, r: &super::Ran, what: &str) {
    if !r.ok() {
        ctx.say(&format!("pre-commit: {what} passed, then its exit passed the time bound (124): counted as passed"));
    }
}

/// A tool's failing lines and each panic's reason, at most 60; never a refusal with no reason: none read, its last 5 lines (a timeout, a missing
/// command). A run its bound cut says so: a cold build reads as no failing test (/code-review).
fn failing(r: &super::Ran, words: &[&str]) -> String {
    let lines: Vec<&str> = r.said.lines().collect();
    // a panic's reason is the line after its « panicked at <file> », which holds none of the words
    // a test cut by its time, or a crash (SIGSEGV, SIGABRT), is named too: three refusals at load 50 said « (above) » over
    // nothing (push-merged-pr, 2026-11-19)
    let cut = |l: &str| {
        status(l).is_some_and(|w| w == "TIMEOUT" || w.starts_with("SIG"))
            || l.contains("due to global timeout")
    };
    let hit: Vec<&str> = (0..lines.len())
        .filter(|&i| {
            words.iter().any(|w| lines[i].contains(w))
                || cut(lines[i])
                || (i > 0 && lines[i - 1].contains("panicked at"))
        })
        .map(|i| lines[i])
        .take(60)
        .collect();
    let said = if hit.is_empty() {
        lines[lines.len().saturating_sub(5)..].join("\n")
    } else {
        hit.join("\n")
    };
    match r.code {
        Some(124) => format!(
            "{said}\npre-commit: cut at its time bound (exit 124), a cold build or a loaded Mac: commit again once it is built"
        ),
        _ => said,
    }
}

/// The script ratchet outside rust/ (dash_gate::ratchet; task rust-scripts-port): the staged scripts against
/// rust/scripts-held.txt, a number never above the [`measured_from`] side's, each listed in rfc-rust-unified's section
/// scripts-port. Only a commit that can move either: a path added, deleted or renamed, a mode changed, a held number or that
/// page, or a line added that opens on #! (a superset of a new line 1), or a script edited, whose shell ones it shellchecks
/// (task rust-lint-scripts-port).
pub fn scripts_count(ctx: &Ctx) -> Result<(), String> {
    let raw = ctx.git(&["diff", "--cached", "--raw", "--no-renames"])?;
    let moves = raw.lines().any(|l| {
        let (meta, path) = l.split_once('\t').unwrap_or((l, ""));
        let w: Vec<&str> = meta.split_whitespace().collect();
        // a sourced shell file (bin/approved-only) by its staged line 1, read only of a path with no extension
        let script = || {
            w.get(1) == Some(&"100755")
                || [".sh", ".bash"].iter().any(|e| path.ends_with(e))
                || (Path::new(path).extension().is_none()
                    && ctx
                        .git(&["show", &format!(":{path}")])
                        .is_ok_and(|t| crate::ratchet::shell(path, t.as_bytes())))
        };
        // the blob read last, after every cheap test (/code-review)
        w.get(4) != Some(&"M")
            || w.first().map(|m| m.trim_start_matches(':')) != w.get(1).copied()
            || path == "rust/scripts-held.txt"
            || path == "private/docs/rfc-rust-unified.html"
            // the other held numbers, which only drop (a rise alone skipped this step, /code-review of PR 1488)
            || [crate::ratchet::JS_HELD, crate::ratchet::BROWSER_HELD].contains(&path)
            || (!path.starts_with("rust/") && script())
    });
    let shebang = || {
        ctx.git(&["diff", "--cached", "--no-renames", "-U0"])
            .is_ok_and(|d| d.lines().any(|l| l.starts_with("+#!")))
    };
    if !moves && !shebang() {
        return Ok(());
    }
    match crate::ratchet::run(&ctx.top, true, Some(&measured_from(ctx))) {
        Ok(said) => {
            ctx.say(&said);
            Ok(())
        }
        Err(why) => Err(format!(
            "scripts count: {why}\npre-commit: dash scripts count failed (above): fix a shellcheck finding where it names one, else port a script to Rust; never raise the number"
        )),
    }
}

/// A commit that touches only rust/, with no script among it, skips the steps whose rules read no Rust file: the CV locks
/// (task brief-tests-prepush-skip: cv-locks 7 s of a 37 s Rust-only commit). The lint reads Rust since task rust-lint-port,
/// so it runs on every commit. --no-renames: a file moved
/// into rust/ names its old path too, which a skipped check may need (a sent CV moved there).
pub fn rust_only(ctx: &Ctx) -> Result<bool, String> {
    let paths = staged(ctx, &["--no-renames"])?;
    let mut raw_args = vec!["diff", "--cached", "--raw"];
    let base = ctx.merged_in();
    raw_args.extend(base.as_deref());
    let raw = ctx.git(&raw_args)?;
    Ok(!paths.is_empty()
        && paths.iter().all(|p| p.starts_with("rust/"))
        && !paths.iter().any(|p| {
            [".py", ".sh", ".bash", ".zsh"]
                .iter()
                .any(|e| p.ends_with(e))
        })
        && !raw
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some("100755")))
}

/// A check's refusal: its own words, then the line naming its step
fn step(name: &str, r: Result<(), String>) -> Result<(), String> {
    r.map_err(|why| format!("{why}\n{}", refuse(name)))
}

/// The AI tells table, built into the binary: its regex, label and what to write instead, a row each.
pub type Tells = fn() -> Result<Vec<crate::lint::Tell>, String>;

/// The lint gate (mix lint's, ported to dash_gate::lint in task rust-lint-port; @nha, 2026-09-28): every rule on the staged
/// tree, the browser slot rule and the tools (ruff, deno, clj-kondo, shellcheck) on the staged paths alone (task
/// precommit-fast). A staged shell script is shellchecked here on disk and by [`scripts_count`] from the index: the
/// done-when of task rust-lint-scripts-port keeps both, one flag ([`crate::ratchet::EXCLUDE`]).
fn lint(ctx: &Ctx, root: &Path, tells: Tells) -> Result<(), String> {
    let paths = staged(ctx, &["--diff-filter=ACMR"])?;
    // a rename deletes its old path: an exempt browser path moved away is gone too
    let deleted = staged(ctx, &["--diff-filter=D", "--no-renames"])?;
    let said = tells().and_then(|t| crate::lint::run(root, Some(&paths), &deleted, &t));
    match said {
        Ok(said) => {
            ctx.say(&said);
            Ok(())
        }
        Err(why) => Err(format!("{why}\n{}", refuse("lint"))),
    }
}

/// What a commit's growth is measured against: HEAD, or in a merge of a side main already holds, that side, so only the
/// branch's own growth counts (rfc-dashboard-comments, review-linkedin-login-no-keychain, 2026-10-19 and 10-20): that side
/// itself, never origin/<main>, whose later removals would offset the branch's growth. A merge of a side main lacks (another
/// branch) stays on HEAD, the stricter (/code-review of PR 1488). Every growth step reads it: the scripts count measured a
/// merge from HEAD, so main's own Chrome test charged the branch a browser test (PR 1488, 2026-11-12).
pub fn measured_from(ctx: &Ctx) -> String {
    ctx.merged_in().unwrap_or_else(|| "HEAD".into())
}

/// Clojure and bb only shrink (task bb-retire; @nha, 2026-10-19: « We also want to stop wasting tokens on clojure / bb code
/// as soon as possible »), from the [`measured_from`] side.
pub fn clojure_growth(ctx: &Ctx) -> Result<(), String> {
    let base = measured_from(ctx);
    let stat = ctx.git(&[
        "diff",
        "--cached",
        &base,
        "--numstat",
        "--",
        "*.clj",
        "*.cljc",
        "*.cljs",
        "*.bb",
        "bb.edn",
    ])?;
    let grow: i64 = stat
        .lines()
        .map(|l| {
            let w: Vec<i64> = l
                .split_whitespace()
                .take(2)
                .map(|n| n.parse().unwrap_or(0))
                .collect();
            w.first().unwrap_or(&0) - w.get(1).unwrap_or(&0)
        })
        .sum();
    if grow > 0 {
        return Err(format!(
            "pre-commit: +{grow} lines of Clojure or bb.edn: port the change to Rust (rust/), or remove as much as you add (task bb-retire)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a run cut by time, no test failed, runs dash-gate again.
    #[dash_meter::dash_test]
    fn a_run_cut_by_time_alone_runs_again() {
        let pass = "        PASS [   1.0s] dash-gate a\n";
        let timeout = "     TIMEOUT [  30.1s] dash-gate b\n";
        let sig = "     SIGTERM [   3.8s] dash-gate c\n";
        let fail = "        FAIL [   0.1s] dash-gate d\n";
        assert!(cut_by_time(&format!("{pass}{timeout}")));
        assert!(cut_by_time(&format!(
            "{pass}{sig}warning: 33/226 tests were not run due to global timeout\n"
        )));
        assert!(
            !cut_by_time(&format!("{pass}{timeout}{fail}")),
            "a failure refuses"
        );
        assert!(!cut_by_time(pass), "nothing cut");
        for crash in ["SIGSEGV", "SIGABRT"] {
            let crash = format!("     {crash} [   0.2s] dash-gate e\n");
            assert!(
                !cut_by_time(&format!("{timeout}{crash}")),
                "{crash}: a crash refuses"
            );
        }
        assert!(
            !cut_by_time("error: could not compile `dash-gate`\n"),
            "a build error refuses"
        );
    }

    /// A refusal names the tests its run cut by time or signal, never only « error: test run failed ».
    #[dash_meter::dash_test]
    fn a_refusal_names_a_test_cut_by_time() {
        let said = "        PASS [   1.0s] dash::hooks a\n     TIMEOUT [  60.9s] dash::hooks b\n     SIGTERM [   3.8s] dash::hooks c\n     SIGSEGV [   0.2s] dash::hooks d\n    sent SIGTERM to the child\nerror: test run failed\n";
        let r = super::super::Ran {
            code: Some(100),
            said: said.into(),
        };
        let out = failing(&r, &["FAIL", "panicked", "error"]);
        assert!(out.contains("TIMEOUT [  60.9s] dash::hooks b"), "{out}");
        assert!(out.contains("SIGTERM [   3.8s] dash::hooks c"), "{out}");
        assert!(
            out.contains("SIGSEGV [   0.2s] dash::hooks d"),
            "a crash is named: {out}"
        );
        assert!(
            !out.contains("PASS") && !out.contains("to the child"),
            "{out}"
        );
    }

    /// A merge passes main's red only on a complete run whose failures are all named: a run cut at its bound, or a failure
    /// status not known here, names none (/code-review of PR 1677).
    #[dash_meter::dash_test]
    fn only_a_complete_run_names_its_failures() {
        let fail = "        FAIL [   0.1s] dash-gate lint::a\n";
        let sum = |s: &str| format!("     Summary [  1.0s] {s}\n");
        let got = failed_tests(&(fail.to_string() + &sum("3 tests run: 2 passed, 1 failed")));
        assert_eq!(
            got.map(|f| f.into_iter().collect::<Vec<_>>()),
            Some(vec!["dash-gate lint::a".to_string()])
        );
        // cut short: « N/M tests run », or no summary at all
        assert_eq!(
            failed_tests(&(fail.to_string() + &sum("2/3 tests run: 1 passed, 1 failed"))),
            None
        );
        assert_eq!(failed_tests(fail), None);
        // a second failure whose status word is unknown here: the count does not match
        assert_eq!(
            failed_tests(
                &(fail.to_string()
                    + "     WEIRD [ 0.1s] dash-gate lint::b\n"
                    + &sum("3 tests run: 1 passed, 2 failed"))
            ),
            None
        );
        // a signal counts
        let sig = "      SIGKILL [ 0.1s] dash-gate lint::b\n";
        assert_eq!(
            failed_tests(&(fail.to_string() + sig + &sum("3 tests run: 1 passed, 2 failed")))
                .map(|f| f.len()),
            Some(2)
        );
    }
}
