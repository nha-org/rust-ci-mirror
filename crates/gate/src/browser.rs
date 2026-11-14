//! The browser test group's count, which only goes down (task rust-browser-tests-fewer; @nha, 2026-11-11: « these kind of
//! chrome tests are usually heavy and if we can avoid or use less of them that's usually good. small tests > big slow tests.
//! pure rust in-process > webdriver > chrome > pupeteer/playwright »). The group is the one override in
//! rust/.config/nextest.toml whose test-group is "browser". Its filter names whole test files (`binary_id(<crate>::<file>)`),
//! tests left out of them (`& not test(=<name>)`) and tests added from other files (`test(=<name>)`), joined by `|`. Each
//! `#[test]` it takes is counted from the source against the one number in [`HELD`], and each one says why it needs a
//! browser on a [`MARK`] comment line above it. A shape the gate cannot read exactly (another term, an `&` past a `not`, a
//! second override, a `mod` in a browser file, an attribute with no `fn` after it) is refused, never guessed (/code-review).
//! ponytail: a `not test(=..)` leaves every named file, whatever its parentheses; group the browser files in one if that bites.
use crate::rust_files;
use std::path::{Path, PathBuf};

/// The held count, from rust/.
pub const HELD: &str = "browser-tests-held.txt";

/// The comment line above each browser test that says why a browser is needed.
pub const MARK: &str = "Browser needed:";

/// What follows a test's attribute on its line: `#[dash_test]`, with its guard or its `dash_meter::` path, or a bare
/// `#[test]` (task rust-test-budget-attr).
fn after_test_attribute(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("#[")?;
    let rest = rest.strip_prefix("dash_meter::").unwrap_or(rest);
    let rest = rest
        .strip_prefix("dash_test")
        .or_else(|| rest.strip_prefix("test"))?;
    match rest.strip_prefix(']') {
        Some(after) => Some(after),
        None => rest
            .strip_prefix('(')?
            .split_once(")]")
            .map(|(_, after)| after),
    }
}

/// One test in the group: its file and line, its name, and whether a comment above it says why it needs a browser.
#[derive(Clone, Debug, PartialEq)]
pub struct Test {
    pub at: String,
    pub name: String,
    pub why: bool,
}

/// The browser group's filter as nextest.toml holds it: the whole files, the tests left out, the tests added.
#[derive(Debug, Default, PartialEq)]
pub struct Group {
    pub files: Vec<(String, String)>,
    pub out: Vec<String>,
    pub added: Vec<String>,
}

