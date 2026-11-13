//! `dash hook pre-push <remote> <url>`, git's refs on stdin. Quiet on a pass (task prepush-quiet; @nha, 2026-10-29: « we know
//! agents LOVE to be verbose … And we measure. And improve. »): all it says goes to prepush.log in this checkout's git dir; a
//! pass prints one line naming it, at most 3 lines of the notes a step left (tests not run here, a red test excused as
//! main's: never a silent « passed »), then his decisions in full; a refusal prints the whole log.
use super::{Ctx, Ran, ran, short};
use std::collections::BTreeSet;
use std::path::PathBuf;

const ZERO: &str = "0000000000000000000000000000000000000000";

/// A pushed ref, as git gives it on stdin
pub struct Ref {
    pub local: String,
    pub remote: String,
    /// the commit the remote holds there, zeros for a new branch
    pub old: String,
}

pub fn refs(stdin: &str) -> Vec<Ref> {
    stdin
        .lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            (w.len() == 4).then(|| Ref {
                local: w[1].into(),
                remote: w[2].into(),
                old: w[3].into(),
            })
        })
        .collect()
}

/// The hook: runs the steps, then says what a pass or a refusal prints. Its exit code.
pub fn main(ctx: &mut Ctx, remote: &str, url: &str, stdin: &str) -> i32 {
    let log = ctx.git_dir.join("prepush.log");
    ctx.quiet(&log);
    let mut decided = String::new();
    let result = run(ctx, remote, url, &refs(stdin), &mut decided);
    if let Err(why) = &result {
        ctx.say(why);
    }
    ctx.total();
    drop_checkouts(ctx);
    let said = ctx.said();
    match result {
        Err(_) => {
            super::tell(&said);
            1
        }
        Ok(()) => {
            if !said.is_empty() {
                super::tell(&format!(
                    "pre-push: passed in {}s, all it said in {}\n",
                    ctx.seconds(),
                    log.display()
                ));
                notes(&said)
                    .iter()
                    .take(3)
                    .for_each(|l| super::tell(&format!("{l}\n")));
                if !decided.is_empty() {
                    let n = decided.lines().filter(|l| l.starts_with("- ")).count();
                    super::tell(&format!(
                        "pre-push: his decisions on what these commits name ({n}), before they leave:\n{decided}\n"
                    ));
                }
            }
            0
        }
    }
}

/// The notes a pass keeps: each « pre-push: » line but a step's time, the total, « testing » and the decisions' line, with the
/// lines under it (the excused tests' names).
pub fn notes(said: &str) -> Vec<&str> {
    let mut keep = false;
    let mut out = vec![];
    for l in said.lines() {
        if let Some(rest) = l.strip_prefix("pre-push: ") {
            let time = rest.ends_with(')') && rest.contains("s (exit ");
            keep = !(time
                || rest.starts_with("total ")
                || rest.starts_with("testing ")
                || rest.starts_with("these commits name a subject"));
        }
        if keep {
            out.push(l);
        }
    }
    out
}

/// The checkouts the tests ran in leave with the push (task worktree-disk-reaped; @nha, 2026-11-03: « make sure we can reclaim
/// space that is reclaimable »): about 1 GB each with _build and deps. PREPUSH_KEEP_CHECKOUTS=1 keeps them. pre-commit's staged
/// checkout stays: dropped, the next commit with an unstaged file built dash-gate cold under its 300 s bound (/code-review).
fn drop_checkouts(ctx: &Ctx) {
    if std::env::var_os("PREPUSH_KEEP_CHECKOUTS").is_some_and(|v| !v.is_empty()) {
        return;
    }
    for c in ["prepush-base", "prepush-tip"] {
        ctx.drop_checkout(c);
    }
}

/// `owner/name` of a GitHub URL, ssh or https
pub fn github_repo(url: &str) -> Option<String> {
    let at = url.find("github.com")?;
    let rest = url[at + "github.com".len()..].strip_prefix([':', '/'])?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    };
    let (owner, name) = rest.split_once('/')?;
    (ok(owner) && ok(name)).then(|| format!("{owner}/{name}"))
}

struct State {
    tested: bool,
    rust_tested: bool,
    note_checked: bool,
    tip_of: Option<String>,
}

