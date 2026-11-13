//! The script ratchet outside rust/ (task rust-scripts-port; @nha, 2026-11-10: « all these python scripts are also very
//! brittle. we want to move all logic to rust and have it well tested. », « there is no reason for elixir or python or bash
//! in this repo long-term. »). Every tracked .py, .sh, .bash or .zsh file, every file git keeps executable and every file
//! whose line 1 opens on #!, outside rust/, is counted from git's index against the one number in [`HELD`]: more fails;
//! fewer fails until a port lowers the number in the same commit; a number above the base's fails, so it never rises.
//! `dash scripts count --check --base <rev>` runs it, in the pre-commit (base HEAD) and the PR gate (explainer-gate.yml,
//! base HEAD^1). The port's order is rfc-rust-unified's section `scripts-port`. Rust's own scripts are lib.rs's `scripts`
//! rule; `Dashboard.Lint.scripts` keeps its own number in private/dashboard/priv/lint/scripts-held.txt until that module
//! is deleted.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The held number, from the repository's top.
pub const HELD: &str = "rust/scripts-held.txt";

/// The extensions that make a file a script, whatever its mode: lib.rs's rule on rust/ reads the same list.
pub const EXTS: [&str; 4] = ["py", "sh", "bash", "zsh"];

/// A script by its name or its executable bit alone; a #! line 1 makes one too ([`shebang`]).
pub fn named(mode: &str, path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str());
    mode == "100755" || ext.is_some_and(|e| EXTS.contains(&e))
}

/// A line 1 opening on #!, which `#![…]`, Rust's inner attribute, is not (a vendored crate's `#![allow(..)]`, task
/// rust-ui-hit-panic), as in [`count`]'s git grep.
pub fn shebang(head: &[u8]) -> bool {
    head.starts_with(b"#!") && head.get(2) != Some(&b'[')
}

/// A shell script shellcheck reads: a .sh or .bash name, a line 1 #! naming sh or bash as a word (`#!/usr/bin/env bash`,
/// never zsh), or a sourced file's line 1 [`DIRECTIVE`] (bin/approved-only), as Dashboard.Lint's `@shell_files` list and
/// `extensionless/2` picked by hand (task rust-lint-scripts-port).
/// No list: a script renamed or deleted leaves nothing behind to fail on, which was `listed_gone`'s whole job.
pub fn shell(path: &str, head: &[u8]) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str());
    if matches!(ext, Some("sh" | "bash")) {
        return true;
    }
    let line = head.split(|b| *b == b'\n').next().unwrap_or_default();
    // the directive's first word, other options after it (`shell=bash disable=SC2034`, /code-review)
    if let Some(dialect) = line.strip_prefix(DIRECTIVE.as_bytes()) {
        let word = dialect.split(u8::is_ascii_whitespace).next();
        return matches!(word, Some(b"sh" | b"bash"));
    }
    // a word as the Elixir rule's \b reads one: `_` inside it, so `my_sh` is none (/code-review)
    shebang(line)
        && line
            .split(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
            .any(|w| w == b"sh" || w == b"bash")
}

/// The dialect to name to shellcheck, which reads stdin and so never sees the name: none when line 1 is a #! or the
/// directive, which it reads itself; else the extension's (/code-review: a .bash with no #! failed SC2148).
fn dialect(path: &str, text: &[u8]) -> Option<&'static str> {
    if text.starts_with(b"#!") || text.starts_with(DIRECTIVE.as_bytes()) {
        return None;
    }
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("bash") => Some("bash"),
        Some("sh") => Some("sh"),
        _ => None,
    }
}

/// A sourced shell file's line 1, which no shebang or bit makes a script: shellcheck reads its dialect from it.
pub const DIRECTIVE: &str = "# shellcheck shell=";

/// What is not ours, counted but never shellchecked, as Dashboard.Lint's `@not_ours` and `@references`: vendored and copied
/// code (bin/vendor holds Blacksmith's and Entire's own install scripts), and the skills' illustrative references.
/// private/dashboard/assets/vendor/ is no-elixir's, frozen with all of assets.
const NOT_OURS: [&str; 4] = [
    ".claude/skills/diagram-design/",
    "private/replicator/originals/",
    "private/dashboard/priv/static/",
    "bin/vendor/",
];

/// Ours and editable: no-elixir's frozen folders (private/dashboard/test among them) refuse any edit, so a finding there
/// could never be fixed; mix lint never listed those but live-data-test, which passes and cannot change.
pub fn ours(path: &str) -> bool {
    let reference = path
        .strip_prefix(".claude/skills/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(_, rest)| rest.starts_with("references/"));
    let frozen = crate::hook::no_elixir::PATHS
        .iter()
        .filter(|p| !p.starts_with('*'))
        .any(|p| path == *p || path.starts_with(&format!("{p}/")));
    !reference && !frozen && !NOT_OURS.iter().any(|d| path.starts_with(d))
}

/// shellcheck as `mix lint` runs it (Dashboard.Lint's `@shellcheck`), on each `(path, text)` from stdin, so a staged text
/// is checked as it will be committed; `program` is shellcheck but in a test. From `top`, where a .shellcheckrc would be
/// read, as mix lint ran it. Every finding, each under its path.
pub fn shellcheck(program: &str, top: &Path, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    use std::io::Write;
    let mut bad = Vec::new();
    for (path, text) in files {
        let mut child = Command::new(program);
        if let Some(d) = dialect(path, text) {
            child.args(["-s", d]);
        }
        let mut child = child
            .args(["-e", "SC1091", "-"])
            .current_dir(top)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("{program}: {e} (bin/ci/lint-tools.sh install pins it)"))?;
        // a write cut short by an early exit is the exit's to report
        let _ = child.stdin.take().expect("piped").write_all(text);
        let out = child
            .wait_with_output()
            .map_err(|e| format!("{program} {path}: {e}"))?;
        if !out.status.success() {
            let said = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
            bad.push(format!("{path}: shellcheck fails\n{}", said.trim()));
        }
    }
    match bad.is_empty() {
        true => Ok(()),
        false => Err(bad.join("\n")),
    }
}

