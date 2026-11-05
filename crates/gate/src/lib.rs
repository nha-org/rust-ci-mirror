//! Tooling, outside the rings of section 3: the rules CI holds the workspace to. crates/gate/tests runs them.
use std::path::{Path, PathBuf};

/// Section 3: at most this much Rust (@nha, 2026-11-04: « 50k seems like a max »).
pub const LINE_LIMIT: usize = 50_000;

/// The crates of section 3's table, its binary, and this tooling.
pub const RINGS: [&str; 11] = [
    "core", "store", "views", "index", "ingest", "ml", "jobs", "sync", "surface", "ui", "ext",
];
pub const BINARY: &str = "dash";
pub const TOOLING: [&str; 2] = ["meter", "gate"];

/// The only edges between rings besides the one to the core, as section 3 names them.
const NAMED_EDGES: [(&str, &str); 4] = [
    ("surface", "views"),
    ("ui", "views"),
    ("jobs", "ml"),
    ("index", "store"),
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
