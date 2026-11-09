//! Section 3's gates on the workspace itself: the dependency graph, the crate list and the line count. The 120 s of
//! section 17 is .config/nextest.toml's global-timeout and CI's step clock.
use cargo_metadata::{DependencyKind, MetadataCommand};
use dash_gate::graph::{
    agents_named, crate_graph, module_cycles, module_graph, module_holes, page,
};
use dash_gate::html::{COMPONENTS, SHOWCASE, html_in_strings, html_tags, page_local_markup};
use dash_gate::{
    AGENT_KINDS, BINARY, DENO_RULES_EXCLUDE, JOB_WORDS, LINE_LIMIT, NEVER_IN_THE_CORE, OLD_CRM,
    PLUGINS, RINGS, TASK_LAYERS, TOOLING, bad_edges, deno_lint, dependency_loop, inline_runs,
    is_tooling_edge, line_count, old_crm_named, rust_files, rustfmt_check, scripts, tools_named,
    tracked, words_named,
};
use dash_meter::TestDir;
use dash_meter::budgets::BUDGETS;
use dash_meter::memory::TestBudget;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Task rust-gate-rule-firings (@nha, 2026-11-06: « slow is smooth and smooth is fast. we aim for slow but systemic
/// improvement. »): each rule that fails adds `<rule>\t<UTC day number>` to one file per machine, wherever the run started,
/// and `dash gate firings` counts them. A body's panic alone: a rule ended by nextest's timeout or past its memory budget
/// did not fire. ponytail: a CI runner's file dies with the runner, so this counts this Mac's runs.
struct Firing {
    rule: &'static str,
    file: Option<PathBuf>,
}

impl Firing {
    fn new(rule: &'static str) -> Self {
        // A relative HOME, an empty one too, would leave a stray file in the crate, never read (/code-review).
        let home = std::env::var_os("HOME").filter(|h| Path::new(h).is_absolute());
        let file = home.map(|h| Path::new(&h).join(".claude/run/gate-firings.tsv"));
        Self { rule, file }
    }

    fn append(file: &Path, rule: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
        // The day only tells two runs apart, and nothing prints it: a number, with no process to spawn during an unwind.
        let day = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() / 86_400);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)?
            .write_all(format!("{rule}\t{day}\n").as_bytes())
    }
}

impl Drop for Firing {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        // Never a panic here, one during an unwind aborts: the rule's own failure must still print. So writeln!, not
        // eprintln!, which panics on a closed stderr (/code-review).
        let why = match &self.file {
            Some(file) => Self::append(file, self.rule).err().map(|e| e.to_string()),
            None => Some("no absolute HOME".to_string()),
        };
        if let Some(why) = why {
            let _ = writeln!(
                std::io::stderr(),
                "{}: firing not recorded: {why}",
                self.rule
            );
        }
    }
}

/// The name of the function it stands in, so a rule's count cannot drift from its name.
macro_rules! rule_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here)
            .rsplit("::")
            .nth(1)
            .expect("inside a function")
    }};
}

/// Every test's first line. The budget comes first, so its refusal outside nextest is no firing; the firing drops first, so
/// it reads the body's panic and not a budget's.
macro_rules! rule {
    () => {{
        let budget = TestBudget::new();
        (Firing::new(rule_name!()), budget)
    }};
}

const TEST: &str = "#[test]";
const GUARD: &str = "let _budget = rule!();";

/// Each test by its own lines: a count over the file let a test with two guards hide one with none (/code-review). The one
/// shape `dash gate firings` lists and `Firing` counts: the attribute alone on its line, a `fn` returning nothing (an `Err`
/// fails with no panic, so no firing), then the guard. No other attribute: a `should_panic` body would fire on every pass,
/// an `ignore`d or `cfg`'d one would read 0 for ever.
fn tests_out_of_shape(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let at = |i: usize| lines.get(i).copied().unwrap_or("");
    let shaped = |i: usize| {
        at(i) == TEST
            && at(i + 1).starts_with("fn ")
            && !at(i + 1).contains("->")
            && at(i + 2) == GUARD
    };
    (0..lines.len())
        .filter(|&i| at(i).starts_with("#[") && !shaped(i))
        .map(|i| format!("line {}: {} {}", i + 1, at(i), at(i + 1)))
        .collect()
}

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
    let _budget = rule!();
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
        .chain(PLUGINS.iter())
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
            let dev = dep.kind == DependencyKind::Development;
            let tooling = is_tooling_edge(dev, &short(&dep.name));
            if !tooling && ours.contains(&dep.name) {
                edges.push(edge(&short(&package.name), &short(&dep.name)));
            }
        }
    }
    assert_eq!(bad_edges(&edges), Vec::<String>::new());
    assert_eq!(dependency_loop(&edges), None);
    edges.sort();
    edges.dedup();
    assert_eq!(
        crate_graph(&rust_root()).1,
        edges,
        "the page's graph is cargo's"
    );
    assert!(
        edges
            .iter()
            .any(|(from, to)| from == "store" && to == "core"),
        "the tracer's path is there"
    );
    for layer in TASK_LAYERS.iter().chain(&PLUGINS) {
        assert!(
            edges.iter().any(|(from, to)| from == layer && to == "core"),
            "{layer} sits on a core port"
        );
    }
}

