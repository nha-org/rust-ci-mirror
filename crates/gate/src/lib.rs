//! Tooling, outside the rings of section 3: the rules CI holds the workspace to. crates/gate/tests runs them.
use std::path::{Path, PathBuf};

/// Section 3: at most this much Rust (@nha, 2026-11-04: « 50k seems like a max »).
pub const LINE_LIMIT: usize = 50_000;

/// The crates of section 3's table, its binary, and this tooling.
pub const RINGS: [&str; 11] = [
    "core", "store", "views", "index", "ingest", "ml", "jobs", "sync", "surface", "ui", "ext",
];
pub const BINARY: &str = "dash";
/// `process` is the bounded call the layers on a command line tool share: it reads a clock and starts threads, which the core
/// may not (a seed reruns the core), and it sits on no port.
pub const TOOLING: [&str; 3] = ["meter", "gate", "process"];
/// Section 3's last row, what phases 1 and 2 build of it, each on a core port. Its own list: rust_budgets_test.exs holds RINGS
/// to the rows of one crate each. `bridge` leaves with the older app's part (section 20b).
pub const TASK_LAYERS: [&str; 7] = [
    "orch", "terminal", "launcher", "agent", "policy", "forge", "bridge",
];
/// Section 20b's rule: a program or provider is a plugin, so the core's source never names one.
pub const NEVER_IN_THE_CORE: [&str; 8] = [
    "herdr",
    "tmux",
    "claude",
    "anthropic",
    "codex",
    "gemini",
    "make",
    "gh",
];

/// The only edges between rings besides the one to the core, as section 3 names them.
const NAMED_EDGES: [(&str, &str); 7] = [
    ("surface", "views"),
    ("ui", "views"),
    ("jobs", "ml"),
    ("index", "store"),
    ("terminal", "process"),
    ("forge", "process"),
    ("bridge", "process"),
];

/// What is wrong in these normal and build dependencies, `from` and `to` without the dash- prefix: the core takes nothing
/// of ours, the rest take the core and section 3's named edges. The binary wires them all, so it is exempt.
pub fn bad_edges(edges: &[(String, String)]) -> Vec<String> {
    edges
        .iter()
        .filter(|(from, to)| {
            let (from, to) = (from.as_str(), to.as_str());
            from != BINARY
                && !(from != "core" && to == "core")
                && !NAMED_EDGES.contains(&(from, to))
        })
        .map(|(from, to)| format!("{from} -> {to}"))
        .collect()
}

/// A dependency loop in these edges, as its path. Cargo refuses one only among normal dependencies: a dev-dependency can
/// close one.
pub fn dependency_loop(edges: &[(String, String)]) -> Option<Vec<String>> {
    fn walk(at: &str, edges: &[(String, String)], path: &mut Vec<String>) -> Option<Vec<String>> {
        if let Some(start) = path.iter().position(|p| p == at) {
            return Some([&path[start..], &[at.to_string()]].concat());
        }
        path.push(at.to_string());
        let next = edges.iter().filter(|(from, _)| from == at);
        let found = next.into_iter().find_map(|(_, to)| walk(to, edges, path));
        path.pop();
        found
    }
    let starts = edges.iter();
    starts
        .into_iter()
        .find_map(|(from, _)| walk(from, edges, &mut vec![]))
}

/// The names of `NEVER_IN_THE_CORE` this text holds as whole words, in any case: « Makefile » is not « make ».
pub fn tools_named(text: &str) -> Vec<&'static str> {
    let named = |tool: &&str| {
        let mut words = text.split(|c: char| !c.is_alphanumeric());
        words.any(|w| w.eq_ignore_ascii_case(tool))
    };
    NEVER_IN_THE_CORE.into_iter().filter(named).collect()
}

/// Every .rs under `root`, build output left out.
pub fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    dirs.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found
}

pub fn line_count(files: &[PathBuf]) -> usize {
    files
        .iter()
        .map(|f| std::fs::read_to_string(f).expect("read").lines().count())
        .sum()
}
