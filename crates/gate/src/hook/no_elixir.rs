//! No change to an Elixir file reaches main: Rust takes over (@nha, 2026-11-10: « no more elixir, so if something needs
//! fixing port and then fix », « no. I suggest no diffs on ex or exs at all, rust HAS to take over »). An Elixir file (the
//! app's folders, and any .ex, .exs or .heex anywhere) added, changed or renamed fails; only a whole file deleted passes, the
//! way a part leaves once Rust holds it. archive/ is frozen the same way, and a pure move into it passes (RFC-0025 section
//! 4.2). One list for both callers: the pre-commit's no-elixir step on what is staged
//! (`staged`), and the PR gate's `dash ci no-elixir <base> <head>` on a pull request's own change (`range`). It was
//! bin/ci/no-elixir.sh; its first form let a file shrink, and the orchestrator grew a private/docs .exs twice.
use super::Ctx;
use std::path::Path;

/// git's pathspec `*` spans folders. archive/ is frozen too ([`in_archive`]), read in any case.
pub const PATHS: [&str; 8] = [
    "private/dashboard/lib",
    "private/dashboard/test",
    "private/dashboard/config",
    "private/dashboard/assets",
    "private/dashboard/mix.exs",
    "*.ex",
    "*.exs",
    "*.heex",
];

/// The archive's folder: the old code as it was (dash/rfcs/RFC-0025, section 4.2), frozen like the app, entered only by a
/// pure move.
pub const ARCHIVE: &str = "archive";

/// Whether `path` is archive/ or under it, in any case: a case-insensitive file system holds one folder for Archive/ and
/// archive/ (/code-review of PR 1787).
pub fn in_archive(path: &str) -> bool {
    path.get(..ARCHIVE.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(ARCHIVE))
        && matches!(path.as_bytes().get(ARCHIVE.len()), None | Some(b'/'))
}

/// Whether `path` is in archive/ or one of PATHS' folders, which the gate freezes whatever the file.
pub fn frozen(path: &str) -> bool {
    in_archive(path)
        || PATHS
            .iter()
            .filter(|p| !p.starts_with('*'))
            .any(|p| path == *p || path.starts_with(&format!("{p}/")))
}

/// Whether the gate refuses a change to `path`: one of `PATHS`, read the way git's pathspec reads it (task
/// rust-style-no-elixir-edit: `dash task add` refuses a check that would edit one).
pub fn elixir(path: &str) -> bool {
    frozen(path)
        || PATHS
            .iter()
            .filter_map(|p| p.strip_prefix('*'))
            .any(|ext| path.ends_with(ext) && path.len() > ext.len())
}

/// The changes `range` makes to an Elixir path, each `<status>\t<path>` or `R<score>\t<old>\t<new>`, judged on the new
/// path: a file moved out of PATHS is the delete and the add git's pathspec read it as. A pure move into archive/ (@nha,
/// 2026-11-18: « I suggest archiving the elixir and old dash code (move to archive/) ») passes: `R100`, the same mode, a
/// regular file on both sides. No pathspec: git pairs a rename only inside it, and a file moved from outside PATHS showed
/// as an add.
fn touched(
    git: impl Fn(&[&str]) -> Result<String, String>,
    range: &[&str],
) -> Result<Vec<String>, String> {
    // -M whatever diff.renames says; -z so a path git would quote is read as it is
    let mut args = vec!["diff", "--raw", "-z", "-M", "--diff-filter=ACMRT"];
    args.extend_from_slice(range);
    let out = git(&args)?;
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    let mut lines = vec![];
    // « :<old mode> <new mode> <old sha> <new sha> <status> NUL <path> [NUL <new path>] NUL »
    while let Some(meta) = fields.next() {
        let m: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let [old_mode, new_mode, _, _, status] = m[..] else {
            continue;
        };
        let paths: Vec<&str> = match status.starts_with(['R', 'C']) {
            true => fields.by_ref().take(2).collect(),
            false => fields.next().into_iter().collect(),
        };
        let new = paths.last().copied().unwrap_or_default();
        let pure =
            status == "R100" && old_mode == new_mode && matches!(new_mode, "100644" | "100755");
        if elixir(new) && !(pure && in_archive(new)) {
            lines.push(format!("{status}\t{}", paths.join("\t")));
        }
    }
    Ok(lines)
}