/// A planted failure leaves its line, and a rule that holds leaves none.
#[test]
fn a_failing_rule_leaves_its_name_and_day() {
    let _budget = rule!();
    fn planted_rule(file: &Path, fail: bool) {
        let _firing = Firing {
            rule: rule_name!(),
            file: Some(file.to_path_buf()),
        };
        assert!(!fail, "planted");
    }
    let dir = TestDir::new("gate-firings");
    let file = dir.0.join("run/firings.tsv");
    planted_rule(&file, false);
    assert!(!file.exists(), "a rule that holds writes nothing");
    std::panic::catch_unwind(|| planted_rule(&file, true)).expect_err("the plant fails");
    let text = std::fs::read_to_string(&file).expect("the line");
    let (rule, day) = text
        .strip_suffix('\n')
        .expect("one line")
        .split_once('\t')
        .expect("a tab");
    assert_eq!(rule, "planted_rule");
    assert!(day.parse::<u64>().expect("a day number") > 20_000, "{day}");
}

/// No rule of this file escapes the count: a missing guard fails here, never as the plant test's firing.
#[test]
fn every_rule_opens_with_its_guard() {
    let _budget = rule!();
    let source = include_str!("workspace_rules.rs");
    assert_eq!(tests_out_of_shape(source), Vec::<String>::new());
    assert!(source.lines().filter(|l| l.trim() == TEST).count() > 10);
    let good = format!("{TEST}\nfn r() {{\n{GUARD}\n}}");
    assert_eq!(tests_out_of_shape(&good), Vec::<String>::new());
    for bad in [
        format!("{TEST}\nfn r() {{\n}}"),
        format!("  {TEST}\n  fn r() {{\n}}"),
        format!("{TEST} fn r() {{\n{GUARD}\n}}"),
        format!("{TEST}\nfn r() {{}}"),
        format!("{TEST}\nfn r() -> Result<(), String> {{\n{GUARD}\n}}"),
        format!("#[should_panic]\n{good}"),
        format!("#[ignore]\n{good}"),
        format!("#[cfg_attr(target_os = \"macos\", ignore)]\n{good}"),
        format!("#[cfg(target_os = \"linux\")]\n{good}"),
    ] {
        assert_eq!(tests_out_of_shape(&bad).len(), 1, "{bad}");
    }
}

#[test]
fn an_edge_that_skips_the_core_is_named() {
    let _budget = rule!();
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
    let _budget = rule!();
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
    let _budget = rule!();
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
    // Each node on the next two: without a set of finished nodes, 60 of these would not end in any test's time (/code-review).
    let name = |n: usize| format!("n{n:02}");
    let ladder: Vec<_> = (0..60)
        .flat_map(|n| [(name(n), name(n + 1)), (name(n), name(n + 2))])
        .collect();
    assert_eq!(dependency_loop(&ladder), None);
}

fn write(root: &Path, path: &str, text: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("mkdir");
    std::fs::write(file, text).expect("write");
}

#[test]
fn no_crate_holds_a_module_cycle() {
    let _budget = rule!();
    assert_eq!(module_cycles(&rust_root()), Vec::<String>::new());
    assert_eq!(module_holes(&rust_root()), Vec::<String>::new());
    let (modules, edges) = module_graph(&rust_root().join("crates/store/src"));
    assert!(
        modules.iter().any(|m| m == "crate::tasks"),
        "the walk found the modules: {modules:?}"
    );
    assert!(
        edges.contains(&edge("crate::tasks", "crate::oplog")),
        "the walk read the uses: {edges:?}"
    );
}

#[test]
fn a_planted_module_cycle_is_named_with_its_crate() {
    let _budget = rule!();
    let dir = TestDir::new("gate-cycle");
    write(&dir.0, "crates/fine/Cargo.toml", "");
    write(&dir.0, "crates/fine/src/lib.rs", "pub mod a;\n");
    write(
        &dir.0,
        "crates/fine/src/a.rs",
        "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    use super::*;\n}\n",
    );
    assert_eq!(module_cycles(&dir.0), Vec::<String>::new());
    write(&dir.0, "crates/looped/Cargo.toml", "");
    write(&dir.0, "crates/looped/src/lib.rs", "mod one;\nmod two;\n");
    write(&dir.0, "crates/looped/src/one.rs", "use crate::{two::g};\n");
    write(
        &dir.0,
        "crates/looped/src/two.rs",
        "fn g() { super::one::f() }\n",
    );
    assert_eq!(
        module_cycles(&dir.0),
        ["looped: crate::one crate::two crate::one"]
    );
}

#[test]
fn only_code_makes_a_module_edge() {
    let _budget = rule!();
    let dir = TestDir::new("gate-lexer");
    let (src, words) = (
        dir.0.join("src"),
        "mod a;\nmod b;\nmod c;\nmod inner {\n    use super::b::X;\n}\n",
    );
    write(&src, "lib.rs", words);
    // None of these is a use: a comment, each kind of string, a brace in a char, another crate's module of the same name, a method.
    let a = r###"
// crate::b::X
/* crate::b /* nested */ crate::c */
const S: &str = "crate::b \" crate::c";
const R: &str = r#"crate::b " crate::c"#;
const B: char = '{';
const Q: char = '\'';
fn f<'x>(v: &'x V) { other::b::go(); v.b::<u8>(); }
"###;
    write(&src, "a.rs", a);
    write(
        &src,
        "b.rs",
        "pub struct X;\nfn g() { assert_eq!(crate::c::h(), self::Y::z()); }\n",
    );
    write(&src, "c.rs", "use crate::{a as first, b::{self, X}};\n");
    let (modules, edges) = module_graph(&src);
    assert_eq!(
        modules,
        ["crate", "crate::a", "crate::b", "crate::c", "crate::inner"]
    );
    assert_eq!(
        edges,
        [
            edge("crate::b", "crate::c"),
            edge("crate::c", "crate::a"),
            edge("crate::c", "crate::b"),
            edge("crate::inner", "crate::b"),
        ]
    );
}

