//! Section 3's gates on the workspace itself: the dependency graph, the crate list and the line count. The 120 s of
//! section 17 is .config/nextest.toml's global-timeout and CI's step clock.
use cargo_metadata::{DependencyKind, MetadataCommand};
use dash_gate::{BINARY, LINE_LIMIT, RINGS, TOOLING, bad_edges, line_count, rust_files};
use dash_meter::memory::TestBudget;
use std::path::{Path, PathBuf};

fn rust_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn short(name: &str) -> String {
    name.strip_prefix("dash-").unwrap_or(name).to_string()
}

fn edge(from: &str, to: &str) -> (String, String) {
    (from.to_string(), to.to_string())
}

#[test]
fn the_graph_keeps_to_section_3() {
    let _budget = TestBudget::new();
    let metadata = MetadataCommand::new()
        .manifest_path(rust_root().join("Cargo.toml"))
        .no_deps()
        .exec()
        .expect("cargo metadata");
    let mut names: Vec<String> = metadata.packages.iter().map(|p| short(&p.name)).collect();
    let mut expected: Vec<String> = RINGS
        .iter()
        .chain(TOOLING.iter())
        .chain([&BINARY])
        .map(|s| s.to_string())
        .collect();
    names.sort();
    expected.sort();
    assert_eq!(names, expected, "the workspace's crates are section 3's");

    let ours: Vec<String> = metadata
        .packages
        .iter()
        .map(|p| p.name.to_string())
        .collect();
    let mut edges = vec![];
    for package in &metadata.packages {
        for dep in &package.dependencies {
            // Every test takes the meter; any other dev-dependency is an edge like the rest.
            let tooling = dep.kind == DependencyKind::Development && short(&dep.name) == "meter";
            if !tooling && ours.contains(&dep.name) {
                edges.push(edge(&short(&package.name), &short(&dep.name)));
            }
        }
    }
    assert_eq!(bad_edges(&edges), Vec::<String>::new());
    assert!(
        edges
            .iter()
            .any(|(from, to)| from == "store" && to == "core"),
        "the tracer's path is there"
    );
}

#[test]
fn an_edge_that_skips_the_core_is_named() {
    let _budget = TestBudget::new();
    for bad in [
        ("core", "store"),
        ("core", "meter"),
        ("store", "views"),
        ("views", "ingest"),
        ("ui", "store"),
    ] {
        assert_eq!(
            bad_edges(&[edge(bad.0, bad.1)]),
            [format!("{} -> {}", bad.0, bad.1)]
        );
    }
    for good in [
        ("store", "core"),
        ("surface", "views"),
        ("ui", "views"),
        ("jobs", "ml"),
        ("index", "store"),
        ("dash", "ui"),
    ] {
        assert!(bad_edges(&[edge(good.0, good.1)]).is_empty(), "{good:?}");
    }
}

/// Nanolathe's rule (its INVARIANTS I4, I6) as a check: core and sync read time, randomness and order through the core's
/// ports, and the op log reads the real clock only in its SystemClock adapter. A map walked in the store has no word to
/// catch here (lookups are fine there): the sweep's rerun of each seed catches it.
#[test]
fn what_a_simulation_runs_reads_nothing_a_seed_does_not_decide() {
    let _budget = TestBudget::new();
    // Whole words of the code, comments left out: « Instantly » in a comment is no clock.
    let words = |path: &Path| -> Vec<String> {
        let text = std::fs::read_to_string(path).expect("read");
        text.lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .flat_map(|l| l.split(|c: char| !c.is_alphanumeric() && c != '_'))
            .map(str::to_string)
            .collect()
    };
    let mut bad = vec![];
    let forbidden = [
        "SystemTime",
        "Instant",
        "HashMap",
        "HashSet",
        "RandomState",
        "thread",
        "env",
    ];
    for crate_name in ["core", "sync"] {
        for file in rust_files(&rust_root().join("crates").join(crate_name)) {
            let found = words(&file);
            for word in forbidden {
                if found.iter().any(|w| w == word) {
                    bad.push(format!("{}: {word}", file.display()));
                }
            }
        }
    }
    assert_eq!(bad, Vec::<String>::new());
    let oplog = words(&rust_root().join("crates/store/src/oplog.rs"));
    let count = |word: &str| oplog.iter().filter(|w| *w == word).count();
    assert_eq!(count("SystemTime"), 1, "SystemClock's read, and no other");
    assert_eq!(
        count("Instant"),
        0,
        "the op log takes its time from the clock port"
    );
}

#[test]
fn the_rust_stays_under_the_line_limit() {
    let _budget = TestBudget::new();
    // The workspace's members, as section 3 says: rust/ui-spike is its own workspace until its pick grows into crates/ui.
    let files = rust_files(&rust_root().join("crates"));
    let lines = line_count(&files);
    println!(
        "{lines} lines of Rust in {} files, limit {LINE_LIMIT}",
        files.len()
    );
    assert!(
        files.len() > 10,
        "the walk found the sources: {}",
        files.len()
    );
    assert!(
        lines <= LINE_LIMIT,
        "{lines} lines of Rust is past the limit of {LINE_LIMIT}"
    );
}
