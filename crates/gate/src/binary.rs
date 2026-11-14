//! The dash binary only shrinks (task rust-binary-ratchet; @nha, 2026-11-13: « the organization seems un-gated and not
//! following the superapp concept », « small core, everything is a plugin »; rfc-rust-unified section 20b). crates/dash's
//! code at the checked-out head is measured against the same code at its merge base with origin/main, read from git with
//! no checkout: more tokens fail, and so does a file under crates/dash/src the base lacks. Tokens, not lines, the counter
//! /deps uses ([`crate::graph`]), so packing lines never passes it (@nha, 2026-11-14, on the line count: « a poor man's
//! tool », agents « will just cram changes in single lines »). No number is committed: the held number every PR rewrote
//! made each landing conflict with every other open PR (task rust-binary-ratchet-red). A new feature goes in a plugin crate
//! ([`crate::PLUGINS`]), never the binary.
use crate::rust_files;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// The binary's source, from rust/.
pub const SRC: &str = "crates/dash/src";

/// A commit the base is read from in place of the merge base with origin/main, for a checkout that has no origin/main.
pub const BASE_ENV: &str = "DASH_BINARY_BASE";

/// The binary's tokens of code and its module files, each from crates/dash/src.
pub type Measure = (usize, BTreeSet<String>);

/// What one file holds of code: every line but its tests' modules, each a top-level `#[cfg(test)]` over `mod <name> {`
/// to its closing `}` on a line of its own, or over `mod <name>;` (two lines); code before or after one counts
/// (/code-review: a cut at the first one let code below it, or a `mod` under it at the top, go uncounted). A `tests.rs`, the
/// out-of-line tests' file, holds none; a `#[cfg(test)]` on anything but a `mod` is code. The `}` alone on its line is
/// rustfmt's layout, which dash-gate's rustfmt_check holds.
/// ponytail: a tests.rs that is no test module counts as none; read the parent's `mod` if one ever is.
pub fn code(name: &str, text: &str) -> String {
    if name == "tests.rs" {
        return String::new();
    }
    let mut lines = text.lines();
    let mut code = vec![];
    while let Some(line) = lines.next() {
        if line != "#[cfg(test)]" {
            code.push(line);
            continue;
        }
        match lines.next() {
            Some(m) if m.starts_with("mod ") && m.ends_with('{') => {
                lines.by_ref().find(|l| *l == "}");
            }
            Some(m) if m.starts_with("mod ") => {}
            Some(m) => code.extend([line, m]),
            None => code.push(line),
        }
    }
    code.join("\n")
}

/// The tokens and modules of these files, each `(path under crates/dash/src, text)`.
fn measure(files: impl IntoIterator<Item = (String, String)>) -> Measure {
    let mut tokens = 0;
    let mut modules = BTreeSet::new();
    for (rel, text) in files {
        let name = rel.rsplit('/').next().unwrap_or_default();
        tokens += crate::graph::tokens(&code(name, &text)).len();
        modules.insert(rel);
    }
    (tokens, modules)
}

/// The binary as checked out under `rust`, uncommitted files too: none when it has no binary.
pub fn count(rust: &Path) -> Measure {
    let src = rust.join(SRC);
    let paths = match src.is_dir() {
        true => rust_files(&src),
        false => vec![],
    };
    measure(paths.into_iter().map(|path| {
        let rel = path.strip_prefix(&src).unwrap_or(&path);
        (
            rel.to_string_lossy().replace('\\', "/"),
            std::fs::read_to_string(&path).unwrap_or_default(),
        )
    }))
}

