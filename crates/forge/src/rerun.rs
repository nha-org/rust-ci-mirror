//! `dash review rerun <pr>`: whether a reviewer reruns its precommit after the default branch moved, Dashboard.Orch.Rerun's
//! `run/3` and `verdict/3` in Rust (task rerun-fixtures-own; @nha, 2026-11-10: « remember we move to rust. no more elixir, so
//! if something needs fixing port and then fix »). Yes only when its new commits touch a file the PR changes, a file every
//! module reads, or a test fixture the PR's files name; else CI's run of the merge commit is the check. A test fixture is no
//! longer read as shared (review-pr-site_perso-666 reran CI for test/fixtures/blacksmith/runs-2026-10.json). The Elixir
//! module graph (a module one of the PR's files calls) is not ported: no-elixir freezes those files, so it can never fire
//! (rust/CLAUDE.md, rule 9). The older app's `check/2` (the hand-over) and `kept/2` still run its Elixir copy until ported.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Duration;

const USAGE: &str = "dash review rerun <pr>";
const FIXTURES: &str = "private/dashboard/test/fixtures/";

/// A git or gh answer: its trimmed output, or why it failed, naming the folder and exit status (task merge-rerun-own-claim).
fn cmd(bin: &str, args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> Result<String, String> {
    let at = cwd.display();
    match dash_process::answer(bin, args, Some(cwd), env, Duration::from_secs(60)) {
        Ok((s, out, _)) if s.success() => Ok(out.trim().to_string()),
        Ok((s, out, err)) => {
            let said = format!("{}{}", out.trim(), err.trim());
            let said = if said.is_empty() {
                "no output".into()
            } else {
                said
            };
            Err(format!(
                "review rerun: {bin} {} in {at} (exit {}): {said}",
                args.join(" "),
                s.code().unwrap_or(-1)
            ))
        }
        Err(why) => Err(format!(
            "review rerun: {bin} {}: {why:?} (in {at})",
            args.join(" ")
        )),
    }
}

/// git with new objects in a dash-owned overlay of the temp dir, the repo's own read as alternates: a reviewed repo's
/// gitdir is never written (review-pr-graal-466). An unwritable overlay leaves git to its own objects dir.
fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let common = cmd(
        "git",
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd,
        &[],
    );
    let Ok(dir) = common else {
        return cmd("git", args, cwd, &[]);
    };
    let mut h = DefaultHasher::new();
    dir.hash(&mut h);
    let mine = std::env::temp_dir().join(format!("dash-git-objects-{:x}", h.finish()));
    // a folder left there unwritable to us (another user, a sandbox) is no overlay either
    let probe = mine.join(format!(".probe-{}", std::process::id()));
    let writable = std::fs::create_dir_all(&mine).is_ok() && std::fs::write(&probe, "").is_ok();
    let _ = std::fs::remove_file(&probe);
    if !writable {
        return cmd("git", args, cwd, &[]);
    }
    let (mine, alt) = (mine.display().to_string(), format!("{dir}/objects"));
    cmd(
        "git",
        args,
        cwd,
        &[
            ("GIT_OBJECT_DIRECTORY", &mine),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES", &alt),
        ],
    )
}

fn fetch_sha(cwd: &Path, sha: &str) -> Result<String, String> {
    let args = [
        "-c",
        "gc.auto=0",
        "fetch",
        "-q",
        "--no-write-fetch-head",
        "--no-auto-maintenance",
        "origin",
        sha,
    ];
    git(cwd, &args)
}

/// `ref`'s sha on origin, its objects fetched into the overlay, no ref written.
fn fetch_remote(cwd: &Path, r: &str) -> Result<String, String> {
    let out = git(
        cwd,
        &[
            "ls-remote",
            "origin",
            &format!("refs/heads/{r}"),
            &format!("refs/{r}"),
        ],
    )?;
    let names = [format!("refs/{r}"), format!("refs/heads/{r}")];
    let sha = out
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(_, n)| names.iter().any(|x| x == n))
        .map(|(s, _)| s.to_string())
        .ok_or(format!("review rerun: origin has no {r}"))?;
    fetch_sha(cwd, &sha)?;
    Ok(sha)
}