pub fn run(
    ctx: &Ctx,
    remote: &str,
    url: &str,
    refs: &[Ref],
    decided: &mut String,
) -> Result<(), String> {
    // A push waits on a finished merge: review-board-linear-keys pushed right after a conflicted merge and sent the
    // pre-merge commit (2026-10-26).
    if ctx.rev("MERGE_HEAD").is_some() {
        return Err("pre-push: a merge is still in progress (conflicts or no commit yet): resolve and commit it, then push".into());
    }
    let repo = ctx
        .git(&["remote", "get-url", remote])
        .ok()
        .and_then(|u| github_repo(u.trim()))
        .or_else(|| github_repo(url));
    let origin_repo = ctx
        .git(&["remote", "get-url", "origin"])
        .ok()
        .and_then(|u| github_repo(u.trim()));
    main_guard(ctx, remote, repo.as_deref(), origin_repo.as_deref(), refs)?;
    review_push(ctx, repo.as_deref(), refs)?;
    let mut st = State {
        tested: false,
        rust_tested: false,
        note_checked: false,
        tip_of: None,
    };
    for r in refs
        .iter()
        .filter(|r| r.local != ZERO && !r.local.is_empty())
    {
        precommit_result(ctx, &r.local)?;
        sandbox_note(ctx, &r.local, &mut st)?;
        app_tests(ctx, &r.local, &mut st)?;
        main_merged_only(ctx, r, &mut st);
        rust_tests(ctx, &r.local, &mut st)?;
        decisions(ctx, &r.local, decided);
    }
    Ok(())
}

