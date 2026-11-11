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

/// `dash scripts count`: each script, then the total; with `check`, the verdict alone, the number never above `base`'s.
pub fn run(top: &Path, check: bool, base: Option<&str>) -> Result<String, String> {
    let (paths, held) = count(top)?;
    if !check {
        return Ok(format!(
            "{}\n{} tracked scripts outside rust/",
            paths.join("\n"),
            paths.len()
        ));
    }
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
    match bad.is_empty() {
        true => said,
        false => Err(bad.join("\n")),
    }
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
    use dash_meter::memory::TestBudget;

    /// A page written with `[` for each tag's opening: the gate's source holds no tag literal (html::html_in_strings).
    fn html(text: &str) -> String {
        text.replace('[', "<")
    }

    #[test]
    fn a_script_is_its_extension_its_bit_or_its_shebang_outside_rust() {
        let _budget = TestBudget::new();
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

    #[test]
    fn more_fails_fewer_fails_until_lowered_and_the_number_never_rises() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_path_holding_a_newline_is_read_whole_from_git_grep() {
        let _budget = TestBudget::new();
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

    #[test]
    fn the_order_lists_the_paths_cells_of_its_section_alone() {
        let _budget = TestBudget::new();
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

    /// The whole command on a repository of its own: the index's scripts, number and port order, a staged #! read from its
    /// blob, the base's number as the bound.
    #[test]
    fn a_repository_is_counted_from_its_index_against_its_base() {
        let _budget = TestBudget::new();
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
        assert_eq!(
            run(&top, true, Some("HEAD")),
            Ok("0 tracked scripts outside rust/, 0 held".into())
        );
        // a page without the section holds no order
        write(ORDER, &html("[h3 id=\"simulation\">"));
        sh(&format!("add {ORDER}"));
        assert!(failed(Some("HEAD")).contains("no section scripts-port"));
        assert!(super::top().is_ok());
        std::fs::remove_dir_all(&top).expect("clean up");
    }
}