/// Each way of naming a module that /code-review found the first lexer got wrong: every edge `crate::a` gets.
#[test]
fn every_way_of_naming_a_module_reads_right() {
    let _budget = rule!();
    let (b, inner, c) = ("crate::b", "crate::b::inner", "crate::c");
    let cases: [(&str, &[&str]); 21] = [
        (
            "const N: u8 = 0; fn f() { for _ in 0..crate::b::M {} }",
            &[b],
        ),
        ("fn f() -> S { S { ..crate::b::d() } }", &[b]),
        ("use crate::b as x;", &[b]),
        ("use super::{b as x};", &[b]),
        ("use crate::b; fn f() { b::inner::g() }", &[b, inner]),
        (
            "use crate::b::{self}; fn f() { b::inner::g() }",
            &[b, inner],
        ),
        ("use crate::b as x; fn f() { x::inner::g() }", &[b, inner]),
        ("use crate::r#type::T;", &["crate::type"]),
        ("fn f() { r#type::g() } mod r#type2 {}", &[]),
        ("pub(in crate::b) fn f() {}", &[]),
        ("pub(crate) fn f() {}", &[]),
        ("use std::fmt; fn f() { fmt::inner::g() }", &[]),
        // `as` in code: the path after it is a path.
        ("fn f() { <crate::b::X as crate::c::T>::g() }", &[b, c]),
        // A tuple field's type, not a visibility.
        ("struct S(pub (crate::b::T, u8));", &[b]),
        // Each block's `use` holds in that block alone.
        (
            "fn f() { use crate::b as m; m::inner::g() } fn g() { use crate::c as m; m::h() }",
            &[b, inner, c],
        ),
        // An exported macro is no use of the root, a macro of a module is one of that module, and `!=` calls nothing.
        ("fn f() { crate::log!() }", &[]),
        ("fn f() { crate::b::m!() }", &[b]),
        ("const K: bool = crate::K != 1;", &["crate"]),
        // Through the root's `use`: another crate's name is no use of the root, one of ours is that module.
        ("use crate::store::Foo;", &[]),
        ("fn f() { crate::bee::inner::g() }", &[inner]),
        // A block's `use` through an earlier one of that block.
        (
            "fn f() { use crate::b as m; use m::inner::g; g() }",
            &[b, inner],
        ),
    ];
    let lib =
        "use dash_store as store;\nuse crate::b as bee;\nmod a;\nmod b;\nmod c;\nmod r#type;\n";
    for (a, expected) in cases {
        let dir = TestDir::new("gate-naming");
        write(&dir.0, "lib.rs", lib);
        write(&dir.0, "a.rs", a);
        write(&dir.0, "b.rs", "pub mod inner;\n");
        write(&dir.0, "b/inner.rs", "");
        write(&dir.0, "c.rs", "");
        write(&dir.0, "type.rs", "");
        let (_, edges) = module_graph(&dir.0);
        let from_a: Vec<&str> = edges
            .iter()
            .filter(|(from, _)| from == "crate::a")
            .map(|(_, to)| to.as_str())
            .collect();
        assert_eq!(from_a, expected, "{a}: {edges:?}");
    }
}

/// A layout the walk cannot read stops no build, since rustc may accept it: the page names it and the gate refuses it.
#[test]
fn what_the_walk_cannot_read_is_named() {
    let _budget = rule!();
    let dir = TestDir::new("gate-holes");
    write(&dir.0, "crates/x/Cargo.toml", "");
    write(
        &dir.0,
        "crates/x/src/lib.rs",
        "#[cfg(windows)]\nmod win;\n#[cfg(windows)]\nmod latin;\n",
    );
    std::fs::write(dir.0.join("crates/x/src/latin.rs"), [0xe9u8]).expect("write");
    write(&dir.0, "crates/x/src/stray.rs", "");
    write(&dir.0, "crates/x/src/<b>.rs", "");
    write(&dir.0, "crates/x/src/a\nb.rs", "");
    write(&dir.0, "crates/w/Cargo.toml", "");
    write(
        &dir.0,
        "crates/w/src/lib.rs",
        "include!(\"../x.rs\");\n#[path = \"o.rs\"]\nmod y;\n",
    );
    write(&dir.0, "crates/y/Cargo.toml", "");
    write(&dir.0, "crates/y/src/lib.rs", "");
    write(&dir.0, "crates/y/src/main.rs", "");
    write(&dir.0, "crates/z/Cargo.toml", "");
    let holes = [
        "w: crate has a #[path]",
        "w: crate has an include!",
        "w: crate::y has no file",
        "x: <b>.rs is read by no module",
        "x: a\nb.rs is read by no module",
        "x: crate::latin cannot be read: stream did not contain valid UTF-8",
        "x: crate::win has no file",
        "x: stray.rs is read by no module",
        "y: 2 crate roots",
        "z: 0 crate roots",
    ];
    assert_eq!(module_holes(&dir.0), holes);
    let page = page(&dir.0);
    assert!(page.contains(holes[0]));
    assert!(page.contains("x: <b>.rs is read"), "{page}");
    assert!(page.contains("x: a b.rs is read"), "{page}");
}

#[test]
fn a_manifest_reads_past_comments_and_indents() {
    let _budget = rule!();
    let dir = TestDir::new("gate-manifest");
    let aa = "[dependencies] # ours\n  dash-bb = { path = \"../bb\" }\n[dev-dependencies]\ndash-meter.workspace = true\n";
    write(&dir.0, "crates/aa/Cargo.toml", aa);
    write(&dir.0, "crates/bb/Cargo.toml", "");
    write(&dir.0, "crates/meter/Cargo.toml", "");
    assert_eq!(crate_graph(&dir.0).1, [edge("aa", "bb")]);
}