/// The app's command line (the Elixir dash, until Rust holds review push and kb decisions)
fn app(ctx: &Ctx) -> PathBuf {
    std::env::var_os("DASH_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.top.join("private/dashboard/bin/dash"))
}

/// A worktree's push never lands on the default branch, a delete of it included (task audit-guards; @nha, 2026-11-06: « Must
/// fix Should fix etc. are we acting on these ? »): main moves by a merged PR, or by dash ship, whose git-ship says so with
/// DASH_SHIP. Origin only, by name or by its GitHub repo under another name or a URL: a mirror such as Codeberg follows origin.
/// `main` is named too: forge.env is the worktree's own, so a branch that edits it would turn the guard off (/code-review).
pub fn main_guard(
    ctx: &Ctx,
    remote: &str,
    repo: Option<&str>,
    origin: Option<&str>,
    refs: &[Ref],
) -> Result<(), String> {
    let shipped = std::env::var_os("DASH_SHIP").is_some_and(|v| !v.is_empty());
    let to_origin = remote == "origin" || repo.is_some() && repo == origin;
    if !ctx.is_worktree() || shipped || !to_origin {
        return Ok(());
    }
    for r in refs {
        let b = r.remote.strip_prefix("refs/heads/").unwrap_or("");
        if b == "main" || b == ctx.main {
            return Err(format!(
                "pre-push: a worktree pushes its own branch and opens a PR, never {b}: git push origin HEAD:task/<id>, or dash ship"
            ));
        }
    }
    Ok(())
}

/// A push to a PR branch another live session holds is refused, each holder named (task push-claimed-branch; @nha,
/// 2026-10-22: « No double work. »). The app judges (dash review push); one that does not answer refuses nothing. A refusal
/// is its words anywhere in the answer: a line dash prints first never turns it into a pass.
pub fn review_push(ctx: &Ctx, repo: Option<&str>, refs: &[Ref]) -> Result<(), String> {
    let Some(repo) = repo else { return Ok(()) };
    for r in refs {
        let Some(b) = r.remote.strip_prefix("refs/heads/") else {
            continue;
        };
        if r.local.is_empty() || r.local == ZERO || b == ctx.main {
            continue;
        }
        let a = ran(ctx
            .cmd(&ctx.top, app(ctx))
            .args(["review", "push", &format!("{repo}:{b}")])
            .env("DASH_TIMEOUT", "10")
            .env("DASH_RESTART_TRIES", "0"));
        if !a.ok() && a.said.contains("review push: refused") {
            return Err(format!("pre-push: {}", a.said.trim()));
        }
    }
    Ok(())
}

/// A commit whose last mix precommit failed is not pushed (task dev-guards-2; @nha, 2026-10-22: « YES. ALWAYS. ALWAYS. »):
/// bin/toolchain files each run as « <tree> <exit> <first failure> » in the common git dir. Only the tip: a fix commit on
/// top of a failed one is the way out.
pub fn precommit_result(ctx: &Ctx, local: &str) -> Result<(), String> {
    let tree = ctx.git(&["rev-parse", &format!("{local}^{{tree}}")])?;
    let tree = tree.trim();
    let results = std::fs::read_to_string(ctx.common.join("precommit-results")).unwrap_or_default();
    let Some(last) = results
        .lines()
        .rfind(|l| l.starts_with(&format!("{tree} ")))
    else {
        return Ok(());
    };
    let mut w = last.splitn(3, ' ').skip(1);
    let (code, first) = (w.next().unwrap_or(""), w.next().unwrap_or(""));
    if code == "0" {
        return Ok(());
    }
    Err(format!(
        "pre-push: {}: its last mix precommit failed (exit {code}): {first}\nfix it, run private/dashboard/bin/toolchain mix precommit until it passes, then push",
        short(local)
    ))
}

fn merge_base(ctx: &Ctx, local: &str) -> Option<String> {
    ctx.git(&["merge-base", local, &format!("origin/{}", ctx.main)])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A branch that changes sandbox/approval.md runs make sandbox/check before it leaves (task approval-check-prepush; @nha,
/// 2026-10-29: « This should be second nature »), on the pushed commit, first, so a refusal costs no compile. Only
/// bin/approval-number's lines refuse, each naming the section; the target's other lines are about the tap after the merge.
/// Worktrees only. One ref per push.
/// ponytail: a hand-written number shipped from the main checkout goes unchecked; a check on the pushed blob if one lands there.
fn sandbox_note(ctx: &Ctx, local: &str, st: &mut State) -> Result<(), String> {
    if st.note_checked || !ctx.is_worktree() {
        return Ok(());
    }
    let Some(base) = merge_base(ctx, local) else {
        st.note_checked = true;
        // never a silent pass (/code-review): the task exists to refuse these before review
        ctx.say(&format!(
            "pre-push: no merge base with origin/{}: sandbox/approval.md was not checked here, the reviewer runs make sandbox/check",
            ctx.main
        ));
        return Ok(());
    };
    if ctx.git_ok(&["diff", "--quiet", &base, local, "--", "sandbox/approval.md"]) {
        return Ok(());
    }
    st.note_checked = true;
    let Some(tip) = ctx.checkout("prepush-tip", local) else {
        ctx.say(&format!("pre-push: no checkout of {local} could be made: make sandbox/check was not run here, the reviewer runs it"));
        return Ok(());
    };
    st.tip_of = Some(local.to_string());
    let r = ctx.timed("sandbox-check", || {
        ran(ctx
            .bounded(&tip, 30, "make")
            .args(["-s", "sandbox/check"])
            .env("DASH_TIMEOUT", "10")
            .env("DASH_RESTART_TRIES", "0"))
    });
    let slog = ctx.git_dir.join("prepush-sandbox-check.log");
    let _ = std::fs::write(&slog, &r.said);
    // what approval-number could not check, or found already on main: « pre-push: », the prefix a pass keeps
    for l in r
        .said
        .lines()
        .filter_map(|l| l.strip_prefix("approval-number note: "))
    {
        ctx.say(&format!("pre-push: approval-number: {l}"));
    }
    let refusals: Vec<&str> = r
        .said
        .lines()
        .filter(|l| l.starts_with("approval-number: "))
        .collect();
    if !r.ok() && !refusals.is_empty() {
        return Err(format!(
            "{}\npre-push: make sandbox/check refuses what {} does to sandbox/approval.md: fix the section it names, then push",
            refusals.join("\n"),
            short(local)
        ));
    }
    if !r.ok() {
        // no claim the note is clean: a crash or a cut run also lands here (/code-review)
        ctx.say(&format!(
            "pre-push: make sandbox/check exited {} with no approval-number refusal, so the push goes; the reviewer reads its lines ({})",
            r.code.unwrap_or(-1),
            slog.display()
        ));
    }
    Ok(())
}

/// The tests of what the branch changed in the app run before it leaves (task push-runs-touched-tests), in a checkout of the
/// pushed commit, so an edit made meanwhile tests nothing that does not leave. The app-skip: a branch that changes no Elixir
/// source has no app test to run (Elixir is frozen, so only a deleted source can break it): seven Rust-only pushes on
/// 2026-11-10 spent about 4 minutes each on a compile, and each booted a BEAM for nothing (@nha, 2026-11-11: « I also see
/// three instances of BEAM, we need only one on main checkout right ? »). The main checkout is skipped, its pre-commit ran
/// the whole suite. One ref per push.
fn app_tests(ctx: &Ctx, local: &str, st: &mut State) -> Result<(), String> {
    if st.tested || !ctx.top.join("private/dashboard").is_dir() || !ctx.is_worktree() {
        return Ok(());
    }
    let Some(base) = merge_base(ctx, local) else {
        return Ok(());
    };
    let elixir = [
        "private/dashboard/*.ex",
        "private/dashboard/*.exs",
        "private/dashboard/*.heex",
    ];
    let mut args = vec!["diff", "--quiet", &base, local, "--"];
    args.extend(elixir);
    if ctx.git_ok(&args) {
        ctx.say(
            "pre-push: no Elixir source changed since the merge-base: no app compile or tests here",
        );
        return Ok(());
    }
    let tip = match &st.tip_of {
        Some(t) if t == local => ctx.git_dir.join("prepush-tip"),
        _ => match ctx.checkout("prepush-tip", local) {
            Some(t) => t,
            None => {
                ctx.say(&format!(
                    "pre-push: no checkout of {local} could be made (in {}): the changed modules' tests were not run here, CI runs them",
                    ctx.git_dir.join("prepush-tip").display()
                ));
                return Ok(());
            }
        },
    };
    st.tested = true;
    ctx.say(&format!(
        "pre-push: testing {local} in a checkout of it ({})",
        tip.display()
    ));
    let dir = tip.join("private/dashboard");
    let mix = |secs: u64| {
        let mut c = ctx.bounded(&dir, secs, "bin/toolchain");
        c.env_remove("GIT_INDEX_FILE").env("MIX_ENV", "test");
        c
    };
    // Compile first, on its own bound, with --warnings-as-errors as CI's (PR 888's review left an unreachable clause). Only
    // « Compilation failed » refuses: a toolchain or deps failure on this Mac (exit 127, a fetch) stays CI's.
    let c = ctx.timed("compile", || {
        ran(mix(180).args(["mix", "compile", "--warnings-as-errors"]))
    });
    let _ = std::fs::write(ctx.git_dir.join("prepush-compile.log"), &c.said);
    if c.code == Some(124) {
        super::partial_deps(&tip).iter().for_each(|n| ctx.say(n));
    }
    if !c.ok() && !c.said.contains("Compilation failed") {
        ctx.say(&format!(
            "pre-push: mix compile did not finish in 180 s (or failed outside the app's code, exit {}): the selection below may not answer",
            c.code.unwrap_or(-1)
        ));
    } else if !c.ok() {
        ctx.say(&c.said);
        return Err(format!(
            "pre-push: mix compile --warnings-as-errors fails (exit {}): fix it before pushing, CI's check refuses it too",
            c.code.unwrap_or(-1)
        ));
    }
    let sel = ctx.timed("test-touched", || {
        let out = mix(30)
            .args(["mix", "test.touched", &base, local])
            .stderr(std::process::Stdio::null())
            .output();
        out.ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    });
    let Some(sel) = sel else {
        ctx.say("pre-push: mix test.touched did not answer in 30 s (or failed): the changed modules' tests were not run here, CI runs them");
        return Ok(());
    };
    // only test paths: a stale build's « Compiling 3 files » lines are no paths to test (/code-review)
    let files: Vec<&str> = sel
        .lines()
        .filter(|l| *l == "test" || l.starts_with("test/") && l.ends_with("_test.exs"))
        .collect();
    if files == ["test"] {
        ctx.say(
            "pre-push: mix.exs, mix.lock or config changed: the whole suite is CI's, not run here",
        );
        return Ok(());
    }
    if files.is_empty() {
        return Ok(());
    }
    // A typical push fits in PREPUSH_BUDGET seconds (task prepush-fast): the tests get what the steps before them left,
    // minus 15 s for the decisions and git, never less than 20 s; the nearest files come first, an overrun lets the push go.
    let budget = std::env::var("PREPUSH_BUDGET")
        .ok()
        .and_then(|b| b.parse::<u64>().ok())
        .unwrap_or(60);
    let mut left = budget.saturating_sub(15 + ctx.seconds());
    if left < 20 {
        ctx.say(&format!("pre-push: the steps before the tests left {left}s of the {budget}s budget: the tests get 20 s"));
        left = 20;
    }
    let t = ctx.timed("touched-tests", || {
        ran(mix(left)
            .env("DASH_TEST_LOAD_WAIT", "0")
            .args(["mix", "test", "--seed", "0"])
            .args(&files))
    });
    let mut code = t.code;
    let mut failed = t
        .said
        .lines()
        .filter(|l| super::pre_commit::test_line(l))
        .count();
    // A test red on main too is not this branch's (task prepush-main-red): each failing file reruns at the merge-base; a
    // failing test that fails there too, by name, no longer refuses. A failure no test line names still does.
    // ponytail: matched by name, so a test red on main hides a second way the branch breaks it; CI's run shows it.
    if failed != 0 {
        let known = names(&ctx.timed("base-tests", || at_base(ctx, &base, &failed_at(&t.said))));
        let now = names(&t.said);
        let old: Vec<&String> = now.iter().filter(|n| known.contains(*n)).collect();
        let new: Vec<&String> = now.iter().filter(|n| !known.contains(*n)).collect();
        let list = |v: &[&String]| v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        if !old.is_empty()
            && new.is_empty()
            && (code == Some(124) || failures(&t.said) == old.len())
        {
            ctx.say(&format!(
                "pre-push: these tests fail at the merge-base {} too, so not because of this branch; the push goes:\n{}",
                short(&base),
                list(&old)
            ));
            failed = 0;
            if code != Some(124) {
                code = Some(0);
            }
        } else if !old.is_empty() {
            let new = if new.is_empty() {
                "(no test named: a setup_all, a doctest or a crash, in the log below)".to_string()
            } else {
                list(&new)
            };
            ctx.say(&format!(
                "pre-push: failing at the merge-base {} too (not counted):\n{}\npre-push: new on this branch:\n{new}",
                short(&base),
                list(&old)
            ));
        }
    }
    if code == Some(124) && failed == 0 {
        ctx.say(&format!(
            "pre-push: the tests of the changed modules did not finish in {left} s ({} files, the nearest first), no failure of its own in that time: CI runs the rest",
            files.len()
        ));
    } else if code != Some(0) {
        // the whole mix log, each failure in full: exit 127 or a crash lists no test
        ctx.say(&t.said);
        if failed == 0 {
            ctx.say(&format!(
                "pre-push: mix test exited {} with no failing test listed (its log above)",
                code.unwrap_or(-1)
            ));
        }
        return Err(format!(
            "pre-push: the tests of the modules this branch changed fail ({} files): fix the failing test above, run private/dashboard/bin/toolchain mix test <its file>, then push",
            files.len()
        ));
    }
    Ok(())
}

/// The mix test log of those files that exist at `base`, there; empty when it cannot run, so every failure stays the
/// branch's. One 150 s bound for its compile and tests.
fn at_base(ctx: &Ctx, base: &str, files: &[String]) -> String {
    let here: Vec<&String> = files
        .iter()
        .filter(|f| ctx.git_ok(&["cat-file", "-e", &format!("{base}:private/dashboard/{f}")]))
        .collect();
    if here.is_empty() {
        return String::new();
    }
    let Some(wt) = ctx.checkout("prepush-base", base) else {
        return String::new();
    };
    let r: Ran = ran(ctx
        .bounded(&wt.join("private/dashboard"), 150, "bin/toolchain")
        .env_remove("GIT_INDEX_FILE")
        .env("MIX_ENV", "test")
        .args(["mix", "test", "--seed", "0"])
        .args(here));
    r.said
}

/// A mix log's failing tests, « test <name> (<module>) », sorted, each once
pub fn names(log: &str) -> BTreeSet<String> {
    log.lines()
        .filter(|l| super::pre_commit::test_line(l))
        .filter_map(|l| l.split_once(") ").map(|(_, t)| t.to_string()))
        .collect()
}

/// The test files a mix log's failures sit in (a file, not its line: a branch that edits it moves its lines)
pub fn failed_at(log: &str) -> Vec<String> {
    let at: BTreeSet<String> = log
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let (file, line) = t.rsplit_once(':')?;
            (l.len() > t.len()
                && file.starts_with("test/")
                && file.ends_with("_test.exs")
                && !file.contains(char::is_whitespace)
                && !line.is_empty()
                && line.chars().all(|c| c.is_ascii_digit()))
            .then(|| file.to_string())
        })
        .collect();
    at.into_iter().collect()
}

/// A nextest run its run limit ended before every test ran, none failing: its summary counts `<run>/<started> tests run`, with
/// no `failed` and no `timed out` (a test's own timeout is a failure).
pub fn cut_short(log: &str) -> bool {
    log.lines()
        .filter_map(|l| l.trim().strip_prefix("Summary ["))
        .filter_map(|l| l.split_once("] ")?.1.split_once(" tests run: "))
        .any(|(counts, outcome)| {
            counts.contains('/') && !outcome.contains("failed") && !outcome.contains("timed out")
        })
}

/// A mix log summary's failure count (Elixir 1.20: « Failed: 2 tests, 1 doctest », « Result: 3/5 passed, 1 invalid »),
/// invalid tests added; summary lines only: an assertion's « Failed: 2 retries » is no count (/code-review).
pub fn failures(log: &str) -> usize {
    let lead = |s: &str| {
        s.split_whitespace()
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0)
    };
    let mut f = 0;
    let mut invalid = 0;
    for l in log.lines() {
        if let Some(rest) = l.strip_prefix("Failed: ") {
            f += rest.split(", ").map(lead).sum::<usize>();
        }
        if let Some(rest) = l.strip_prefix("Result: ")
            && let Some(i) = rest.split(", ").find(|p| p.trim_end().ends_with("invalid"))
        {
            invalid = lead(i);
        }
    }
    f + invalid
}