/// The filter of the one override whose `test-group` is "browser", read into its three kinds of terms.
pub fn group(toml: &str) -> Result<Group, String> {
    let blocks: Vec<&str> = toml
        .split("[[")
        .filter(|b| b.lines().any(|l| l.trim() == r#"test-group = "browser""#))
        .collect();
    let [block] = blocks[..] else {
        return Err(format!(
            "nextest.toml: {} overrides take test-group = \"browser\", the gate reads one",
            blocks.len()
        ));
    };
    let filter = block
        .lines()
        .find_map(|l| l.trim().strip_prefix("filter = \""))
        .and_then(|f| f.strip_suffix('"'))
        .ok_or("nextest.toml: the browser override has no one-line filter")?;
    let refused = || {
        format!(
            "nextest.toml's browser filter holds a shape the gate does not count, past binary_id(..) | test(=..) and \
             & not test(=..): {filter}"
        )
    };
    let mut g = Group::default();
    let mut rest = filter.to_string();
    for open in ["not test(=", "test(=", "binary_id("] {
        while let Some(i) = rest.find(open) {
            let end = rest[i..].find(')').ok_or_else(refused)? + i;
            let arg = rest[i + open.len()..end].to_string();
            let mut from = i;
            match open {
                "binary_id(" => {
                    let (krate, file) = arg.split_once("::").ok_or_else(refused)?;
                    g.files.push((krate.to_string(), file.to_string()));
                }
                "not test(=" => {
                    // a `not` only narrows what an `&` joins it to: `a | not test(=x)` is the workspace less x
                    from = rest[..i]
                        .trim_end()
                        .strip_suffix('&')
                        .ok_or_else(refused)?
                        .len();
                    g.out.push(arg);
                }
                _ => g.added.push(arg),
            }
            rest.replace_range(from..=end, " ");
        }
    }
    // an `&` left joins two sets nextest intersects, which a count of each cannot read
    match rest.chars().all(|c| "()| ".contains(c)) {
        true => Ok(g),
        false => Err(refused()),
    }
}

/// What a test file holds: each `#[test]`'s line, its name with the inline modules it sits in (`tests::fills`, as nextest
/// names it), and whether the comment lines just above it hold [`MARK`]; the line of each module, inline or declared.
#[derive(Default)]
struct Found {
    tests: Vec<(usize, String, bool)>,
    mods: Vec<usize>,
}

/// The brackets a line opens less those it closes, outside its string literals: a wall budget's « [0, 300) ms » counts none.
fn depth_of(line: &str) -> i64 {
    let code: String = line.split('"').step_by(2).collect();
    let code = code.split("//").next().unwrap_or_default();
    code.matches('[').count() as i64 - code.matches(']').count() as i64
}

/// The tests of `source`, or what in it the gate cannot read: a test attribute with no `fn` after it, or one sharing its line.
fn tests_in(source: &str) -> Result<Found, String> {
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let mut found = Found::default();
    let attribute = |l: &str| l.starts_with("#[") || l.starts_with("//");
    // the inline modules open at each line, each with the brace depth it opened at; it closes when the depth falls back
    // ponytail: braces are counted outside `//` comments only; a string with one unbalanced across lines misnames the
    // tests after it, which the gate then reports as « no such test »
    let (mut open, mut depth): (Vec<(String, i64)>, i64) = (vec![], 0);
    for (i, line) in lines.iter().enumerate() {
        // `pub mod`, `pub(crate) mod`, `pub(super) mod` and `mod`
        let bare = match line.strip_prefix("pub(") {
            Some(rest) => rest.split_once(") ").map_or("", |(_, r)| r),
            None => line.strip_prefix("pub ").unwrap_or(line),
        };
        let code = line.split("//").next().unwrap_or_default();
        if let Some(m) = bare.strip_prefix("mod ") {
            found.mods.push(i + 1);
            if !m.trim_end().ends_with(';') {
                let name = m.split([' ', '{']).next().unwrap_or_default();
                open.push((name.to_string(), depth));
            }
        }
        depth += code.matches('{').count() as i64 - code.matches('}').count() as i64;
        while open.last().is_some_and(|(_, at)| depth <= *at) {
            open.pop();
        }
        let Some(after) = after_test_attribute(line) else {
            continue;
        };
        if !(after.is_empty() || after.trim_start().starts_with("//")) {
            return Err(format!("line {}: a test attribute shares its line", i + 1));
        }
        // an attribute rustfmt wrapped over lines (a long `#[wall_budget("..")]`) is skipped to its closing bracket
        let mut brackets = 0i64;
        let at = (i + 1..lines.len()).find(|&k| {
            let l = lines[k];
            let inside = brackets > 0 || l.starts_with("#[");
            if inside {
                brackets += depth_of(l);
            }
            !inside && !attribute(l)
        });
        let name = at
            .and_then(|k| lines[k].split_once("fn "))
            .and_then(|(_, rest)| rest.split(['(', '<']).next())
            .filter(|n| !n.is_empty())
            .ok_or(format!(
                "line {}: a test attribute with no fn after it",
                i + 1
            ))?;
        // up over the comments and attributes above, a wrapped one's lines too, to the first line of neither (/code-review)
        let mut closing = 0i64;
        let why = lines[..i]
            .iter()
            .rev()
            .take_while(|l| {
                let inside = closing < 0 || (!l.starts_with("//") && depth_of(l) < 0);
                if inside {
                    closing += depth_of(l);
                }
                inside || attribute(l)
            })
            .any(|l| l.contains(MARK));
        let path: String = open.iter().map(|(m, _)| format!("{m}::")).collect();
        found
            .tests
            .push((at.unwrap_or(i) + 1, format!("{path}{name}"), why));
    }
    Ok(found)
}

/// Every test the browser group of `root`'s .config/nextest.toml takes, and what breaks the rule: a file or a named test
/// that is not there or is named twice, a test with no reason given, a shape the gate cannot read.
pub fn browser_tests(root: &Path) -> (Vec<Test>, Vec<String>) {
    let (mut tests, mut bad) = (vec![], vec![]);
    let toml = std::fs::read_to_string(root.join(".config/nextest.toml")).unwrap_or_default();
    let g = match group(&toml) {
        Ok(g) => g,
        Err(e) => return (tests, vec![e]),
    };
    let rel = |p: &Path| {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    };
    // each crate's package name, read under its [package], to its folder
    let package = |manifest: &str| {
        let after = manifest.split_once("[package]")?.1;
        let name = after
            .lines()
            .find_map(|l| l.trim().strip_prefix("name = \""))?;
        Some(name.trim_end_matches('"').to_string())
    };
    let crates: Vec<_> = std::fs::read_dir(root.join("crates"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let manifest = std::fs::read_to_string(e.path().join("Cargo.toml")).ok()?;
            Some((package(&manifest)?, e.path()))
        })
        .collect();
    let mut whole = vec![];
    for (krate, file) in &g.files {
        match crates.iter().find(|(n, _)| n == krate) {
            Some((_, dir)) => whole.push(dir.join("tests").join(format!("{file}.rs"))),
            None => bad.push(format!("binary_id({krate}::{file}): no crate {krate}")),
        }
    }
    // each file read once: those taken whole, then the others where an added name is looked for: a crate's tests/ file, or
    // a src/ file whose module path the name starts (nextest names a unit test `apply::tests::<fn>`; main's bare name for
    // one matched no test, so it ran outside this group, rust-apply-greenhouse's PR 1487)
    let dir = root.join("crates");
    let mut others: Vec<(PathBuf, String)> = rust_files(&dir)
        .into_iter()
        .filter(|p| !whole.contains(p))
        .filter_map(|p| {
            // crate, then tests or src, then the file's path inside it
            let parts: Vec<String> = p
                .strip_prefix(&dir)
                .ok()?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let m = match parts.get(1).map(String::as_str) {
                Some("tests") if parts.len() == 3 => String::new(),
                Some("src") => module_of(&parts[2..])?,
                _ => return None,
            };
            g.added.iter().any(|n| n.starts_with(&m)).then_some((p, m))
        })
        .collect();
    others.sort();
    let mut read = |path: &Path, taken_whole: bool, module: &str| -> Vec<Test> {
        let found = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| tests_in(&s));
        match found {
            Ok(Found { mods, .. }) if taken_whole && !mods.is_empty() => {
                let line = mods[0];
                bad.push(format!(
                    "{}:{line}: a module; a browser test file keeps its tests at its top level",
                    rel(path)
                ));
                vec![]
            }
            Ok(Found { tests, .. }) => tests
                .into_iter()
                .map(|(line, name, why)| Test {
                    at: format!("{}:{line}", rel(path)),
                    name: format!("{module}{name}"),
                    why,
                })
                .collect(),
            Err(e) if taken_whole => {
                bad.push(format!("{}: {e}", rel(path)));
                vec![]
            }
            // a file only searched for an added name fails nothing it does not hold: that name's « no such test » names it
            Err(_) => vec![],
        }
    };
    let mut out_seen = vec![];
    for path in &whole {
        for t in read(path, true, "") {
            match g.out.contains(&t.name) {
                true => out_seen.push(t.name),
                false => tests.push(t),
            }
        }
    }
    let mut hits: Vec<Test> = vec![];
    if !g.added.is_empty() {
        for (path, module) in &others {
            hits.extend(
                read(path, false, module)
                    .into_iter()
                    .filter(|t| g.added.contains(&t.name)),
            );
        }
    }
    for n in g.out.iter().filter(|n| !out_seen.contains(n)) {
        bad.push(format!("not test(={n}): no such test in its files"));
    }
    for name in &g.added {
        let mut named: Vec<Test> = hits.iter().filter(|t| &t.name == name).cloned().collect();
        match named.len() {
            1 => tests.append(&mut named),
            0 => bad.push(format!(
                "test(={name}): no such test under crates/*/tests past the files taken whole, nor crates/*/src (a unit \
                 test is named by its module path, apply::tests::<fn>)"
            )),
            n => bad.push(format!("test(={name}): {n} tests by that name")),
        }
    }
    for t in tests.iter().filter(|t| !t.why) {
        bad.push(format!(
            "{} {}: no « {MARK} <why> » comment above it",
            t.at, t.name
        ));
    }
    (tests, bad)
}

/// The module path nextest puts before a test in a file at `segs` inside a crate's src/: `apply::` for apply.rs, `a::b::`
/// for a/b.rs or a/b/mod.rs, empty for lib.rs or main.rs; None under bin/, another binary.
fn module_of(segs: &[String]) -> Option<String> {
    let mut segs = segs.to_vec();
    let file = segs.pop()?.strip_suffix(".rs")?.to_string();
    if segs.first().is_some_and(|s| s == "bin") {
        return None;
    }
    if !(file == "mod" || segs.is_empty() && (file == "lib" || file == "main")) {
        segs.push(file);
    }
    Some(segs.iter().map(|s| format!("{s}::")).collect())
}

/// `n` browser tests against the number held: the line to print, or why it fails.
pub fn verdict(n: usize, held: Option<usize>) -> Result<String, String> {
    match held {
        None => Err(format!("rust/{HELD}: no count, write {n}")),
        Some(h) if n > h => Err(format!(
            "{n} tests in nextest's browser group, {h} held: test it in-process, the rendered tree or its text form \
             (rust/CLAUDE.md rule 1), never one more Chrome"
        )),
        Some(h) if n < h => Err(format!(
            "{n} tests in nextest's browser group, {h} held: lower it to {n} in rust/{HELD}, in this commit"
        )),
        Some(h) => Ok(format!("{n} tests in nextest's browser group, {h} held")),
    }
}