/// The whole path of the done-when: a real build of a small workspace wired as crates/dash is, an edge added, built again, read
/// from what the binary serves. No registry crate, so it builds offline in seconds.
#[test]
fn a_new_edge_is_on_the_page_after_the_next_build() {
    let _budget = rule!();
    let dir = TestDir::new("gate-build");
    let gate = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = |name: &str, more: &str| {
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{more}")
    };
    write(
        &dir.0,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n",
    );
    write(
        &dir.0,
        "crates/app/Cargo.toml",
        &manifest(
            "app",
            &format!(
                "[build-dependencies]\ndash-gate = {{ path = {:?} }}\n",
                gate
            ),
        ),
    );
    write(
        &dir.0,
        "crates/app/build.rs",
        include_str!("../../dash/build.rs"),
    );
    write(
        &dir.0,
        "crates/app/src/main.rs",
        r#"fn main() { print!("{}", include_str!(concat!(env!("OUT_DIR"), "/deps.txt"))); }"#,
    );
    write(&dir.0, "crates/aa/Cargo.toml", &manifest("dash-aa", ""));
    write(&dir.0, "crates/aa/src/lib.rs", "mod one;\nmod two;\n");
    write(&dir.0, "crates/aa/src/one.rs", "");
    write(&dir.0, "crates/aa/src/two.rs", "");
    write(&dir.0, "crates/bb/Cargo.toml", &manifest("dash-bb", ""));
    write(&dir.0, "crates/bb/src/lib.rs", "");
    let served = || {
        let built = std::process::Command::new(env!("CARGO"))
            .args(["build", "--offline", "--quiet", "--manifest-path"])
            .arg(dir.0.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", dir.0.join("target"))
            .output()
            .expect("cargo");
        let said = String::from_utf8_lossy(&built.stderr);
        assert!(built.status.success(), "{said}");
        let run = std::process::Command::new(dir.0.join("target/debug/app"))
            .output()
            .expect("the binary");
        String::from_utf8(run.stdout).expect("utf-8")
    };
    let module_edge = "crate::one\tcrate::two\n";
    let crate_edge = "aa\tbb\n";
    let before = served();
    assert!(before.contains("crate::one\t\n"), "{before}");
    assert!(before.contains("aa\t\n"), "{before}");

    write(&dir.0, "crates/aa/src/one.rs", "use super::two as other;\n");
    let after = served();
    assert!(after.contains(module_edge), "{after}");

    write(
        &dir.0,
        "crates/aa/Cargo.toml",
        &manifest(
            "dash-aa",
            "[dependencies]\ndash-bb = { path = \"../bb\" }\n",
        ),
    );
    let after = served();
    assert!(after.contains(crate_edge), "{after}");
}

#[test]
fn the_core_names_no_tool_or_provider() {
    let _budget = rule!();
    let files = rust_files(&rust_root().join("crates/core/src"));
    assert!(!files.is_empty(), "the walk found the core's source");
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(tools_named(&text), Vec::<&str>::new(), "{}", file.display());
    }
    assert_eq!(tools_named("// opens a herdr pane"), ["herdr"]);
    assert_eq!(tools_named("Claude Code's transcript"), ["claude"]);
    assert_eq!(tools_named("Command::new(\"make\")"), ["make"]);
    assert_eq!(tools_named("DD_API_KEY"), ["dd"]);
    assert_eq!(tools_named("the Datadog agent"), ["datadog"]);
    assert!(tools_named("a Makefile, gather, shepherd").is_empty());
}

/// rfc-rust-plugins-ui section 3: mail works with the job hunt removed, so nothing in its source, tests or manifest names it.
#[test]
fn the_mail_names_no_job_word() {
    let _budget = rule!();
    let mail = rust_root().join("crates/mail");
    let mut files = rust_files(&mail);
    files.push(mail.join("Cargo.toml"));
    files.push(mail.join("tests/inbox.json"));
    assert!(files.len() > 3, "the walk found mail's source and tests");
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(
            words_named(&text, &JOB_WORDS),
            Vec::<&str>::new(),
            "{}",
            file.display()
        );
    }
    assert_eq!(
        words_named("the Lead's CV, an ad", &JOB_WORDS),
        ["lead", "ad", "cv"]
    );
    assert!(words_named("a header, already loaded, adds", &JOB_WORDS).is_empty());
}

#[test]
fn the_orch_names_no_agent_kind() {
    let _budget = rule!();
    let orch = rust_root().join("crates/orch");
    let files = rust_files(&orch);
    assert!(files.len() > 1, "the walk found orch's source and tests");
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(
            agents_named(&text),
            Vec::<&str>::new(),
            "{}",
            file.display()
        );
    }
    // A dependency or a feature named for an agent is an orch line too (/code-review); its # comments are allowed.
    let manifest = std::fs::read_to_string(orch.join("Cargo.toml")).expect("read");
    let manifest: Vec<&str> = manifest
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    assert!(
        manifest.join("\n").contains("dash-orch"),
        "the manifest was read"
    );
    assert_eq!(
        agents_named(&manifest.join("\n")),
        Vec::<&str>::new(),
        "orch's Cargo.toml"
    );
    // One list of agents: the core bans each one too, so a new kind added here is never left out there.
    assert!(
        AGENT_KINDS.iter().all(|a| NEVER_IN_THE_CORE.contains(a)),
        "{AGENT_KINDS:?}"
    );
    assert_eq!(agents_named("env(\"ANTHROPIC_API_KEY\")"), ["anthropic"]);
    assert_eq!(agents_named("Command::new(\"claude\")"), ["claude"]);
    assert_eq!(agents_named("struct ClaudeRun;"), ["claude"]);
    assert_eq!(agents_named("env(\"CODEX_HOME\")"), ["codex"]);
    assert_eq!(agents_named("let s = r#\"gemini -p\"#;"), ["gemini"]);
    assert!(agents_named("// a Claude's id\n/// codex too\n/* Gemini */ fn run() {}").is_empty());
}