/// A push that only merges main into the head the origin already holds (the orchestrator's update before a merge) adds no
/// change of the branch's own: CI tests the merge, and the orchestrator merges on CI alone. Run here at load 43, the crates'
/// tests timed out and refused PR 1441's update (2026-11-11).
fn main_merged_only(ctx: &Ctx, r: &Ref, st: &mut State) {
    let main = format!("origin/{}", ctx.main);
    let only_main = ctx.rev(&format!("{}^1", r.local)).as_deref() == Some(r.old.as_str())
        && ctx
            .rev(&format!("{}^2", r.local))
            .is_some_and(|second| ctx.git_ok(&["merge-base", "--is-ancestor", &second, &main]));
    if only_main && !st.rust_tested {
        st.rust_tested = true;
        ctx.say("pre-push: only main merged into the pushed head since the last push: CI tests it, no Rust tests here");
    }
}

/// The Rust crates this branch changed, and those depending on them, run their tests before it leaves (task
/// rust-affected-tests; @nha, 2026-11-11: « we can then skip CI if the crate doesn't change »), with CI=1 as CI runs them
/// (task rust-git-hooks-port). In this worktree, warm, for the pushed ref it has checked out: skipped with a note when the
/// workspace holds changes that ref lacks. Only the slice that fits the budget runs, fastest first by each test's time in
/// the common git dir's rust-test-times; the rest is named and left to CI. A red test reruns alone: at load 42 five tests
/// that pass alone in 3 s timed out, so one that passes alone is load, and only one red alone refuses the push.
fn rust_tests(ctx: &Ctx, local: &str, st: &mut State) -> Result<(), String> {
    let rust = ctx.top.join("rust");
    if st.rust_tested
        || !ctx.is_worktree()
        || !rust.join("Cargo.toml").is_file()
        || ctx.rev("HEAD").as_deref() != Some(local)
    {
        return Ok(());
    }
    let Some(base) = merge_base(ctx, local) else {
        return Ok(());
    };
    if ctx.git_ok(&["diff", "--quiet", &base, local, "--", "rust/"]) {
        return Ok(());
    }
    st.rust_tested = true;
    // « testing » ends the note before it: cargo's lines are the log's, never the 3 kept at exit
    ctx.say(&format!(
        "pre-push: testing the Rust crates {local} affects"
    ));
    use crate::affected;
    match affected::not_as_pushed(&rust, local) {
        Ok(None) => {}
        Ok(Some(why)) => {
            ctx.say(&format!(
                "pre-push: {why}: the Rust tests were not run here, CI runs them"
            ));
            return Ok(());
        }
        Err(why) => return Err(format!("pre-push: {why}")),
    }
    let crates = affected::of_repo(
        &rust,
        &base,
        &std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()),
    )
    .map_err(|e| format!("pre-push: {e}"))?;
    if crates.is_empty() {
        return Ok(());
    }
    // The build has its own bound, 60 s: a cold build of 30 crates took 130 s of a 138 s push (2026-11-11). Cut, its
    // finished crates stay in sccache, so the next push builds on from there; CI runs the tests meanwhile.
    let build_bound = env_secs("PREPUSH_RUST_BUDGET", 60);
    let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let cargo = |secs: u64, args: &[String]| {
        ran(ctx
            .bounded(&rust, secs, &cargo_bin)
            .env_remove("GIT_INDEX_FILE")
            .env("CI", "1")
            .args(args))
    };
    let list = affected::cargo_args(
        &crates,
        &["nextest", "list", "--locked", "--message-format", "json"].map(String::from),
    );
    let l = ctx.timed("rust-build", || cargo(build_bound, &list));
    match l.code {
        Some(0) => {}
        Some(124) => {
            ctx.say(&format!("pre-push: the affected crates did not build in {build_bound} s: CI runs their tests"));
            return Ok(());
        }
        _ => {
            ctx.say(&l.said);
            return Err("pre-push: the Rust crates this branch affects do not build (above): fix them, then push".into());
        }
    }
    let tests = affected::listed(&l.said).map_err(|e| format!("pre-push: {e}"))?;
    let store = ctx.common.join("rust-test-times");
    // the times learnt from this checkout's last plain `cargo nextest run` too (the pre-commit's dash-gate run, an agent's
    // own): a test never timed counts 1 s, and the first push here ran 60 of 1180 tests on that guess
    // once: only while newer than the store, so an old run never keeps pulling the times back (/code-review)
    let mut known = std::fs::read_to_string(&store).unwrap_or_default();
    let plain = rust.join("target/nextest/default/junit.xml");
    let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if mtime(&plain) > mtime(&store)
        && let Ok(xml) = std::fs::read_to_string(&plain)
    {
        known = affected::learn(&known, &affected::junit(&xml));
    }
    // The slice (task pre-push-fits-budget; @nha, 2026-11-11: « Think about the system and what would improve it for the
    // future. Spike, measure. »): the tests get what PREPUSH_BUDGET leaves after the steps before them, less 15 s for git
    // and nextest's start, never under 20 s; their times here, summed, fill that across the test threads, fewer under load.
    // ponytail: capacity = seconds x threads x min(1, threads / load), a guess the times log corrects; a fit if it misses
    let budget = env_secs("PREPUSH_BUDGET", 60);
    let left = budget.saturating_sub(15 + ctx.seconds()).max(20);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) as f64;
    let load = super::load()
        .trim_end_matches(',')
        .replace(',', ".")
        .parse::<f64>()
        .unwrap_or(threads);
    let capacity = left as f64 * threads * (threads / load.max(threads));
    let (run, rest) = affected::slice(tests, &affected::times(&known), capacity);
    if run.is_empty() {
        return Ok(());
    }
    if !rest.is_empty() {
        // named in a file of their own: a refusal prints the whole log, and 1120 names buried its reason (/code-review)
        let left_file = ctx.git_dir.join("prepush-rust-left.txt");
        let _ = std::fs::write(
            &left_file,
            rest.iter()
                .map(|(b, n)| format!("{b} {n}\n"))
                .collect::<String>(),
        );
        ctx.say(&format!(
            "pre-push: {} of the affected crates' {} tests fit the {left} s left at load {load}, fastest first; CI runs the {} slowest, named in {}",
            run.len(),
            run.len() + rest.len(),
            rest.len(),
            left_file.display()
        ));
    }
    // nextest's store is under the workspace's own target/, whatever CARGO_TARGET_DIR says
    let junit = rust.join("target/nextest/prepush/junit.xml");
    let run_tests = |label: &str, secs: u64, tests: Option<&[affected::Test]>, extra: &[&str]| {
        let mut args = affected::cargo_args(
            &crates,
            &["nextest", "run", "--locked", "--profile", "prepush"].map(String::from),
        );
        args.extend(extra.iter().map(|a| a.to_string()));
        if let Some(t) = tests {
            args.extend(["-E".to_string(), affected::filterset(t)]);
        }
        let _ = std::fs::remove_file(&junit);
        let r = ctx.timed(label, || cargo(secs, &args));
        ctx.say(&r.said);
        let cases = affected::junit(&std::fs::read_to_string(&junit).unwrap_or_default());
        (r, cases)
    };
    ctx.say(&format!(
        "pre-push: testing {} tests of {} (CI=1 cargo nextest run --profile prepush)",
        run.len(),
        crates.join(" ")
    ));
    let (r, cases) = run_tests(
        "rust-tests",
        left + 30,
        (!rest.is_empty()).then_some(&run[..]),
        &[],
    );
    // a run cut before its report learns nothing from nextest: each test of its slice is taken as twice its cost, so the
    // next push picks a smaller one rather than the same over-budget slice (/code-review)
    // nextest's own 120 s run limit cuts too (main's cut_short, rust-task-page-edits)
    let cut = (r.code == Some(124) || cut_short(&r.said)) && cases.is_empty();
    let learnt = if cut {
        affected::learn(&known, &affected::slower(&run, &affected::times(&known)))
    } else {
        affected::learn(&known, &cases)
    };
    let tmp = store.with_extension(format!("{}", std::process::id()));
    if std::fs::write(&tmp, learnt).is_ok() {
        let _ = std::fs::rename(&tmp, &store);
    }
    let again = affected::rerun(&cases);
    if !r.ok() && cases.is_empty() && !cut {
        // no report: a missing profile, a setup script's failure, a crash; never a pass (/code-review)
        return Err(format!(
            "pre-push: cargo nextest exited {} with no junit.xml (log above): fix it, then push",
            r.code.unwrap_or(-1)
        ));
    }
    if r.ok() || again.is_empty() {
        if !r.ok() {
            ctx.say(&format!(
                "pre-push: cargo nextest exited {} with no test failing (cut at {} s or by its run-wide cap): CI runs the rest",
                r.code.unwrap_or(-1),
                left + 30
            ));
        }
        return Ok(());
    }
    // Done-when 2: each red test reruns alone; one that passes alone was the load, never this push's failure.
    let (_, alone) = run_tests(
        "rust-tests-alone",
        40 * again.len() as u64 + 30,
        Some(&again),
        &["--test-threads", "1"],
    );
    let (failed, load_only) = affected::verdict(&cases, &alone);
    let names = |v: &[affected::Test]| {
        v.iter()
            .map(|(b, n)| format!("  {b} {n}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if !load_only.is_empty() {
        ctx.say(&format!(
            "pre-push: load, not this push: past their cap or red beside others at load {load}, they pass alone (or, past the first 3 of their kind, those 3 did):\n{}",
            names(&load_only)
        ));
    }
    if failed.is_empty() {
        return Ok(());
    }
    Err(format!(
        "pre-push: the tests of the Rust crates this branch affects fail, run alone too (log above):\n{}\nfix them, then push",
        names(&failed)
    ))
}

/// A whole number of seconds from the environment, else `default`
fn env_secs(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(default)
}

/// His decisions on what the pushed commits name are shown before they leave, once per head (task his-facts-loop; @nha,
/// 2026-10-29: « Read before write is a good idea »). Shown, never refused: the commits are written already. An app that does
/// not answer shows nothing.
fn decisions(ctx: &Ctx, local: &str, decided: &mut String) {
    let shown = ctx.git_dir.join("decisions-shown");
    if std::fs::read_to_string(&shown).is_ok_and(|s| s.lines().any(|l| l == local)) {
        return;
    }
    let said = ctx
        .git(&["log", "--format=%B", local, "--not", "--remotes"])
        .unwrap_or_default();
    if said.trim().is_empty() {
        return;
    }
    let out = std::process::Command::new(app(ctx))
        .args(["kb", "decisions", &said])
        .current_dir(&ctx.top)
        .env("DASH_TIMEOUT", "10")
        .env("DASH_RESTART_TRIES", "0")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Some(out) = out
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    else {
        return;
    };
    if !out.starts_with("- ") {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&shown)
    {
        let _ = writeln!(f, "{local}");
    }
    let out = out.trim_end();
    if !decided.is_empty() {
        decided.push('\n');
    }
    decided.push_str(out);
    ctx.say(&format!("pre-push: these commits name a subject he decided on; his decisions, before they leave:\n{out}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// rust-task-page-edits' push, refused twice at load 19: nextest's 120 s run limit ended 1169 tests at 761, then 393, every
    /// one run passing. Cut short is let through as the hook's own budget is; a failure or a test's timeout, cut or not, is not.
    #[dash_test]
    fn a_nextest_run_its_limit_cut_with_none_failed_is_cut_short() {
        let cut = "  TRY 1 TERM [   0.021s] (─────────) dash-orch::path a\n     Summary [ 120.007s] 393/1169 tests run: 393 passed (47 slow), 3 skipped\nerror: test run failed\n";
        assert!(cut_short(cut));
        for not_cut in [
            "     Summary [  89.888s] 735 tests run: 735 passed (56 slow), 1 skipped\n",
            "     Summary [ 120.007s] 393/1169 tests run: 390 passed, 3 failed, 3 skipped\n",
            "     Summary [ 120.007s] 393/1169 tests run: 392 passed, 1 timed out, 3 skipped\n",
            "     Summary [  50.000s] 700 tests run: 699 passed, 1 failed\n",
            "a log with no summary\n",
        ] {
            assert!(!cut_short(not_cut), "{not_cut}");
        }
    }

    #[dash_test]
    fn a_mix_log_reads_its_failures() {
        let log = "  1) test it works (A.Test)\n     test/a_test.exs:12\n  2) test other (B.Test)\n     test/b_test.exs:3\nFailed: 1 test, 1 doctest\nResult: 3/5 passed, 1 invalid\nassert Failed: 2 retries\n";
        assert_eq!(names(log).len(), 2);
        assert_eq!(failed_at(log), ["test/a_test.exs", "test/b_test.exs"]);
        assert_eq!(failures(log), 3);
    }

    #[dash_test]
    fn a_pass_keeps_its_notes_never_its_times() {
        let said = "pre-push: compile 3s (exit 0)\npre-push: these tests fail at the merge-base abc too:\ntest x (A)\npre-push: total 9s\npre-push: testing abc\ncargo line\n";
        assert_eq!(
            notes(said),
            [
                "pre-push: these tests fail at the merge-base abc too:",
                "test x (A)"
            ]
        );
        assert_eq!(
            github_repo("git@github.com:nha-org/site_perso.git").as_deref(),
            Some("nha-org/site_perso")
        );
        assert_eq!(github_repo("/tmp/mirror"), None);
    }
}
