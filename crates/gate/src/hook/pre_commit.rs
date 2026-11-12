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

/// The steps in order: no-elixir and rust-gate first (the task's done-when 1), the staged checkout rust-gate builds in
/// between them, then the rest as the shell hook ran them. crates/dash/tests/hooks.rs reads the order from hook-times.log.
pub const STEPS: [&str; 6] = [
    "no-elixir",
    "staged-checkout",
    "rust-gate",
    "conflict-markers",
    "hooks-tests",
    "scripts-count",
];

/// `tells`: the AI tells table's reader, the outbound gate's (dash_hooks), which this crate cannot take; the binary hands it.
pub fn run(ctx: &Ctx, tells: Tells) -> Result<(), String> {
    ctx.timed("no-elixir", || super::no_elixir::staged(ctx))?;
    let root = ctx.timed("staged-checkout", || staged_root(ctx))?;
    ctx.timed("rust-gate", || rust_gate(ctx, &root))?;
    ctx.timed("conflict-markers", || conflict_markers(ctx))?;
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
    if !only_rust {
        ctx.timed("lint", || lint(ctx, &root, tells))?;
    }
    scripts_model_test(ctx, &root)?;
    main_suite(ctx, &root)?;
    clojure_growth(ctx)?;
    drift(ctx);
    Ok(())
}

/// The staged paths, git's own words (`--name-only` and the given flags)
fn staged(ctx: &Ctx, args: &[&str]) -> Result<Vec<String>, String> {
    let mut a = vec!["diff", "--cached", "--name-only"];
    a.extend_from_slice(args);
    Ok(ctx.git(&a)?.lines().map(String::from).collect())
}

/// No conflict marker lands: git takes a file once it is added, markers and all (a refused Write left sandbox/approval.md
/// conflicted, and the merge commit of dcg-hook-other-repos held its <<<<<<< lines, 2026-11-01). git's own check names
/// path:line; its whitespace complaints are left out, and so is a bare ======= (a setext heading, a table rule), read in the
/// staged file: a real conflict always holds a <<<<<<< and a >>>>>>> line too (/code-review of PR 861).
pub fn conflict_markers(ctx: &Ctx) -> Result<(), String> {
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
        if text.starts_with("<<<<<<<") || text.starts_with(">>>>>>>") {
            found.push(format!("{path}:{n}: leftover conflict marker"));
        }
    }
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
/// source as text, so a warm run takes seconds. On the index being committed: in `root`, with GIT_INDEX_FILE at the top.
pub fn rust_gate(ctx: &Ctx, root: &Path) -> Result<(), String> {
    if ctx.git_ok(&["diff", "--cached", "--quiet", "--", "rust"]) {
        return Ok(());
    }
    let r = ran(ctx.bounded(&root.join("rust"), 300, "cargo").args([
        "nextest",
        "run",
        "--locked",
        "-p",
        "dash-gate",
    ]));
    if r.ok() {
        return Ok(());
    }
    Err(format!(
        "{}\npre-commit: dash-gate fails on this tree (above): fix it, then commit (rust/CLAUDE.md rule 2)",
        failing(
            &r,
            &["FAIL", "panicked", "past its budget", "Found:", "error"]
        )
    ))
}

