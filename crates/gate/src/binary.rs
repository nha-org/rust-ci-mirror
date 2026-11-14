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
/// origin/main, MERGE_HEAD counted with HEAD while a merge is being committed. `Err` says why there is none (no
/// origin/main, or the mirror's parentless commits), and the gate skips with it printed; `env` naming no commit is no skip
/// but a fault, `Ok(Err(..))`.
pub fn base(rust: &Path, env: Option<String>) -> Result<Result<String, String>, String> {
    if let Some(rev) = env.filter(|r| !r.trim().is_empty()) {
        let commit = format!("{}^{{commit}}", rev.trim());
        return Ok(git(rust, &["rev-parse", "--verify", "-q", &commit])
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("{BASE_ENV}={rev} names no commit: {e}")));
    }
    // a merge being committed is measured as the commit it makes: its base with origin/main counts MERGE_HEAD too, else
    // the pre-commit of a merge of main reads main's own new code as the branch's (2026-11-14, rust-task-ended-own-store)
    let merging = git(rust, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok();
    let mut args = vec!["merge-base", "origin/main", "HEAD"];
    if merging {
        args.push("MERGE_HEAD");
    }
    let heads = args[2..].join(" and ");
    git(rust, &args)
        .map(|s| Ok(s.trim().to_string()))
        .map_err(|e| {
            format!("no merge base of {heads} with origin/main ({e}), and no {BASE_ENV}: the binary is not compared")
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

/// What a done report's branch builds where standing rule 19 forbids (task brief-placement-blocks; @nha, 2026-11-14: « there
/// needs to be some questioning sometimes instead of blind execution. »): its HEAD in `cwd` against the merge-base with
/// origin/main, what ships, never the working tree, by [`faults`]. `dash task report` refuses it, so the run ends with a
/// TASK_BLOCKED naming the core port it lacks. None for a blocked report or a folder git cannot read: CI's gate still holds
/// both rules. ponytail: origin/main typed, as every Rust run is site_perso's; read forge.env's branch if another repo runs one.
pub fn misplaced(record: &serde_json::Value, cwd: &str) -> Vec<String> {
    if record["kind"] != "done" || cwd.is_empty() {
        return vec![];
    }
    // the folder's own repository, whatever hook or config runs us (/code-review: a GIT_DIR, a quoted path, fsmonitor)
    let git = |args: &[&str]| {
        let mut git = std::process::Command::new("git");
        git.args(["-c", "core.quotePath=false", "-c", "core.fsmonitor=false"]);
        for var in ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"] {
            git.env_remove(var);
        }
        let out = git.args(args).current_dir(cwd).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let Some(base) = git(&["merge-base", "origin/main", "HEAD"]) else {
        return vec![];
    };
    let base = base.trim();
    // a renamed or moved module is no new one: -M reads it as R, which the A filter leaves out
    let names = [
        "diff",
        "--name-only",
        "-M",
        "--diff-filter=A",
        base,
        "HEAD",
        "--",
        ":/rust/crates/dash/src",
    ];
    let added: Vec<String> = (git(&names).unwrap_or_default().lines())
        .map(|p| p.strip_prefix("rust/").unwrap_or(p).to_string())
        .collect();
    let show = |rev: &str, p: &str| {
        git(&["show", &format!("{rev}:rust/crates/{p}/Cargo.toml")]).unwrap_or_default()
    };
    let manifests: Vec<(&str, String, String)> = (crate::PLUGINS.iter())
        .map(|p| (*p, show(base, p), show("HEAD", p)))
        .collect();
    let block = "a done report never builds there: end with a TASK_BLOCKED naming the core port it lacks (rule 19)";
    faults(&added, &manifests)
        .into_iter()
        .map(|f| format!("{f}, {block}"))
        .collect()
}

/// The placement faults of a branch: each `.rs` file it adds under crates/dash/src (`added`, from rust/), and each edge to
/// another plugin a plugin's Cargo.toml holds at its head and not at its base (`manifests`: plugin, base text, head text),
/// read by [`crate::graph::manifest_edges`], the graph's own reader, so a comment or a dev-dependency the graph allows is none.
pub fn faults(added: &[String], manifests: &[(&str, String, String)]) -> Vec<String> {
    let modules = added
        .iter()
        .filter(|p| p.starts_with(&format!("{SRC}/")) && p.ends_with(".rs"));
    let mut out: Vec<String> = modules
        .map(|m| format!("{m}: a new module in the binary"))
        .collect();
    let plugins: Vec<String> = crate::PLUGINS.iter().map(|p| p.to_string()).collect();
    for (from, base, head) in manifests {
        let before = crate::graph::manifest_edges(from, base, &plugins);
        let new = crate::graph::manifest_edges(from, head, &plugins).into_iter();
        for to in new.filter(|to| !before.contains(to)) {
            out.push(format!(
                "crates/{from}/Cargo.toml: a plugin-to-plugin edge, {from} -> {to}"
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// Done-when 2 of brief-placement-blocks, its reading of a branch: a module added to the binary and a plugin's Cargo.toml
    /// taking another plugin are faults; a module added to a plugin, a plugin taking the core, an edge its base held, one in a
    /// comment, a dev-dependency on the design system the graph allows, and a removed edge are not.
    #[dash_test]
    fn a_new_binary_module_or_a_plugin_to_plugin_edge_is_a_fault() {
        let added = ["crates/dash/src/linear.rs", "crates/mail/src/thread.rs"].map(String::from);
        let base = "[dependencies]\ndash-core.workspace = true\ndash-hooks.workspace = true\n";
        let head = "[dependencies]\n# never dash-comments\ndash-core.workspace = true\ndash-drafts = { workspace = true }\n\
                    [dev-dependencies]\ndash-surface.workspace = true\n";
        let kept = "[dependencies]\ndash-hooks = { workspace = true }\n";
        let manifests = [
            ("mail", base.to_string(), head.to_string()),
            ("drafts", base.to_string(), kept.to_string()),
        ];
        assert_eq!(
            faults(&added, &manifests),
            vec![
                "crates/dash/src/linear.rs: a new module in the binary",
                "crates/mail/Cargo.toml: a plugin-to-plugin edge, mail -> drafts",
            ]
        );
        assert!(
            faults(&[], &[]).is_empty(),
            "a branch adding nothing builds nothing anywhere"
        );
    }

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
        let sh = |args: &str| {
            let words: Vec<&str> = args.split(' ').collect();
            git(&rust, &words).expect(args)
        };
        let put = |file: &str, text: &str| std::fs::write(src.join(file), text).expect(file);
        let main = "fn main() {\n    let a = 1;\n    let b = 2;\n    println!(\"{}\", a + b);\n}\n";
        let tests = "#[cfg(test)]\nmod tests {\n    fn t() { let x = 1; }\n}\n";
        sh("init -q");
        // the identity in the repo's own config, for every command that writes one (commit, merge): a CI runner
        // has none, where the Mac guesses one from its hostname (ci-mirror job 132432321584)
        sh("config user.name t");
        sh("config user.email t@t");
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
        let shrunk = sh("rev-parse HEAD").trim().to_string();
        sh("checkout -q -");
        assert_eq!(base(&rust, None), Ok(Ok(at.clone())));
        // a merge of main being committed: measured against main as it merges
        let merge_head = rust.join(".git/MERGE_HEAD");
        std::fs::write(&merge_head, format!("{shrunk}\n")).expect("MERGE_HEAD");
        assert_eq!(base(&rust, None), Ok(Ok(shrunk)));
        std::fs::remove_file(&merge_head).expect("rm");

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
            skip.contains("no merge base of HEAD with origin/main"),
            "{skip}"
        );
    }
}