#[test]
fn the_dash_crate_holds_no_html() {
    let _budget = rule!();
    assert_eq!(html_in_strings(&rust_root()), Vec::<String>::new());
}

/// The rule above on a planted tree: each place a page could come back from, and what must not trip it.
#[test]
fn a_planted_tag_fails_the_gate() {
    let _budget = rule!();
    let dir = TestDir::new("gate-html");
    let clean = r##"// a <td> in a comment
fn main() { println!("usage: dash --data <dir> --port <n> <https://x> <a-task> 1 < 2"); }
"##;
    write(&dir.0, "crates/dash/src/main.rs", clean);
    write(&dir.0, "crates/gate/src/lib.rs", clean);
    assert_eq!(html_in_strings(&dir.0), Vec::<String>::new());

    let planted = [
        ("crates/dash/src/main.rs", r#""<td>{x}</td>""#, "<td"),
        ("crates/dash/src/tasks.rs", r##"r#"<a href="/">"#"##, "<a"),
        ("crates/dash/build.rs", r#""<!doctype html>""#, "<!"),
        ("crates/dash/tests/t.rs", r#""\n<TABLE>""#, "<TABLE"),
        ("crates/gate/src/graph.rs", r#""<svg width>""#, "<svg"),
    ];
    for (path, literal, tag) in planted {
        write(&dir.0, path, &format!("const X: &str = {literal};\n"));
        let found = html_in_strings(&dir.0);
        assert!(
            found.iter().any(|f| f.contains(path) && f.ends_with(tag)),
            "{path}: {found:?}"
        );
        write(&dir.0, path, clean);
    }
    assert_eq!(html_tags("x</td"), ["</td"]);
    assert_eq!(html_tags("<p> <!-- x"), ["<p", "<!"]);
    assert_eq!(html_tags("Foo<S> Wrapper<P> x<!y"), Vec::<String>::new());

    let latin = dir.0.join("crates/dash/src/latin.rs");
    std::fs::write(&latin, [0xe9u8]).expect("write");
    let found = html_in_strings(&dir.0);
    assert!(found.iter().any(|f| f.contains("latin.rs: ")), "{found:?}");
}

/// Done-when 2 of rust-design-components: the tasks board and the tools page draw with crates/surface/src/components.rs, and
/// no page draws by hand what a component covers (@nha, 2026-11-09: « We want all components to be dual-use »).
#[test]
fn no_page_draws_what_a_component_covers() {
    let _budget = rule!();
    assert_eq!(page_local_markup(&rust_root()), Vec::<String>::new());
}

/// The rule above on a planted tree: a page's own button or card fails it, the component's file, the showcase and plain
/// Rust that merely names a field `input` pass.
#[test]
fn a_planted_button_or_card_fails_the_gate() {
    let _budget = rule!();
    let dir = TestDir::new("gate-components");
    let owned = "pub const OWNED_TAGS: [&str; 2] = [\"button\", \"input\"];\npub const OWNED_CLASSES: [&str; 1] = [\"card\"];\n";
    write(
        &dir.0,
        COMPONENTS,
        &format!("{owned}fn b() {{ rsx! {{ button {{ class: \"card\" }} }} }}\n"),
    );
    write(
        &dir.0,
        SHOWCASE[0],
        "fn s() { rsx! { div { button { class: \"card x\" } } } }\n",
    );
    let clean = "fn f(input: u8) { match input { _ => {} } }\nfn g() { rsx! { div { class: \"cards\", Button { \"Go\" } } } }\n";
    write(&dir.0, "crates/surface/src/page.rs", clean);
    assert_eq!(page_local_markup(&dir.0), Vec::<String>::new());

    let planted = [
        ("fn g() { rsx! { div { button { \"Go\" } } } }", "button {"),
        ("fn g() { rsx! { p {}, input { name: \"q\" } } }", "input {"),
        (
            "fn g() { rsx! { a { class: \"meta card\", href } } }",
            "class: \"card\"",
        ),
        (
            "fn g() { rsx! { a { class:\"card\" } } }",
            "class: \"card\"",
        ),
        (
            "const X: &str = r#\"<form><INPUT name=q></form>\"#;",
            "1 button or input in strings, 0 held",
        ),
    ];
    for (code, what) in planted {
        write(&dir.0, "crates/surface/src/page.rs", code);
        let found = page_local_markup(&dir.0);
        assert_eq!(
            found,
            [format!("crates/surface/src/page.rs: {what}")],
            "{code}"
        );
    }
    // a test's own HTML is no page: past `#[cfg(test)]` a string is not counted
    let tested = "// draws class: \"card\"\nfn g() {}\n#[cfg(test)]\nmod tests { const X: &str = \"<button>\"; fn t() { rsx! { div { button { class: \"card\" } } } } }\n";
    write(&dir.0, "crates/surface/src/page.rs", tested);
    assert_eq!(page_local_markup(&dir.0), Vec::<String>::new());
    write(&dir.0, COMPONENTS, "pub const NOTHING: u8 = 0;\n");
    let found = page_local_markup(&dir.0);
    assert!(
        found.iter().any(|f| f.contains("no OWNED_TAGS")),
        "{found:?}"
    );
}

/// No Steel in any build, whatever its features; a first use waits for his word (@nha, 2026-11-05: « in fact Steel is a FUTURE
/// nice to have. There should be zero of this now. gated on my approval for the first. »).
#[test]
fn no_build_links_steel() {
    let _budget = rule!();
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
    let _budget = rule!();
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

/// What CrmGoneTest (private/dashboard/test/dashboard/crm_gone_test.exs) refuses in rust/, here so a Rust PR meets it before
/// merge: the mirror runs no Elixir, and #1180 merged a fixture naming the old folder that turned main red.
#[test]
fn no_rust_file_names_the_old_crm() {
    let _budget = rule!();
    let files = tracked(&rust_root());
    assert!(
        files.len() > 50,
        "git listed the tracked files: {}",
        files.len()
    );
    assert_eq!(old_crm_named(&rust_root(), &files), Vec::<String>::new());
}

/// The rule above on a planted crate: #1180's fixture, each spelling, and what must not trip it.
#[test]
fn a_fixture_naming_private_crm_fails_the_gate() {
    let _budget = rule!();
    let dir = TestDir::new("gate-crm");
    let [folder, app] = OLD_CRM;
    let planted = [
        (
            "crates/drafts/src/lib.rs",
            format!("let path = \"{folder}/drafts/x.md\";"),
        ),
        (
            "crates/drafts/tests/t.rs",
            format!("// see {}/notes", folder.to_uppercase()),
        ),
        ("crates/ui/src/a.rs", format!("// the {app} page")),
        ("Cargo.toml", format!("# {app}: gone")),
    ];
    let clean = [
        (
            "crates/drafts/src/ok.rs",
            format!("// old/drafts/x.md, twenty-{app}, x{app}"),
        ),
        (
            "crates/drafts/log.jsonl",
            format!("{{\"path\":\"{folder}/drafts/x.md\"}}"),
        ),
    ];
    for (path, text) in planted.iter().chain(&clean) {
        write(&dir.0, path, text);
    }
    let files: Vec<(String, String)> = planted
        .iter()
        .chain(&clean)
        .map(|(path, _)| ("100644".to_string(), path.to_string()))
        .collect();
    let found = old_crm_named(&dir.0, &files);
    let expected: Vec<String> = planted.iter().map(|(p, _)| p.to_string()).collect();
    assert_eq!(found, expected);
}

/// `mix lint`'s script count (Dashboard.Lint.scripts) on rust/: a Rust PR runs no lint, so a script or a #! opening merged here
/// fails main's next commit.
#[test]
fn rust_holds_no_script() {
    let _budget = rule!();
    let files = tracked(&rust_root());
    assert_eq!(scripts(&rust_root(), &files), Vec::<String>::new());
}

#[test]
fn a_planted_script_fails_the_gate() {
    let _budget = rule!();
    let dir = TestDir::new("gate-scripts");
    let files = [
        ("100644", "tools/run.sh", "echo"),
        ("100644", "tools/fit.py", "x = 1"),
        ("100755", "tools/go", "fn"),
        ("100755", "bin/dash", "#!/bin/bash\n"),
        ("100755", "crates/bin/dash", "#!/bin/bash\n"),
        ("100644", "crates/a/src/lib.rs", "#![deny(warnings)]\n"),
        (
            "100644",
            "crates/a/src/ok.rs",
            "//! a module\n#![deny(warnings)]\n",
        ),
        ("100644", "crates/a/empty.rs", ""),
        // the CI's own, executable: held by the lint's count
        ("100755", "ci/build.sh", "#!/bin/bash\n"),
        ("100644", "ci/lost.sh", "#!/bin/bash\n"),
    ];
    for (_, path, text) in files {
        write(&dir.0, path, text);
    }
    let listed: Vec<(String, String)> = files
        .iter()
        .map(|(mode, path, _)| (mode.to_string(), path.to_string()))
        .collect();
    assert_eq!(
        scripts(&dir.0, &listed),
        [
            "tools/run.sh: a script",
            "tools/fit.py: a script",
            "tools/go: a script",
            "crates/bin/dash: a script",
            "crates/a/src/lib.rs: opens on #! without its executable bit",
            "ci/lost.sh: a script",
        ]
    );
    // the held script without its executable bit is any #! file
    assert_eq!(
        scripts(&dir.0, &[("100644".into(), "bin/dash".into())]),
        ["bin/dash: opens on #! without its executable bit"]
    );
}

/// The mirror's workflow runs one line a step, its logic in ci/'s scripts.
#[test]
fn ci_steps_are_one_line() {
    let _budget = rule!();
    let ci = std::fs::read_to_string(rust_root().join(".github/workflows/ci.yml")).expect("ci.yml");
    assert_eq!(inline_runs(&ci), Vec::<String>::new());
}

/// site_perso's workflows too, their logic in bin/ci/ (task ci-steps-scripts). The public mirror holds rust/ alone, so
/// there this reads nothing; it holds on the Mac before each push (rust/CLAUDE.md rule 2).
#[test]
fn site_perso_ci_steps_are_one_line() {
    let _budget = rule!();
    let dir = rust_root().join("../.github/workflows");
    // the monorepo is told by its dashboard: there a missing folder fails, never reads as the mirror (/code-review)
    if !rust_root().join("../private/dashboard").is_dir() {
        println!("no private/dashboard beside rust/: the mirror, which holds rust/ alone");
        return;
    }
    let entries = std::fs::read_dir(&dir).expect("site_perso's .github/workflows");
    let mut bad = vec![];
    for path in entries.map(|e| e.expect("entry").path()) {
        if path.extension().is_some_and(|e| e == "yml") {
            let text = std::fs::read_to_string(&path).expect("workflow");
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            bad.extend(
                inline_runs(&text)
                    .into_iter()
                    .map(|b| format!("{name} {b} in bin/ci/")),
            );
        }
    }
    // each script runs with -e as GitHub ran its step: six of the nine first moved without it, and a failing
    // bin/approval-number went green (/code-review of ci-steps-scripts)
    let scripts = std::fs::read_dir(rust_root().join("../bin/ci")).expect("bin/ci/");
    for path in scripts.map(|e| e.expect("entry").path()) {
        let text = std::fs::read_to_string(&path).expect("script");
        if !text.lines().any(|l| l.starts_with("set -e")) {
            bad.push(format!(
                "{}: no set -e line, as its step ran with bash -e",
                path.display()
            ));
        }
    }
    assert_eq!(bad, Vec::<String>::new());
}

#[test]
fn a_planted_run_block_fails_the_gate() {
    let _budget = rule!();
    let planted = "steps:\n  - run: ci/ok.sh\n  - run: |\n      echo a\n  - name: x\n    run: >-\n      echo b\n  - run: a &&\n      # a comment line\n      b\n  - run: one line\n    # its comment\n    shell: bash\n  -   run: |\n      echo c\ndefaults:\n  run:\n    shell: bash\n";
    assert_eq!(
        inline_runs(planted),
        [
            "line 3: a run of several lines, make it a script",
            "line 6: a run of several lines, make it a script",
            "line 8: a run of several lines, make it a script",
            "line 14: a run of several lines, make it a script",
        ]
    );
}

/// The section of rfc-rust-unified that opens on `heading`, to its `</section>`.
fn rfc_section<'a>(rfc: &'a str, heading: &str) -> &'a str {
    let (_, rest) = rfc.split_once(heading).expect(heading);
    rest.split_once("</section>").expect("its end").0
}

/// The first two cells of each `<tr><td>` row, both plain text: rust_budgets_test.exs reads `[^<]+` cells (/code-review).
fn rows(section: &str) -> Vec<(&str, &str)> {
    section
        .split("<tr><td>")
        .skip(1)
        .filter_map(|row| {
            let mut cells = row.split("</td><td>");
            Some((cells.next()?, cells.next()?.split("</td>").next()?))
        })
        .filter(|(a, b)| !a.contains('<') && !b.contains('<'))
        .collect()
}

/// « 4 GB » in megabytes: the first whole number followed by « MB » or « GB », as the Elixir side's `(\d+) (MB|GB)` reads it.
fn limit_mb(budget: &str) -> Option<f64> {
    let digit = |i: usize| budget.as_bytes().get(i).is_some_and(u8::is_ascii_digit);
    (0..budget.len()).filter(|&i| digit(i)).find_map(|start| {
        let end = (start..).find(|&i| !digit(i))?;
        let n: f64 = budget[start..end].parse().ok()?;
        match budget.get(end..end + 3)? {
            " MB" => Some(n),
            " GB" => Some(n * 1024.0),
            _ => None,
        }
    })
}

/// rust_budgets_test.exs, Rust side: section 3's crates and named edges, and section 4's budgets, are the gate's RINGS and
/// edges and the meter's BUDGETS. Its own rows of each: the RFC is private, so on the public mirror, which holds rust/
/// alone, this reads nothing; it holds on the Mac before each push (rust/CLAUDE.md rule 2), and the Elixir test on an RFC edit.
#[test]
fn sections_3_and_4_are_the_gate_and_the_meter() {
    let _budget = rule!();
    assert_eq!(
        rows("<tr><td>a</td><td>b</td></tr><tr><td>c</td></tr>"),
        [("a", "b")]
    );
    assert_eq!(limit_mb("+4 GB, for the job only"), Some(4096.0));
    assert_eq!(limit_mb("Nothing below"), None);
    assert_eq!(
        limit_mb("(1.5 GB)"),
        Some(5120.0),
        "the Elixir side's reading"
    );
    assert_eq!(rows("<tr><td>a</td><td><code>4 GB</code></td></tr>"), []);
    let path = rust_root().join("../private/docs/rfc-rust-unified.html");
    if !path.is_file() {
        println!(
            "no {} beside rust/: the mirror, which holds rust/ alone",
            path.display()
        );
        return;
    }
    let rfc = std::fs::read_to_string(&path).expect("rfc-rust-unified.html");

    let section = rfc_section(&rfc, "<h2>3. Architecture</h2>");
    let mut crates: Vec<&str> = rows(section)
        .into_iter()
        .filter(|(name, kind)| {
            ["Core", "Adapter", "Extension"].contains(kind)
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
        .map(|(name, _)| name)
        .collect();
    let mut rings = RINGS.to_vec();
    crates.sort();
    rings.sort();
    assert_eq!(crates, rings, "section 3's one-crate rows are RINGS");
    assert!(section.contains(
        "surface and ui on views' diff stream, jobs on ml, index on store for its rebuild"
    ));
    let plugins: Vec<&str> = rows(section)
        .into_iter()
        .filter(|(_, kind)| *kind == "Plugin")
        .flat_map(|(names, _)| names.split(", "))
        .collect();
    assert_eq!(plugins, PLUGINS, "section 3's plugin row is PLUGINS");
    for (from, to) in [
        ("surface", "views"),
        ("ui", "views"),
        ("jobs", "ml"),
        ("index", "store"),
    ] {
        assert_eq!(
            bad_edges(&[edge(from, to)]),
            Vec::<String>::new(),
            "{from} -> {to}"
        );
    }

    let section = rfc_section(&rfc, "<h2>4. Low memory</h2>");
    let mut in_rfc: Vec<(String, String, f64)> = rows(section)
        .into_iter()
        .filter(|(_, budget)| {
            budget
                .trim_start_matches('+')
                .starts_with(|c: char| c.is_ascii_digit())
        })
        .filter_map(|(state, budget)| {
            Some((state.to_string(), budget.to_string(), limit_mb(budget)?))
        })
        .collect();
    let mut in_meter: Vec<(String, String, f64)> = BUDGETS
        .iter()
        .map(|b| (b.state.to_string(), b.budget.to_string(), b.limit_mb))
        .collect();
    assert_eq!(in_rfc.len(), 4, "{in_rfc:?}");
    in_rfc.sort_by(|a, b| a.0.cmp(&b.0));
    in_meter.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(in_meter, in_rfc, "the meter's BUDGETS are section 4's");
}

/// `mix lint`'s deno run (Dashboard.Lint's `@deno`) on rust/'s scripts for the browser, web-check.mjs today: a Rust PR runs no
/// lint, so a finding merged here fails main's next commit.
#[test]
fn rust_js_passes_mix_lints_deno() {
    let _budget = rule!();
    let files = tracked(&rust_root());
    assert!(
        files
            .iter()
            .any(|(_, path)| path == "ui-spike/dioxus-page/web-check.mjs"),
        "the walk found the .mjs"
    );
    assert_eq!(deno_lint(&rust_root(), &files), Vec::<String>::new());
}

#[test]
fn a_planted_lint_finding_fails_the_gate() {
    let _budget = rule!();
    let dir = TestDir::new("gate-deno");
    write(&dir.0, "ok.mjs", "export const page = window.location;\n");
    write(&dir.0, "bad.mjs", "const unused = 1;\n");
    write(&dir.0, "notes.md", "const unused = 1;\n");
    let listed = |paths: &[&str]| -> Vec<(String, String)> {
        paths
            .iter()
            .map(|p| ("100644".to_string(), p.to_string()))
            .collect()
    };
    assert_eq!(
        deno_lint(&dir.0, &listed(&["ok.mjs", "notes.md"])),
        Vec::<String>::new()
    );
    let found = deno_lint(&dir.0, &listed(&["ok.mjs", "bad.mjs"])).join("\n");
    assert!(found.contains("no-unused-vars"), "{found}");
    assert!(found.contains("bad.mjs"), "{found}");
}

/// Task rust-fmt-gate: rust/ stays as `cargo fmt` leaves it, so a format run rewrites only the files it touches
/// (rust-nav-polish's rewrote untouched ones). The fix: `git ls-files -z '*.rs' | xargs -0 rustfmt --edition 2024` in rust/.
#[test]
fn rust_is_rustfmt_clean() {
    let _budget = rule!();
    let files = tracked(&rust_root());
    assert_eq!(rustfmt_check(&rust_root(), &files), Vec::<String>::new());
}

#[test]
fn a_planted_unformatted_file_fails_the_gate() {
    let _budget = rule!();
    let dir = TestDir::new("gate-rustfmt");
    write(&dir.0, "ok.rs", "fn main() {}\n");
    // bad.rs also reached through parent.rs's `mod`, so rustfmt names it twice: the gate names it once
    write(&dir.0, "parent.rs", "mod bad;\n");
    write(&dir.0, "bad.rs", "fn  main(){ let x=1; }\n");
    write(&dir.0, "notes.md", "fn  main(){}\n");
    let listed = |paths: &[&str]| -> Vec<(String, String)> {
        paths
            .iter()
            .map(|p| ("100644".to_string(), p.to_string()))
            .collect()
    };
    assert_eq!(
        rustfmt_check(&dir.0, &listed(&["ok.rs", "notes.md"])),
        Vec::<String>::new()
    );
    let found = rustfmt_check(&dir.0, &listed(&["ok.rs", "parent.rs", "bad.rs"]));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].ends_with("bad.rs"), "{found:?}");
    // a file rustfmt cannot parse fails too, with its error
    write(&dir.0, "broken.rs", "fn (\n");
    let found = rustfmt_check(&dir.0, &listed(&["broken.rs"])).join("\n");
    assert!(found.contains("unclosed delimiter"), "{found}");
}

/// The deno the gate runs is mix lint's (/code-review): the mirror's pin and checksum are bin/ci/lint-tools.sh's, the flags are
/// Dashboard.Lint's `@deno`, and the deno on PATH is the pinned version, as bin/ci/lint-tools.sh's Mac pin refuses a brew upgrade. The
/// two files sit beside rust/ only in the monorepo: on the mirror, the version alone.
#[test]
fn the_deno_copies_are_mix_lints() {
    let _budget = rule!();
    let ci = std::fs::read_to_string(rust_root().join("ci/deno.sh")).expect("ci/deno.sh");
    let url = ci
        .split_whitespace()
        .find(|w| w.starts_with("https://github.com/denoland/deno/releases/download/"))
        .expect("the mirror's deno url");
    let sum = ci
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|w| w.len() == 64)
        .expect("its sha256");
    let version = url.split('/').nth(7).expect("/download/v<version>/");
    let out = std::process::Command::new("deno")
        .arg("--version")
        .output()
        .expect("deno on PATH");
    let installed = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        installed.starts_with(&format!("deno {} ", version.trim_start_matches('v'))),
        "deno on PATH is not the pinned {version}: {installed}"
    );
    let monorepo = rust_root().join("..");
    let (check, lint) = (
        monorepo.join("bin/ci/lint-tools.sh"),
        monorepo.join("private/dashboard/lib/dashboard/lint.ex"),
    );
    if !check.is_file() || !lint.is_file() {
        println!("no bin/ci/lint-tools.sh or lint.ex beside rust/: the mirror, the version alone");
        return;
    }
    let check = std::fs::read_to_string(check).expect("bin/ci/lint-tools.sh");
    assert!(
        check.contains(url),
        "bin/ci/lint-tools.sh pins another deno than {url}"
    );
    assert!(
        check.contains(sum),
        "bin/ci/lint-tools.sh's deno checksum is not {sum}"
    );
    let lint = std::fs::read_to_string(lint).expect("lint.ex");
    let line = lint
        .lines()
        .find(|l| l.trim_start().starts_with("@deno ~w("))
        .expect("lint.ex's @deno");
    assert!(
        line.contains(&format!("deno lint --quiet {DENO_RULES_EXCLUDE})")),
        "lint.ex's {line} is not the gate's {DENO_RULES_EXCLUDE}"
    );
}