fn files(cwd: &Path, from: &str, to: &str) -> Result<Vec<String>, String> {
    let out = git(cwd, &["diff", "--name-only", "--no-renames", from, to])?;
    Ok(out
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// What every module reads: deps, config, the test setup, migrations, lint and format rules, and the app's files no module
/// scan reads (scripts, templates, assets). A fixture under test/fixtures/ is not: only the files naming it read it.
pub fn shared(f: &str) -> bool {
    let name = f.rsplit('/').next().unwrap_or(f);
    let under = |dir: &str| f.starts_with(dir) || f.contains(&format!("/{dir}"));
    [
        "mix.exs",
        "mix.lock",
        "test_helper.exs",
        ".formatter.exs",
        ".credo.exs",
    ]
    .contains(&name)
        || (under("config/") && f.ends_with(".exs"))
        || under("test/support/")
        || under("priv/repo/migrations/")
        || under("lib/mix/tasks/")
        || (f.starts_with("private/dashboard/")
            && !f.starts_with(FIXTURES)
            && ![".ex", ".exs", ".md"].iter().any(|x| f.ends_with(x)))
}

/// The text a file reading fixture `f` holds: `fixtures/` then its folder there or, for a file there, its name up to its
/// first `-` (a test builds `linear-#{page}.tsv`). ponytail: a name built otherwise is missed, and CI's run of the merge
/// commit still checks it.
pub fn fixture_needle(f: &str) -> Option<String> {
    let rel = f.strip_prefix(FIXTURES)?;
    let head = match rel.split_once('/') {
        Some((dir, _)) => dir,
        None => rel.split_inclusive('-').next().unwrap_or(rel),
    };
    Some(format!("fixtures/{head}"))
}

/// The fixtures `theirs` changes that the PR's files, read at `rev`, name: one git grep of those files for all. A reader
/// the PR does not change (a sibling test, a helper) is not the PR's: main's own run checked it.
fn fixtures_read(
    cwd: &Path,
    theirs: &[String],
    ours: &[String],
    rev: &str,
) -> Result<Vec<String>, String> {
    let wanted: Vec<(&String, String)> = theirs
        .iter()
        .filter_map(|f| Some((f, fixture_needle(f)?)))
        .collect();
    if wanted.is_empty() || ours.is_empty() {
        return Ok(vec![]);
    }
    let mut args = vec!["--literal-pathspecs", "grep", "-I", "-h", "-o", "-F"];
    for (_, n) in &wanted {
        args.extend(["-e", n.as_str()]);
    }
    args.extend([rev, "--"]);
    args.extend(ours.iter().map(String::as_str));
    let said = match git(cwd, &args) {
        Ok(o) => o,
        // exit 1 is no match, whatever git warned beside it
        Err(why) if why.contains("(exit 1): ") => return Ok(vec![]),
        Err(why) => return Err(why),
    };
    // each line a match, the longest needle at its place: a shorter needle it starts with matched there too
    Ok(wanted
        .iter()
        .filter(|(_, n)| said.lines().any(|m| m.starts_with(n.as_str())))
        .map(|(f, _)| (*f).clone())
        .collect())
}

/// The verdict, a line opening on yes or no, for `rev` in `cwd` against the default branch's sha `r`.
pub fn verdict(cwd: &Path, branch: &str, r: &str, rev: &str) -> Result<String, String> {
    let base = git(cwd, &["merge-base", rev, r])?;
    let count = git(cwd, &["rev-list", "--count", &format!("{base}..{r}")])?;
    if count == "0" {
        return Ok(format!(
            "no: {branch} has no new commit since the PR's head merged it"
        ));
    }
    let (theirs, ours) = (files(cwd, &base, r)?, files(cwd, &base, rev)?);
    let new = format!("{branch}'s {count} new commits");
    // store data and docs need no new merge (review-dashboard-comments-fast merged twice for dumps alone)
    let same: Vec<&str> = theirs
        .iter()
        .filter(|f| {
            ours.contains(f)
                && !f.starts_with("private/store/")
                && !f.starts_with("private/docs/")
                && !f.ends_with(".md")
        })
        .map(String::as_str)
        .collect();
    if !same.is_empty() {
        return Ok(format!(
            "yes: {new} touch {}, which this PR changes",
            same.join(", ")
        ));
    }
    let shared: Vec<&str> = theirs
        .iter()
        .filter(|f| shared(f))
        .map(String::as_str)
        .collect();
    if !shared.is_empty() {
        return Ok(format!(
            "yes: {new} change {}, which every module reads",
            shared.join(", ")
        ));
    }
    let read = fixtures_read(cwd, &theirs, &ours, rev)?;
    Ok(if read.is_empty() {
        format!(
            "no: {new} touch no file this PR changes, no file every module reads and no fixture its files name: \
             merge on CI's run of the merge commit"
        )
    } else {
        format!(
            "yes: {new} change {}, which this PR's files read",
            read.join(", ")
        )
    })
}

/// `dash review rerun <pr>` from any worktree: a base change to the PR's files its head never met is a yes; else the
/// verdict on the head when `cwd` holds it, « unknown » and why when not. It reruns no CI. ponytail: main typed, as the
/// forge's own `base` is (`GitHub::on`), every Rust run being site_perso's; read forge.env's branch if another repo runs one.
pub fn run(pr: &str, cwd: &Path) -> Result<String, String> {
    let gh = crate::checks::gh_in("gh", cwd);
    // the stale read names the head too: a second gh pr view only when it did not answer
    let (head, unread) = match crate::checks::stale(&gh, pr, None) {
        Some((h, files)) if !files.is_empty() => {
            let at = &h[..h.len().min(7)];
            return Ok(format!(
                "yes: its base changed {} since its head {at} last met it",
                files.join(", ")
            ));
        }
        Some((h, _)) => (h, ""),
        None => {
            let h = gh(&[
                "pr",
                "view",
                pr,
                "--json",
                "headRefOid",
                "-q",
                ".headRefOid",
            ])
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .ok_or(format!("review rerun: gh pr view {pr} named no head"))?;
            (h, "\nstale run: not read: gh did not answer")
        }
    };
    let head = head.as_str();
    let branch = "main";
    let said = match git(cwd, &["merge-base", "--is-ancestor", head, "HEAD"]) {
        Err(why) => format!(
            "unknown: {} does not hold {pr}'s head {head} ({why}): merge that head in it first, or hand the PR over \
             from a worktree that holds it",
            cwd.display()
        ),
        Ok(_) => {
            let sha = fetch_remote(cwd, branch).map_err(|e| format!("{e}{unread}"))?;
            verdict(cwd, branch, &sha, head).map_err(|e| format!("{e}{unread}"))?
        }
    };
    Ok(format!("{said}{unread}"))
}

dash_core::submit! {
    dash_core::Command { usage: USAGE, words: &["review", "rerun"], run: Some(|_, rest, out| {
        // `--no-ci`, older briefs' word, changes nothing
        let ([pr] | [pr, "--no-ci"]) = rest else {
            eprintln!("usage: {USAGE}");
            std::process::exit(2)
        };
        let here = std::env::current_dir().unwrap_or_default();
        match run(pr, &here) {
            Ok(said) => writeln!(out, "{said}").unwrap_or_default(),
            Err(why) => {
                eprintln!("{why}");
                std::process::exit(1)
            }
        }
    }) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn put(dir: &Path, f: &str, text: &str) {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn commit(dir: &Path, m: &str) -> String {
        sh(dir, &["add", "-A"]);
        sh(
            dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                m,
            ],
        );
        sh(dir, &["rev-parse", "HEAD"])
    }

    /// A main move touching only a fixture is a no unless the PR's files read it (task rerun-fixtures-own).
    #[dash_test]
    fn a_main_move_of_a_fixture_alone_is_a_yes_only_when_the_pr_reads_it() {
        let dir = dash_meter::TestDir::new("rerun-fixtures");
        let t = dir.0.clone();
        sh(&t, &["init", "-q", "-b", "main"]);
        let fx = "private/dashboard/test/fixtures/blacksmith/runs-2026-10.json";
        put(&t, fx, "[]");
        put(&t, "rust/a.rs", "fn a() {}\n");
        // a sibling test the PR does not touch reads the fixture: it is that test's own, never the PR's
        put(
            &t,
            "private/dashboard/test/b_test.exs",
            "@f \"../fixtures/blacksmith/runs-2026-10.json\"\n",
        );
        let base = commit(&t, "base");
        put(&t, fx, "[1]");
        let main = commit(&t, "main moves the fixture");
        sh(&t, &["checkout", "-qb", "pr", &base]);
        // the fixture folder's word alone, the CI provider's name, is no read of it
        put(
            &t,
            "rust/a.rs",
            "fn a() -> &'static str { \"blacksmith\" }\n",
        );
        let blind = commit(&t, "pr reads no fixture");
        let said = verdict(&t, "main", &main, &blind).unwrap();
        assert!(said.starts_with("no: "), "{said}");
        put(
            &t,
            "rust/a.rs",
            "const F: &str = \"private/dashboard/test/fixtures/blacksmith/runs.json\";\n",
        );
        let reads = commit(&t, "pr reads it");
        let said = verdict(&t, "main", &main, &reads).unwrap();
        assert!(said.starts_with("yes: ") && said.contains(fx), "{said}");
    }

    #[dash_test]
    fn shared_keeps_the_app_files_but_not_its_fixtures() {
        assert!(shared("private/dashboard/mix.exs"));
        assert!(shared("private/dashboard/priv/static/app.js"));
        assert!(shared("private/dashboard/config/test.exs"));
        assert!(!shared("private/dashboard/lib/x.ex"));
        assert!(!shared(
            "private/dashboard/test/fixtures/blacksmith/runs-2026-10.json"
        ));
        let n = fixture_needle("private/dashboard/test/fixtures/linear-insights.tsv");
        assert_eq!(n.as_deref(), Some("fixtures/linear-"));
    }
}