/// What is staged. In a merge of main the index is held to main's side ([`Ctx::merged_in`]), not HEAD: main's
/// own Elixir edits are not the branch's, and against HEAD they refused every merge of main (rust-automations' merge for PR
/// 1370, 2026-11-10). A merge of another branch stays on HEAD: against any MERGE_HEAD, its Elixir edit passed (/code-review
/// of PR 1677, task no-elixir-merged-in). A staged file whose mode and content are origin/<main>'s is no change, whatever
/// HEAD holds: a branch taking its own Elixir edit back out commits, as does main's newer file staged by hand
/// (rust-search-port, 2026-11-10). A stale origin/<main> reads a merge of main as another branch's, so the refusal
/// says to fetch first; a missed edit is CI's to catch.
pub fn staged(ctx: &Ctx) -> Result<(), String> {
    let merge_head = ctx.merged_in();
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
    // a merge whose side origin's main lacks: main itself when origin is stale (/code-review of PR 1681: the hint showed on
    // every refusal, a plain commit's and CI's too)
    let stale = ctx.rev("MERGE_HEAD").is_some() && merge_head.is_none();
    let hint = format!(
        "This merge's side is not on {main}: if it is main, git fetch origin first, then commit again.\n"
    );
    verdict(&lines, if stale { &hint } else { "" })
}

/// A pull request's own change, `<base>..<head>` (explainer-gate.yml: HEAD^1 HEAD).
pub fn range(top: &Path, base: &str, head: &str) -> Result<(), String> {
    // a failing git still names its step: the gate tees it into the shared ci-failure.log (/code-review)
    verdict(
        &touched(|a| super::git_in(top, a), &[base, head])
            .map_err(|e| format!("no-elixir: {e}"))?,
        "",
    )
}

fn verdict(lines: &[String], hint: &str) -> Result<(), String> {
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
{hint}Why: « no diffs on ex or exs at all, rust HAS to take over » (@nha, 2026-11-10; CLAUDE.md, Coordination 5).",
        files.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// What `touched` keeps of git's `diff --raw -z` records, « :<modes> <shas> <status> NUL <path> [NUL <path>] NUL ».
    fn kept(records: &[(&str, &str, &str, &[&str])]) -> Vec<String> {
        let raw: String = records
            .iter()
            .map(|(old, new, status, paths)| {
                format!(
                    ":{old} {new} 1111111 2222222 {status}\0{}\0",
                    paths.join("\0")
                )
            })
            .collect();
        touched(|_| Ok(raw.clone()), &[]).expect("read")
    }

    /// RFC-0025 section 4.2, the reviewed rule: a move into archive/ passes only as the same bytes, the same mode and a
    /// regular file on both sides; archive/ is read in any case; a frozen file moved outside archive/ is a delete plus an
    /// add at its new path.
    #[dash_test]
    fn only_a_pure_move_of_a_regular_file_enters_the_archive() {
        let file = "100644";
        assert!(
            kept(&[(
                file,
                file,
                "R100",
                &["private/dashboard/lib/a.ex", "archive/lib/a.ex"]
            )])
            .is_empty(),
            "a pure move into archive/ refused: it must pass"
        );
        assert!(
            kept(&[(file, file, "R100", &["bin/x", "Archive/bin/x"])]).is_empty(),
            "a pure move into Archive/ refused: it is archive/ on this file system"
        );
        for (old, new, status, why) in [
            (file, "100755", "R100", "chmod +x"),
            ("120000", "120000", "R100", "a symlink"),
            (file, file, "R097", "an edit"),
        ] {
            assert_eq!(
                kept(&[(
                    old,
                    new,
                    status,
                    &["private/dashboard/lib/a.ex", "archive/lib/a.ex"]
                )])
                .len(),
                1,
                "{why} entered the archive"
            );
        }
        assert_eq!(
            kept(&[("000000", file, "A", &["ARCHIVE/x"])]).len(),
            1,
            "an add under ARCHIVE/"
        );
        assert_eq!(
            kept(&[(file, file, "M", &["Archive/lib/a.ex"])]).len(),
            1,
            "an edit under Archive/"
        );
        assert!(
            kept(&[(
                file,
                file,
                "R100",
                &[
                    "private/dashboard/test/fixtures/x.json",
                    "rust/crates/forge/tests/fixtures/x.json"
                ]
            )])
            .is_empty(),
            "a fixture moved to a Rust crate is a delete and an add of no Elixir path"
        );
        assert_eq!(
            kept(&[(
                file,
                file,
                "R100",
                &["private/dashboard/lib/a.ex", "rust/a.ex"]
            )])
            .len(),
            1,
            "an .ex moved anywhere but archive/ is an add of an .ex"
        );
        assert!(!elixir("archived/x") && !in_archive("rust/archive/x") && in_archive("ARCHIVE"));
    }
}