/// The hooks tests (rust/crates/dash/tests/hooks.rs: each hook through git on scratch repositories), when a commit touches
/// what they test: the hooks, this module, the scripts they call, or the tests. Here, on the Mac: no CI machine can (task
/// rust-ci-off-mac). Without GIT_INDEX_FILE: they build their own repositories.
pub fn hooks_tests(ctx: &Ctx, root: &Path) -> Result<(), String> {
    if ctx.git_ok(&[
        "diff",
        "--cached",
        "--quiet",
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
    let r = ran(ctx
        .bounded(&root.join("rust"), 600, "cargo")
        .env_remove("GIT_INDEX_FILE")
        .args([
            "nextest", "run", "--locked", "-p", "dash", "--test", "hooks", "--test", "bounded",
        ]));
    if r.ok() {
        return Ok(());
    }
    Err(format!(
        "{}\npre-commit: the hooks tests fail on this tree (above): fix them, then commit",
        failing(&r, &["FAIL", "panicked", "error"])
    ))
}

/// A tool's failing lines, at most 40; never a refusal with no reason: none read, its last 5 lines (a timeout, a missing
/// command). A run its bound cut says so: a cold build reads as no failing test (/code-review).
fn failing(r: &super::Ran, words: &[&str]) -> String {
    let hit: Vec<&str> = r
        .said
        .lines()
        .filter(|l| words.iter().any(|w| l.contains(w)))
        .take(40)
        .collect();
    let said = if hit.is_empty() {
        let all: Vec<&str> = r.said.lines().collect();
        all[all.len().saturating_sub(5)..].join("\n")
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
/// and the repository's lint (task brief-tests-prepush-skip: cv-locks 7 s and mix-lint 12 s of a 37 s Rust-only commit). A staged
/// .sh, .py or executable there still runs the whole lint, whose held script count it changes. --no-renames: a file moved
/// into rust/ names its old path too, which a skipped check may need (a sent CV moved there).
pub fn rust_only(ctx: &Ctx) -> Result<bool, String> {
    let paths = staged(ctx, &["--no-renames"])?;
    let raw = ctx.git(&["diff", "--cached", "--raw"])?;
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

/// The AI tells table read from a checkout's top: its regex, label and what to write instead, a row each.
pub type Tells = fn(&Path) -> Result<Vec<crate::lint::Tell>, String>;

/// The lint gate (mix lint's, ported to dash_gate::lint in task rust-lint-port; @nha, 2026-09-28): every rule on the staged
/// tree, the browser slot rule and the tools (ruff, deno, clj-kondo) on the staged paths alone (task precommit-fast).
fn lint(ctx: &Ctx, root: &Path, tells: Tells) -> Result<(), String> {
    let paths = staged(ctx, &["--diff-filter=ACMR"])?;
    // a rename deletes its old path: an exempt browser path moved away is gone too
    let deleted = staged(ctx, &["--diff-filter=D", "--no-renames"])?;
    let said = tells(root).and_then(|t| crate::lint::run(root, Some(&paths), &deleted, &t));
    match said {
        Ok(said) => {
            ctx.say(&said);
            Ok(())
        }
        Err(why) => Err(format!("{why}\n{}", refuse("lint"))),
    }
}

/// A staged script of priv/commands passes scripts_model_test (12 s): e8d7de87 named a store path in git-ship, master stayed
/// red and a reviewer fixed it (review-dashboard-comments-fast, 2026-10-19).
fn scripts_model_test(ctx: &Ctx, root: &Path) -> Result<(), String> {
    if staged(ctx, &["--", "private/dashboard/priv/commands/*"])?.is_empty() {
        return Ok(());
    }
    ctx.timed("scripts-model-test", || {
        let r = ran(ctx
            .cmd(&root.join("private/dashboard"), "bin/toolchain")
            .env_remove("GIT_INDEX_FILE")
            .env("MIX_ENV", "test")
            .args(["mix", "test", "test/dashboard/scripts_model_test.exs"]));
        match r.ok() {
            true => Ok(()),
            false => Err("pre-commit: private/dashboard/bin/toolchain mix test test/dashboard/scripts_model_test.exs fails".to_string()),
        }
    })
}

/// In the main checkout a commit lands on main as it is (dash ship): staged dashboard code passes the whole suite first,
/// compiled with --warnings-as-errors as mix precommit (fe84f46b and e8d7de87 each turned master red, 2026-10-19). Task
/// worktrees run mix precommit before their PR instead. It never waits for a load hold (DASH_TEST_LOAD_WAIT=0, /code-review of
/// PR 971); its log is this checkout's git dir's.
fn main_suite(ctx: &Ctx, root: &Path) -> Result<(), String> {
    let dirs = ["lib", "bin", "priv", "config", "assets", "test", "mix.exs"]
        .map(|d| format!("private/dashboard/{d}"));
    let mut args = vec!["--"];
    args.extend(dirs.iter().map(String::as_str));
    if ctx.is_worktree() || staged(ctx, &args)?.is_empty() {
        return Ok(());
    }
    let log = ctx.git_dir.join("pre-commit-mix-test.log");
    ctx.timed("main-suite", || {
        let r = ran(ctx
            .cmd(&root.join("private/dashboard"), "sh")
            .env_remove("GIT_INDEX_FILE")
            .env("MIX_ENV", "test")
            .env("DASH_TEST_LOAD_WAIT", "0")
            .args([
                "-c",
                "bin/toolchain mix compile --warnings-as-errors && bin/toolchain mix test",
            ]));
        let _ = fs::write(&log, &r.said);
        if r.ok() {
            return Ok(());
        }
        let lines: Vec<&str> = r
            .said
            .lines()
            .filter(|l| test_line(l) || l.contains("Result"))
            .take(10)
            .collect();
        Err(format!(
            "{}\npre-commit: the dashboard suite fails (log: {})",
            lines.join("\n"),
            log.display()
        ))
    })
}

/// « <spaces>N) test ... », a failing test in a mix log
pub fn test_line(l: &str) -> bool {
    let t = l.trim_start();
    t.len() < l.len()
        && t.split_once(") test")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// What a commit's growth is measured against: HEAD, or in a merge of a side main already holds, that side, so only the
/// branch's own growth counts (rfc-dashboard-comments, review-linkedin-login-no-keychain, 2026-10-19 and 10-20): that side
/// itself, never origin/<main>, whose later removals would offset the branch's growth. A merge of a side main lacks (another
/// branch) stays on HEAD, the stricter (/code-review of PR 1488). Every growth step reads it: the scripts count measured a
/// merge from HEAD, so main's own Chrome test charged the branch a browser test (PR 1488, 2026-11-12).
pub fn measured_from(ctx: &Ctx) -> String {
    let main = format!("origin/{}", ctx.main);
    ctx.rev("MERGE_HEAD")
        .filter(|mh| ctx.git_ok(&["merge-base", "--is-ancestor", mh, &main]))
        .unwrap_or_else(|| "HEAD".into())
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

/// A CRM file written around the dashboard's model is named, never blocking (task dashboard-retire-writers). Main checkout
/// only, the one that dashboard serves; a dashboard that is down says nothing.
fn drift(ctx: &Ctx) {
    let dash = ctx.top.join("private/dashboard/bin/dash");
    if ctx.is_worktree() || !super::is_exec(&dash) {
        return;
    }
    let r = ctx.timed("dash-drift", || ran(ctx.cmd(&ctx.top, &dash).arg("drift")));
    if !r.ok() && r.said.contains("differ from the model") {
        ctx.say(&format!("pre-commit: {}", r.said.trim()));
    }
}