/// The shell scripts among `paths` as the index holds them, each with its staged text.
fn staged_shell(top: &Path, paths: &[String]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut found = Vec::new();
    // a .py or .zsh is never shell: no blob read for it (/code-review)
    let never = |p: &&String| {
        Path::new(p.as_str())
            .extension()
            .is_some_and(|e| e == "py" || e == "zsh")
    };
    for path in paths.iter().filter(|p| !never(p)) {
        let text = Command::new("git")
            .args(["-c", "core.fsmonitor=false", "show", &format!(":{path}")])
            .current_dir(top)
            .output()
            .map_err(|e| format!("git show :{path}: {e}"))?;
        if !text.status.success() {
            return Err(format!(
                "git show :{path}: {}",
                String::from_utf8_lossy(&text.stderr).trim()
            ));
        }
        if shell(path, &text.stdout) {
            found.push((path.clone(), text.stdout));
        }
    }
    Ok(found)
}

/// The scripts among `git ls-files --stage -z`'s entries, sorted, each once (a conflicted path holds a line per stage);
/// `shebangs` the paths whose line 1 opens on #!. A symlink or submodule is no script.
pub fn scripts(stage: &str, shebangs: &BTreeSet<String>) -> Vec<String> {
    let mut found = BTreeSet::new();
    for entry in stage.split('\0') {
        let Some((meta, path)) = entry.split_once('\t') else {
            continue;
        };
        let mode = meta.split(' ').next().unwrap_or_default();
        if path.starts_with("rust/") || !matches!(mode, "100644" | "100755") {
            continue;
        }
        if named(mode, path) || shebangs.contains(path) {
            found.insert(path.to_string());
        }
    }
    found.into_iter().collect()
}

/// `n` scripts against the number held now and the base's: the line to print, or why it fails.
pub fn verdict(n: usize, held: Option<usize>, base: Option<usize>) -> Result<String, String> {
    let said = |h| format!("{n} tracked scripts outside rust/, {h} held");
    match (held, base) {
        (None, _) => Err(format!("{HELD}: no count, write {n}")),
        (Some(h), Some(b)) if h > b => Err(format!(
            "{HELD} rose from {b} to {h}: the number only drops; port a script to Rust instead"
        )),
        (Some(h), _) if n > h => Err(format!(
            "{}: write it in Rust, a dash command with its test (CLAUDE.md, Coordination 5)",
            said(h)
        )),
        (Some(h), _) if n < h => Err(format!(
            "{}: lower it to {n} in {HELD}, in this commit",
            said(h)
        )),
        (Some(h), _) => Ok(said(h)),
    }
}

/// The page whose section [`SECTION`] lists every script in the wave it is ported in.
pub const ORDER: &str = "private/docs/rfc-rust-unified.html";
// Each tag put together with concat!: the gate's own source holds no tag literal (html::html_in_strings).
const SECTION: &str = concat!("<", "h3 id=\"scripts-port\">");
const PATHS: &str = concat!("<", "td class=\"paths\">");
const CELL_END: &str = concat!("<", "/td>");
const CODE: &str = concat!("<", "code>");
const CODE_END: &str = concat!("<", "/code>");
const NEXT: [&str; 3] = [
    concat!("<", "h2"),
    concat!("<", "h3"),
    concat!("<", "/section>"),
];

/// The paths the port's order lists: each `<code>` of a `<td class="paths">` cell, from [`SECTION`] to the next heading
/// or the section's end; none when the page has no such section.
pub fn listed(page: &str) -> Option<BTreeSet<String>> {
    let rest = &page[page.find(SECTION)? + SECTION.len()..];
    let end = NEXT
        .iter()
        .filter_map(|t| rest.find(t))
        .min()
        .unwrap_or(rest.len());
    let mut found = BTreeSet::new();
    for cell in rest[..end].split(PATHS).skip(1) {
        let cell = &cell[..cell.find(CELL_END).unwrap_or(cell.len())];
        for code in cell.split(CODE).skip(1) {
            if let Some((path, _)) = code.split_once(CODE_END) {
                let path = path
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&amp;", "&");
                found.insert(path);
            }
        }
    }
    Some(found)
}

/// Where the port's order and the scripts part: a script it does not list, a path it lists that is no script.
pub fn unlisted(paths: &[String], listed: &BTreeSet<String>) -> Vec<String> {
    let tracked: BTreeSet<&String> = paths.iter().collect();
    let missing = paths.iter().filter(|p| !listed.contains(*p)).map(|p| {
        format!("{p}: a script the port's order misses, add it to its wave in {ORDER}#scripts-port")
    });
    let gone = listed
        .iter()
        .filter(|p| !tracked.contains(p))
        .map(|p| format!("{p}: listed in {ORDER}#scripts-port, no longer a script: drop it there"));
    missing.chain(gone).collect()
}

/// A bare count only: « 196 (after x) » or -1 is none.
fn number(text: &str) -> Option<usize> {
    text.trim().parse().ok()
}

