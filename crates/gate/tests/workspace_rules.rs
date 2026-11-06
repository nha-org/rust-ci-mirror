//! Section 3's gates on the workspace itself: the dependency graph, the crate list and the line count. The 120 s of
//! section 17 is .config/nextest.toml's global-timeout and CI's step clock.
use cargo_metadata::{DependencyKind, MetadataCommand};
use dash_gate::{
    BINARY, LINE_LIMIT, RINGS, TASK_LAYERS, TOOLING, bad_edges, dependency_loop, line_count,
    rust_files, tools_named,
};
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
        .chain(TASK_LAYERS.iter())
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
    assert_eq!(dependency_loop(&edges), None);
    assert!(
        edges
            .iter()
            .any(|(from, to)| from == "store" && to == "core"),
        "the tracer's path is there"
    );
    for layer in TASK_LAYERS {
        assert!(
            edges.iter().any(|(from, to)| from == layer && to == "core"),
            "{layer} sits on a core port"
        );
    }
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
        "thread_local",
        "env",
        "option_env",
        "DefaultHasher",
        "fs",
        "getrandom",
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
    // all of its src: tasks.rs stamps through write_if too (/code-review of PR 1115)
    let store: Vec<String> = rust_files(&rust_root().join("crates/store/src"))
        .iter()
        .flat_map(|file| words(file))
        .collect();
    let count = |word: &str| store.iter().filter(|w| *w == word).count();
    assert_eq!(count("SystemTime"), 1, "SystemClock's read, and no other");
    assert_eq!(
        count("Instant"),
        0,
        "the store takes its time from the clock port"
    );
}

#[test]
fn a_dependency_loop_is_named_with_its_path() {
    let _budget = TestBudget::new();
    let found = dependency_loop(&[
        edge("dash", "orch"),
        edge("orch", "agent"),
        edge("agent", "terminal"),
        edge("terminal", "orch"),
    ]);
    assert_eq!(found.expect("a loop").join(" "), "orch agent terminal orch");
    let diamond = [
        edge("x", "a"),
        edge("x", "b"),
        edge("a", "c"),
        edge("b", "c"),
    ];
    assert_eq!(dependency_loop(&diamond), None, "two paths are no loop");
}

#[test]
fn the_core_names_no_tool_or_provider() {
    let _budget = TestBudget::new();
    let files = rust_files(&rust_root().join("crates/core/src"));
    assert!(!files.is_empty(), "the walk found the core's source");
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(tools_named(&text), Vec::<&str>::new(), "{}", file.display());
    }
    assert_eq!(tools_named("// opens a herdr pane"), ["herdr"]);
    assert_eq!(tools_named("Claude Code's transcript"), ["claude"]);
    assert_eq!(tools_named("Command::new(\"make\")"), ["make"]);
    assert!(tools_named("a Makefile, gather, shepherd").is_empty());
}

/// No Steel in any build, whatever its features; a first use waits for his word (@nha, 2026-11-05: « in fact Steel is a FUTURE
/// nice to have. There should be zero of this now. gated on my approval for the first. »).
#[test]
fn no_build_links_steel() {
    let _budget = TestBudget::new();
    // A lockfile holds every optional dependency, so no feature hides one. These two are what ci-mirror.txt carries: ui-spike is
    // its own workspace, and the other spikes' folders are not on the mirror, where reading them would fail.
    for lock in ["Cargo.lock", "ui-spike/Cargo.lock"] {
        let text = std::fs::read_to_string(rust_root().join(lock)).expect(lock);
        assert!(!text.contains("name = \"steel"), "{lock}");
        assert!(
            text.contains("name = \"serde_json\""),
            "{lock}: the packages were read"
        );
    }
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
