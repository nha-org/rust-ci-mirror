//! No change to an Elixir file reaches main: Rust takes over (@nha, 2026-11-10: « no more elixir, so if something needs
//! fixing port and then fix », « no. I suggest no diffs on ex or exs at all, rust HAS to take over »). An Elixir file (the
//! app's folders, and any .ex, .exs or .heex anywhere) added, changed or renamed fails; only a whole file deleted passes, the
//! way a part leaves once Rust holds it. One list for both callers: the pre-commit's no-elixir step on what is staged
//! (`staged`), and the PR gate's `dash ci no-elixir <base> <head>` on a pull request's own change (`range`). It was
//! bin/ci/no-elixir.sh; its first form let a file shrink, and the orchestrator grew a private/docs .exs twice.
use super::Ctx;
use std::path::Path;

/// git's pathspec `*` spans folders.
const PATHS: [&str; 8] = [
    "private/dashboard/lib",
    "private/dashboard/test",
    "private/dashboard/config",
    "private/dashboard/assets",
    "private/dashboard/mix.exs",
    "*.ex",
    "*.exs",
    "*.heex",
];

/// Whether the gate refuses a change to `path`: one of `PATHS`, read the way git's pathspec reads it (task
/// rust-style-no-elixir-edit: `dash task add` refuses a check that would edit one).
pub fn elixir(path: &str) -> bool {
    PATHS.iter().any(|p| match p.strip_prefix('*') {
        Some(ext) => path.ends_with(ext) && path.len() > ext.len(),
        None => path == *p || path.starts_with(&format!("{p}/")),
    })
}

fn touched(
    git: impl Fn(&[&str]) -> Result<String, String>,
    range: &[&str],
) -> Result<Vec<String>, String> {
    let mut args = vec!["diff", "--name-status", "--diff-filter=ACMRT"];
    args.extend_from_slice(range);
    args.push("--");
    args.extend_from_slice(&PATHS);
    Ok(git(&args)?
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// What is staged. In a merge of main the index is held to MERGE_HEAD, not HEAD: main's own Elixir edits are not the
/// branch's, and against HEAD they refused every merge of main (rust-automations' merge for PR 1370, 2026-11-10). A staged
/// file whose mode and content are origin/<main>'s is no change, whatever HEAD holds: a branch taking its own Elixir edit
/// back out commits, as does main's newer file staged by hand (rust-search-port, 2026-11-10). A stale origin/<main> is CI's
/// to catch.
pub fn staged(ctx: &Ctx) -> Result<(), String> {
    let merge_head = ctx.rev("MERGE_HEAD");
    let mut range = vec!["--cached"];
    range.extend(merge_head.as_deref());
    let mut lines = touched(|a| ctx.git(a), &range)?;
    let main = format!("origin/{}", ctx.main);
    if ctx.rev(&main).is_some() {
        lines.retain(|line| {
            // the staged path, a rename's second
            let path = line.rsplit('\t').next().unwrap_or_default();
            let staged = ctx.git(&["ls-files", "-s", "--", path]).unwrap_or_default();
            let on_main = ctx.git(&["ls-tree", &main, "--", path]).unwrap_or_default();
            let mode_sha = |s: &str, i: usize| {
                let w: Vec<String> = s.split_whitespace().map(String::from).collect();
                (w.first().cloned(), w.get(i).cloned())
            };
            mode_sha(&staged, 1) != mode_sha(&on_main, 2)
        });
    }
    verdict(&lines)
}

/// A pull request's own change, `<base>..<head>` (explainer-gate.yml: HEAD^1 HEAD).
pub fn range(top: &Path, base: &str, head: &str) -> Result<(), String> {
    // a failing git still names its step: the gate tees it into the shared ci-failure.log (/code-review)
    verdict(
        &touched(|a| super::git_in(top, a), &[base, head])
            .map_err(|e| format!("no-elixir: {e}"))?,
    )
}

fn verdict(lines: &[String]) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    let files: Vec<String> = lines.iter().map(|l| format!("  {l}")).collect();
    Err(format!(
        "refused: Elixir is being retired. No change to an Elixir file is committed or merged, only a whole file deleted.
These Elixir files change:
{}
Move it to Rust instead, in this order:
  1. Port the part you need: a module in rust/crates/<crate>, its test, and its dash command if it has one.
  2. Make your fix there, in Rust; the front door serves Rust's route first, so the Elixir file can stay as it is.
  3. Delete the Elixir file whole once nothing calls it.
Then take the Elixir edit out (git restore --staged <file>, or revert it on the branch). There is no flag to skip this.
Why: « no diffs on ex or exs at all, rust HAS to take over » (@nha, 2026-11-10; CLAUDE.md, Coordination 5).",
        files.join("\n")
    ))
}
