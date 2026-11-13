//! The dash binary only shrinks (task rust-binary-ratchet; @nha, 2026-11-13: « the organization seems un-gated and not
//! following the superapp concept », « small core, everything is a plugin »; rfc-rust-unified section 20b). crates/dash's
//! lines of code are counted against the number on [`HELD`]'s line 1, and its modules against the files listed below it:
//! more lines fail, fewer fail until lowered in the same commit, and a file under crates/dash/src the list lacks fails.
//! `dash scripts count --check --base` refuses the number rising, or the list growing, past its base's (ratchet.rs). A new
//! feature goes in a plugin crate ([`crate::PLUGINS`]), never the binary.
use crate::rust_files;
use std::collections::BTreeSet;
use std::path::Path;

/// The held count, then each module file, from rust/.
pub const HELD: &str = "binary-held.txt";

/// The binary's source, from rust/.
pub const SRC: &str = "crates/dash/src";

/// What one file holds of code: every line but its tests' modules, each a top-level `#[cfg(test)]` over `mod <name> {`
/// to its closing `}` on a line of its own, or over `mod <name>;` (two lines); code before or after one counts
/// (/code-review: a cut at the first one let code below it, or a `mod` under it at the top, go uncounted). A `tests.rs`, the
/// out-of-line tests' file, holds none; a `#[cfg(test)]` on anything but a `mod` is code.
/// ponytail: a tests.rs that is no test module counts as none; read the parent's `mod` if one ever is.
pub fn code_lines(name: &str, text: &str) -> usize {
    if name == "tests.rs" {
        return 0;
    }
    let mut lines = text.lines();
    let mut code = 0;
    while let Some(line) = lines.next() {
        if line != "#[cfg(test)]" {
            code += 1;
            continue;
        }
        match lines.next() {
            Some(m) if m.starts_with("mod ") && m.ends_with('{') => {
                lines.by_ref().find(|l| *l == "}");
            }
            Some(m) if m.starts_with("mod ") => {}
            Some(_) => code += 2,
            None => code += 1,
        }
    }
    code
}

/// The binary's code lines and its module files, each from crates/dash/src, under `rust`: none when it has no binary.
pub fn count(rust: &Path) -> (usize, BTreeSet<String>) {
    let src = rust.join(SRC);
    let mut lines = 0;
    let mut modules = BTreeSet::new();
    let paths = match src.is_dir() {
        true => rust_files(&src),
        false => vec![],
    };
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        lines += code_lines(&name, &text);
        let rel = path.strip_prefix(&src).unwrap_or(&path);
        modules.insert(rel.to_string_lossy().replace('\\', "/"));
    }
    (lines, modules)
}

/// [`HELD`]'s number and its module list: no number when line 1 holds none. ratchet.rs reads the list past line 1 itself:
/// this module takes the crate root, which takes ratchet.
pub fn held(text: &str) -> (Option<usize>, BTreeSet<String>) {
    let mut lines = text.lines().map(str::trim);
    let n = lines.next().and_then(|l| l.parse().ok());
    (
        n,
        lines.filter(|l| !l.is_empty()).map(String::from).collect(),
    )
}

/// The binary against what is held: each fault with its fix, or the count against the number.
pub fn verdict(
    lines: usize,
    modules: &BTreeSet<String>,
    held_text: &str,
) -> Result<String, String> {
    let (held, listed) = held(held_text);
    let mut bad: Vec<String> = modules
        .difference(&listed)
        .map(|m| {
            format!(
                "{SRC}/{m}: a new module in the binary: put the feature in a plugin crate (dash-gate's PLUGINS: {}), never crates/dash",
                crate::PLUGINS.join(", ")
            )
        })
        .collect();
    bad.extend(
        (listed.difference(modules))
            .map(|m| format!("{SRC}/{m}: gone, drop it from rust/{HELD}, in this commit")),
    );
    match held {
        None => bad.push(format!("rust/{HELD}: no count on line 1, write {lines}")),
        Some(h) if lines > h => bad.push(format!(
            "{lines} lines of code in crates/dash, {h} held: the binary only shrinks, put the feature in a plugin crate \
             (dash-gate's PLUGINS: {})",
            crate::PLUGINS.join(", ")
        )),
        Some(h) if lines < h => bad.push(format!(
            "{lines} lines of code in crates/dash, {h} held: lower it to {lines} in rust/{HELD}, in this commit"
        )),
        Some(_) => {}
    }
    match bad.is_empty() {
        true => Ok(format!(
            "{lines} lines of code in crates/dash, {lines} held, {} modules",
            modules.len()
        )),
        false => Err(bad.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    #[dash_test]
    fn code_stops_at_the_tests_module_and_a_tests_file_holds_none() {
        let text = "fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn b() {}\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n";
        assert_eq!(code_lines("a.rs", text), 4);
        assert_eq!(
            code_lines("a.rs", "fn a() {}\n#[cfg(test)]\nmod tests;\n"),
            1
        );
        assert_eq!(code_lines("tests.rs", text), 0);
        assert_eq!(code_lines("a.rs", "fn a() {}\n"), 1);
        // code below a tests module, or around a `mod` under cfg(test) at the top, still counts
        let below =
            "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() {\n    }\n}\nfn b() {}\nfn c() {}\n";
        assert_eq!(code_lines("a.rs", below), 3);
        assert_eq!(
            code_lines("a.rs", "#[cfg(test)]\nmod t;\nfn a() {}\nfn b() {}\n"),
            2
        );
    }

    #[dash_test]
    fn more_lines_or_a_new_module_fail_fewer_must_be_lowered() {
        let modules: BTreeSet<String> = ["main.rs", "a.rs"].map(String::from).into();
        let held = "10\na.rs\nmain.rs\n";
        assert_eq!(
            verdict(10, &modules, held),
            Ok("10 lines of code in crates/dash, 10 held, 2 modules".into())
        );
        let more = verdict(11, &modules, held).unwrap_err();
        assert!(
            more.contains("11 lines of code in crates/dash, 10 held: the binary only shrinks"),
            "{more}"
        );
        let fewer = verdict(9, &modules, held).unwrap_err();
        assert!(
            fewer.contains("lower it to 9 in rust/binary-held.txt"),
            "{fewer}"
        );
        let mut grown = modules.clone();
        grown.insert("feature.rs".into());
        let new = verdict(10, &grown, held).unwrap_err();
        assert!(
            new.contains("crates/dash/src/feature.rs: a new module in the binary"),
            "{new}"
        );
        assert!(new.contains("drafts, comments, mail, hooks"), "{new}");
        let gone = verdict(10, &["main.rs".to_string()].into(), held).unwrap_err();
        assert!(gone.contains("crates/dash/src/a.rs: gone"), "{gone}");
        assert!(
            verdict(10, &modules, "a.rs\n")
                .unwrap_err()
                .contains("no count on line 1, write 10")
        );
    }
}