fn git(top: &Path, args: &[&str], ok: &[i32]) -> Result<String, String> {
    let out = Command::new("git")
        .args(["-c", "core.quotePath=false", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(top)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    match out.status.code() {
        Some(code) if ok.contains(&code) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        code => Err(format!(
            "git {} failed ({code:?}): {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// The repository holding the current folder, at its top.
pub fn top() -> Result<PathBuf, String> {
    let said = git(Path::new("."), &["rev-parse", "--show-toplevel"], &[0])?;
    Ok(PathBuf::from(said.trim()))
}

/// The scripts outside rust/ as git's index holds them (GIT_INDEX_FILE's, a commit's own, when set) and the number held
/// there. Line 1 is read from the index's blobs, never the worktree, so a staged #! counts as it will be committed.
pub fn count(top: &Path) -> Result<(Vec<String>, Option<usize>), String> {
    let stage = git(top, &["ls-files", "--stage", "-z"], &[0])?;
    // -G and --no-color pin it against grep.patternType and color.grep; -a keeps a binary-looking #! file; `#![` is Rust's
    // inner attribute, never a shebang (as Dashboard.Lint.shebangs). Exit 1 is no match.
    let found = git(
        top,
        &[
            "grep",
            "--cached",
            "-G",
            "--no-color",
            "-a",
            "-z",
            "-n",
            "--max-count=1",
            "-e",
            "^#!$",
            "-e",
            "^#![^[]",
        ],
        &[0, 1],
    )?;
    let held = match indexed(&stage, HELD) {
        true => number(&git(top, &["show", &format!(":{HELD}")], &[0])?),
        false => None,
    };
    Ok((scripts(&stage, &matched_line_one(&found)), held))
}

/// The paths of `git grep -z -n`'s records matched on line 1: « path NUL line NUL text LF », a path itself free to hold a
/// newline (as ls-files -z allows), so each record is read on from the NULs, never split on lines.
fn matched_line_one(found: &str) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let mut fields = found.split('\0');
    let mut path = fields.next();
    while let (Some(p), Some(line), Some(rest)) = (path, fields.next(), fields.next()) {
        if line == "1" {
            paths.insert(p.to_string());
        }
        path = rest.split_once('\n').map(|(_, next)| next);
    }
    paths
}

/// Whether the index listing holds `path`, at stage 0 or in a conflict.
fn indexed(stage: &str, path: &str) -> bool {
    stage
        .split('\0')
        .any(|e| e.split_once('\t').is_some_and(|(_, p)| p == path))
}

/// `dash scripts count`: each script, then the total; with `check`, the verdict alone, the number never above `base`'s,
/// and shellcheck on each shell script ([`shell`]) the index changes from `base`, or on every one with no base.
pub fn run(top: &Path, check: bool, base: Option<&str>) -> Result<String, String> {
    let (paths, held) = count(top)?;
    if !check {
        return Ok(format!(
            "{}\n{} tracked scripts outside rust/",
            paths.join("\n"),
            paths.len()
        ));
    }
    let rose = base.map(|rev| rose(top, ":", rev)).unwrap_or_default();
    // the base's scripts, as its port order lists them (its own check held them equal)
    let before = base.and_then(|rev| {
        listed(&git(top, &["show", &format!("{rev}:{ORDER}")], &[0]).unwrap_or_default())
            .map(|set| (rev, set))
    });
    let base_rev = base;
    let base = match base {
        None => None,
        Some(rev) => bound(top, rev)?,
    };
    // the order as this commit holds it, as the count is; a git that fails says why, never « no section »
    let stage = git(top, &["ls-files", "--stage", "-z", "--", ORDER], &[0])?;
    let order = match indexed(&stage, ORDER) {
        true => listed(&git(top, &["show", &format!(":{ORDER}")], &[0])?),
        false => None,
    };
    let said = verdict(paths.len(), held, base);
    let mut bad: Vec<String> = said.clone().err().into_iter().collect();
    match &order {
        Some(listed) => bad.extend(unlisted(&paths, listed)),
        None => bad.push(format!(
            "{ORDER}: no section scripts-port in the index, the port's order"
        )),
    }
    bad.extend(rose);
    // shellcheck on the shell scripts the index changes from the base (the pre-commit's), every one with no base: the
    // scripts and the sourced shell files beside them, each once, outside rust/ as the count is
    let directive = format!("^{DIRECTIVE}");
    let found = git(
        top,
        &[
            "grep", "--cached", "-l", "-z", "-G", "-e", &directive, "--", ".", ":!rust/",
        ],
        &[0, 1],
    )?;
    let sourced: BTreeSet<String> = paths
        .iter()
        .cloned()
        .chain(
            found
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(String::from),
        )
        .filter(|p| ours(p))
        .collect();
    // an unborn HEAD (a first commit; any other missing base failed in `bound`) changes every path
    let born = |rev: &str| {
        git(
            top,
            &["rev-parse", "--verify", "-q", &format!("{rev}^{{commit}}")],
            &[0],
        )
        .is_ok()
    };
    let linted: Vec<String> = match base_rev.filter(|rev| born(rev)) {
        Some(rev) => {
            let diff = git(
                top,
                &[
                    "diff",
                    "--cached",
                    "--name-only",
                    "-z",
                    "--no-renames",
                    "--diff-filter=ACMRT",
                    rev,
                ],
                &[0],
            )?;
            let changed: BTreeSet<&str> = diff.split('\0').collect();
            sourced
                .into_iter()
                .filter(|p| changed.contains(p.as_str()))
                .collect()
        }
        None => sourced.into_iter().collect(),
    };
    if let Err(why) = staged_shell(top, &linted).and_then(|f| shellcheck("shellcheck", top, &f)) {
        bad.push(why);
    }
    match bad.is_empty() {
        true => said.map(|s| {
            s + &before
                .map(|(rev, b)| since(rev, &b, &paths))
                .unwrap_or_default()
        }),
        false => Err(bad.join("\n")),
    }
}

/// Each script gone since `rev` (`- path`) and each new one (`+ path`) under a line with both counts, so a drop in the number
/// shows which files made it (the PM check could not tie rust-hooks-rest-port's 187 to 180 to its deletions, 2026-11-12).
/// Empty when nothing changed.
pub fn since(rev: &str, before: &BTreeSet<String>, now: &[String]) -> String {
    let now: BTreeSet<&str> = now.iter().map(String::as_str).collect();
    let gone: Vec<&str> = before
        .iter()
        .map(String::as_str)
        .filter(|p| !now.contains(p))
        .collect();
    let new: Vec<&str> = now
        .iter()
        .copied()
        .filter(|p| !before.contains(*p))
        .collect();
    if gone.is_empty() && new.is_empty() {
        return String::new();
    }
    let lines = gone
        .iter()
        .map(|p| format!("\n- {p}"))
        .chain(new.iter().map(|p| format!("\n+ {p}")));
    format!(
        "\nsince {rev}: {} gone, {} new{}",
        gone.len(),
        new.len(),
        lines.collect::<String>()
    )
}

/// `dash scripts count --check --base <rev> --changed`: the same verdict from `rev`'s port order and HEAD's own change alone,
/// for a gate whose checkout fetches no blob it does not read (explainer-gate.yml on the VPS, task rust-ci-off-mac: no
/// private/ file reaches that box but a pull request's own). `rev`'s order lists exactly its scripts, which its own check
/// held; HEAD's scripts are those, less each path the diff makes no script, plus each it makes one. Line 1 is read only of a
/// changed path its name does not already make a script.
pub fn changed(top: &Path, rev: &str) -> Result<String, String> {
    let base = bound(top, rev)?;
    let shown = |r: &str, path: &str| git(top, &["show", &format!("{r}:{path}")], &[0]);
    let before = listed(&shown(rev, ORDER).unwrap_or_default()).ok_or(format!(
        "{ORDER}: no section scripts-port at {rev}, the port's order"
    ))?;
    let raw = git(
        top,
        &[
            "diff",
            "--raw",
            "-z",
            "--no-renames",
            "--no-abbrev",
            rev,
            "HEAD",
        ],
        &[0],
    )?;
    // « :old new oldsha newsha status NUL path NUL », the path free to hold any byte but NUL
    let script = |mode: &str, r: &str, path: &str| -> Result<bool, String> {
        if !matches!(mode, "100644" | "100755") {
            return Ok(false);
        }
        if named(mode, path) {
            return Ok(true);
        }
        let blob = Command::new("git")
            .args([
                "-c",
                "core.fsmonitor=false",
                "cat-file",
                "blob",
                &format!("{r}:{path}"),
            ])
            .current_dir(top)
            .output()
            .map_err(|e| format!("git cat-file {r}:{path}: {e}"))?;
        match blob.status.success() {
            true => Ok(shebang(&blob.stdout)),
            false => Err(format!(
                "git cat-file {r}:{path}: {}",
                String::from_utf8_lossy(&blob.stderr).trim()
            )),
        }
    };
    let mut paths: BTreeSet<String> = before.clone();
    let mut fields = raw.split('\0');
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let m: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        // a changed path leaves the set whatever it was, and comes back only as HEAD makes it a script: its base side
        // is never read
        let [_, new, ..] = m[..] else { continue };
        if path.starts_with("rust/") {
            continue;
        }
        paths.remove(path);
        if script(new, "HEAD", path)? {
            paths.insert(path.to_string());
        }
    }
    let paths: Vec<String> = paths.into_iter().collect();
    let held = number(&shown("HEAD", HELD).unwrap_or_default());
    let order = listed(&shown("HEAD", ORDER).unwrap_or_default());
    let said = verdict(paths.len(), held, base);
    let mut bad: Vec<String> = said.clone().err().into_iter().collect();
    match &order {
        Some(listed) => bad.extend(unlisted(&paths, listed)),
        None => bad.push(format!(
            "{ORDER}: no section scripts-port at HEAD, the port's order"
        )),
    }
    bad.extend(rose(top, "HEAD:", rev));
    match bad.is_empty() {
        true => said.map(|s| s + &since(rev, &before, &paths)),
        false => Err(bad.join("\n")),
    }
}

/// Where the inline script bytes are held (inline_js.rs), from the repository's top.
pub const JS_HELD: &str = "rust/inline-js-held.txt";

/// Where the browser test group's count is held (browser.rs), from the repository's top.
pub const BROWSER_HELD: &str = "rust/browser-tests-held.txt";

/// Each held number rust/'s gate keeps, and what to do in place of raising it.
const DROPS: [(&str, &str); 2] = [
    (
        JS_HELD,
        "the inline script bytes only drop; write the behaviour as Dioxus handlers",
    ),
    (
        BROWSER_HELD,
        "the browser tests only drop; test in-process (rust/CLAUDE.md rule 1)",
    ),
];

/// Each of [`DROPS`] held at `now` (`:` the index, `HEAD:` a commit) above `rev`'s: they only drop (tasks
/// rust-components-no-js, rust-browser-tests-fewer). A file with no number on either side adds no line: the rust-gate
/// test reads a missing one.
fn rose(top: &Path, now: &str, rev: &str) -> Vec<String> {
    let held = |at: &str, file: &str| {
        git(top, &["show", &format!("{at}{file}")], &[0])
            .ok()
            .and_then(|t| number(&t))
    };
    DROPS
        .iter()
        .filter_map(
            |(file, instead)| match (held(now, file), held(&format!("{rev}:"), file)) {
                (Some(n), Some(b)) if n > b => {
                    Some(format!("{file} rose from {b} to {n}: {instead}"))
                }
                _ => None,
            },
        )
        .collect()
}

/// The number `rev` holds, the bound this one may not pass: none before the number existed, or on a branch with no commit
/// yet (a first commit has no base); a name that is no commit fails.
fn bound(top: &Path, rev: &str) -> Result<Option<usize>, String> {
    let commit = format!("{rev}^{{commit}}");
    if git(top, &["rev-parse", "--verify", "-q", &commit], &[0]).is_err() {
        let unborn = git(top, &["rev-parse", "--verify", "-q", "HEAD"], &[0]).is_err();
        return match unborn && rev == "HEAD" {
            true => Ok(None),
            false => Err(format!("--base {rev}: no such commit")),
        };
    }
    match git(top, &["cat-file", "-e", &format!("{rev}:{HELD}")], &[0]) {
        Ok(_) => Ok(number(&git(
            top,
            &["show", &format!("{rev}:{HELD}")],
            &[0],
        )?)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// A page written with `[` for each tag's opening: the gate's source holds no tag literal (html::html_in_strings).
    fn html(text: &str) -> String {
        text.replace('[', "<")
    }

    #[dash_test]
    fn a_script_is_its_extension_its_bit_or_its_shebang_outside_rust() {
        let stage = [
            "100644 a 0\tbin/a.py",
            "100644 a 0\tbin/b.sh",
            "100644 a 0\tbin/c.zsh",
            "100755 a 0\tbin/tool",
            "100644 a 0\tbin/plain-shebang",
            "100644 a 0\tREADME.md",
            "100755 a 0\trust/bin/dash",
            "100644 a 0\trust/ci/x.sh",
            "120000 a 0\tbin/link.sh",
            "160000 a 0\tvendor/sub.py",
            "100644 a 1\tbin/conflict.py",
            "100755 a 2\tbin/conflict.py",
        ]
        .join("\0");
        let shebangs = BTreeSet::from(["bin/plain-shebang".to_string(), "rust/x".to_string()]);
        assert_eq!(
            scripts(&stage, &shebangs),
            [
                "bin/a.py",
                "bin/b.sh",
                "bin/c.zsh",
                "bin/conflict.py",
                "bin/plain-shebang",
                "bin/tool"
            ]
        );
    }

    /// Done-when 1 of rust-lint-scripts-port: the shell scripts are known by name or shebang, no list.
    #[dash_test]
    fn a_shell_script_is_its_sh_name_or_a_sh_or_bash_shebang() {
        for (path, head) in [
            ("bin/x.sh", &b"echo\n"[..]),
            ("bin/x.bash", b""),
            ("bin/hook", b"#!/bin/sh\necho"),
            ("bin/hook", b"#!/bin/sh -e\n"),
            ("bin/hook", b"#!/usr/bin/env bash\n"),
            ("bin/hook", b"#!/opt/homebrew/bin/bash"),
            ("bin/approved-only", b"# shellcheck shell=bash\n# sourced"),
            ("bin/lib", b"# shellcheck shell=bash disable=SC2034\n"),
        ] {
            assert!(
                shell(path, head),
                "{path} {:?}",
                String::from_utf8_lossy(head)
            );
        }
        for (path, head) in [
            ("bin/tool", &b"#!/usr/bin/env python3\n"[..]),
            ("bin/tool", b"#!/bin/zsh\n"),
            ("bin/tool", b"#![allow(x)] sh\n"),
            ("bin/tool", b"echo\n#!/bin/sh\n"),
            ("bin/tool.zsh", b"#!/bin/zsh\n"),
            ("bin/shell-notes", b"plain\n"),
            ("bin/lib", b"# shellcheck shell=zsh\n"),
            ("bin/lib", b"# a note\n# shellcheck shell=sh\n"),
            ("bin/tool", b"#!/usr/local/bin/my_sh\n"),
        ] {
            assert!(
                !shell(path, head),
                "{path} {:?}",
                String::from_utf8_lossy(head)
            );
        }
        assert!(ours("bin/hook") && ours(".claude/skills/a/run.sh") && ours("x/bin/vendor/a.sh"));
        for path in [
            "bin/vendor/get-blacksmith.sh",
            ".claude/skills/a/references/x.sh",
            "private/replicator/originals/a.sh",
            "private/dashboard/test/scripts/phone-links-check",
        ] {
            assert!(!ours(path), "{path}");
        }
        // stdin hides the name: the extension's dialect is named, a #! or the directive left to shellcheck
        assert_eq!(dialect("lib.bash", b"f() { :; }\n"), Some("bash"));
        assert_eq!(dialect("a.sh", b"echo\n"), Some("sh"));
        assert_eq!(dialect("a.sh", b"#!/bin/bash\n"), None);
        assert_eq!(dialect("lib", b"# shellcheck shell=bash\n"), None);
    }

    /// Done-when 1 of rust-lint-scripts-port: every path of Dashboard.Lint's `@shell_files` still in the tree is a shell
    /// script by [`shell`], so the Rust check needs no list to replace it. Where the monorepo is: the mirror holds rust/ alone.
    #[dash_test]
    fn every_listed_shell_file_of_the_elixir_lint_is_found_without_its_list() {
        let Some(top) = dash_meter::monorepo() else {
            return;
        };
        let lint = std::fs::read_to_string(top.join("private/dashboard/lib/dashboard/lint.ex"))
            .expect("lint.ex: once it is deleted, drop this test with it");
        let line = lint
            .lines()
            .find_map(|l| l.trim().strip_prefix("@shell_files ~w("))
            .expect("lint.ex's @shell_files");
        let listed: Vec<&str> = line.trim_end_matches(')').split_whitespace().collect();
        assert!(listed.len() > 10, "{listed:?}");
        let missed: Vec<&str> = listed
            .into_iter()
            .filter(|p| match std::fs::read(top.join(p)) {
                Ok(text) => !shell(p, &text),
                Err(_) => false,
            })
            .collect();
        assert!(
            missed.is_empty(),
            "listed in lint.ex, not found by shell(): {missed:?}"
        );
    }

    #[dash_test]
    fn more_fails_fewer_fails_until_lowered_and_the_number_never_rises() {
        assert_eq!(
            verdict(4, Some(4), Some(4)),
            Ok("4 tracked scripts outside rust/, 4 held".into())
        );
        assert_eq!(
            verdict(4, Some(4), None),
            Ok("4 tracked scripts outside rust/, 4 held".into())
        );
        assert!(
            verdict(5, Some(4), Some(4))
                .unwrap_err()
                .contains("write it in Rust")
        );
        assert!(
            verdict(3, Some(4), Some(4))
                .unwrap_err()
                .contains("lower it to 3")
        );
        assert_eq!(
            verdict(3, Some(3), Some(4)),
            Ok("3 tracked scripts outside rust/, 3 held".into())
        );
        assert!(
            verdict(5, Some(5), Some(4))
                .unwrap_err()
                .contains("rose from 4 to 5")
        );
        assert!(
            verdict(4, None, Some(4))
                .unwrap_err()
                .contains("no count, write 4")
        );
        assert_eq!(number("196\n"), Some(196));
        assert_eq!(number("196 (after x)"), None);
        assert_eq!(number("-1"), None);
    }

    #[dash_test]
    fn a_path_holding_a_newline_is_read_whole_from_git_grep() {
        let found = "a\nb\x001\x00#!/bin/sh\nc.md\x005\x00#!x\nd\x001\x00#!\n";
        assert_eq!(
            matched_line_one(found),
            BTreeSet::from(["a\nb".to_string(), "d".to_string()])
        );
        assert!(
            matched_line_one("").is_empty(),
            "no output reads as a script: matched_line_one splits nothing"
        );
        assert!(shebang(b"#!/") && shebang(b"#!") && !shebang(b"#![a") && !shebang(b"# !"));
        assert!(named("100755", "bin/x") && named("100644", "a.zsh") && !named("100644", "a.rs"));
    }

    #[dash_test]
    fn the_order_lists_the_paths_cells_of_its_section_alone() {
        let page = html(
            "[h3 id=\"x\">[td class=\"paths\">[code>before[/code>[/td>[h3 id=\"scripts-port\">[p>[code>dash[/code>[/p>\
             [tr>[td>[code>how[/code>[/td>[td class=\"paths\">[details>[code>bin/a[/code>, [code>b&amp;c.sh[/code>[/details>[/td>[/tr>\
             [h3 id=\"simulation\">[td class=\"paths\">[code>after[/code>[/td>",
        );
        let listed = listed(&page).expect("the section");
        assert_eq!(listed, BTreeSet::from(["bin/a".into(), "b&c.sh".into()]));
        assert_eq!(super::listed(&html("[h3 id=\"other\">")), None);
        let bad = unlisted(&["bin/a".into(), "new.py".into()], &listed);
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(bad[0].starts_with("new.py: a script the port's order misses"));
        assert!(bad[1].starts_with("b&c.sh: listed in "));
    }

    /// Done-when 2 of rust-ci-off-mac: the gate's count from the base's order and the PR's own change, the same verdicts as
    /// the full count; a file the PR leaves alone is never read, so a partial clone fetches no other blob.
    #[dash_test]
    fn a_change_is_counted_from_its_base_order_and_its_own_diff() {
        unsafe {
            for name in ["GIT_INDEX_FILE", "GIT_DIR", "GIT_WORK_TREE"] {
                std::env::remove_var(name);
            }
        }
        let top = std::env::temp_dir().join(format!("dash-ratchet-changed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        for dir in ["rust", "private/docs"] {
            std::fs::create_dir_all(top.join(dir)).expect("mkdir");
        }
        let sh = |line: &str| {
            let words: Vec<&str> = line.split(' ').collect();
            git(&top, &words, &[0]).expect(line);
        };
        let write = |path: &str, text: &str| std::fs::write(top.join(path), text).expect(path);
        let order = |held: usize, paths: &[&str]| {
            write(HELD, &format!("{held}\n"));
            let codes: Vec<String> = paths
                .iter()
                .map(|p| html(&format!("[code>{p}[/code>")))
                .collect();
            write(
                ORDER,
                &format!("{SECTION}{PATHS}{}{CELL_END}{}", codes.join(", "), NEXT[2]),
            );
        };
        let commit = |m: &str| {
            sh(&format!(
                "-c user.name=t -c user.email=t@t commit -q -a -m {m}"
            ))
        };
        sh("init -q");
        write("a.py", "print(1)\n");
        write("hook", "#!/bin/sh\n");
        write("notes.md", "a note\n");
        order(2, &["a.py", "hook"]);
        sh("add .");
        commit("base");
        // nothing changed past the order: the base's two
        assert_eq!(
            changed(&top, "HEAD"),
            Ok("2 tracked scripts outside rust/, 2 held".into())
        );
        // the inline script bytes held at the base may drop, never rise (inline_js.rs)
        write(JS_HELD, "10\n");
        sh(&format!("add {JS_HELD}"));
        commit("js");
        write(JS_HELD, "11\n");
        commit("js-up");
        let said = changed(&top, "HEAD~1").expect_err("a rise");
        assert!(
            said.contains("rust/inline-js-held.txt rose from 10 to 11"),
            "{said}"
        );
        write(JS_HELD, "9\n");
        commit("js-down");
        assert_eq!(
            changed(&top, "HEAD~2"),
            Ok("2 tracked scripts outside rust/, 2 held".into())
        );
        // the browser tests held likewise (browser.rs, task rust-browser-tests-fewer)
        write(BROWSER_HELD, "5\n");
        sh(&format!("add {BROWSER_HELD}"));
        commit("browser");
        write(BROWSER_HELD, "6\n");
        commit("browser-up");
        let said = changed(&top, "HEAD~1").expect_err("a rise");
        assert!(
            said.contains(
                "rust/browser-tests-held.txt rose from 5 to 6: the browser tests only drop"
            ),
            "{said}"
        );
        write(BROWSER_HELD, "4\n");
        commit("browser-down");
        assert_eq!(
            changed(&top, "HEAD~2"),
            Ok("2 tracked scripts outside rust/, 2 held".into())
        );

        // a #! added on line 1 of a tracked file: one more, which the number and the order must take, and the number
        // may not rise to let it in
        write("notes.md", "#!/usr/bin/env python3\n");
        commit("shebang");
        let said = changed(&top, "HEAD~1").expect_err("one more script");
        assert!(
            said.contains("3 tracked scripts outside rust/, 2 held: write it in Rust"),
            "{said}"
        );
        assert!(
            said.contains("notes.md: a script the port's order misses"),
            "{said}"
        );
        order(3, &["a.py", "hook", "notes.md"]);
        commit("raised");
        assert!(
            changed(&top, "HEAD~2")
                .expect_err("a rise")
                .contains("rose from 2 to 3")
        );

        // a port: the script deleted, the number lowered and the order dropping it, against the commit before
        sh("rm -q hook");
        order(2, &["a.py", "notes.md"]);
        commit("port");
        // the drop names the file that made it
        assert_eq!(
            changed(&top, "HEAD~1"),
            Ok(
                "2 tracked scripts outside rust/, 2 held\nsince HEAD~1: 1 gone, 0 new\n- hook"
                    .into()
            )
        );
        // the port alone, its number left: fewer fails until lowered
        sh("rm -q a.py");
        commit("half");
        let said = changed(&top, "HEAD~1").expect_err("not lowered");
        assert!(
            said.contains("1 tracked scripts outside rust/, 2 held: lower it to 1"),
            "{said}"
        );
        assert!(said.contains("a.py: listed in "), "{said}");

        // an unchanged file's blob is never read: one made unreadable passes as before (its base lowered, as it passed)
        write("big.md", "text\n");
        sh("add big.md");
        order(1, &["notes.md"]);
        commit("big");
        write("other.md", "plain\n");
        sh("add other.md");
        commit("after");
        // dropped once both commits stand: a commit's own tree build may read it (the pre-commit's run did)
        let blob = git(&top, &["rev-parse", "HEAD:big.md"], &[0]).expect("blob");
        let object = top.join(format!(".git/objects/{}/{}", &blob[..2], blob[2..].trim()));
        std::fs::remove_file(&object).expect("drop the blob");
        assert!(
            git(&top, &["cat-file", "-e", blob.trim()], &[0]).is_err(),
            "the blob is gone"
        );
        assert_eq!(
            changed(&top, "HEAD~1"),
            Ok("1 tracked scripts outside rust/, 1 held".into())
        );
        let _ = std::fs::remove_dir_all(&top);
    }

    /// The whole command on a repository of its own: the index's scripts, number and port order, a staged #! read from its
    /// blob, the base's number as the bound.
    #[dash_test]
    fn a_repository_is_counted_from_its_index_against_its_base() {
        // the pre-commit's rust-gate runs this suite with its GIT_INDEX_FILE set: never the outer repository's index.
        // Each test is a process of its own under nextest (rust/CLAUDE.md rule 1), so no other thread reads the env.
        unsafe {
            for name in ["GIT_INDEX_FILE", "GIT_DIR", "GIT_WORK_TREE"] {
                std::env::remove_var(name);
            }
        }
        let top = std::env::temp_dir().join(format!("dash-ratchet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        for dir in ["rust/ci", "private/docs"] {
            std::fs::create_dir_all(top.join(dir)).expect("mkdir");
        }
        let sh = |line: &str| {
            let words: Vec<&str> = line.split(' ').collect();
            git(&top, &words, &[0]).expect(line);
        };
        let write = |path: &str, text: &str| std::fs::write(top.join(path), text).expect(path);
        let staged = |held: usize, order: &[&str]| {
            write(HELD, &format!("{held}\n"));
            let codes: Vec<String> = order
                .iter()
                .map(|p| html(&format!("[code>{p}[/code>")))
                .collect();
            write(
                ORDER,
                &format!("{SECTION}{PATHS}{}{CELL_END}{}", codes.join(", "), NEXT[2]),
            );
            sh(&format!("add {HELD} {ORDER}"));
        };
        let failed = |base: Option<&str>| run(&top, true, base).expect_err("a failing check");
        sh("init -q");
        write("a.py", "print(1)\n");
        write("rust/ci/x.sh", "#!/bin/sh\n");
        write("notes.md", "a note\n#!/bin/sh on line 2 is no shebang\n");
        write("lib.rs", "#![allow(dead_code)]\n");
        staged(1, &["a.py"]);
        sh("add .");
        // a first commit: HEAD is no commit yet, so no base bounds the number; a name that is no commit fails
        let one = Ok("1 tracked scripts outside rust/, 1 held".into());
        assert_eq!(run(&top, true, Some("HEAD")), one);
        assert!(failed(Some("main~3")).contains("--base main~3: no such commit"));
        sh("-c user.name=t -c user.email=t@t commit -q -m one");
        assert_eq!(run(&top, true, Some("HEAD")), one);
        // the inline script bytes held at the base may drop, never rise (inline_js.rs)
        write(JS_HELD, "10\n");
        sh(&format!("add {JS_HELD}"));
        sh("-c user.name=t -c user.email=t@t commit -q -m js");
        write(JS_HELD, "11\n");
        sh(&format!("add {JS_HELD}"));
        let said = failed(Some("HEAD"));
        assert!(
            said.contains("rust/inline-js-held.txt rose from 10 to 11"),
            "{said}"
        );
        assert_eq!(run(&top, true, None), one, "no base, no bound");
        write(JS_HELD, "9\n");
        sh(&format!("add {JS_HELD}"));
        assert_eq!(run(&top, true, Some("HEAD")), one);

        // a #! staged on line 1, the worktree's copy without it: the index is what counts, and the order misses it
        write("tool", "#!/usr/bin/env python3\n");
        sh("add tool");
        write("tool", "plain\n");
        let said = failed(Some("HEAD"));
        assert!(
            said.contains("2 tracked scripts outside rust/, 1 held: write it in Rust"),
            "{said}"
        );
        assert!(
            said.contains("tool: a script the port's order misses"),
            "{said}"
        );
        // raising the number to let it in fails against the base
        staged(2, &["a.py", "tool"]);
        assert!(failed(Some("HEAD")).contains("rose from 1 to 2"));
        assert_eq!(
            run(&top, true, None),
            Ok("2 tracked scripts outside rust/, 2 held".into())
        );
        assert_eq!(
            run(&top, false, None),
            Ok("a.py\ntool\n2 tracked scripts outside rust/".into())
        );

        // a port: the scripts gone, the number and the order still naming them fail until both follow
        sh("rm -q -f --cached tool a.py");
        staged(1, &["a.py"]);
        let said = failed(Some("HEAD"));
        assert!(said.contains("lower it to 0"), "{said}");
        assert!(said.contains("a.py: listed in "), "{said}");
        staged(0, &[]);
        // the drop names the file that made it; HEAD's order never listed `tool`, so only a.py is gone from it
        assert_eq!(
            run(&top, true, Some("HEAD")),
            Ok("0 tracked scripts outside rust/, 0 held\nsince HEAD: 1 gone, 0 new\n- a.py".into())
        );
        // a page without the section holds no order
        write(ORDER, &html("[h3 id=\"simulation\">"));
        sh(&format!("add {ORDER}"));
        assert!(failed(Some("HEAD")).contains("no section scripts-port"));
        assert!(super::top().is_ok());
        std::fs::remove_dir_all(&top).expect("clean up");
    }

    /// Done-when 1 of rust-lint-scripts-port: `dash scripts count --check` shellchecks the shell scripts, an extensionless
    /// one by its shebang, as staged; with a base only those the index changes, with none every one. A stand-in shellcheck
    /// on PATH fails on « BAD », so the test reads the same on the Mac and the mirror.
    #[dash_test]
    fn the_check_shellchecks_the_staged_shell_scripts_found_by_shebang() {
        let top = std::env::temp_dir().join(format!("dash-ratchet-sh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        // the stand-in outside the repository, which would count it
        let bin = std::env::temp_dir().join(format!("dash-ratchet-sh-bin-{}", std::process::id()));
        for dir in ["rust", "private/docs", "bin"] {
            std::fs::create_dir_all(top.join(dir)).expect("mkdir");
        }
        std::fs::create_dir_all(&bin).expect("mkdir");
        let fake = bin.join("shellcheck");
        std::fs::write(
            &fake,
            "#!/bin/sh\ncase \"$*\" in *\"-e SC1091 -\") ;; *) echo \"args: $*\"; exit 2;; esac\n! grep -n BAD\n",
        )
        .expect("stand-in");
        let mut mode = std::fs::metadata(&fake).expect("stand-in").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
        std::fs::set_permissions(&fake, mode).expect("chmod");
        // each test is a process of its own under nextest (rust/CLAUDE.md rule 1), so no other thread reads the env
        unsafe {
            for name in ["GIT_INDEX_FILE", "GIT_DIR", "GIT_WORK_TREE"] {
                std::env::remove_var(name);
            }
            let path = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", format!("{}:{path}", bin.display()));
        }
        let sh = |line: &str| {
            let words: Vec<&str> = line.split(' ').collect();
            git(&top, &words, &[0]).expect(line);
        };
        let write = |path: &str, text: &str| std::fs::write(top.join(path), text).expect(path);
        write(
            ORDER,
            &format!(
                "{SECTION}{PATHS}{}{CELL_END}{}",
                html("[code>hook[/code>, [code>tool[/code>"),
                NEXT[2]
            ),
        );
        write(HELD, "2\n");
        write("hook", "#!/bin/sh\necho ok\n");
        write("tool", "#!/usr/bin/env python3\nBAD = 1\n");
        // a sourced library, neither a script nor counted, still shellchecked (bin/approved-only)
        write("bin/lib", "# shellcheck shell=bash\nlib=ok\n");
        sh("init -q");
        sh("add .");
        sh("-c user.name=t -c user.email=t@t commit -q -m one");
        let ok = Ok("2 tracked scripts outside rust/, 2 held".into());
        assert_eq!(run(&top, true, Some("HEAD")), ok);
        write("bin/lib", "# shellcheck shell=bash\nlib=BAD\n");
        sh("add bin/lib");
        let said = run(&top, true, Some("HEAD")).expect_err("a sourced finding");
        assert!(
            said.starts_with("bin/lib: shellcheck fails\n2:lib=BAD"),
            "{said}"
        );
        sh("-c user.name=t -c user.email=t@t commit -q -m lib");
        // staged with a finding, its worktree copy clean: the index is what is checked
        write("hook", "#!/bin/sh\necho BAD\n");
        sh("add hook");
        write("hook", "#!/bin/sh\necho ok\n");
        let said = run(&top, true, Some("HEAD")).expect_err("a finding");
        assert!(
            said.starts_with("hook: shellcheck fails\n2:echo BAD"),
            "{said}"
        );
        // committed: no change from HEAD, nothing to check; no base checks every one
        sh("-c user.name=t -c user.email=t@t commit -q -m two");
        assert_eq!(run(&top, true, Some("HEAD")), ok);
        let all = run(&top, true, None).expect_err("every one");
        assert!(all.contains("hook: shellcheck fails"), "{all}");
        assert!(all.contains("bin/lib: shellcheck fails"), "{all}");
        assert!(
            !all.contains("tool"),
            "a Python script is never shellchecked: {all}"
        );
        std::fs::remove_dir_all(&top).expect("clean up");
        std::fs::remove_dir_all(&bin).expect("clean up");
    }
}
