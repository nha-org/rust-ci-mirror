//! `dash hook pre-push <remote> <url>`, git's refs on stdin. Quiet on a pass (task prepush-quiet; @nha, 2026-10-29: « we know
//! agents LOVE to be verbose … And we measure. And improve. »): all it says goes to prepush.log in this checkout's git dir; a
//! pass prints one line naming it, at most 3 lines of the notes a step left (tests not run here, a red test excused as
//! main's: never a silent « passed »), then his decisions in full; a refusal prints the whole log.
use super::{Ctx, ran, short};
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

/// `dash hook pre-push <remote> <url>`, run by git with the pushed refs on stdin. Moved from the binary, as review_claim's
/// `run` (the binary only wires).
pub fn from_git(remote: &str, url: &str) -> ! {
    use std::io::Read;
    let mut ctx = super::from_hook_env("pre-push").unwrap_or_else(|why| {
        super::tell(&format!("pre-push: {why}\n"));
        std::process::exit(1)
    });
    let mut refs = String::new();
    let _ = std::io::stdin().read_to_string(&mut refs);
    std::process::exit(main(&mut ctx, remote, url, &refs))
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

/// The notes a pass keeps: each « pre-push: » line but a step's time, the total, « testing », the decisions' line and the PR
/// gates' passes on an open PR ([`pr_gates`]: nothing is left to do, and they took two of a pass's three lines; with no PR
/// yet its line stays, saying what the description and title must), with the lines under it (the excused tests' names).
pub fn notes(said: &str) -> Vec<&str> {
    let mut keep = false;
    let mut out = vec![];
    for l in said.lines() {
        if let Some(rest) = l.strip_prefix("pre-push: ") {
            let time = rest.ends_with(')') && rest.contains("s (exit ");
            keep = !(time
                || rest.starts_with("total ")
                || rest.starts_with("testing ")
                || rest.starts_with("these commits name a subject")
                || rest.contains("(read from PR #"));
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
    rust_tested: bool,
    note_checked: bool,
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
    let opened = done_pr(ctx, repo.as_deref(), refs)?;
    let mut st = State {
        rust_tested: false,
        note_checked: false,
    };
    for r in refs
        .iter()
        .filter(|r| r.local != ZERO && !r.local.is_empty())
    {
        precommit_result(ctx, &r.local)?;
        pr_gates(ctx, r, opened.get(&r.remote))?;
        sandbox_note(ctx, &r.local, &mut st)?;
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

/// A worktree's push onto a branch whose PR is merged or closed is refused, the PR named (task push-merged-pr): review-lane
/// pushed fixes onto merged PRs 1015, 1686 and 1649, and none reached main (@nha, 2026-10-29: « You fixed the mistake but
/// didn't find the cause and what could actually fix that class of errors right ? »). Only commits built on that PR's head:
/// a rerun's branch of the same name, cut from main again, passes, as does a branch with an open PR of this repository. gh
/// that does not answer in 10 s, or a head not held here, refuses nothing and says so. What it read is kept: each pushed
/// ref's open PR, by its remote ref, for [`pr_gates`] (one `gh pr list` a branch, never two).
pub fn done_pr(ctx: &Ctx, repo: Option<&str>, refs: &[Ref]) -> Result<Opened, String> {
    let mut opened = Opened::new();
    let Some(repo) = repo.filter(|_| ctx.is_worktree()) else {
        return Ok(opened);
    };
    for r in refs {
        let Some(b) = r.remote.strip_prefix("refs/heads/") else {
            continue;
        };
        if r.local.is_empty() || r.local == ZERO || b == ctx.main {
            continue;
        }
        let json = "number,state,url,headRefOid,isCrossRepository,title,body,createdAt,baseRefName";
        let args = ["pr", "list", "-R", repo, "--head", b, "--state", "all"];
        let a = super::ran_within(
            ctx.cmd(&ctx.top, "gh")
                .args(args)
                .args(["--limit", "20", "--json", json]),
            Some(std::time::Duration::from_secs(10)),
        );
        let prs = a
            .ok()
            .then(|| serde_json::from_str::<serde_json::Value>(a.said.trim()).ok())
            .flatten();
        let Some(prs) = prs else {
            ctx.say(&format!(
                "pre-push: {b}'s PRs unread, a merged one not checked: {}",
                a.said.trim()
            ));
            continue;
        };
        // only the PR's head under the pushed commits refuses, never the remote branch sitting on it: a squash merge keeps
        // the branch there, and a rerun cut from main is force-pushed over it (PR 1778's review). A head main holds (a merge
        // commit or a fast-forward, not our squash) is every fresh branch's base, so it never counts.
        let main = format!("origin/{}", ctx.main);
        let built_on = |head: &str| {
            if !ctx.git_ok(&["cat-file", "-e", &format!("{head}^{{commit}}")]) {
                ctx.say(&format!(
                    "pre-push: {b}'s PR head {} is not here, not checked",
                    short(head)
                ));
                return false;
            }
            ctx.git_ok(&["merge-base", "--is-ancestor", head, &r.local])
                && !ctx.git_ok(&["merge-base", "--is-ancestor", head, &main])
        };
        if let Some(why) = done_on(&prs, b, built_on) {
            return Err(why);
        }
        let open = (prs.as_array().into_iter().flatten())
            .find(|p| p["state"] == "OPEN" && p["isCrossRepository"] != true);
        opened.insert(r.remote.clone(), open.cloned());
    }
    Ok(opened)
}

/// Each pushed branch's open PR, `None` for a branch with none yet, by its remote ref; a branch gh did not answer for is
/// left out.
pub type Opened = std::collections::HashMap<String, Option<serde_json::Value>>;

/// The PR gate's text checks on what is pushed, before CI reds on them (task rust-prepush-gates: PRs 1847, 1851 and 1855
/// went red on these alone, 2026-11-20): the explainer gate on the branch's own range, read with its open PR's
/// description, else, with no PR yet, its commits' messages, which then say what the description must; the title gate on
/// that PR's title, and with no PR, CI's alone: the rule reads a title, which a branch has not. Only where `done_pr` read
/// the branch's PRs: a worktree's push to GitHub.
fn pr_gates(ctx: &Ctx, r: &Ref, opened: Option<&Option<serde_json::Value>>) -> Result<(), String> {
    let Some(pr) = opened else { return Ok(()) };
    let text = |k: &str| {
        (pr.as_ref())
            .and_then(|p| p[k].as_str())
            .unwrap_or("")
            .to_string()
    };
    // its PR's base at origin, main with no PR yet: the tip, as CI's base (HEAD^1), never the merge base. A file the base
    // already holds as the branch has it is no change there (the gate's `still`), so a branch whose work main
    // squash-merged is not refused here for it, nor a PR onto another branch for that branch's changes
    let onto = Some(text("baseRefName")).filter(|b| !b.is_empty());
    let base = format!("origin/{}", onto.as_deref().unwrap_or(&ctx.main));
    if !ctx.git_ok(&["merge-base", &base, &r.local]) {
        return Ok(());
    }
    let (body, read) = match pr {
        Some(p) => (text("body"), format!("PR #{}'s description", p["number"])),
        None => (
            (ctx.git(&["log", "--format=%B", &format!("{base}..{}", r.local)])).unwrap_or_default(),
            "the pushed commits' messages, no PR being open yet: what passes there goes in its description too, and its \
             title reads [fix|feat|chore][<task-id>] <title>, which CI checks"
                .into(),
        ),
    };
    let created = text("createdAt");
    let git = |a: &[&str]| ctx.git(a);
    match super::pr_gates::explainer(&git, &base, &r.local, &body, &created) {
        Ok(line) => ctx.say(&format!("pre-push: {line} (read from {read})")),
        Err(why) => {
            return Err(format!(
                "pre-push: {why}\n(read from {read}; CI runs the same check)"
            ));
        }
    }
    // no PR yet: the rule reads a title, which a branch has not, and the line above says what CI will ask of it
    if pr.is_none() {
        return Ok(());
    }
    let line = super::pr_gates::title(&text("title"), &body, &created)
        .map_err(|why| format!("pre-push: {why}"))?;
    ctx.say(&format!("pre-push: {line} (read from {read})"));
    Ok(())
}

/// The refusal for a push to `branch` whose commits are `built_on` the head of a merged or closed PR among `prs` (gh's
/// `pr list --json number,state,url,headRefOid,isCrossRepository`), when none is open. A fork's PR of the same branch name
/// is not this branch's.
pub fn done_on(
    prs: &serde_json::Value,
    branch: &str,
    built_on: impl Fn(&str) -> bool,
) -> Option<String> {
    let prs: Vec<&serde_json::Value> = prs
        .as_array()?
        .iter()
        .filter(|p| p["isCrossRepository"] != true)
        .collect();
    let field = |p: &serde_json::Value, k: &str| p[k].as_str().unwrap_or("").to_string();
    if prs.iter().any(|p| field(p, "state") == "OPEN") {
        return None;
    }
    let p = prs.into_iter().find(|p| {
        let head = field(p, "headRefOid");
        !head.is_empty() && built_on(&head)
    })?;
    let (n, state) = (&p["number"], field(p, "state").to_lowercase());
    let way = if state == "closed" {
        format!("Run gh pr reopen {n} first, or push to a new branch and open a PR there")
    } else {
        format!("Push to a new branch (git push origin HEAD:{branch}-2) and open a PR there")
    };
    Some(format!(
        "pre-push: {branch}'s PR #{n} is {state} ({}): a push there reaches no main. {way}.",
        field(p, "url")
    ))
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
    // a push with rust/ untouched too: a file outside it that a crate's test reads runs that crate (PR 1868's bin/dash
    // alone reddened main; task rust-affected-outside-paths)
    use crate::affected;
    let crates = affected::of_repo(
        &rust,
        &base,
        &std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()),
    )
    .map_err(|e| format!("pre-push: {e}"))?;
    if crates.is_empty() {
        return Ok(());
    }
    st.rust_tested = true;
    // « testing » ends the note before it: cargo's lines are the log's, never the 3 kept at exit
    ctx.say(&format!(
        "pre-push: testing the Rust crates {local} affects"
    ));
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
    // The build has its own bound, 60 s: a cold build of 30 crates took 130 s of a 138 s push (2026-11-11). Cut, its
    // finished crates stay in sccache, so the next push builds on from there; CI runs the tests meanwhile.
    let build_bound = super::env_secs("PREPUSH_RUST_BUDGET", 60);
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
    let budget = super::env_secs("PREPUSH_BUDGET", 60);
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

    /// done_on refuses only commits built on a merged or closed PR's head, and never while the branch has an open PR.
    #[dash_test]
    fn a_push_onto_a_done_pr_is_refused_an_open_or_unrelated_one_passes() {
        let pr = |n: u64, state: &str, head: &str| serde_json::json!({"number": n, "state": state, "url": format!("u/{n}"), "headRefOid": head});
        let on_a = |h: &str| h == "a";
        let merged = serde_json::json!([pr(1649, "MERGED", "a")]);
        assert_eq!(
            done_on(&merged, "task/x", on_a).as_deref(),
            Some(
                "pre-push: task/x's PR #1649 is merged (u/1649): a push there reaches no main. Push to a new branch (git push origin HEAD:task/x-2) and open a PR there."
            )
        );
        let reopened = serde_json::json!([pr(1649, "MERGED", "a"), pr(1700, "OPEN", "b")]);
        assert_eq!(done_on(&reopened, "task/x", on_a), None, "an open PR");
        let mut fork = pr(1700, "OPEN", "b");
        fork["isCrossRepository"] = true.into();
        let forked = serde_json::json!([pr(1649, "MERGED", "a"), fork]);
        assert!(
            done_on(&forked, "task/x", on_a).is_some(),
            "a fork's open PR"
        );
        let rerun = serde_json::json!([pr(1649, "CLOSED", "c")]);
        assert_eq!(done_on(&rerun, "task/x", on_a), None, "cut from main again");
        assert_eq!(done_on(&serde_json::json!([]), "task/x", on_a), None);
    }

    #[dash_test]
    fn a_pass_keeps_its_notes_never_its_times() {
        let said = "pre-push: compile 3s (exit 0)\npre-push: explainer-gate: no behaviour changed (read from PR #7's description)\npre-push: pr-title-gate: title follows [fix|feat|chore][<task-id>] (read from PR #7's description)\npre-push: these tests fail at the merge-base abc too:\ntest x (A)\npre-push: total 9s\npre-push: testing abc\ncargo line\n";
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