/// git in `rust`, its own repository found from there: a hook's GIT_DIR or GIT_INDEX_FILE never points it elsewhere.
fn git(rust: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(["-c", "core.quotePath=false", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(rust)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    match out.status.success() {
        true => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        false => Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// The binary at commit `rev`, read with git under `rust` (no checkout): ls-tree's paths are relative to it, so the same
/// call reads site_perso's rust/ and the mirror's root.
/// ponytail: one git process a file, about 75; `git cat-file --batch` if the binary ever holds hundreds.
pub fn count_at(rust: &Path, rev: &str) -> Result<Measure, String> {
    let tree = git(rust, &["ls-tree", "-r", "-z", rev, "--", SRC])?;
    let mut files = vec![];
    for entry in tree.split('\0').filter(|e| !e.is_empty()) {
        let (meta, path) = entry.split_once('\t').ok_or(format!("ls-tree: {entry}"))?;
        let Some(rel) = path.strip_prefix(&format!("{SRC}/")) else {
            continue;
        };
        if !rel.ends_with(".rs") {
            continue;
        }
        let blob = meta.rsplit(' ').next().unwrap_or_default();
        files.push((rel.to_string(), git(rust, &["cat-file", "blob", blob])?));
    }
    Ok(measure(files))
}

/// The commit the head is measured against: `env` ([`BASE_ENV`]'s value) when set, else the merge base of HEAD and
/// origin/main. While a merge is being committed (MERGE_HEAD set), the merge itself counts as the head: HEAD is still its
/// first parent, so a merge of main would be measured against the branch's old base with main's new modules in the tree
/// (the pre-commit refused PR 1566's merge of main, 2026-11-14). `Err` says why there is none (no origin/main, or the
/// mirror's parentless commits), and the gate skips with it printed; `env` naming no commit is no skip but a fault,
/// `Ok(Err(..))`.
pub fn base(rust: &Path, env: Option<String>) -> Result<Result<String, String>, String> {
    if let Some(rev) = env.filter(|r| !r.trim().is_empty()) {
        let commit = format!("{}^{{commit}}", rev.trim());
        return Ok(git(rust, &["rev-parse", "--verify", "-q", &commit])
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("{BASE_ENV}={rev} names no commit: {e}")));
    }
    let merging = git(rust, &["rev-parse", "--verify", "-q", "MERGE_HEAD"]).is_ok();
    let args: &[&str] = match merging {
        true => &["merge-base", "origin/main", "HEAD", "MERGE_HEAD"],
        false => &["merge-base", "HEAD", "origin/main"],
    };
    git(rust, args)
        .map(|s| Ok(s.trim().to_string()))
        .map_err(|e| {
            format!("no merge base of HEAD and origin/main ({e}), and no {BASE_ENV}: the binary is not compared")
        })
}

/// The head's binary against the base's at `rev`: each fault with its fix, or the two counts.
pub fn verdict(head: &Measure, base: &Measure, rev: &str) -> Result<String, String> {
    let plugins = crate::PLUGINS.join(", ");
    let mut bad: Vec<String> = head
        .1
        .difference(&base.1)
        .map(|m| {
            format!(
                "{SRC}/{m}: a new module in the binary, which {rev} lacks: put the feature in a plugin crate (dash-gate's \
                 PLUGINS: {plugins}), never crates/dash"
            )
        })
        .collect();
    if head.0 > base.0 {
        bad.push(format!(
            "{} tokens of code in crates/dash, {} at {rev}: the binary only shrinks, put the feature in a plugin crate \
             (dash-gate's PLUGINS: {plugins})",
            head.0, base.0
        ));
    }
    match bad.is_empty() {
        true => Ok(format!(
            "{} tokens of code in crates/dash, {} at {rev}, {} modules",
            head.0,
            base.0,
            head.1.len()
        )),
        false => Err(bad.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    #[dash_test]
    fn code_stops_at_the_tests_module_and_a_tests_file_holds_none() {
        let lines = |name: &str, text: &str| match code(name, text) {
            c if c.is_empty() => 0,
            c => c.lines().count(),
        };
        let text = "fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn b() {}\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n";
        assert_eq!(lines("a.rs", text), 4);
        assert_eq!(lines("a.rs", "fn a() {}\n#[cfg(test)]\nmod tests;\n"), 1);
        assert_eq!(lines("tests.rs", text), 0);
        assert_eq!(lines("a.rs", "fn a() {}\n"), 1);
        // code below a tests module, or around a `mod` under cfg(test) at the top, still counts
        let below =
            "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() {\n    }\n}\nfn b() {}\nfn c() {}\n";
        assert_eq!(lines("a.rs", below), 3);
        assert_eq!(
            lines("a.rs", "#[cfg(test)]\nmod t;\nfn a() {}\nfn b() {}\n"),
            2
        );
    }

    /// Done-when 4 of rust-binary-ratchet-red: a planted repository, its origin/main and a branch off it. A growth or a new
    /// module fails, a shrink passes, packing lines changes nothing, and the merge base is the reference, never main's tip.
    #[dash_test]
    fn the_head_is_measured_against_its_merge_base_in_tokens() {
        let dir = TestDir::new("binary-base");
        let rust = dir.0.join("rust");
        let src = rust.join(SRC);
        std::fs::create_dir_all(&src).expect("src");
        // every call carries an identity: CI's runner has no global git config, so a merge there refused with
        // « Committer identity unknown » while it passed on the Mac (PR 1591's first mirror run)
        let sh = |args: &str| {
            let words: Vec<&str> = ["-c", "user.name=t", "-c", "user.email=t@t"]
                .into_iter()
                .chain(args.split(' '))
                .collect();
            git(&rust, &words).expect(args)
        };
        let put = |file: &str, text: &str| std::fs::write(src.join(file), text).expect(file);
        let main = "fn main() {\n    let a = 1;\n    let b = 2;\n    println!(\"{}\", a + b);\n}\n";
        let tests = "#[cfg(test)]\nmod tests {\n    fn t() { let x = 1; }\n}\n";
        sh("init -q");
        put("main.rs", main);
        sh("add .");
        let commit = |m: &str| sh(&format!("commit -q -a -m {m}"));
        commit("base");
        sh("update-ref refs/remotes/origin/main HEAD");
        let at = sh("rev-parse HEAD").trim().to_string();
        assert_eq!(base(&rust, None), Ok(Ok(at.clone())));
        let check = || verdict(&count(&rust), &count_at(&rust, &at).expect("base"), "base");
        assert_eq!(
            check(),
            Ok("25 tokens of code in crates/dash, 25 at base, 1 modules".into())
        );

        // main moves on and shrinks: the branch is still measured against where it left main
        sh("checkout -q -b side");
        put("main.rs", "fn main() {}\n");
        commit("main-shrinks");
        sh("update-ref refs/remotes/origin/main HEAD");
        sh("checkout -q -");
        assert_eq!(base(&rust, None), Ok(Ok(at.clone())));

        // main merged in, not yet committed: measured against main, whose modules the merge brings
        sh("merge -q --no-commit --no-ff side");
        let side = sh("rev-parse side").trim().to_string();
        assert_eq!(base(&rust, None), Ok(Ok(side)), "the merge being made");
        sh("merge --abort");
        assert_eq!(base(&rust, None), Ok(Ok(at.clone())));

        // packed on one line, and a tests module added: the same tokens of code
        put("main.rs", &format!("{}\n{tests}", main.replace('\n', " ")));
        assert!(check().is_ok(), "{:?}", check());
        let grown = main.replace("let b = 2;", "let b = 2; let c = 3;");
        put("main.rs", &grown);
        let said = check().expect_err("a growth");
        assert!(
            said.contains("30 tokens of code in crates/dash, 25 at base: the binary only shrinks"),
            "{said}"
        );
        put("main.rs", main);
        put("feature.rs", "");
        let said = check().expect_err("a new module");
        assert!(
            said.contains(
                "crates/dash/src/feature.rs: a new module in the binary, which base lacks"
            ),
            "{said}"
        );
        std::fs::remove_file(src.join("feature.rs")).expect("rm");
        put("main.rs", &main.replace("    let b = 2;\n", ""));
        assert_eq!(
            check(),
            Ok("20 tokens of code in crates/dash, 25 at base, 1 modules".into())
        );

        // the base named outright, or none at all: a fault for a name that is no commit, a skip with its reason
        assert_eq!(base(&rust, Some(at.clone())), Ok(Ok(at.clone())));
        assert!(base(&rust, Some("nope".into())).expect("set").is_err());
        sh("update-ref -d refs/remotes/origin/main");
        let skip = base(&rust, None).expect_err("no origin/main");
        assert!(
            skip.contains("no merge base of HEAD and origin/main"),
            "{skip}"
        );
    }
}
