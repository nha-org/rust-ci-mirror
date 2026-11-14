//! Section 3's gates on the workspace itself: the dependency graph, the crate list and the line count. The 120 s of
//! section 17 is .config/nextest.toml's global-timeout and CI's step clock.
use cargo_metadata::{DependencyKind, MetadataCommand};
use dash_gate::browser::{
    MARK as BROWSER_MARK, browser_tests, group as browser_group, verdict as browser_verdict,
};
use dash_gate::build_paths::build_paths_at_run_time;
use dash_gate::graph::{
    CRATE_LOOP_FIX, EDGE_FIX, HOLE_FIX, Lexeme, MODULE_LOOP_FIX, PLACE_FIX, PORT_FIX, agents_named,
    crate_graph, layer, lexed, module_cycles, module_graph, module_holes, violations,
};
use dash_gate::html::{
    BARE_REFUSED, BOARD, COMPONENTS, EDITABLE, FIELDS, NOT_WORDS, SHOWCASE, board_markup,
    components_without_a_terminal_form, editable_outside, fields_outside, html_in_strings,
    html_tags, hub_named_pages, page_local_markup, registrations, reload_promised, rsx_in_dash,
    shared_component_lists,
};
use dash_gate::inline_js::{MARK, inline_scripts, verdict as inline_js_verdict};
use dash_gate::{
    AGENT_KINDS, BINARY, BROWSER_LAUNCH, DENO_RULES_EXCLUDE, JOB_WORDS, LEAVES, MARKERS_HOME,
    NEVER_IN_THE_CORE, OLD_CRM, PLANNED_PLUGINS, PLUGINS, REACHES_PAST_RUST, RINGS, TASK_LAYERS,
    TOOLING, apt_in_workflow, bad_edges, binary_fixtures, browsers_without_slot, deno_lint,
    dependency_loop, includes_off_the_mirror, inline_runs, is_tooling_edge, marker_literals,
    old_crm_named, run_time_paths_past_rust, rust_files, rustfmt_check, scripts, tools_named,
    tracked, words_named,
};
use dash_meter::TestDir;
use dash_meter::budgets::BUDGETS;
use dash_meter::dash_test;
use dash_meter::memory::TestBudget;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

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
        let file = dash_gate::firings::log_from_env();
        Self { rule, file }
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
            Some(file) => dash_gate::firings::record(file, self.rule)
                .err()
                .map(|e| e.to_string()),
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

/// Every rule's guard, `#[dash_test(rule!())]`, which opens the body with it. The budget comes first, so its refusal outside
/// nextest is no firing; the firing drops first, so it reads the body's panic and not a budget's.
macro_rules! rule {
    () => {{
        let budget = TestBudget::new();
        (Firing::new(rule_name!()), budget)
    }};
}

const TEST: &str = dash_gate::firings::RULE;

/// Each test by its own lines: a count over the file let a test with two guards hide one with none (/code-review). The one
/// shape `dash gate firings` lists and `Firing` counts: the attribute, which holds the guard, alone on its line, then a `fn`
/// returning nothing (an `Err` fails with no panic, so no firing). No other attribute: a `should_panic` body would fire on every pass,
/// an `ignore`d or `cfg`'d one would read 0 for ever.
fn tests_out_of_shape(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let at = |i: usize| lines.get(i).copied().unwrap_or("");
    let shaped =
        |i: usize| at(i) == TEST && at(i + 1).starts_with("fn ") && !at(i + 1).contains("->");
    (0..lines.len())
        .filter(|&i| at(i).starts_with("#[") && !shaped(i))
        .map(|i| format!("line {}: {} {}", i + 1, at(i), at(i + 1)))
        .collect()
}

/// Each `assert!` or `assert_eq!` call of this source, its arguments split at their top-level commas. The gate's own lexer
/// reads it, so a comment, a string or a char literal holds no call and splits nothing (/code-review).
fn assert_calls(source: &str) -> Vec<(&'static str, Vec<Vec<Lexeme>>)> {
    let lexemes = lexed(source);
    let code = |i: usize, t: &str| matches!(lexemes.get(i), Some(Lexeme::Code(c)) if c == t);
    let mut calls = vec![];
    for i in 0..lexemes.len() {
        let Some(name) = ["assert", "assert_eq"].into_iter().find(|n| code(i, n)) else {
            continue;
        };
        if !(code(i + 1, "!") && code(i + 2, "(")) {
            continue;
        }
        let (mut args, mut arg, mut depth) = (vec![], vec![], 0);
        for lexeme in &lexemes[i + 3..] {
            match lexeme {
                Lexeme::Code(c) if matches!(c.as_str(), "(" | "[" | "{") => depth += 1,
                Lexeme::Code(c) if matches!(c.as_str(), ")" | "]" | "}") => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                Lexeme::Code(c) if c == "," && depth == 0 => {
                    args.push(std::mem::take(&mut arg));
                    continue;
                }
                _ => {}
            }
            arg.push(lexeme.clone());
        }
        args.push(arg);
        args.retain(|a| !a.is_empty());
        calls.push((name, args));
    }
    calls
}

/// An argument as code, with no spaces; a string reads `""`, so its `&&` or `(` is none.
fn code_text(arg: &[Lexeme]) -> String {
    arg.iter()
        .map(|l| match l {
            Lexeme::Code(c) => c.as_str(),
            Lexeme::Str(_) => "\"\"",
        })
        .collect()
}

/// The words a message's strings say of their own, `{}` placeholders left out and `{{`, `}}` kept: « {} »,
/// file.display() names where, no fix.
fn own_words(message: &[Vec<Lexeme>]) -> usize {
    let mut text = String::new();
    for lexeme in message.iter().flatten() {
        let Lexeme::Str(s) = lexeme else { continue };
        let (mut chars, mut inside) = (s.chars().peekable(), false);
        while let Some(c) = chars.next() {
            match c {
                '{' | '}' if !inside && chars.peek() == Some(&c) => {
                    chars.next();
                    text.push(c);
                }
                '{' => inside = true,
                '}' => inside = false,
                _ if !inside => text.push(c),
                _ => {}
            }
        }
        text.push(' ');
    }
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphabetic))
        .count()
}

/// A condition's parts joined by a top-level `&&`.
fn conjuncts(condition: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (vec![], 0i32, 0);
    let bytes = condition.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'&' if depth == 0 && i >= start && bytes.get(i + 1) == Some(&b'&') => {
                parts.push(&condition[start..i]);
                start = i + 2;
            }
            _ => {}
        }
    }
    parts.push(&condition[start.min(condition.len())..]);
    parts
}

/// Task rust-gate-says-fix (@nha, 2026-10-29: « You fixed the mistake but didn't find the cause and what could actually fix
/// that class of errors right ? »): an assertion that a found list is empty says what to do when it is not, in a message of
/// four words or more of its own; a bare assert_eq of the list against an empty Vec prints only where. ponytail: counts words,
/// so a four-word message naming no fix passes, and a message held in a const reads as none; reviewers read the wording.
/// An `||` condition is not read: either side may hold.
fn bare_list_asserts(source: &str) -> Vec<String> {
    let empty = |arg: &str| {
        let arg = arg.trim_start_matches('&');
        let empty_new = ["Vec", "BTreeSet", "HashSet", "BTreeMap", "HashMap"]
            .iter()
            .any(|t| arg.starts_with(t) && arg.ends_with("::new()"));
        empty_new || ["[]", "vec![]"].contains(&arg)
    };
    let refuses = |part: &str| {
        (part.ends_with(".is_empty()") && !part.starts_with('!')) || part.ends_with(".len()==0")
    };
    assert_calls(source)
        .into_iter()
        .filter_map(|(name, args)| {
            let texts: Vec<String> = args.iter().map(|a| code_text(a)).collect();
            let (list, message) = match (name, texts.as_slice()) {
                ("assert", [check, ..]) => (conjuncts(check).into_iter().any(refuses), &args[1..]),
                ("assert_eq", [a, b, ..]) => {
                    let len_zero =
                        (a.ends_with(".len()") && b == "0") || (b.ends_with(".len()") && a == "0");
                    (empty(a) || empty(b) || len_zero, &args[2..])
                }
                _ => return None,
            };
            (list && own_words(message) < 4).then(|| format!("{name}!({})", texts[0]))
        })
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

#[dash_test(rule!())]
fn the_graph_keeps_to_section_3() {
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
        .chain(LEAVES.iter())
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
            let tooling = is_tooling_edge(&short(&package.name), dev, &short(&dep.name));
            if !tooling && ours.contains(&dep.name) {
                edges.push(edge(&short(&package.name), &short(&dep.name)));
            }
        }
    }
    assert_eq!(
        bad_edges(&edges),
        Vec::<String>::new(),
        "an edge section 3 forbids: {EDGE_FIX}"
    );
    assert_eq!(dependency_loop(&edges), None, "{CRATE_LOOP_FIX}");
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
            "{layer} sits on a core port: {PORT_FIX}"
        );
    }
}

/// A planted failure leaves its line, and a rule that holds leaves none.
#[dash_test(rule!())]
fn a_failing_rule_leaves_its_name_and_day() {
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
#[dash_test(rule!())]
fn every_rule_opens_with_its_guard() {
    let source = include_str!("workspace_rules.rs");
    assert_eq!(
        tests_out_of_shape(source),
        Vec::<String>::new(),
        "reshape each listed test: #[dash_test(rule!())] alone on its line, then a fn returning nothing, no other \
         attribute"
    );
    assert!(source.lines().filter(|l| l.trim() == TEST).count() > 10);
    // dash gate firings lists each of them, by its own reader (/code-review)
    assert_eq!(
        dash_gate::firings::rules(source).len(),
        source
            .lines()
            .filter(|l| l.trim().starts_with("#["))
            .count(),
        "dash gate firings misses a rule: make dash_gate::firings::rules read this shape"
    );
    // Task rust-test-budget-attr, done-when 2: the attribute holds the guard, so the body asks for no line.
    let good = format!("{TEST}\nfn r() {{\n}}");
    assert_eq!(
        tests_out_of_shape(&good),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    for bad in [
        "#[test]\nfn r() {\n}".to_string(),
        "#[dash_test]\nfn r() {\n}".to_string(),
        format!("{TEST} fn r() {{\n}}"),
        format!("{TEST}\nfn r() -> Result<(), String> {{\n}}"),
        format!("#[should_panic]\n{good}"),
        format!("#[ignore]\n{good}"),
        format!("#[cfg_attr(target_os = \"macos\", ignore)]\n{good}"),
        format!("#[cfg(target_os = \"linux\")]\n{good}"),
    ] {
        assert_eq!(tests_out_of_shape(&bad).len(), 1, "{bad}");
    }
}

/// Done-when 1 of rust-gate-says-fix: every gate test that refuses on a found list says what to do (e6d2f1b61 and ba36501d2
/// each cost a task a round, their gate naming only the offence).
#[dash_test(rule!())]
fn every_list_a_gate_refuses_on_says_the_fix() {
    let gate = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = vec![];
    for file in rust_files(gate) {
        let text = std::fs::read_to_string(&file).expect("read");
        let name = file
            .strip_prefix(gate)
            .unwrap_or(&file)
            .display()
            .to_string();
        found.extend(
            bare_list_asserts(&text)
                .into_iter()
                .map(|b| format!("{name} {b}")),
        );
    }
    assert!(
        found.is_empty(),
        "a gate refusing on a found list says what to do, not only where: give the assert a message of its own naming the \
         fix, then the list (« draw it with a component. Found: {{found:?}} »). Found: {found:#?}"
    );
    // Plain strings: the lexer reads a string as no code, so these plant nothing in this file's own scan.
    for bad in [
        "assert_eq!(f(), Vec::<String>::new());",
        "assert_eq!(\n    f(&x, \"a, b\"),\n    Vec::<&str>::new()\n);",
        "assert_eq!(f(), []);",
        "assert_eq!(f(), &[]);",
        "assert_eq!(f(), BTreeSet::new());",
        "assert_eq!(found.len(), 0, \"{found:?}\");",
        "assert!(found.len() == 0);",
        "assert_eq!(f(), vec![], \"{}\", file.display());",
        "assert!(found.is_empty());",
        "assert!(found.is_empty(), \"{found:?}\");",
        "assert!(found.is_empty() && x, \"{found:?}\");",
        "assert_eq!(f(), Vec::new(), \"orch's Cargo.toml\");",
        "assert!(f(')', '\\'', \"(\").is_empty());",
        "assert!(f(r#\"\"a\" (\"#).is_empty(), r\"C:\\\");",
    ] {
        assert_eq!(
            bare_list_asserts(bad).len(),
            1,
            "a bare plant reads as fine: {bad}"
        );
    }
    for good in [
        "assert_eq!(f(), Vec::<String>::new(), \"move the call into a slot: {x}\");",
        "assert!(found.is_empty(), \"draw it with a component, not a tag. Found: {found:?}\");",
        "assert!(found.is_empty(), r\"draw it with a component, not a tag\");",
        "assert!(found.is_empty(), \"{{draw it with a component}} {found:?}\");",
        "assert!(!found.is_empty());",
        "assert_ne!(found, Vec::<String>::new());",
        "assert_eq!(f(), [\"a\"]);",
        "debug_assert_eq!(f(), []);",
        "assert_eq!(f(','), [\"a\"]);",
        "// assert!(found.is_empty());\n/* assert_eq!(f(), []); */",
        "const X: &str = \"assert!(found.is_empty());\";",
    ] {
        assert_eq!(
            bare_list_asserts(good),
            Vec::<String>::new(),
            "a good plant reads as bare: {good}"
        );
    }
    assert_eq!(
        bare_list_asserts("fn a() {}\n\n  assert!(v.is_empty());"),
        ["assert!(v.is_empty())"]
    );
}

#[dash_test(rule!())]
fn an_edge_that_skips_the_core_is_named() {
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
        assert!(
            bad_edges(&[edge(good.0, good.1)]).is_empty(),
            "an allowed edge is refused, fix bad_edges: {good:?}"
        );
    }
}

/// Nanolathe's rule (its INVARIANTS I4, I6) as a check: core and sync read time, randomness and order through the core's
/// ports, and the op log reads the real clock only in its SystemClock adapter. A map walked in the store has no word to
/// catch here (lookups are fine there): the sweep's rerun of each seed catches it.
#[dash_test(rule!())]
fn what_a_simulation_runs_reads_nothing_a_seed_does_not_decide() {
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
    assert_eq!(
        bad,
        Vec::<String>::new(),
        "core and sync read no clock, randomness, thread, env or file: take a core port (WallClock, Rng, Scheduler, \
         Store) as an argument and let the caller pass the real one"
    );
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

#[dash_test(rule!())]
fn a_dependency_loop_is_named_with_its_path() {
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

#[dash_test(rule!())]
fn no_crate_holds_a_module_cycle() {
    // The message says what to do, not only where: design-stage-notion-toggle's test in families.rs called its parent's
    // page and spent a fix round on the loop it made (lesson, 2026-11-10). Tests count: their uses are read like the rest.
    let cycles = module_cycles(&rust_root());
    assert!(
        cycles.is_empty(),
        "a module loop, each path leading back to its start: {MODULE_LOOP_FIX}. Found: {cycles:?}"
    );
    assert_eq!(
        module_holes(&rust_root()),
        Vec::<String>::new(),
        "code the module walk cannot read: {HOLE_FIX}"
    );
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

/// Task rust-code-health: /deps lists what the gate refuses, each with its fix. Its core, store and mail are the plant
/// boundary 0.27 and archunit 0.0.2 were run on (PR body): boundary found core -> store alone, archunit the four edges and
/// loops found here.
#[dash_test(rule!())]
fn every_graph_finding_is_listed_with_its_fix() {
    let dir = TestDir::new("gate-health");
    let manifest = |uses: &[&str]| {
        let deps: String = uses
            .iter()
            .map(|u| format!("dash-{u} = {{ path = \"../{u}\" }}\n"))
            .collect();
        format!("[package]\nname = \"x\"\n[dependencies]\n{deps}")
    };
    write(&dir.0, "crates/core/Cargo.toml", &manifest(&["store"]));
    write(&dir.0, "crates/core/src/lib.rs", "mod a;\nmod b;\n");
    write(
        &dir.0,
        "crates/core/src/a.rs",
        "pub fn x() { crate::b::y(); }\n",
    );
    write(
        &dir.0,
        "crates/core/src/b.rs",
        "pub fn y() { crate::a::x(); }\n",
    );
    write(
        &dir.0,
        "crates/store/Cargo.toml",
        &manifest(&["core", "mail"]),
    );
    write(&dir.0, "crates/store/src/lib.rs", "pub struct Db;\n");
    write(&dir.0, "crates/mail/Cargo.toml", &manifest(&["core"]));
    write(&dir.0, "crates/mail/src/lib.rs", "pub struct Inbox;\n");
    // a plugin on no core port, and a crate no list names
    write(&dir.0, "crates/drafts/Cargo.toml", &manifest(&[]));
    write(&dir.0, "crates/drafts/src/lib.rs", "pub struct Draft;\n");
    write(&dir.0, "crates/extra/Cargo.toml", &manifest(&[]));
    write(&dir.0, "crates/extra/src/lib.rs", "pub struct X;\n");
    let found = violations(&dir.0);
    assert_eq!(
        found
            .iter()
            .map(|(rule, what, _)| format!("{rule}: {what}"))
            .collect::<Vec<_>>(),
        [
            "unplaced crate: extra",
            "no core port: drafts",
            "layer edge: core -> store",
            "layer edge: store -> mail",
            "crate loop: core -> store -> core",
            "module loop: core: crate::a crate::b crate::a",
        ]
    );
    assert_eq!(
        found.iter().map(|f| f.2).collect::<Vec<_>>(),
        [
            PLACE_FIX,
            PORT_FIX,
            EDGE_FIX,
            EDGE_FIX,
            CRATE_LOOP_FIX,
            MODULE_LOOP_FIX
        ]
    );
}

#[dash_test(rule!())]
fn a_planted_module_cycle_is_named_with_its_crate() {
    let dir = TestDir::new("gate-cycle");
    write(&dir.0, "crates/fine/Cargo.toml", "");
    write(&dir.0, "crates/fine/src/lib.rs", "pub mod a;\n");
    write(
        &dir.0,
        "crates/fine/src/a.rs",
        "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    use super::*;\n}\n",
    );
    assert_eq!(
        module_cycles(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
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

#[dash_test(rule!())]
fn only_code_makes_a_module_edge() {
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
#[dash_test(rule!())]
fn every_way_of_naming_a_module_reads_right() {
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
#[dash_test(rule!())]
fn what_the_walk_cannot_read_is_named() {
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
}

#[dash_test(rule!())]
fn a_manifest_reads_past_comments_and_indents() {
    let dir = TestDir::new("gate-manifest");
    let aa = "[dependencies] # ours\n  dash-bb = { path = \"../bb\" }\n[dev-dependencies]\ndash-meter.workspace = true\n";
    write(&dir.0, "crates/aa/Cargo.toml", aa);
    write(&dir.0, "crates/bb/Cargo.toml", "");
    write(&dir.0, "crates/meter/Cargo.toml", "");
    assert_eq!(crate_graph(&dir.0).1, [edge("aa", "bb")]);
}

#[dash_test(rule!())]
fn the_core_names_no_tool_or_provider() {
    let files = rust_files(&rust_root().join("crates/core/src"));
    assert!(!files.is_empty(), "the walk found the core's source");
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(
            tools_named(&text),
            Vec::<&str>::new(),
            "the core names no tool or provider: move this behind a core port into the crate that owns it, \
             or reword the comment, in {}",
            file.display()
        );
    }
    assert_eq!(tools_named("// opens a herdr pane"), ["herdr"]);
    assert_eq!(tools_named("Claude Code's transcript"), ["claude"]);
    assert_eq!(tools_named("Command::new(\"make\")"), ["make"]);
    assert_eq!(tools_named("DD_API_KEY"), ["dd"]);
    assert_eq!(tools_named("the Datadog agent"), ["datadog"]);
    assert!(
        tools_named("a Makefile, gather, shepherd").is_empty(),
        "a word holding a tool's name reads as the tool: tools_named matches whole words only"
    );
}

/// rfc-rust-plugins-ui section 3: mail works with the job hunt removed, so nothing in its source, tests or manifest names it.
#[dash_test(rule!())]
fn the_mail_names_no_job_word() {
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
            "mail runs with the job hunt removed: move this to the jobs crate or reword it, in {}",
            file.display()
        );
    }
    assert_eq!(
        words_named("the Lead's CV, an ad", &JOB_WORDS),
        ["lead", "ad", "cv"]
    );
    assert!(
        words_named("a header, already loaded, adds", &JOB_WORDS).is_empty(),
        "a word holding a job word reads as it: words_named matches whole words only"
    );
}

#[dash_test(rule!())]
fn the_orch_names_no_agent_kind() {
    let orch = rust_root().join("crates/orch");
    let files = rust_files(&orch);
    assert!(files.len() > 1, "the walk found orch's source and tests");
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(
            agents_named(&text),
            Vec::<&str>::new(),
            "orch names no agent kind: move this to the agent crate behind orch's port, or reword it, in {}",
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
        "orch's Cargo.toml names an agent: the dependency or feature belongs to the agent crate"
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
    assert!(
        agents_named("// a Claude's id\n/// codex too\n/* Gemini */ fn run() {}").is_empty(),
        "a comment naming an agent reads as code: agents_named leaves comments out"
    );
}

#[dash_test(rule!())]
fn the_dash_crate_holds_no_html() {
    // The message says what to do, not only where: rust-one-doc-component spent turns finding it (lesson, 2026-11-10).
    let found = html_in_strings(&rust_root());
    assert!(
        found.is_empty(),
        "HTML is dash_surface's to draw: build the page from its components, and in a dash test assert on the text form \
         (Accept: text/plain) or the surface tree, never a tag literal. Found: {found:?}"
    );
}

/// The dash crate writes no `rsx!` outside its tests: a page part is a design system function given data (task
/// design-doc-tree-menu: live.rs composed /documents' island and the compass in dash, and the layer judge held it).
#[dash_test(rule!())]
fn the_dash_crate_draws_no_rsx() {
    let found = rsx_in_dash(&rust_root());
    assert!(
        found.is_empty(),
        "rsx! is dash_surface's: move the markup into a surface function taking the data, and call it from dash. Found: {found:?}"
    );
}

/// The rule above on a planted tree: an `rsx!` in dash's code is named; one in its test module, a comment or another crate is not.
#[dash_test(rule!())]
fn a_planted_rsx_in_dash_fails_the_gate() {
    let dir = TestDir::new("gate-rsx-dash");
    write(
        &dir.0,
        "crates/dash/src/live.rs",
        "// rsx! in a comment\nfn f() {}\n#[cfg(test)]\nmod tests { fn t() { rsx! { div {} } } }\n",
    );
    write(
        &dir.0,
        "crates/surface/src/x.rs",
        "fn f() -> Element { rsx! { div {} } }\n",
    );
    assert_eq!(
        rsx_in_dash(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule"
    );
    write(
        &dir.0,
        "crates/dash/src/live.rs",
        "fn f() -> Element { rsx! { div {} } }\n",
    );
    assert_eq!(rsx_in_dash(&dir.0), ["crates/dash/src/live.rs: 1 rsx!"]);
}

/// Task rust-task-page-edits, done-when 2 (@nha, 2026-11-11: « I can edit the description but it is lost after a refresh »):
/// the design system's editable field saves or is read only. Text is editable only in the editor's field, inside the form
/// that posts it, and on the Notion tab's specimen: a page that makes its own text editable is refused.
#[dash_test(rule!())]
fn text_is_editable_only_in_the_editor_that_saves_it() {
    let found = editable_outside(&rust_root());
    assert!(
        found.is_empty(),
        "a page's words are edited in the design system's Composer, whose Save posts them (`words` opens it on the saved \
         text; a task's description posts to /tasks/<id>/set); anything else is DocView, read only. Allowed: {EDITABLE:?}. \
         Found: {found:?}"
    );
}

/// The rule above on a planted tree: an editable element a page draws is named, a comment, a string or a test is not.
#[dash_test(rule!())]
fn a_planted_editable_field_fails_the_gate() {
    let dir = TestDir::new("gate-editable");
    write(
        &dir.0,
        "crates/surface/src/documents.rs",
        "// contenteditable\nfn f() { rsx! { div { contenteditable: \"plaintext-only\" } } }\nconst JS: &str = \"d.contentEditable=p\";\n",
    );
    write(
        &dir.0,
        "crates/dash/src/tasks.rs",
        "fn f() {}\n#[cfg(test)]\nmod tests { fn t() { rsx! { div { contenteditable: \"true\" } } } }\n",
    );
    assert_eq!(
        editable_outside(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    write(
        &dir.0,
        "crates/surface/src/design/board.rs",
        "fn f() { rsx! { div { contenteditable: \"plaintext-only\" } } }\n",
    );
    write(
        &dir.0,
        "crates/surface/src/documents.rs",
        "fn f() { rsx! { div { contenteditable: \"a\" } div { contenteditable: \"b\" } } }\n",
    );
    // the quoted rsx name and HTML in a string (/code-review)
    write(
        &dir.0,
        "crates/surface/src/forms.rs",
        "fn f() { rsx! { div { \"contenteditable\": \"true\" } } }\nconst H: &str = \"@div contenteditable=x>\";\n",
    );
    let mut found = editable_outside(&dir.0);
    found.sort();
    assert_eq!(
        found,
        [
            "crates/surface/src/design/board.rs: 1 contenteditable, 0 allowed",
            "crates/surface/src/documents.rs: 2 contenteditable, 1 allowed",
            "crates/surface/src/forms.rs: 2 contenteditable, 0 allowed",
        ]
    );
}

/// Task rust-doc-rich-blocks, done-when 3: a field of words is the document component's reduced mode (DocField), which
/// Textarea and Input draw through; a page or a component that draws its own textarea or text input is refused.
#[dash_test(rule!())]
fn every_text_field_is_the_document_component() {
    let found = fields_outside(&rust_root());
    assert!(
        found.is_empty(),
        "a field of words is drawn with the design system's Textarea or Input (or DocField, to hear its input), which hold \
         its words as a document of the one model; a raw `textarea {{` or text `input {{` is not. Allowed: {FIELDS:?}. \
         Found: {found:?}"
    );
    // the gate's list of inputs of no words is surface's Input's, which decides what a field holds (/code-review: two copies)
    let input =
        std::fs::read_to_string(rust_root().join("crates/surface/src/components/each/input.rs"))
            .expect("input.rs");
    let at = input.find("const NOT_WORDS").expect("Input's NOT_WORDS");
    let listed: Vec<String> = (lexed(&input[at..at + input[at..].find("];").expect("its end")])
        .into_iter())
    .filter_map(|l| match l {
        Lexeme::Str(s) => Some(s),
        Lexeme::Code(_) => None,
    })
    .collect();
    assert_eq!(listed, NOT_WORDS, "the gate's NOT_WORDS and Input's");
}

/// The rule above on a planted tree: a raw textarea or text input is named, the components, a typed non-text input, a
/// comment, a string or a test are not.
#[dash_test(rule!())]
fn a_planted_text_field_fails_the_gate() {
    let dir = TestDir::new("gate-fields");
    write(
        &dir.0,
        "crates/surface/src/components/each/doc_field.rs",
        "fn f() { rsx! { textarea { class: \"textarea\" } } }\nfn g() { rsx! { input { class: \"input\", ..attributes } } }\n",
    );
    write(
        &dir.0,
        "crates/surface/src/components/each/switch.rs",
        "// textarea { }\nfn f() { rsx! { input { r#type: \"checkbox\", onclick: move |_| { x() } } Textarea { rows: 3 } \
         Input { r#type: \"text\" } } }\nconst H: &str = \"textarea {\";\n",
    );
    write(
        &dir.0,
        "crates/dash/src/tasks.rs",
        "fn f() {}\n#[cfg(test)]\nmod tests { fn t() { rsx! { textarea {} } } }\n",
    );
    assert_eq!(
        fields_outside(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    // a page's textarea, one after a text node, one in rsx!( ), an input of no type, one typed text, a quoted type, and a
    // type set inside a nested block only
    write(
        &dir.0,
        "crates/surface/src/design/board.rs",
        "fn f() { rsx! { div { textarea { rows: 3 } } } }\nfn g() { rsx! { label { \"Rate\" input { name: \"rate\" } } } }\n\
         fn h() { rsx!(textarea {}) }\n",
    );
    write(
        &dir.0,
        "crates/surface/src/components/each/switch.rs",
        "fn f() { rsx! { input { class: \"input\" }, input { r#type: \"search\" } input { \"type\": \"email\" } \
         input { onclick: move |_| { let r#type: &str = \"hidden\"; } } } }\n",
    );
    write(
        &dir.0,
        "crates/surface/src/components/each/doc_field.rs",
        "fn f() { rsx! { textarea {} textarea {} input {} } }\n",
    );
    let mut found = fields_outside(&dir.0);
    found.sort();
    assert_eq!(
        found,
        [
            "crates/surface/src/components/each/doc_field.rs: 3 textarea or text input, 2 allowed",
            "crates/surface/src/components/each/switch.rs: 4 textarea or text input, 0 allowed",
            "crates/surface/src/design/board.rs: 3 textarea or text input, 0 allowed",
        ]
    );
}

/// The rule above on a planted tree: each place a page could come back from, and what must not trip it.
#[dash_test(rule!())]
fn a_planted_tag_fails_the_gate() {
    let dir = TestDir::new("gate-html");
    let clean = r##"// a <td> in a comment
fn main() { println!("usage: dash --data <dir> --port <n> <https://x> <a-task> 1 < 2"); }
"##;
    write(&dir.0, "crates/dash/src/main.rs", clean);
    write(&dir.0, "crates/gate/src/lib.rs", clean);
    assert_eq!(
        html_in_strings(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );

    let planted = [
        ("crates/dash/src/main.rs", r#""<td>{x}</td>""#, "</td"),
        ("crates/dash/src/tasks.rs", r##"r#"<a href="/">"#"##, "<a"),
        ("crates/dash/build.rs", r#""<!doctype html>""#, "<!"),
        ("crates/dash/tests/t.rs", r#""\n<TABLE>""#, "<TABLE"),
        ("crates/gate/src/graph.rs", r#""<svg width>""#, "<svg"),
        ("crates/dash/src/x.rs", r#""<td{}>""#, "<td"),
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
    // rust-mail-parity: a mail's HTML body built outside the design system, which the dash-only scan passed; its tags built
    // at run time, so this file holds none outside surface
    let lt = '<';
    let mail = "crates/mail/src/html.rs";
    write(
        &dir.0,
        mail,
        &format!("const X: &str = \"{lt}ul>{{}}{lt}/ul>\";\n"),
    );
    let found = html_in_strings(&dir.0);
    assert!(
        found
            .iter()
            .any(|f| f.contains(mail) && f.ends_with(&format!("{lt}/ul"))),
        "{mail}: {found:?}"
    );
    write(&dir.0, mail, clean);
    // the design system draws HTML; a held file passes until its move, and a crate's tests/ folder is test code
    for path in [
        "crates/surface/src/x.rs",
        "crates/ui/src/lib.rs",
        "crates/mail/tests/t.rs",
    ] {
        write(&dir.0, path, &format!("const X: &str = \"{lt}div>\";\n"));
    }
    assert_eq!(
        html_in_strings(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    assert_eq!(html_tags("x</td"), ["</td"]);
    assert_eq!(html_tags("<p> <p class> <!-- x"), ["<p", "<!"]);
    // the dialog's answer: every bare name but BARE_REFUSED's passes as a placeholder, a void <br> and <hr> too
    assert_eq!(
        html_tags("<br> <hr> <li> <div>"),
        Vec::<String>::new(),
        "a bare tag outside BARE_REFUSED reads as HTML: html_tags lets every other bare name pass as a placeholder"
    );
    assert_eq!(
        html_tags("Foo<S> Wrapper<P> x<!y"),
        Vec::<String>::new(),
        "a generic type or a comparison reads as a tag: narrow html_tags to tag names"
    );

    let latin = dir.0.join("crates/dash/src/latin.rs");
    std::fs::write(&latin, [0xe9u8]).expect("write");
    let found = html_in_strings(&dir.0);
    assert!(found.iter().any(|f| f.contains("latin.rs: ")), "{found:?}");
}

/// What html_in_strings finds in a tree whose dash crate holds this one string (task rust-gate-placeholders).
fn gate_on(name: &str, literal: &str) -> Vec<String> {
    let dir = TestDir::new(name);
    write(
        &dir.0,
        "crates/dash/src/main.rs",
        &format!("const X: &str = {literal:?};\n"),
    );
    html_in_strings(&dir.0)
}

/// Done-when 1 of rust-gate-placeholders: a tag with attributes is HTML.
#[dash_test(rule!())]
fn a_tag_with_attributes_fails_the_gate() {
    let found = gate_on("gate-attrs", "<svg width=\"10\">");
    assert!(found.iter().any(|f| f.ends_with(": <svg")), "{found:?}");
}

/// Done-when 1 of rust-gate-placeholders: a closing tag is HTML, the bare opening beside it a placeholder.
#[dash_test(rule!())]
fn a_closing_tag_fails_the_gate() {
    let found = gate_on("gate-closing", "<p>text</p>");
    assert!(found.iter().any(|f| f.ends_with(": </p")), "{found:?}");
}

/// Done-when 1 of rust-gate-placeholders: a self-closing tag is HTML, with or without a space.
#[dash_test(rule!())]
fn a_self_closing_tag_fails_the_gate() {
    let found = gate_on("gate-self-closing", "line<br/>next<hr />");
    assert!(found.iter().any(|f| f.ends_with(": <br")), "{found:?}");
    assert!(found.iter().any(|f| f.ends_with(": <hr")), "{found:?}");
}

/// Done-when 1 of rust-gate-placeholders: a bare `<path>` in usage text is a placeholder (three tasks lost a commit to it,
/// 2026-11-11).
#[dash_test(rule!())]
fn a_bare_path_placeholder_passes_the_gate() {
    let found = gate_on("gate-path", "usage: dash store text <path> [--json]");
    assert_eq!(
        found,
        Vec::<String>::new(),
        "a bare <path> read as an SVG tag"
    );
}

/// Done-when 1 of rust-gate-placeholders: a bare `<id>` in usage text is a placeholder.
#[dash_test(rule!())]
fn a_bare_id_placeholder_passes_the_gate() {
    let found = gate_on("gate-id", "usage: dash task show <id>");
    assert_eq!(found, Vec::<String>::new(), "a bare <id> read as a tag");
}

/// Done-when 2 of rust-gate-placeholders: a real `<path d=…>` and a `<button>`, bare too (the orchestrator's answer: « Bare
/// <button> refused »), are still refused; BARE_REFUSED holds every tag a component registers.
#[dash_test(rule!())]
fn a_real_path_or_button_still_fails_the_gate() {
    let found = gate_on("gate-real-path", "<path d=\"M0 0L1 1\"/>");
    assert!(found.iter().any(|f| f.ends_with(": <path")), "{found:?}");
    for button in ["<button>", "<BUTTON>", "<button class=\"x\">Go</button>"] {
        let found = gate_on("gate-button", button);
        assert!(
            found
                .iter()
                .any(|f| f.ends_with(": <button") || f.ends_with(": <BUTTON")),
            "{button}: {found:?}"
        );
    }
    for r in registrations(&rust_root()) {
        for tag in r.tags {
            assert!(
                BARE_REFUSED.contains(&tag.as_str()),
                "{}: its tag {tag} is missing from html::BARE_REFUSED, so a bare one would pass as a placeholder",
                r.file
            );
        }
    }
}

/// Done-when 2 of rust-design-components: the tasks board and the tools page draw with crates/surface/src/components.rs, and
/// no page draws by hand what a component covers (@nha, 2026-11-09: « We want all components to be dual-use »).
#[dash_test(rule!())]
fn no_page_draws_what_a_component_covers() {
    assert_eq!(
        page_local_markup(&rust_root()),
        Vec::<String>::new(),
        "a page draws what a component covers: use the component from crates/surface/src/components.rs, \
         adding it there first (HTML and text forms) when it is missing"
    );
}

/// The rule above on a planted tree: a page's own button or card fails it, the component's file, the showcase and plain
/// Rust that merely names a field `input` pass.
#[dash_test(rule!())]
fn a_planted_button_or_card_fails_the_gate() {
    let dir = TestDir::new("gate-components");
    let owned = registration("button", &["card"], &["button", "input", "textarea"]);
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
    let clean = "fn f(input: u8) { match input { _ => {} } }\nfn g() { rsx! { div { class: \"cards\", Button { \"Go\" } } } }\nfn h() { rsx! { p { class: \"head\", Badge { look: \"card\" } } } }\n";
    write(&dir.0, "crates/surface/src/page.rs", clean);
    assert_eq!(
        page_local_markup(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );

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
            "fn g() { rsx! { div { class: if up { \"x\" } else { \"card x\" }, p { class: \"y\", \"card\" } } } }",
            "class: \"card\"",
        ),
        (
            "const X: &str = r#\"<form><INPUT name=q></form>\"#;",
            "1 button, input or textarea in strings",
        ),
        (
            "const X: &str = \"<textarea></textarea>\";",
            "1 button, input or textarea in strings",
        ),
        ("fn g() { rsx! { div { textarea {} } } }", "textarea {"),
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
    assert_eq!(
        page_local_markup(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    write(&dir.0, COMPONENTS, "pub const NOTHING: u8 = 0;\n");
    let found = page_local_markup(&dir.0);
    assert!(
        found.iter().any(|f| f.contains("no component registers")),
        "{found:?}"
    );
}

/// A component's registration as rustfmt writes it (surface's crate::component).
fn registration(slug: &str, classes: &[&str], tags: &[&str]) -> String {
    let list = |xs: &[&str]| {
        xs.iter()
            .map(|x| format!("\"{x}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "inventory::submit! {{\n    Component {{\n        slug: \"{slug}\",\n        classes: &[{}],\n        tags: &[{}],\n        example: Some(|_| rsx! {{ div {{ class: \"x\" }} }}),\n        ..Component::BARE\n    }}\n}}\n",
        list(classes),
        list(tags)
    )
}

/// Done-when 1 of rust-components-register: each component registers its classes and tags from its own module, as pages
/// do, and the design system holds no central list of them: Kbd, in a module of its own, is read from there.
#[dash_test(rule!())]
fn each_component_registers_from_its_own_module() {
    let found = registrations(&rust_root());
    let kbd = found
        .iter()
        .find(|r| r.slug == "kbd")
        .expect("kbd registered");
    assert_eq!(kbd.file, "crates/surface/src/components/each/kbd.rs");
    assert_eq!(
        (kbd.classes.clone(), kbd.tags.clone()),
        (
            vec!["kbd".to_string(), "kbd-group".into()],
            vec!["kbd".to_string()]
        )
    );
    // every component in a module of its own: components.rs, the folder's root, holds none (the PM check of PR 1416)
    let elsewhere: Vec<&String> = (found.iter())
        .filter(|r| !r.file.starts_with("crates/surface/src/components/each/"))
        .map(|r| &r.file)
        .collect();
    assert_eq!(
        elsewhere,
        Vec::<&String>::new(),
        "move each component into a module of its own under crates/surface/src/components/each/, its registration beside it"
    );
    assert!(found.len() > 40, "{} registrations", found.len());
    let source = std::fs::read_to_string(rust_root().join(COMPONENTS)).expect("read");
    for gone in [
        "OWNED_TAGS",
        "OWNED_CLASSES",
        "inventory::submit!",
        "#[component]",
    ] {
        assert!(!source.contains(gone), "{gone} is back in {COMPONENTS}");
    }
    let catalog = std::fs::read_to_string(rust_root().join("crates/surface/src/design/catalog.rs"))
        .expect("read");
    assert!(!catalog.contains("CATALOG"), "the central catalog is back");
    let mut classes: Vec<&String> = found.iter().flat_map(|r| &r.classes).collect();
    let all = classes.len();
    classes.sort();
    classes.dedup();
    assert_eq!(classes.len(), all, "a class two components register");
}

/// Done-when 2 of rust-tui-tracer: every component carries its terminal form, the third of rust/CLAUDE.md rule 4's forms
/// (@nha, 2026-11-13: « better get this in early to get it right »).
#[dash_test(rule!())]
fn every_component_carries_its_terminal_form() {
    assert_eq!(
        components_without_a_terminal_form(&rust_root()),
        Vec::<String>::new(),
        "give each its terminal form in its registration: `terminal: Some(crate::component::Terminal::Words)` where its \
         text form is the whole of it, else `Terminal::Drawn` with its own ratatui lines (crates/surface/src/component.rs)"
    );
}

/// The rule above on a planted tree: a registration naming no terminal form is named with its file, one naming one passes.
#[dash_test(rule!())]
fn a_planted_component_with_no_terminal_form_fails_the_gate() {
    let dir = TestDir::new("gate-terminal-form");
    let named = registration("kbd", &["kbd"], &["kbd"]).replacen(
        "..Component::BARE",
        "terminal: Some(Terminal::Words),\n        ..Component::BARE",
        1,
    );
    assert!(named.contains("terminal: Some("), "{named}");
    write(&dir.0, "crates/surface/src/components/each/kbd.rs", &named);
    let bare = registration("badge", &["badge"], &[]);
    write(&dir.0, "crates/surface/src/components/each/badge.rs", &bare);
    assert_eq!(
        components_without_a_terminal_form(&dir.0),
        ["crates/surface/src/components/each/badge.rs: badge"]
    );
}

/// Done-when 2 of rust-components-register: no list outside a component's module names a component (@nha, 2026-11-10:
/// « this then finally unblocks adding parallelism and unblocks velocity development »).
#[dash_test(rule!())]
fn no_shared_list_names_the_components() {
    assert_eq!(
        shared_component_lists(&rust_root()),
        Vec::<String>::new(),
        "a list naming two components is one two tasks both edit: let each component register itself in its own module \
         (crates/surface/src/component.rs), and drop the list"
    );
}

/// The rule above on a planted tree: a list naming two components fails, a `const`'s, a `let`'s or one inline, by slug or
/// `.class`, in a component's own file too, and so does a component registered outside the components' files; a list naming
/// one component's classes, a single string, a `const fn` taking a slice, the same list in a comment, a test's list and
/// another crate's list pass (/code-review of PR 1416 and of 114248eb4).
#[dash_test(rule!())]
fn a_shared_list_of_components_fails_the_gate() {
    let dir = TestDir::new("gate-component-lists");
    let both = [
        registration("card", &["card", "li-card"], &[]),
        registration("badge", &["badge"], &[]),
    ]
    .concat();
    write(
        &dir.0,
        COMPONENTS,
        &format!("{both}const LOOKS: [&str; 2] = [\"card\", \"li-card\"];\n"),
    );
    let own = registration("kbd", &["kbd", "kbd-group"], &["kbd"]);
    write(
        &dir.0,
        "crates/surface/src/components/each/kbd.rs",
        &format!("{own}const KEYS: &[&str] = &[\"kbd\", \"kbd-group\"];\n"),
    );
    let clean = "// const LOOKS: [&str; 2] = [\"card\", \"badge\"];\nconst ONE: &str = \"card\";\nconst OTHER: [&str; 2] = [\"cards\", \"x\"];\nconst KEY: [&str; 1] = [\"kbd\"];\npub const fn f(x: &[u8]) -> u8 { x[0] }\nfn g() -> Vec<&'static str> { vec![\"card\", \"x\"] }\n/// `inventory::submit! { Component {` in words\npub struct Component {}\n#[cfg(test)]\nmod tests {\n    const T: [&str; 2] = [\"card\", \"badge\"];\n}\n";
    write(&dir.0, "crates/surface/src/page.rs", clean);
    let two = "pub const FAMILIES: [&str; 2] = [\"card\", \"badge\"];\n";
    write(&dir.0, "crates/dash/src/page.rs", two);
    assert_eq!(
        shared_component_lists(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    let planted = [
        (
            "pub const OWNED_CLASSES: [&str; 2] = [\n    \"card\",\n    \"badge\",\n];\n",
            "OWNED_CLASSES names card, badge",
        ),
        (
            "pub(crate) static SHOWN: &[(&str, u8)] = &[(\"kbd\", 1), (\"li-card\", 2)];\n",
            "SHOWN names kbd, card",
        ),
        (
            "pub(super) const LOOKS: [&str; 2] = [\"card\", \"badge\"];\n",
            "LOOKS names card, badge",
        ),
        (
            "fn f(l: &str) -> bool { let picked = vec![\".kbd\", \".card\"]; picked.contains(&l) }\n",
            "picked names kbd, card",
        ),
        (
            "fn f(l: &str) -> bool { [\".badge\", \".li-card\"].iter().any(|c| l.starts_with(c)) }\n",
            "a list in f names badge, card",
        ),
        (
            "inventory::submit! {\n    crate::component::Component {\n        slug: \"bubble\",\n        ..Component::BARE\n    }\n}\n",
            "registers a component outside crates/surface/src/components.rs and crates/surface/src/components/",
        ),
    ];
    for (code, what) in planted {
        write(&dir.0, "crates/surface/src/page.rs", code);
        assert_eq!(
            shared_component_lists(&dir.0),
            [format!("crates/surface/src/page.rs: {what}")],
            "{code}"
        );
    }
    write(&dir.0, "crates/surface/src/page.rs", "");
    write(
        &dir.0,
        COMPONENTS,
        &format!("{both}const LOOKS: [&str; 2] = [\"li-card\", \"kbd\"];\n"),
    );
    assert_eq!(
        shared_component_lists(&dir.0),
        [format!("{COMPONENTS}: LOOKS names card, kbd")]
    );
    // two components of one module, listed in it: still a list two tasks would both edit
    write(
        &dir.0,
        COMPONENTS,
        &format!("{both}const LOOKS: [&str; 2] = [\"li-card\", \"badge\"];\n"),
    );
    assert_eq!(
        shared_component_lists(&dir.0),
        [format!("{COMPONENTS}: LOOKS names card, badge")]
    );
}

/// Done-when 3 of rust-components-register: two branches that each add a component, its module and its two lines in
/// components/each.rs at the same place, merge with no conflict, under the repository's own .gitattributes line.
#[dash_test(rule!())]
fn two_branches_each_adding_a_component_merge_clean() {
    let dir = TestDir::new("gate-component-merge");
    let each = "crates/surface/src/components/each.rs";
    let union = format!("rust/{each} merge=union");
    let read = |p: &Path| std::fs::read_to_string(p).expect("read");
    // the mirror holds rust/ alone and refuses any .gitattributes in its tree (Dashboard.CiMirror's @scan_config), so the
    // repository's line is checked where the monorepo is, told by its dashboard; the merge below runs on both (PR 1416's
    // mirror run read a missing ../.gitattributes)
    if rust_root().join("../private/dashboard").is_dir() {
        let attributes = read(&rust_root().join("../.gitattributes"));
        assert!(attributes.lines().any(|l| l == union), "{attributes}");
    }
    write(&dir.0, ".gitattributes", &format!("{union}\n"));
    write(
        &dir.0,
        &format!("rust/{each}"),
        &read(&rust_root().join(each)),
    );
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(&dir.0)
            .output()
            .expect("git");
        (
            out.status.success(),
            String::from_utf8_lossy(&[out.stdout, out.stderr].concat()).into_owned(),
        )
    };
    let ok = |args: &[&str]| {
        let (done, said) = git(args);
        assert!(done, "git {args:?}: {said}");
    };
    ok(&["init", "-q", "-b", "main"]);
    ok(&["add", "-A"]);
    ok(&["commit", "-qm", "base"]);
    let base = read(&dir.0.join("rust").join(each));
    for name in ["bubble", "timeline"] {
        ok(&["checkout", "-qb", name, "main"]);
        write(
            &dir.0,
            &format!("rust/{each}"),
            &format!("{base}pub mod {name};\npub use {name}::*;\n"),
        );
        write(
            &dir.0,
            &format!("rust/crates/surface/src/components/each/{name}.rs"),
            &registration(name, &[name], &[]),
        );
        ok(&["add", "-A"]);
        ok(&["commit", "-qm", name]);
    }
    ok(&["checkout", "-q", "main"]);
    ok(&["merge", "-q", "--no-edit", "bubble"]);
    let (merged, said) = git(&["merge", "--no-edit", "timeline"]);
    assert!(merged, "the second component conflicted: {said}");
    let joined = read(&dir.0.join("rust").join(each));
    for name in ["bubble", "timeline"] {
        assert!(
            joined.contains(&format!("pub mod {name};\npub use {name}::*;\n")),
            "{joined}"
        );
    }
    assert!(!joined.contains("<<<<<<<"), "{joined}");
}

/// Done-when 3 of rust-board-linear-order: the tasks board draws only the design system's components, and a div or an HTML
/// string planted in its file fails the gate (@nha, 2026-11-09: « let's not have drift. Shared components. »).
#[dash_test(rule!())]
fn the_board_draws_only_components() {
    assert_eq!(
        board_markup(&rust_root()),
        Vec::<String>::new(),
        "the board draws its own element: add the part to COMPONENTS (crates/surface) first, then compose it"
    );
    let dir = TestDir::new("gate-board");
    let found = board_markup(&dir.0);
    assert!(
        found[0].starts_with(BOARD),
        "a missing file fails: {found:?}"
    );
    let clean = "fn b(id: &str, title: &str) -> Element { rsx! { Board { id, Column { title, if x { Card {} } else { Card {} } } } } }\n";
    write(&dir.0, BOARD, clean);
    assert_eq!(
        board_markup(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    let planted = [
        (
            "fn b() { rsx! { Board { div { class: \"x\" } } } }",
            "div {",
        ),
        ("fn b() { rsx! { Card {}, span { \"x\" } } }", "span {"),
        ("fn b() { rsx!(div {}) }", "div {"),
        ("fn b() { rsx![p {}] }", "p {"),
        ("fn b() { rsx! { Card { \"x\" b { \"y\" } } } }", "b {"),
        (
            "const X: &str = \"<section class=x>\";",
            "1 HTML tags in strings",
        ),
        // the bare-placeholder pass is html_in_strings' alone (/code-review of rust-gate-placeholders)
        ("const X: &str = \"<div>\";", "1 HTML tags in strings"),
    ];
    for (code, what) in planted {
        write(&dir.0, BOARD, code);
        assert_eq!(board_markup(&dir.0), [format!("{BOARD}: {what}")], "{code}");
    }
}

/// No Steel in any build, whatever its features; a first use waits for his word (@nha, 2026-11-05: « in fact Steel is a FUTURE
/// nice to have. There should be zero of this now. gated on my approval for the first. »).
#[dash_test(rule!())]
fn no_build_links_steel() {
    // A lockfile holds every optional dependency, so no feature hides one; rust/'s is the one workspace on the mirror.
    let text = std::fs::read_to_string(rust_root().join("Cargo.lock")).expect("Cargo.lock");
    assert!(!text.contains("name = \"steel"));
    assert!(
        text.contains("name = \"serde_json\""),
        "the packages were read"
    );
}

/// `mix lint`'s browser slot rule, held on rust/ by the gate (task brief-tests-prepush-skip): a commit touching only rust/
/// skips the app's lint.
#[dash_test(rule!())]
fn every_rust_file_that_starts_a_browser_takes_a_slot() {
    let files = tracked(&rust_root());
    assert!(
        files.len() > 50,
        "git listed the tracked files: {}",
        files.len()
    );
    assert_eq!(
        browsers_without_slot(&rust_root(), &files),
        Vec::<String>::new(),
        "start the browser through bin/browser-slot, so it takes one of the machine's slots"
    );
}

/// The rule above on planted files: each launch without a slot, and what must not trip it.
#[dash_test(rule!())]
fn a_planted_browser_launch_without_a_slot_fails_the_gate() {
    let dir = TestDir::new("gate-browser");
    let headless = BROWSER_LAUNCH[4];
    let mut planted: Vec<(String, String)> = BROWSER_LAUNCH
        .iter()
        .enumerate()
        .map(|(n, p)| (format!("bad{n}.rs"), format!("let a = \"{p} x\";")))
        .collect();
    planted.extend([
        (
            "slot.rs".into(),
            format!("run(\"{headless}\"); // bin/browser-slot first"),
        ),
        ("doc.md".into(), format!("run `{headless}`")),
        ("prefix.rs".into(), format!("let a = \"{headless}_shell\";")),
        ("end.rs".into(), format!("let a = \"x\";\n{headless}")),
    ]);
    for (path, text) in &planted {
        std::fs::write(dir.0.join(path), text).expect("planted");
    }
    let files: Vec<(String, String)> = planted
        .iter()
        .map(|(p, _)| ("100644".into(), p.clone()))
        .collect();
    let mut want: Vec<String> = (0..BROWSER_LAUNCH.len())
        .map(|n| format!("bad{n}.rs"))
        .collect();
    want.push("end.rs".into());
    assert_eq!(browsers_without_slot(&dir.0, &files), want);
}

/// What CrmGoneTest (private/dashboard/test/dashboard/crm_gone_test.exs) refuses in rust/, here so a Rust PR meets it before
/// merge: the mirror runs no Elixir, and #1180 merged a fixture naming the old folder that turned main red.
#[dash_test(rule!())]
fn no_rust_file_names_the_old_crm() {
    let files = tracked(&rust_root());
    assert!(
        files.len() > 50,
        "git listed the tracked files: {}",
        files.len()
    );
    assert_eq!(
        old_crm_named(&rust_root(), &files),
        Vec::<String>::new(),
        "the old CRM is gone: read and write the store through dash, and rename a fixture path naming its folder"
    );
}

/// Task rust-older-markers-one-reader: the older store's markers have one reader, `dash_store::older`, and every caller goes
/// through it (@nha, 2026-10-29: « You fixed the mistake but didn't find the cause and what could actually fix that class of
/// errors right ? »).
#[dash_test(rule!())]
fn marker_code_points_live_in_dash_store_older_alone() {
    let root = rust_root();
    let files = rust_files(&root.join("crates"));
    assert!(
        files.len() > 50,
        "the crates' sources listed: {}",
        files.len()
    );
    let found: Vec<String> = (files.iter())
        .filter(|f| !f.ends_with(MARKERS_HOME))
        .flat_map(|f| {
            let source = std::fs::read_to_string(f).expect("read");
            let lines = marker_literals(&source);
            lines
                .into_iter()
                .map(move |l| format!("{}:{l}", f.display()))
        })
        .collect();
    assert_eq!(
        found,
        Vec::<String>::new(),
        "read the older store's markers through dash_store::older (pieces, holds_marker, COL, STAGE, EVENTS), and write \
         a test's rows with older::marked"
    );
    // OPEN and CLOSE are private to the module, so a reader elsewhere has only `pieces` to call; each named reader calls
    // it on a line of code, never only in a comment (older.rs's own readers are held by its `pieces_reads_…` test)
    let readers = [
        "crates/store/src/older_drafts.rs",
        "crates/dash/src/chances.rs",
        "crates/dash/src/search.rs",
    ];
    for reader in readers {
        let source = std::fs::read_to_string(root.join(reader)).expect("read");
        let code = |l: &&str| !l.trim_start().starts_with("//");
        // a call: « pieces( » after no letter, digit or `_` (`split_pieces(` is another function)
        let calls = |l: &str| {
            (l.match_indices("pieces("))
                .any(|(at, _)| !l[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        };
        assert!(
            source.lines().filter(code).any(calls),
            "{reader} reads its markers with older::pieces"
        );
    }
}

/// The rule above on planted lines: each spelling of either code point is found, a line through dash_store::older is not.
#[dash_test(rule!())]
fn a_marker_literal_outside_dash_store_older_fails_the_gate() {
    // each code point from its neighbour, so no token of this test is one of them
    let (e000, e001) = (0xDFFF_u32 + 1, 0xDFFF_u32 + 2);
    let open = char::from_u32(e000).expect("U+E000");
    let planted = [
        format!("let s = \"{open}col\";"),
        format!("let o = '\\u{{{}}}';", "E000"),
        format!("let c = \"\\u{{{}}}\";", "e001"),
        format!("r#\"{{\"q\":\"\\u{}m:x\"}}\"#", "E000"),
        format!("blocked_by = char({e001}) || '@waits-on/0'"),
        format!("blocked_by = CHAR( {e000} )"),
        format!("let o = char::from_u32({e000}).unwrap();"),
        format!("WHERE unicode(data) = {e000}"),
        format!("let o = char::from_u32({e000}_u32);"),
        format!("WHERE instr(data, x'{}8081')", "EE"),
        format!("let o = '\\u{{{}}}';", "0e000"),
        format!("let o = '\\u{{{}}}';", "E0_00"),
        format!("let n = 0x{};", "E000"),
        format!("let b = b\"\\x{}\\x80\\x81\";", "ee"),
        // a blank ends a token, a hex number takes leading zeros, a blob holds the bytes anywhere (/code-review)
        format!("WHERE unicode(data) = {e000} AND pos = 0"),
        format!("let o = 0x{} as u32;", "E000"),
        format!("let o = 0x0{};", "E001"),
        format!("WHERE instr(data, x'636F6C{}8080')", "EE"),
    ];
    // a colour's hex, built here so the colour lint reads none
    let colour = format!("color: {}ee8080;", '#');
    let clean = [
        colour.as_str(),
        "let held = dash_store::older::holds_marker(a);",
        "let mask = 0xE000_0000 | 0xe00123;",
        "let n = 573440 + 157344;",
        "WHERE data = x'0EE8080F'",
        "let s = older::marked(\"⟦m:companies:x⟧\");",
    ];
    for line in &planted {
        assert_eq!(marker_literals(line), vec![1], "{line}");
    }
    for line in clean {
        assert_eq!(
            marker_literals(line),
            Vec::<usize>::new(),
            "a line through dash_store::older holds no marker: {line}"
        );
    }
    let file = format!("{}\n{}\n{}", clean[0], planted[0], clean[1]);
    assert_eq!(marker_literals(&file), vec![2], "the line named");
}

/// The rule above on a planted crate: #1180's fixture, each spelling, and what must not trip it.
#[dash_test(rule!())]
fn a_fixture_naming_private_crm_fails_the_gate() {
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
#[dash_test(rule!())]
fn rust_holds_no_script() {
    let files = tracked(&rust_root());
    assert_eq!(
        scripts(&rust_root(), &files),
        Vec::<String>::new(),
        "new logic is Rust: port the script to a dash command with its test, then delete the script"
    );
}

#[dash_test(rule!())]
fn a_planted_script_fails_the_gate() {
    let dir = TestDir::new("gate-scripts");
    let files = [
        ("100644", "tools/run.sh", "echo"),
        ("100644", "tools/fit.py", "x = 1"),
        ("100755", "tools/go", "fn"),
        ("100755", "bin/dash", "#!/bin/bash\n"),
        ("100755", "crates/bin/dash", "#!/bin/bash\n"),
        // Rust's inner attribute, never a shebang
        ("100644", "crates/a/src/lib.rs", "#![deny(warnings)]\n"),
        ("100644", "crates/a/bare", "#!"),
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
            "crates/a/bare: opens on #! without its executable bit",
            "ci/lost.sh: a script",
        ]
    );
    // the held script without its executable bit is any #! file
    assert_eq!(
        scripts(&dir.0, &[("100644".into(), "bin/dash".into())]),
        ["bin/dash: opens on #! without its executable bit"]
    );
}

#[dash_test(rule!())]
fn no_binary_fixture_is_tracked_under_crates() {
    let files = tracked(&rust_root());
    assert_eq!(
        binary_fixtures(&rust_root(), &files),
        Vec::<String>::new(),
        "a binary test file is fetched, never tracked: list `<sha256> <name> <url>` in its crate's fixtures.txt, read it \
         with dash_meter::fixtures::fixture, then git rm it"
    );
}

#[dash_test(rule!())]
fn a_planted_binary_fixture_fails_the_gate() {
    let dir = TestDir::new("gate-binary");
    let files: [(&str, &[u8]); 7] = [
        (
            "crates/a/tests/fixtures/dot.png",
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR",
        ),
        (
            "crates/a/tests/fixtures/scan.pdf",
            b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\nstream\n\0\x9c",
        ),
        (
            "crates/a/tests/fixtures/page.html",
            "<p>caf\u{e9}</p>".as_bytes(),
        ),
        ("crates/a/tests/fixtures/empty.pdf", b""),
        // a reader's page in another charset is text, as git reads it
        ("crates/a/tests/fixtures/latin1.html", b"<p>caf\xe9</p>"),
        ("crates/a/fixtures.txt", b"# a list\n"),
        // outside crates/, as the mirror's ci/
        ("ci/blob.bin", b"\0"),
    ];
    for (path, bytes) in files {
        let file = dir.0.join(path);
        std::fs::create_dir_all(file.parent().expect("a folder")).expect("folders");
        std::fs::write(file, bytes).expect("write");
    }
    let listed: Vec<(String, String)> = files
        .iter()
        .map(|(path, _)| ("100644".to_string(), path.to_string()))
        .collect();
    assert_eq!(
        binary_fixtures(&dir.0, &listed),
        [
            "crates/a/tests/fixtures/dot.png: binary",
            "crates/a/tests/fixtures/scan.pdf: binary",
        ]
    );
}

/// The mirror's workflow runs one line a step, its logic in ci/'s scripts.
#[dash_test(rule!())]
fn ci_steps_are_one_line() {
    let ci = std::fs::read_to_string(rust_root().join(".github/workflows/ci.yml")).expect("ci.yml");
    assert_eq!(
        inline_runs(&ci),
        Vec::<String>::new(),
        "a workflow step is one line calling `dash`: move the block into a dash command with its test"
    );
}

/// Done-when 2 of rust-ci-apt-retry: the mirror's packages come from the .debs we hold, apt only once they failed; an apt
/// in the workflow is refused (an Ubuntu outage turned every run red, 2026-11-11). held-debs' own order, dpkg before apt,
/// is crates/held-debs's the_held_debs_install_first_and_apt_runs_only_when_they_fail.
#[dash_test(rule!())]
fn apt_runs_after_held_debs() {
    let ci = std::fs::read_to_string(rust_root().join(".github/workflows/ci.yml")).expect("ci.yml");
    assert!(
        ci.contains("dash-held-debs -- bubblewrap libfontconfig1-dev poppler-utils"),
        "the test job installs its packages through held-debs"
    );
    // the workflow and every script its steps run (/code-review: an apt in ci/deno.sh passed)
    let mut found: Vec<String> = apt_in_workflow(&ci)
        .into_iter()
        .map(|f| format!("ci.yml {f}"))
        .collect();
    for script in std::fs::read_dir(rust_root().join("ci")).expect("ci/") {
        let path = script.expect("entry").path();
        let text = std::fs::read_to_string(&path).expect("script");
        let name = path.display().to_string();
        found.extend(
            apt_in_workflow(&text)
                .into_iter()
                .map(|f| format!("{name} {f}")),
        );
    }
    assert!(
        found.is_empty(),
        "install a package with `cargo run -q --locked -p dash-held-debs -- <package>`, never apt. Found: {found:?}"
    );
    // the refusal itself, on the step this task replaced; a comment, whole line or trailing, read as text
    let old = "steps:\n  # apt-get once\n  - run: sudo apt-get update && sudo apt install -y bubblewrap\n  - run: true # no apt\n";
    assert_eq!(
        apt_in_workflow(old),
        ["line 3: apt with no held .debs first"]
    );
}

/// The lines of `source` that shell out to kill (`Command::new`, `process::run`, any path to it) with a negative pid within
/// the call's next 6 lines: `"-{pgid}"`, `"--"` or `(-pid)`. Ubuntu 24.04's procps kill read such a pgid by its first digit.
fn shelled_group_kills(source: &str) -> Vec<usize> {
    // built, so this file never matches itself
    let programs = [
        concat!("\"", "kill\""),
        concat!("\"/bin/", "kill\""),
        concat!("\"/usr/bin/", "kill\""),
    ];
    let negative = [
        concat!("\"-", "{"),
        concat!("\"-", "-\""),
        concat!("(", "-"),
    ];
    let lines: Vec<&str> = source.lines().collect();
    (0..lines.len())
        .filter(|&i| {
            let shelled = lines[i].contains("Command::new(") || lines[i].contains("run(");
            shelled
                && programs.iter().any(|p| lines[i].contains(p))
                && lines[i..(i + 7).min(lines.len())]
                    .iter()
                    .any(|l| negative.iter().any(|n| l.contains(n)))
        })
        .map(|i| i + 1)
        .collect()
}

/// A process group is ended with libc::killpg, never a shelled `kill -<sig> -<pgid>`: Ubuntu 24.04's procps kill read the
/// pgid by its first digit, and a pgid starting with 1 became kill(-1), every process of the user, the Actions runner's
/// agent with them (PR 1444's mirror runs hung to the job's 20 minutes, 2026-11-11; procps issue 354).
#[dash_test(rule!())]
fn no_group_is_ended_by_a_shelled_kill() {
    let mut bad = vec![];
    for file in rust_files(&rust_root().join("crates")) {
        let text = std::fs::read_to_string(&file).expect("read");
        for line in shelled_group_kills(&text) {
            bad.push(format!("{}:{line}", file.display()));
        }
    }
    assert_eq!(
        bad,
        Vec::<String>::new(),
        "end the group with libc::killpg(pgid, signal), as dash::launchd and surface::pdf do"
    );
}

/// The detector on the forms a group kill takes, and on the pid kills and look-alikes it leaves alone (/code-review).
#[dash_test(rule!())]
fn a_shelled_group_kill_is_found_in_each_form() {
    let k = concat!("ki", "ll");
    let found = |code: String| shelled_group_kills(&code);
    let pdf_rs_195 = format!(
        "let _ = Command::new(\"{k}\")\n    .args([\"-KILL\", &format!(\"-{{}}\", child.id())])\n    .status();"
    );
    assert_eq!(found(pdf_rs_195), [1], "the old pdf.rs, rustfmt's split");
    let split = format!(
        "Command::new(\"/bin/{k}\")\n.arg(\"-s\")\n.arg(\"KILL\")\n.arg(\"--\")\n.arg(pgid)\n.status();"
    );
    assert_eq!(found(split), [1], "a path to it, its group behind --");
    let run = format!("process::run(\"{k}\", &[\"-KILL\", &(-pid).to_string()], None, CALL);");
    assert_eq!(found(run), [1], "through process::run, a negated pid");
    for alone in [
        format!("let _ = Command::new(\"{k}\").args([\"-9\", &pid]).status();"),
        format!("let _ = process::run(\"{k}\", &[&pid], None, CALL);"),
        format!("self.call(&[\"{k}\", pane]).map(drop)"),
        format!("hc.close(\"t7\").expect(\"{k}\");"),
    ] {
        assert_eq!(
            found(alone.clone()),
            Vec::<usize>::new(),
            "a pid's kill, or no kill run: {alone}"
        );
    }
}

/// site_perso's workflows too, their logic in bin/ci/ (task ci-steps-scripts). The public mirror holds rust/ alone, so
/// there this reads nothing; it holds on the Mac before each push (rust/CLAUDE.md rule 2).
#[dash_test(rule!())]
fn site_perso_ci_steps_are_one_line() {
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
    assert_eq!(
        bad,
        Vec::<String>::new(),
        "a site_perso workflow step is one line calling `dash`: move its block into a dash command with its test; \
         a bin/ci/ script still held opens with `set -e`"
    );
}

/// Done-when 2 and 3 of rust-ci-off-mac (@nha, 2026-11-11: « that and the CI runner should probably go to the VPS »): each
/// active workflow runs on the VPS alone, never a Mac runner, and builds nothing there: the gate and the site check run
/// main's binaries, which the mirror's publish job builds once a push of main. site.yml holds no Fly token and no deploy
/// (bin/ci-runner-test's site.yml part, ported: a pull request edits this file).
#[dash_test(rule!())]
fn the_active_workflows_run_on_the_vps_and_build_nothing_there() {
    if !rust_root().join("../private/dashboard").is_dir() {
        println!("no private/dashboard beside rust/: the mirror, which holds rust/ alone");
        return;
    }
    let read = |path: &str| std::fs::read_to_string(rust_root().join(path)).expect(path);
    let mut bad = vec![];
    for name in ["explainer-gate.yml", "approval-number.yml", "site.yml"] {
        let text = read(&format!("../.github/workflows/{name}"));
        let places: Vec<&str> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("runs-on:"))
            .collect();
        if places.is_empty() || places.iter().any(|p| p.trim() != "[site-perso-vps]") {
            bad.push(format!(
                "{name}: runs-on {places:?}, not [site-perso-vps] alone"
            ));
        }
        let code: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect();
        for word in [
            "cargo ",
            "rustup",
            "site-host.sh",
            "self-hosted",
            "define-ci",
        ] {
            if code.iter().any(|l| l.contains(word)) {
                bad.push(format!("{name}: « {word} » outside its comments"));
            }
        }
        if !code.iter().any(|l| l.contains("filter: blob:none")) {
            bad.push(format!(
                "{name}: a checkout that fetches every blob, private/ too"
            ));
        }
    }
    // CI never touches Fly (@nha, 2026-10-25: « Let's not use it for CI. »): no workflow and no bin/ci/ script holds its
    // token or a deploy, as bin/ci-runner-test checked; site.yml, which a pull request edits, names no secret at all
    let mut texts = vec![];
    for dir in ["../.github/workflows", "../bin/ci"] {
        let entries = std::fs::read_dir(rust_root().join(dir)).expect(dir);
        for path in entries.map(|e| e.expect("entry").path()) {
            let text = std::fs::read_to_string(&path).expect("a workflow or script");
            texts.push((path.display().to_string(), text));
        }
    }
    for (path, text) in &texts {
        for word in ["--remote-only", "secrets.FLY_API_TOKEN", "flyctl deploy"] {
            if text.contains(word) {
                bad.push(format!(
                    "{path}: « {word} »: CI holds no Fly token and deploys nothing"
                ));
            }
        }
    }
    let site = read("../.github/workflows/site.yml");
    for word in ["FLY_", "flyctl", "secrets", "continue-on-error"] {
        if site.contains(word) {
            bad.push(format!(
                "site.yml: « {word} »: no secret, no deploy, and its check fails the job"
            ));
        }
    }
    if !site.contains("run: bin/site-check local \"$RUNNER_TEMP/site\" ")
        || !site.contains("with: {name: site}")
    {
        bad.push("site.yml: no page check on main's site binary".into());
    }
    // Done-when 2: the gate runs main's prebuilt dash, fetched by main-bin, never a build (cargo is refused above)
    let gate = read("../.github/workflows/explainer-gate.yml");
    if !gate.contains("uses: ./.github/actions/main-bin")
        || !gate.contains("with: {name: dash}")
        || !gate.contains("run: $RUNNER_TEMP/dash scripts count --check")
    {
        bad.push("explainer-gate.yml: its script count does not run main's prebuilt dash".into());
    }
    let mirror = read(".github/workflows/ci.yml");
    if !mirror.contains("gh release upload main-bins dash.gz site.gz") {
        bad.push(
            "rust/.github/workflows/ci.yml: main's binaries are no longer published to main-bins"
                .into(),
        );
    }
    assert_eq!(
        bad,
        Vec::<String>::new(),
        "run the job on [site-perso-vps] with a blob:none sparse checkout, and fetch main's binary from the mirror's \
         release main-bins in place of a build"
    );
}

/// The script ratchet (dash_gate::ratchet, task rust-scripts-port) runs at both doors: the pre-commit against HEAD's number
/// (in a merge, the side main holds: pre_commit::measured_from), the PR gate against main's side of the merge ref, so a
/// number raised to let a script in fails at either.
#[dash_test(rule!())]
fn the_precommit_and_the_pr_gate_run_the_script_count() {
    if !rust_root().join("../private/dashboard").is_dir() {
        println!("no private/dashboard beside rust/: the mirror, which holds rust/ alone");
        return;
    }
    let read = |path: &str| std::fs::read_to_string(rust_root().join(path)).expect(path);
    // the hook is `dash hook pre-commit`, whose scripts-count step is dash_gate::hook::pre_commit's (task rust-git-hooks-port)
    let hook = read("../.githooks/pre-commit");
    let step = read("crates/gate/src/hook/pre_commit.rs");
    assert!(
        hook.contains(" hook pre-commit\n")
            && step.contains(r#"ctx.timed("scripts-count", || scripts_count(ctx))?;"#)
            && step.contains(r#"crate::ratchet::run(&ctx.top, true, Some(&measured_from(ctx)))"#),
        "the pre-commit no longer runs dash scripts count --check from HEAD or a merge's main side: keep the \
         scripts-count step in dash_gate::hook::pre_commit::run, and .githooks/pre-commit calling dash hook pre-commit"
    );
    let gate = read("../.github/workflows/explainer-gate.yml");
    assert!(
        gate.contains("run: $RUNNER_TEMP/dash scripts count --check --base HEAD^1 --changed "),
        "explainer-gate.yml no longer runs dash scripts count --check --base HEAD^1 --changed"
    );
    let held = read("scripts-held.txt");
    assert!(
        held.trim().parse::<usize>().is_ok(),
        "rust/scripts-held.txt holds no bare count: {held:?}"
    );
}

#[dash_test(rule!())]
fn a_planted_run_block_fails_the_gate() {
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

/// Task sandbox-one-for-all's part 4: the launchd section of rfc-rust-unified compares safehouse with mxc-sdk, the
/// FilesystemPolicy lists of the layer named. On the mirror, which holds rust/ alone, this reads nothing.
#[dash_test(rule!())]
fn the_launchd_section_compares_safehouse_with_mxc() {
    let path = rust_root().join("../private/docs/rfc-rust-unified.html");
    if !path.is_file() {
        println!("no {} beside rust/: the mirror", path.display());
        return;
    }
    let rfc = std::fs::read_to_string(&path).expect("rfc-rust-unified.html");
    let launchd = rfc_section(&rfc, "<h3 id=\"launchd\">");
    for words in [
        "<th>safehouse (agent-safehouse, shell)</th><th>mxc-sdk",
        "<code>FilesystemPolicy</code>",
        "read-write, read-only",
        "make -C ~/repo/site_perso claude DIR=",
    ] {
        assert!(
            launchd.contains(words),
            "rfc-rust-unified#launchd lacks « {words} »: the safehouse and mxc-sdk comparison belongs there"
        );
    }
}

/// rust_budgets_test.exs, Rust side: section 3's crates and named edges, and section 4's budgets, are the gate's RINGS and
/// edges and the meter's BUDGETS. Its own rows of each: the RFC is private, so on the public mirror, which holds rust/
/// alone, this reads nothing; it holds on the Mac before each push (rust/CLAUDE.md rule 2), and the Elixir test on an RFC edit.
#[dash_test(rule!())]
fn sections_3_and_4_are_the_gate_and_the_meter() {
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
    assert_eq!(
        rows("<tr><td>a</td><td><code>4 GB</code></td></tr>"),
        [],
        "a cell holding markup is no budget row: rows leaves it out"
    );
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
            "an allowed edge is refused, fix bad_edges: {from} -> {to}"
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

/// `mix lint`'s deno run (Dashboard.Lint's `@deno`) on rust/'s scripts for the browser, the design page's today: a Rust PR runs no
/// lint, so a finding merged here fails main's next commit.
#[dash_test(rule!())]
fn rust_js_passes_mix_lints_deno() {
    let files = tracked(&rust_root());
    assert!(
        files
            .iter()
            .any(|(_, path)| path == "crates/surface/src/design/design.js"),
        "the walk found the .js"
    );
    assert_eq!(
        deno_lint(&rust_root(), &files),
        Vec::<String>::new(),
        "fix each line deno names; when deno did not run, install the pinned one with bin/ci/lint-tools.sh"
    );
}

#[dash_test(rule!())]
fn a_planted_lint_finding_fails_the_gate() {
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
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    let found = deno_lint(&dir.0, &listed(&["ok.mjs", "bad.mjs"])).join("\n");
    assert!(found.contains("no-unused-vars"), "{found}");
    assert!(found.contains("bad.mjs"), "{found}");
}

/// Task rust-fmt-gate: rust/ stays as `cargo fmt` leaves it, so a format run rewrites only the files it touches
/// (rust-nav-polish's rewrote untouched ones). The fix: `git ls-files -z '*.rs' | xargs -0 rustfmt --edition 2024` in rust/.
#[dash_test(rule!())]
fn rust_is_rustfmt_clean() {
    let files = tracked(&rust_root());
    assert!(
        files.iter().all(|(_, path)| !path.starts_with("vendor/")),
        "vendor/ is not ours"
    );
    assert_eq!(
        rustfmt_check(&rust_root(), &files),
        Vec::<String>::new(),
        "run `cargo fmt` in rust/ and commit what it changed"
    );
}

#[dash_test(rule!())]
fn a_planted_unformatted_file_fails_the_gate() {
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
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
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
#[dash_test(rule!())]
fn the_deno_copies_are_mix_lints() {
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
    // the mirror checks the version alone
    let Some(monorepo) = dash_meter::monorepo() else {
        return;
    };
    let (check, lint) = (
        monorepo.join("bin/ci/lint-tools.sh"),
        monorepo.join("private/dashboard/lib/dashboard/lint.ex"),
    );
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

/// Done-when 2 of rust-skeleton-live: no page's words send him to a reload; a section still read is a skeleton that fills
/// itself (@nha, 2026-11-09: « Reading every access: the next reload shows it. does not make sense. »).
#[dash_test(rule!())]
fn no_page_promises_the_next_reload() {
    assert_eq!(
        reload_promised(&rust_root()),
        Vec::<String>::new(),
        "a page never promises the next reload: show the state it holds now, or have it refresh itself"
    );
}

/// The rule above on a planted tree: the phrase in any case or over a continued line fails it; a comment, a test's own
/// words and the gate's list pass.
#[dash_test(rule!())]
fn a_planted_reload_promise_fails_the_gate() {
    let dir = TestDir::new("gate-reload");
    let clean = "// the next reload shows it, said in a comment\nfn f() -> &'static str { \"Next: being read.\" }\n#[cfg(test)]\nmod tests { const X: &str = \"the next reload\"; }\n";
    write(&dir.0, "crates/dash/src/page.rs", clean);
    write(
        &dir.0,
        "crates/gate/src/html.rs",
        "const N: [&str; 1] = [\"next reload\"];\n",
    );
    assert_eq!(
        reload_promised(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    for code in [
        "fn f() -> &'static str { \"Reading: the next reload shows it.\" }",
        "const X: &str = r#\"The NEXT Reload\"#;",
        "fn f() -> &'static str { \"the next \\\n        reload shows it\" }",
    ] {
        write(&dir.0, "crates/dash/src/page.rs", code);
        assert_eq!(
            reload_promised(&dir.0),
            ["crates/dash/src/page.rs: « next reload »"],
            "{code}"
        );
    }
}

/// Done-when 2 of rust-registry-no-hubs: no page's route is in main.rs, and tasks.rs names no other module's page, so a new
/// page edits neither (@nha, 2026-11-09: « is it an orchestration failure or is it because the architecture is not decoupled
/// enough yet ? »).
#[dash_test(rule!())]
fn a_new_page_edits_no_hub() {
    assert_eq!(
        hub_named_pages(&rust_root()),
        Vec::<String>::new(),
        "register the page's route in its own module (crates/dash/src/pages.rs), never in a hub"
    );
}

/// The rule above on a planted tree: a page module registering its paths passes, and so do a hub's tests naming them; a
/// route for it in main.rs, by its path or its prefix, in tasks.rs, or registered in main.rs, fails it. The registrations
/// here spell `step` before `path`: a page is read whatever its field order, or a hub naming it would pass.
#[dash_test(rule!())]
fn a_planted_hub_route_fails_the_gate() {
    let dir = TestDir::new("gate-hubs");
    let page = |path: &str| {
        format!(
            "inventory::submit! {{\n    crate::pages::Page {{ step: \"route_x\", path: \"{path}\", serve: |a| a.form(None) }}\n}}\n"
        )
    };
    let main = "mod new;\nfn main() {}\n#[cfg(test)]\nmod tests { const P: &str = \"/new\"; }\n";
    write(&dir.0, "crates/dash/src/main.rs", main);
    write(&dir.0, "crates/dash/src/tasks.rs", &page("/tasks"));
    let new = [page("/new"), page("/new/*")].concat();
    write(&dir.0, "crates/dash/src/new.rs", &new);
    assert_eq!(
        hub_named_pages(&dir.0),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
    let owner = "the page of crates/dash/src/new.rs: route it there";
    for (hub, code, said) in [
        (
            "main.rs",
            "fn r(p: &str) -> bool { p == \"/new\" }",
            "names /new,",
        ),
        (
            "main.rs",
            "fn r(p: &str) -> bool { p.starts_with(\"/new/\") }",
            "names /new/,",
        ),
        ("tasks.rs", "const LINK: &str = \"/new\";", "names /new,"),
    ] {
        write(&dir.0, &format!("crates/dash/src/{hub}"), code);
        assert_eq!(
            hub_named_pages(&dir.0),
            [format!("crates/dash/src/{hub} {said} {owner}")],
            "{code}"
        );
        let clean = if hub == "main.rs" {
            main.to_string()
        } else {
            page("/tasks")
        };
        write(&dir.0, &format!("crates/dash/src/{hub}"), &clean);
    }
    write(&dir.0, "crates/dash/src/main.rs", &page("/old"));
    assert_eq!(
        hub_named_pages(&dir.0),
        ["crates/dash/src/main.rs registers /old: register it in its page's module"]
    );
}

/// Task rust-deps-view: every crate sits in a layer of section 3, the boxes /deps nests it in.
#[dash_test(rule!())]
fn every_crate_sits_in_a_layer_of_section_3() {
    let (crates, _) = crate_graph(&rust_root());
    let unplaced: Vec<&String> = crates.iter().filter(|c| layer(c) == "Other").collect();
    assert!(
        unplaced.is_empty(),
        "{unplaced:?}: in no layer of section 3: {PLACE_FIX}"
    );
}

/// Task design-cards-not-default: a page lists things as a `list` (rows) or a `table`; a `cards` container only where a card
/// was approved (@nha, 2026-11-09: « Cards are not the default UI element », « I am tempted to ban them as a UI element
/// unless explicitly approved »). The design system itself (crates/surface) may draw and show one.
#[dash_test(rule!())]
fn cards_only_where_approved() {
    // none since /drafts became tables (task rust-drafts-table; @nha, 2026-11-09: « propose a different UI not a sea of
    // cards. »): a card a page wants is his approval first, added here
    const APPROVED: [&str; 0] = [];
    let root = rust_root();
    let pages: Vec<String> = rust_files(&root.join("crates"))
        .into_iter()
        .filter(|f| {
            !f.starts_with(root.join("crates/surface"))
                && f.components().any(|c| c.as_os_str() == "src")
        })
        .filter(|f| std::fs::read_to_string(f).is_ok_and(|s| s.contains(r#""type": "cards""#)))
        .map(|f| {
            f.strip_prefix(&root)
                .expect("under rust/")
                .display()
                .to_string()
        })
        .filter(|f| !APPROVED.contains(&f.as_str()))
        .collect();
    assert!(
        pages.is_empty(),
        "{pages:?}: a list of things is a \"list\" or a \"table\""
    );
}

/// The pre-commit's no-elixir step (dash_gate::hook::no_elixir, once bin/ci/no-elixir.sh) refuses an Elixir edit, yet lets a
/// branch take its own back out: the index read against HEAD alone refused the revert its message asks for (rust-search-port,
/// 2026-11-10).
#[dash_test(rule!())]
fn no_elixir_lets_a_branch_take_its_edit_back_out() {
    use dash_gate::hook::{Ctx, no_elixir};
    let dir = TestDir::new("gate-no-elixir");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
            ])
            .args(args)
            .current_dir(&dir.0)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let stage = |text: &str| {
        write(&dir.0, "a.ex", text);
        git(&["add", "a.ex"]);
    };
    let gate = || match no_elixir::staged(&Ctx::new(&dir.0, "pre-commit").expect("a repository")) {
        Ok(()) => (Some(0), String::new()),
        Err(said) => (Some(1), said),
    };
    git(&["init", "-q"]);
    write(&dir.0, "forge.env", "DEFAULT_BRANCH=main\n");
    stage("one\n");
    git(&["commit", "-qm", "main"]);
    git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    stage("two\n");
    git(&["commit", "-qm", "the branch's edit"]);
    stage("one\n");
    assert_eq!(
        gate(),
        (Some(0), String::new()),
        "main's own content back is no change"
    );
    git(&["update-index", "--chmod=+x", "a.ex"]);
    assert_eq!(
        gate().0,
        Some(1),
        "main's content with another mode is a change"
    );
    git(&["update-index", "--chmod=-x", "a.ex"]);
    git(&["update-ref", "-d", "refs/remotes/origin/main"]);
    assert_eq!(gate().0, Some(1), "with no origin/main, HEAD alone decides");
    stage("three\n");
    let (code, out) = gate();
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("a.ex"), "the refusal names the file: {out}");
}

/// A PR gate runs main's dash ($RUNNER_TEMP/dash, the release main-bins), which lags the PR: a step calling a command main's
/// dash lacks fails on its own PR with usage, exit 2 (PR 1446's no-elixir step, 2026-11-11: « CI is red on PR 1446: gate »).
/// Each `$RUNNER_TEMP/dash <command>` names one origin/main's usage lists, or falls back on exit 2 (`[ $? = 2 ]`), and a
/// fallback's `git show <ref>:<path>` names a file that ref holds: PR 1446 deleted the script its fallback read from the
/// base, and every PR's gate went red while this test passed (PR 1450, 2026-11-11). Read in the monorepo, with an origin/main.
#[dash_test(rule!())]
fn a_gate_step_runs_a_command_mains_dash_answers() {
    if !rust_root().join("../private/dashboard").is_dir() {
        println!("no private/dashboard beside rust/: the mirror, which holds rust/ alone");
        return;
    }
    // its usage lines, each registered beside its command in any of dash's modules (usage.rs), or main.rs's one USAGE before
    // task rust-usage-registered
    let shown = std::process::Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "grep",
            "-h",
            "-E",
            r#"Command::usage\(|usage: "|const USAGE: &str = ""#,
            "origin/main",
            "--",
            "crates/dash/src",
            "crates/terminal/src/say.rs",
        ])
        .current_dir(rust_root())
        .output()
        .expect("git");
    if !shown.status.success() {
        println!("no origin/main here: nothing to hold the gate's commands to");
        return;
    }
    let usage = String::from_utf8_lossy(&shown.stdout).into_owned();
    assert!(
        usage.contains("dash serve"),
        "origin/main's dash lists its usage: {usage}"
    );
    let dir = rust_root().join("../.github/workflows");
    let mut bad = vec![];
    for path in std::fs::read_dir(&dir)
        .expect("workflows")
        .map(|e| e.expect("entry").path())
    {
        let text = std::fs::read_to_string(&path).expect("workflow");
        for line in text.lines() {
            for call in line.split("$RUNNER_TEMP/dash ").skip(1) {
                let words: Vec<&str> = call
                    .split_whitespace()
                    .take_while(|w| {
                        !w.starts_with('-') && w.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                    })
                    .collect();
                let command = format!("dash {}", words.join(" "));
                // the whole command, never a prefix of a longer one (« dash ci » is no « dash ci no-elixir »)
                let answered = usage.match_indices(&command).any(|(at, _)| {
                    !usage[at + command.len()..]
                        .starts_with(|c: char| c.is_ascii_lowercase() || c == '-')
                });
                if !answered && !line.contains("[ $? = 2 ]") {
                    bad.push(format!(
                        "{}: {command}",
                        path.file_name().expect("name").to_string_lossy()
                    ));
                }
            }
            // a fallback's `git show <ref>:<path>` names a file that ref holds; HEAD^1, the base, is origin/main
            for shown in line.split("git show ").skip(1) {
                let spec = shown.split_whitespace().next().unwrap_or("");
                let spec = spec.replacen("HEAD^1:", "origin/main:", 1);
                let held = std::process::Command::new("git")
                    .args(["cat-file", "-e", &spec])
                    .current_dir(rust_root())
                    .status()
                    .expect("git")
                    .success();
                if !held {
                    bad.push(format!(
                        "{}: git show {spec}, no such file",
                        path.file_name().expect("name").to_string_lossy()
                    ));
                }
            }
        }
    }
    assert_eq!(
        bad,
        Vec::<String>::new(),
        "main's dash answers these with usage, exit 2, until main-bins republishes: fall back on `[ $? = 2 ]` to a file \
         a ref still holds, or land the command in main first"
    );
}

/// A file a crate compiles in rides the mirror, or the mirror alone fails to build (rust/parity.tsv, PR 1407, 2026-11-11).
#[dash_test(rule!())]
fn every_included_file_rides_the_mirror() {
    let files = tracked(&rust_root());
    let mirror = std::fs::read_to_string(rust_root().join("ci-mirror.txt")).expect("ci-mirror.txt");
    assert_eq!(
        includes_off_the_mirror(&rust_root(), &files, &mirror),
        Vec::<String>::new(),
        "add each included file, or its folder, to rust/ci-mirror.txt"
    );
}

/// A file read past rust/ at run time is missing on the mirror: every test reaches it through dash_meter::monorepo(), which
/// stops there (PR 1426, 2026-11-11).
#[dash_test(rule!())]
fn every_run_time_path_past_rust_goes_through_the_monorepo_helper() {
    assert_eq!(
        run_time_paths_past_rust(&rust_root(), &tracked(&rust_root())),
        Vec::<String>::new(),
        "reach the monorepo through dash_meter::monorepo(), which stops the test on the mirror"
    );
}

/// Done-when 3 of rust-no-build-path-at-runtime: no code run outside a test reads the build's crate folder, which sccache
/// shares across worktrees (rust-merge-gate-port, 2026-11-12).
#[dash_test(rule!())]
fn no_code_outside_a_test_reads_the_build_s_crate_folder() {
    let found = build_paths_at_run_time(&rust_root(), &tracked(&rust_root()));
    assert_eq!(
        found,
        Vec::<String>::new(),
        "{} uses of the build's crate folder outside tests: read it at run time",
        found.len()
    );
}

/// Done-when 1 of rust-no-build-path-at-runtime, on planted files: the one in src fails with its line; one under
/// `#[cfg(test)]` (a module, a function, a one-line item), in a comment, in tests/, in a tests.rs or in a file under
/// `#![cfg(test)]` passes, and code after a test module is still read.
#[dash_test(rule!())]
fn a_build_path_in_run_time_code_fails_the_gate_and_one_in_a_test_passes() {
    let dir = TestDir::new("gate-build-path");
    // split, so this test's own text bakes nothing
    let baked = concat!("env!(\"CARGO_", "MANIFEST_DIR\")");
    // named, so no line of this file opens with an attribute every_rule_opens_with_its_guard would read
    let cfg = "#[cfg(test)]";
    let all = "#[cfg(all(test, unix))]";
    let src = format!(
        "fn slot() -> PathBuf {{\n    Path::new({baked}).join(\"x\")\n}}\n\n\
         {cfg}\nmod tests {{\n    fn fixture() {{\n        let p = {baked};\n    }}\n}}\n\n\
         {cfg}\nfn helper(\n    a: u8,\n) -> &'static str {{\n    {baked}\n}}\n\n\
         {cfg} const DIR: &str = {baked};\n\
         // was {baked}, and a comment naming #![cfg(test)]\n\
         const LATE: &str = option_{baked};\n\
         {cfg}\nmod quoted {{\n}} // quoted\n\
         fn build() -> S {{\n    S {{\n        {cfg}\n        t: 1,\n        u: {{\n            {baked}\n        }},\n    }}\n}}\n\
         {all}\nmod unix {{ // why\n    use super::*;\n    fn f() {{ let p = {baked}; }}\n}}\n\
         /* {baked} */\nfn after() {{}}\n"
    );
    write(&dir.0, "crates/a/src/lib.rs", &src);
    let test_only = format!("let p = {baked};\n");
    write(&dir.0, "crates/a/tests/t.rs", &test_only);
    write(&dir.0, "crates/a/src/x/tests.rs", &test_only);
    write(
        &dir.0,
        "crates/a/src/only.rs",
        &format!("#![cfg(test)]\n{test_only}"),
    );
    let files: Vec<(String, String)> = [
        "crates/a/src/lib.rs",
        "crates/a/tests/t.rs",
        "crates/a/src/x/tests.rs",
        "crates/a/src/only.rs",
    ]
    .iter()
    .map(|p| ("100644".to_string(), p.to_string()))
    .collect();
    // lines 2, 21 and 30: a cfg(all(test, ..)) module opened with a comment and a block comment count nothing
    assert_eq!(
        build_paths_at_run_time(&dir.0, &files),
        [
            "crates/a/src/lib.rs: 3 uses of the build's crate folder at run time; read it when the code runs"
        ]
    );
}

/// Claude Code's hooks are `dash hook` commands: .claude/hooks keeps only the two scripts frozen Elixir still runs, each
/// naming the task that retires it (task rust-hooks-rest-port, its check 4). outbound_gate.py, the third, went with task
/// outbound-gate.
#[dash_test(rule!())]
fn claude_hooks_keep_only_the_two_scripts_elixir_runs_each_naming_its_retiring_task() {
    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let kept = [
        ("agent_inbox.py", "rust-inbox-take-port"),
        ("merge-claim.py", "rerun-stale-timeout"),
    ];
    // the index itself: a tracked file missing on disk or a symlink counts, and a folder gone lists nothing
    let out = std::process::Command::new("git")
        .args(["ls-files", "-z", "--", ".claude/hooks"])
        .current_dir(&top)
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files: {:?}", out.status);
    let listing = String::from_utf8(out.stdout).expect("git ls-files: a path that is not UTF-8");
    let files: Vec<&str> = listing.split_terminator('\0').collect();
    let names = kept.map(|(f, _)| format!(".claude/hooks/{f}"));
    assert_eq!(files, names, "a new hook is a dash hook command");
    for (file, task) in kept {
        let text = std::fs::read_to_string(top.join(".claude/hooks").join(file)).expect(file);
        assert!(
            text.contains(&format!("# Retired by task {task} ")),
            "{file} names {task}, the task that retires it"
        );
    }
}

/// The rule above on planted files: a reach fails, with its line, and so do one rustfmt wrapped and a `format!` placeholder
/// one; one in a comment, a commented old path after an in-rust/ join, a path with no CARGO_MANIFEST_DIR before it and the
/// helper's own file pass.
#[dash_test(rule!())]
fn a_planted_run_time_path_past_rust_fails_the_gate() {
    let dir = TestDir::new("gate-past-rust");
    let reach = concat!(
        "let p = Path::new(env!(\"CARGO_",
        "MANIFEST_DIR\")).join(\"../",
        "../../bin/x\");\n"
    );
    write(&dir.0, "a.rs", &format!("// the mirror stops it\n{reach}"));
    write(&dir.0, "comment.rs", &format!("// {reach}"));
    write(&dir.0, "plain.rs", concat!("let p = \"../", "../../x\";\n"));
    // rustfmt's wrap of a `concat!`, its path led by a slash (PR 1480's fill-merge.js read)
    let wrapped = concat!(
        "let lib = read(concat!(\n    env!(\"CARGO_",
        "MANIFEST_DIR\"),\n    \"/../",
        "../../.claude/x.js\"\n));\n"
    );
    write(&dir.0, "wrapped.rs", wrapped);
    // a `format!` placeholder before the dots, and a commented old path that no reach follows
    let placeholder = concat!(
        "let p = format!(\"{}/../",
        "../../x.js\", env!(\"CARGO_MANIFEST_DIR\"));\n"
    );
    write(&dir.0, "placeholder.rs", placeholder);
    let noted = concat!(
        "let p = Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"src\");\n// was \"/../",
        "../../x\" before\n"
    );
    write(&dir.0, "noted.rs", noted);
    write(&dir.0, REACHES_PAST_RUST[0], reach);
    let files: Vec<(String, String)> = [
        "a.rs",
        "comment.rs",
        "plain.rs",
        "wrapped.rs",
        "placeholder.rs",
        "noted.rs",
        REACHES_PAST_RUST[0],
    ]
    .iter()
    .map(|p| ("100644".to_string(), p.to_string()))
    .collect();
    assert_eq!(
        run_time_paths_past_rust(&dir.0, &files),
        [
            "a.rs:2: reaches past rust/; take dash_meter::monorepo()",
            "wrapped.rs:2: reaches past rust/; take dash_meter::monorepo()",
            "placeholder.rs:1: reaches past rust/; take dash_meter::monorepo()"
        ]
    );
}

/// The rule above on planted files: an untracked target, one under no line, one past the root, a wrapped, a raw and an
/// `include!` one; a listed folder (with or without its `/`) and file pass, and so do a comment and a crate off the mirror.
#[dash_test(rule!())]
fn a_planted_include_off_the_mirror_fails_the_gate() {
    let dir = TestDir::new("gate-mirror");
    let code = "const A: &str = include_str!(\"../logo.svg\");\n\
                const B: &[u8] = include_bytes!(\"../../../marks.tsv\");\n\
                const C: &str = include_str!(\"../../../notes.md\");\n\
                const D: &str = include_str!(\"./gone.css\");\n\
                const E: &str = include_str!(\"../../../../out.txt\");\n\
                const F: &str = include_str!(\n    \"../../../wrapped.md\"\n);\n\
                const G: &str = include_str!(r#\"../../../raw.md\"#);\n\
                include!(\"../../../gen.rs\");\n\
                /// include_str!(\"../../../doc.md\") in a comment compiles nothing in\n";
    write(&dir.0, "crates/a/src/lib.rs", code);
    write(
        &dir.0,
        "bench/src/main.rs",
        "const Q: &str = include_str!(\"../q.txt\");\n",
    );
    let files: Vec<(String, String)> = [
        "crates/a/src/lib.rs",
        "crates/a/logo.svg",
        "marks.tsv",
        "notes.md",
        "wrapped.md",
        "raw.md",
        "gen.rs",
        "bench/src/main.rs",
        "bench/q.txt",
    ]
    .iter()
    .map(|p| ("100644".to_string(), p.to_string()))
    .collect();
    let off = |what: &str| format!("crates/a/src/lib.rs: {what}");
    let expected = [
        off("../../../notes.md is not on ci-mirror.txt's list"),
        off("./gone.css is not tracked"),
        off("../../../../out.txt is outside rust/"),
        off("../../../wrapped.md is not on ci-mirror.txt's list"),
        off("../../../raw.md is not on ci-mirror.txt's list"),
        off("../../../gen.rs is not on ci-mirror.txt's list"),
    ];
    for list in ["# a comment\ncrates/\nmarks.tsv\n", "crates\nmarks.tsv\n"] {
        assert_eq!(
            includes_off_the_mirror(&dir.0, &files, list),
            expected,
            "{list}"
        );
    }
}

/// Task rust-components-no-js (@nha, 2026-11-11: « we want pure rust components as much as possible. »): our inline script
/// bytes in rust/crates, held in rust/inline-js-held.txt, only go down; each script names what blocks its port, and a page
/// puts one on by its name alone.
#[dash_test(rule!())]
fn inline_js_only_goes_down() {
    let (scripts, bad) = inline_scripts(&rust_root());
    assert!(
        bad.is_empty(),
        "each script a NAME_JS const naming what blocks its port, put on a page by that name alone:\n{}",
        bad.join("\n")
    );
    let bytes = scripts.iter().map(|s| s.bytes).sum();
    // compiled in, never read at run time: every_included_file_rides_the_mirror then checks it rides the mirror (the
    // mirror lacked it, PR 1448's red ci-mirror/rust, 2026-11-11)
    let held = include_str!("../../../inline-js-held.txt")
        .trim()
        .parse()
        .ok();
    if let Err(why) = inline_js_verdict(bytes, held) {
        let each: Vec<String> = scripts
            .iter()
            .map(|s| format!("{} {} {}", s.at, s.name, s.bytes))
            .collect();
        panic!("{why}\n{}", each.join("\n"));
    }
}

/// Done-when 1 of rust-binary-ratchet (@nha, 2026-11-13: « small core, everything is a plugin »): crates/dash's tokens of
/// code at the checked-out head never pass its merge base's with origin/main, and a file under crates/dash/src the base lacks
/// is refused. No number is committed (task rust-binary-ratchet-red): the base, read from git, is the reference. A checkout
/// with no merge base (the mirror's parentless commits) and no DASH_BINARY_BASE skips, its reason printed. binary.rs's
/// the_head_is_measured_against_its_merge_base_in_tokens plants the growth, the new module and the shrink.
#[dash_test(rule!())]
fn the_binary_only_shrinks_and_takes_no_new_module() {
    let root = rust_root();
    let env = std::env::var(dash_gate::binary::BASE_ENV).ok();
    let rev = match dash_gate::binary::base(&root, env) {
        Ok(rev) => rev.unwrap_or_else(|why| panic!("{why}")),
        Err(skip) => {
            eprintln!("skipped: {skip}");
            return;
        }
    };
    let base = dash_gate::binary::count_at(&root, &rev).unwrap_or_else(|why| panic!("{why}"));
    let head = dash_gate::binary::count(&root);
    match dash_gate::binary::verdict(&head, &base, &rev) {
        Ok(said) => eprintln!("{said}"),
        Err(why) => panic!("{why}"),
    }
}

/// Task rust-forwarded-top, done-when 3 (@nha, 2026-11-14, in its dialog: « ratchet what code controls, the count of commands
/// Rust answers; the live share shows on /status and gates nothing. »): every command the front door answered from Rust at
/// the merge base, each a `served::answers("<command>", ..)` mark in its branch's condition, is answered still. The count is
/// read from the source at both ends, never stored.
#[dash_test(rule!())]
fn the_commands_rust_answers_only_grow() {
    let root = rust_root();
    let env = std::env::var(dash_gate::binary::BASE_ENV).ok();
    let rev = match dash_gate::binary::base(&root, env) {
        Ok(rev) => rev.unwrap_or_else(|why| panic!("{why}")),
        Err(skip) => {
            eprintln!("skipped: {skip}");
            return;
        }
    };
    let door = "crates/dash/src/main.rs";
    let head = std::fs::read_to_string(root.join(door)).expect("the front door's source");
    let base = std::process::Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "show",
            &format!("{rev}:rust/{door}"),
        ])
        .current_dir(&root)
        .output()
        .expect("git show");
    let base = String::from_utf8_lossy(&base.stdout);
    let (head, base) = (
        dash_gate::served::answered(&head),
        dash_gate::served::answered(&base),
    );
    match dash_gate::served::kept(&head, &base, &rev) {
        Ok(said) => eprintln!("{said}"),
        Err(why) => panic!("{why}"),
    }
}

/// A table row's cells, each with its tags dropped.
fn row_cells(row: &str) -> Vec<String> {
    let plain = |cell: &str| {
        let mut out = String::new();
        let mut in_tag = false;
        for c in cell.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.trim().to_string()
    };
    row.split("<td>")
        .skip(1)
        .map(|cell| plain(cell.split("</td>").next().unwrap_or("")))
        .collect()
}

/// Each file with no row in `table`, in more than one, whose row's crate (its second cell) is neither core nor a plugin
/// crate, or whose phase (its first cell) is neither « stays », core's, nor a move's number beside that move's task. A file is
/// matched by its `<code>` in the third cell only, so a cell that cites a file elsewhere names no crate.
fn port_map_misses(table: &str, files: &BTreeSet<String>) -> Vec<String> {
    let known = |name: &str| {
        name == "core"
            || PLUGINS.contains(&name)
            || TASK_LAYERS.contains(&name)
            || PLANNED_PLUGINS.contains(&name)
    };
    map_misses(table, files, known, |f, phase, name, row| {
        let moves = phase.parse::<u32>().is_ok() && row.contains("issue=rust-move-");
        (!(moves || phase == "stays" && name == "core")).then(|| {
            format!("{f}: phase « {phase} » is neither « stays » (core) nor a move's number with its task")
        })
    })
}

/// The crates graal's table may name: the rings, leaves and tooling it moves into, and its three plugins (flare, sources,
/// meetings), which no binary module moves into. `site` takes graal's server (G8).
const GRAAL_CRATES: [&str; 16] = [
    "core", "ingest", "pdf", "ocr", "process", "index", "store", "ml", "statsd", "hooks", "jobs",
    "surface", "site", "flare", "sources", "meetings",
];

/// Graal's commit the port map counts (its writer notes say « graal at 8bb2c21c »): the test reads this tree, never graal's
/// working copy, so a file added there fails nothing here until the map is recounted.
const GRAAL_PIN: &str = "8bb2c21c";

/// [`port_map_misses`] for graal's table: a crate of [`GRAAL_CRATES`], a phase G1 to G8 beside a task, each phase one task and
/// no task two phases.
fn graal_misses(table: &str, files: &BTreeSet<String>) -> Vec<String> {
    let task = |row: &str| {
        row.split("?issue=")
            .nth(1)
            .map(|rest| rest.split('"').next().unwrap_or(rest).to_string())
    };
    let mut misses = map_misses(
        table,
        files,
        |name| GRAAL_CRATES.contains(&name),
        |f, phase, _, row| {
            let phased = phase
                .strip_prefix('G')
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|n| (1..=8).contains(&n));
            (!phased || task(row).is_none())
                .then(|| format!("{f}: phase « {phase} » is not G1 to G8 with its task"))
        },
    );
    let mut tasks: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in table.split("<tr>").skip(1) {
        if let (Some(phase), Some(t)) = (
            row_cells(row)
                .first()
                .filter(|p| p.starts_with('G'))
                .cloned(),
            task(row),
        ) {
            tasks.entry(phase).or_default().insert(t);
        }
    }
    let mut seen = BTreeMap::new();
    for (phase, ts) in &tasks {
        if ts.len() > 1 {
            misses.push(format!("{phase}: {} tasks, one wanted", ts.len()));
        }
        for t in ts {
            if let Some(other) = seen.insert(t.clone(), phase.clone()) {
                misses.push(format!(
                    "{t}: the task of {other} and {phase}, one phase wanted"
                ));
            }
        }
    }
    misses
}

/// Each file with no row in `table` or in more than one, whose row's crate (its second cell) `known` refuses, or whose phase
/// (its first cell) `phased` refuses, by the line it returns.
fn map_misses(
    table: &str,
    files: &BTreeSet<String>,
    known: impl Fn(&str) -> bool,
    phased: impl Fn(&str, &str, &str, &str) -> Option<String>,
) -> Vec<String> {
    files
        .iter()
        .filter_map(|f| {
            let code = format!("<code>{f}</code>");
            let rows: Vec<&str> = table
                .split("<tr>")
                .filter(|row| {
                    row.split("<td>")
                        .nth(3)
                        .is_some_and(|cell| cell.contains(&code))
                })
                .collect();
            let [row] = rows[..] else {
                return Some(format!("{f}: in {} rows, one wanted", rows.len()));
            };
            let cells = row_cells(row);
            let (phase, name) = (cells[0].as_str(), cells[1].as_str());
            if !known(name) {
                return Some(format!(
                    "{f}: « {name} » is neither core nor a plugin crate"
                ));
            }
            phased(f, phase, name, row)
        })
        .collect()
}

/// rfc-rust-plugins-ui section 3's port map: from its heading to the section's end.
fn port_map(doc: &str) -> &str {
    doc.split(r#"<h3 id="port-map">"#)
        .nth(1)
        .and_then(|rest| rest.split("</section>").next())
        .expect("section 3's port map, <h3 id=\"port-map\">")
}

/// Done-when 1 of rust-port-map (@nha, 2026-11-13: « small core, everything is a plugin »): each module of crates/dash/src and
/// each file of crates/surface/src names core or one plugin crate, and its phase, in rfc-rust-plugins-ui section 3's port map,
/// so a module or page the map misses fails here. Surface's components and design/ files are named by their folder's row.
#[dash_test(rule!())]
fn every_binary_module_names_its_crate_in_the_port_map() {
    let path = rust_root().join("../private/docs/rfc-rust-plugins-ui.html");
    if !path.is_file() {
        println!(
            "no {} beside rust/: the mirror, which holds rust/ alone",
            path.display()
        );
        return;
    }
    let doc = std::fs::read_to_string(&path).expect("rfc-rust-plugins-ui.html");
    let map = port_map(&doc);
    let (binary, surface) = map
        .split_once("<summary>Surface's pages")
        .expect("the surface table after the binary's");
    let surface = surface.split("<summary>Graal's").next().unwrap_or(surface);
    let (_, modules) = dash_gate::binary::count(&rust_root());
    let mut misses = port_map_misses(binary, &modules);
    let src = rust_root().join("crates/surface/src");
    let pages: BTreeSet<String> = rust_files(&src)
        .iter()
        .map(|p| {
            p.strip_prefix(&src)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .map(|f| match f {
            _ if f.starts_with("components/each/") => "components/each.rs".to_string(),
            _ if f.starts_with("design/") => "design/".to_string(),
            f => f,
        })
        .collect();
    misses.extend(port_map_misses(surface, &pages));
    assert!(
        misses.is_empty(),
        "give each one row in rfc-rust-plugins-ui#port-map:\n{}",
        misses.join("\n")
    );
    let mut planted = modules.clone();
    planted.insert("feature.rs".into());
    assert_eq!(
        port_map_misses(binary, &planted),
        ["feature.rs: in 0 rows, one wanted"]
    );
    let one = |f: &str| BTreeSet::from([f.to_string()]);
    let elsewhere = "<tr><td><b>9</b></td><td><b>dash</b></td><td><code>a.rs</code> 1</td></tr>";
    assert_eq!(
        port_map_misses(elsewhere, &one("a.rs")),
        ["a.rs: « dash » is neither core nor a plugin crate"]
    );
    let cited = "<tr><td>1</td><td><b>core</b></td><td>helpers</td><td>calls <code>a.rs</code></td></tr>\
                 <tr><td>2</td><td><b>orch</b></td><td><code>a.rs</code> 1</td><td>?issue=rust-move-orch</td></tr>";
    assert_eq!(
        port_map_misses(cited, &one("a.rs")),
        Vec::<String>::new(),
        "a citation outside the files cell names no crate"
    );
    let unphased = "<tr><td></td><td><b>orch</b></td><td><code>a.rs</code> 1</td></tr>\
                    <tr><td>stays</td><td><b>orch</b></td><td><code>b.rs</code> 1</td></tr>";
    assert_eq!(
        port_map_misses(
            unphased,
            &BTreeSet::from(["a.rs".to_string(), "b.rs".to_string()])
        ),
        [
            "a.rs: phase «  » is neither « stays » (core) nor a move's number with its task",
            "b.rs: phase « stays » is neither « stays » (core) nor a move's number with its task",
        ],
        "every file names its phase: a move's number with its task, or core's « stays »"
    );
    for planned in PLANNED_PLUGINS {
        assert!(
            !PLUGINS.contains(&planned),
            "{planned} joined PLUGINS: drop it from PLANNED_PLUGINS"
        );
        assert!(
            binary.contains(&format!("<td><b>{planned}</b></td>")),
            "{planned}: no move in the port map names it"
        );
    }
}

/// Task rust-port-map's title (« Every dash and graal module has a target crate and a phase »): each .clj of graal's src/ at
/// [`GRAAL_PIN`], by its path below its part's hello_world/, names its crate and its phase in the port map's graal table.
/// Graal is beside this repo, never on the mirror or CI: there the test says so and passes; the Mac runs it.
#[dash_test(rule!())]
fn every_graal_file_names_its_crate_and_phase_in_the_port_map() {
    let doc = rust_root().join("../private/docs/rfc-rust-plugins-ui.html");
    let Some(home) = std::env::var_os("HOME") else {
        println!("no HOME: no ~/repo/graal to read");
        return;
    };
    let graal = Path::new(&home).join("repo/graal");
    let listed = std::process::Command::new("git")
        .args([
            "-C",
            &graal.to_string_lossy(),
            "ls-tree",
            "-r",
            "--name-only",
            GRAAL_PIN,
            "--",
            "src",
        ])
        .output()
        .ok()
        .filter(|out| out.status.success());
    let (true, Some(listed)) = (doc.is_file(), listed) else {
        println!(
            "no {} or no graal at {GRAAL_PIN}: the mirror or CI, which hold neither",
            doc.display()
        );
        return;
    };
    let doc = std::fs::read_to_string(&doc).expect("rfc-rust-plugins-ui.html");
    let table = port_map(&doc)
        .split("<summary>Graal's")
        .nth(1)
        .and_then(|rest| rest.split("</table>").next())
        .expect("the port map's graal table, <summary>Graal's");
    assert!(
        doc.contains(&format!("graal at {GRAAL_PIN}")),
        "the doc counts graal at {GRAAL_PIN}"
    );
    let paths: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|p| p.ends_with(".clj"))
        .map(|p| {
            p.split_once("/hello_world/")
                .map_or(format!("outside hello_world/: {p}"), |(_, f)| f.to_string())
        })
        .collect();
    let files: BTreeSet<String> = paths.iter().cloned().collect();
    assert!(files.len() > 200, "graal's files, {} found", files.len());
    assert_eq!(
        files.len(),
        paths.len(),
        "two of graal's parts share a path below hello_world/: key them by part"
    );
    let misses = graal_misses(table, &files);
    assert!(
        misses.is_empty(),
        "give each graal file one row in rfc-rust-plugins-ui#port-map's graal table:\n{}",
        misses.join("\n")
    );
}

/// [`graal_misses`]' rules on planted rows, run everywhere, graal or not.
#[dash_test(rule!())]
fn a_graal_row_with_a_bad_crate_phase_or_task_fails() {
    let row = |phase: &str, name: &str, file: &str, task: &str| {
        format!(
            r#"<tr><td>{phase}</td><td><b>{name}</b></td><td><code>{file}</code> 1</td><td><a href="?issue={task}">t</a></td></tr>"#
        )
    };
    let table = [
        row("2", "index", "a.clj", "rust-graal-index"),
        row("G3", "gate", "b.clj", "rust-graal-ml"),
        row("G9", "ml", "c.clj", "rust-graal-nine"),
        row("G1", "ingest", "d.clj", "rust-graal-ingest"),
        row("G1", "pdf", "e.clj", "rust-graal-pdf"),
        row("G4", "flare", "f.clj", "rust-graal-ingest"),
    ]
    .concat();
    let files = [
        "a.clj", "b.clj", "c.clj", "d.clj", "e.clj", "f.clj", "g.clj",
    ]
    .map(String::from)
    .into();
    assert_eq!(
        graal_misses(&table, &files),
        [
            "a.clj: phase « 2 » is not G1 to G8 with its task",
            "b.clj: « gate » is neither core nor a plugin crate",
            "c.clj: phase « G9 » is not G1 to G8 with its task",
            "g.clj: in 0 rows, one wanted",
            "G1: 2 tasks, one wanted",
            "rust-graal-ingest: the task of G1 and G4, one phase wanted",
        ]
    );
    for name in GRAAL_CRATES {
        assert!(
            ["flare", "sources", "meetings"].contains(&name)
                || name == "core"
                || [&RINGS[..], &LEAVES, &TOOLING, &PLUGINS]
                    .iter()
                    .any(|l| l.contains(&name)),
            "{name}: neither a crate of ours nor one of graal's three plugins"
        );
    }
}

/// Each open question and open unknown of a page, as its line (a page writes each row and caption on one), that does not say its
/// default is his to change.
fn defaults_unmarked(doc: &str) -> Vec<&str> {
    doc.lines()
        .filter(|l| {
            l.contains(r#"class="chip"#)
                && (l.contains(">yours</span>") || l.contains(">open</span>"))
        })
        .filter(|l| !l.contains("yours to change"))
        .collect()
}

/// Done-when 4 of rust-port-map: the two plugin RFCs' open questions (a « yours » block, an open unknown) each carry the default
/// the build follows, marked his to change.
#[dash_test(rule!())]
fn the_plugin_rfcs_open_questions_carry_defaults_his_to_change() {
    let docs = rust_root().join("../private/docs");
    if !docs.is_dir() {
        println!(
            "no {} beside rust/: the mirror, which holds rust/ alone",
            docs.display()
        );
        return;
    }
    for page in ["rfc-rust-plugins-ui.html", "rfc-superapp-plugins.html"] {
        let doc = std::fs::read_to_string(docs.join(page)).expect(page);
        let unmarked = defaults_unmarked(&doc);
        assert!(
            unmarked.is_empty(),
            "{page}: give each its default, « Default until you answer: A; yours to change. » or « Default meanwhile, yours \
             to change: »:\n{}",
            unmarked.join("\n")
        );
    }
    let planted = r#"<tr id="p-x"><td>q</td><td><span class="chip warn small">open</span></td><td>a test</td></tr>"#;
    assert_eq!(defaults_unmarked(planted), [planted]);
}

/// Task rust-browser-tests-fewer (@nha, 2026-11-11: « small tests > big slow tests. pure rust in-process > webdriver > chrome >
/// pupeteer/playwright »): the tests nextest's browser group takes, held in rust/browser-tests-held.txt, only go down; each
/// says why it needs a browser. One added fails here until the number is raised, which `dash scripts count --check --base`
/// refuses (ratchet.rs).
#[dash_test(rule!())]
fn browser_tests_only_go_down() {
    let (tests, bad) = browser_tests(&rust_root());
    assert!(
        bad.is_empty(),
        "each browser test found, a « {BROWSER_MARK} <why> » comment above it:\n{}",
        bad.join("\n")
    );
    // compiled in, never read at run time: every_included_file_rides_the_mirror checks it rides the mirror
    let held = include_str!("../../../browser-tests-held.txt")
        .trim()
        .parse()
        .ok();
    let each: Vec<String> = tests
        .iter()
        .map(|t| format!("{} {}", t.at, t.name))
        .collect();
    if let Err(why) = browser_verdict(tests.len(), held) {
        panic!("{why}\n{}", each.join("\n"));
    }
    // one more Chrome fails: the count one past what is held
    let one_more = browser_verdict(tests.len() + 1, held).expect_err("one added fails");
    assert!(one_more.contains("never one more Chrome"), "{one_more}");
}

#[dash_test(rule!())]
fn a_planted_browser_test_is_counted_and_one_added_fails() {
    let dir = TestDir::new("browser-tests");
    let toml = r#"[test-groups]
browser = { max-threads = 5 }

[[profile.default.overrides]]
filter = "(binary_id(dash-a::web) & not test(=slots)) | test(=prints)"
test-group = "browser"

[[profile.default.overrides]]
filter = "test(=other)"
success-output = "immediate"
"#;
    write(&dir.0, ".config/nextest.toml", toml);
    write(
        &dir.0,
        "crates/a/Cargo.toml",
        "[package]\nname = \"dash-a\"\n",
    );
    write(
        &dir.0,
        "crates/b/Cargo.toml",
        "[package]\nname = \"dash-b\"\n",
    );
    // `@` for each attribute's `#`: every_rule_opens_with_its_guard reads this file's lines
    // each form of the attribute (task rust-test-budget-attr), the bare one too
    let web = "/// MARK a layout.\n@[dash_test]\nfn fits() {}\n\n/// Words.\n// MARK a click.\n@[dash_test(TestBudget::of(100.0))]\n@[ignore]\npub fn clicks() {}\n\n@[test]\nfn slots() {}\n\n/// Only words.\n@[dash_meter::dash_test]\nfn bare() {}\n"
        .replace("MARK", BROWSER_MARK)
        .replace('@', "#");
    write(&dir.0, "crates/a/tests/web.rs", &web);
    // an attribute rustfmt wrapped over lines between the test's and its fn (the front door's #[wall_budget], task
    // rust-store-rebuild-write-flake): it read « no fn after it » and dropped the file's every test
    // and above it, between the reason and the test's attribute; a bracket in its string counts none (/code-review)
    let pdf = "// MARK Chrome prints it.\n@[dash_meter::wall_budget(\n    \"[0, 300) ms\"\n)]\n@[test]\n@[dash_meter::wall_budget(\n    \"a [bound\"\n)]\nfn prints() {}\n@[test]\nfn other() {}\n"
        .replace("MARK", BROWSER_MARK)
        .replace('@', "#");
    write(&dir.0, "crates/b/tests/pdf.rs", &pdf);
    let (tests, bad) = browser_tests(&dir.0);
    let named: Vec<(&str, bool)> = tests.iter().map(|t| (t.name.as_str(), t.why)).collect();
    assert_eq!(
        named,
        [
            ("fits", true),
            ("clicks", true),
            ("bare", false),
            ("prints", true)
        ],
        "the file's tests less the one left out, plus the one added"
    );
    assert_eq!(
        bad,
        [format!(
            "crates/a/tests/web.rs:16 bare: no « {BROWSER_MARK} <why> » comment above it"
        )]
    );
    assert_eq!(
        browser_verdict(4, Some(4)),
        Ok("4 tests in nextest's browser group, 4 held".into())
    );
    // one added to the file: five against the four held
    let more =
        format!("{web}// {BROWSER_MARK} one more.\n@[test]\nfn more() {{}}\n").replace('@', "#");
    write(&dir.0, "crates/a/tests/web.rs", &more);
    let (tests, _) = browser_tests(&dir.0);
    let said = browser_verdict(tests.len(), Some(4)).expect_err("one added");
    assert!(
        said.starts_with("5 tests in nextest's browser group, 4 held"),
        "{said}"
    );
    assert!(
        browser_verdict(3, Some(4))
            .unwrap_err()
            .contains("lower it to 3")
    );
    assert!(
        browser_verdict(4, None)
            .unwrap_err()
            .contains("no count, write 4")
    );
    // a shape the gate cannot count is refused, never guessed: another term, a `not` joined by `|`, any other `&`, a
    // second override (/code-review)
    for (from, to) in [
        ("test(=prints)", "test(/^print/)"),
        (
            "(binary_id(dash-a::web) & not test(=slots))",
            "binary_id(dash-a::web) | not test(=slots)",
        ),
        ("| test(=prints)", "& test(=prints)"),
    ] {
        let said = browser_group(&toml.replace(from, to)).unwrap_err();
        assert!(
            said.contains("a shape the gate does not count"),
            "{to}: {said}"
        );
    }
    let twice = format!(
        "{toml}\n[[profile.default.overrides]]\nfilter = \"test(=more)\"\ntest-group = \"browser\"\n"
    );
    assert!(
        browser_group(&twice)
            .unwrap_err()
            .contains("2 overrides take test-group"),
        "one override"
    );
    // a module in a file taken whole, whose tests nextest names by its path, and a test attribute sharing its line
    write(
        &dir.0,
        "crates/a/tests/web.rs",
        &format!("mod helpers;\n{more}"),
    );
    let (_, bad) = browser_tests(&dir.0);
    assert_eq!(
        bad.first().map(String::as_str),
        Some(
            "crates/a/tests/web.rs:1: a module; a browser test file keeps its tests at its top level"
        ),
        "{bad:?}"
    );
    write(
        &dir.0,
        "crates/a/tests/web.rs",
        &format!("{more}@[test] fn inline() {{}}\n").replace('@', "#"),
    );
    let (_, bad) = browser_tests(&dir.0);
    assert!(
        bad.iter()
            .any(|b| b.contains("a test attribute shares its line")),
        "{bad:?}"
    );
    write(
        &dir.0,
        ".config/nextest.toml",
        &toml.replace("test(=prints)", "test(=gone)"),
    );
    let (_, bad) = browser_tests(&dir.0);
    assert!(
        bad.iter().any(|b| b.contains("test(=gone): no such test")),
        "{bad:?}"
    );
    // a unit test in a crate's src/ is named by its module path: the bare name matched no test on main, so a Chrome test ran
    // outside the group (PR 1487); the declared `mod b;` above names nothing, the inline `mod tests` does
    write(&dir.0, "crates/a/tests/web.rs", &web);
    // the inline modules a test sits in, each closed at its brace (/code-review of PR 1488): a declared `mod b;` and a closed
    // `mod helpers` name nothing, `pub(crate) mod tests` and its nested `mod chrome` do, in src/ and in tests/ alike
    let unit = "mod b;\nmod helpers { fn h() {} }\npub(crate) mod tests {\n    mod chrome {\n        // MARK a form.\n        \
                @[test]\n        fn fills() {}\n    }\n    // MARK a top.\n    @[test]\n    fn top() {}\n}\n"
        .replace("MARK", BROWSER_MARK)
        .replace('@', "#");
    write(&dir.0, "crates/b/src/apply.rs", &unit);
    let ui = "mod chrome {\n    // MARK opens.\n    @[test]\n    fn opens() {}\n}\n"
        .replace("MARK", BROWSER_MARK)
        .replace('@', "#");
    write(&dir.0, "crates/b/tests/ui.rs", &ui);
    // a file that holds no named test fails nothing, however it reads
    write(
        &dir.0,
        "crates/a/src/lib.rs",
        &"@[test] fn inline() {}\n".replace('@', "#"),
    );
    write(
        &dir.0,
        ".config/nextest.toml",
        &toml.replace("test(=prints)", "test(=fills)"),
    );
    let (_, bad) = browser_tests(&dir.0);
    assert!(
        bad.iter()
            .any(|b| b.contains("test(=fills): no such test") && b.contains("module path")),
        "{bad:?}"
    );
    write(
        &dir.0,
        ".config/nextest.toml",
        &toml.replace(
            "test(=prints)",
            "test(=apply::tests::chrome::fills) | test(=apply::tests::top) | test(=chrome::opens)",
        ),
    );
    let (tests, bad) = browser_tests(&dir.0);
    let added: Vec<(&str, &str, bool)> = tests
        .iter()
        .filter(|t| t.name.contains("::"))
        .map(|t| (t.name.as_str(), t.at.as_str(), t.why))
        .collect();
    assert_eq!(
        added,
        [
            (
                "apply::tests::chrome::fills",
                "crates/b/src/apply.rs:7",
                true
            ),
            ("apply::tests::top", "crates/b/src/apply.rs:11", true),
            ("chrome::opens", "crates/b/tests/ui.rs:4", true),
        ],
        "{tests:?}"
    );
    assert!(
        bad.iter().all(|b| b.contains(" bare: ")),
        "only web.rs's bare test: {bad:?}"
    );
}

#[dash_test(rule!())]
fn a_planted_script_is_counted_and_a_bare_one_refused() {
    let dir = TestDir::new("inline-js");
    // `^` for a tag's opening, `@` for an attribute's and INCLUDE for include_str: the gate's own source holds none
    // (html_in_strings, every_rule_opens_with_its_guard and every_included_file_rides_the_mirror read this file)
    let lib = r##"/// MARK a Dioxus gap (https://example.org/1).
const A_JS: &str = "abcd";
/// MARK none.
const B_JS: &str = INCLUDE!("b.js");
const C_JS: &str = r#"xy"#;
/// see https://example.org/2
/// MARK none.
static D_JS: &str = "a\
    b\n";
/// MARK wrapped across
/// two lines (https://example.org/3).
const E_JS: &str = "e";
trait T { const X_JS: &str; }
const OTHER: &str = "zzz";
@[cfg(test)]
mod early { const T: &str = "^script>x"; }
fn page() { rsx! { div { script { dangerous_inner_html: A_JS } script { dangerous_inner_html: js } "Loading" script { dangerous_inner_html: "fetch('/x')" } script { dangerous_inner_html: GHOST_JS } } } }
const PAGE: &str = "^script>{B_JS}^/script>^script>alert(1)^/script>^SCRIPT>{E_JS}^/SCRIPT>^script src=\"{LIVE_CLIENT}\">^/script>^script src=\"{OTHER}\">^/script>";
mod tests;
"##
    .replace("MARK", MARK)
    .replace("INCLUDE", "include_str")
    .replace('^', "<")
    .replace('@', "#");
    write(&dir.0, "crates/a/src/lib.rs", &lib);
    write(&dir.0, "crates/a/src/b.js", "123456");
    write(&dir.0, "crates/a/src/c.js", "never included");
    write(&dir.0, "crates/a/src/e.mjs", "never included");
    write(&dir.0, "crates/a/tests/t.rs", "const T_JS: &str = \"x\";");
    write(&dir.0, "crates/a/tests/t.js", "a test's own page");
    let (scripts, bad) = inline_scripts(&dir.0);
    let counted: Vec<(&str, usize)> = scripts.iter().map(|s| (s.name.as_str(), s.bytes)).collect();
    assert_eq!(
        counted,
        [
            ("A_JS", 4),
            ("B_JS", 6),
            ("C_JS", 2),
            ("D_JS", 3),
            ("E_JS", 1)
        ],
        "each script by the bytes Rust builds, a static too, a valueless trait const and tests/ skipped"
    );
    let said = bad.join("\n");
    for refusal in [
        "B_JS: the comment above it names what blocks its port",
        "C_JS: the comment above it names what blocks its port",
        "D_JS: the comment above it names what blocks its port",
        "a script element puts on \"js\"",
        "a script element puts on \"\"",
        "GHOST_JS is put on a page but is no counted NAME_JS",
        "a string writes ^script>alert(1)",
        "a string writes ^script src=\\\"{OTHER}",
        "crates/a/src/c.js: a script no NAME_JS const includes",
        "crates/a/src/e.mjs: a script no NAME_JS const includes",
    ] {
        assert!(
            said.contains(&refusal.replace('^', "<")),
            "{refusal} in:\n{said}"
        );
    }
    assert_eq!(
        bad.len(),
        10,
        "nothing else refused, the test module's string, the upper-case tag and Dioxus's client included: {said}"
    );
    assert!(
        inline_js_verdict(13, Some(12))
            .unwrap_err()
            .contains("13 inline script bytes in rust/crates, 12 held")
    );
    assert!(
        inline_js_verdict(11, Some(12))
            .unwrap_err()
            .contains("lower it to 11")
    );
    assert!(
        inline_js_verdict(12, None)
            .unwrap_err()
            .contains("no count, write 12")
    );
    assert_eq!(
        inline_js_verdict(12, Some(12)),
        Ok("12 inline script bytes in rust/crates, 12 held".into())
    );
}

/// Task outbound-gate, done-when 3: every tool that sends words to a person runs `dash hook outbound-gate` before the call, and
/// that hook fails closed: a dash that is missing, or that exits other than 0 (one older than the gate, a crash), is exit 2,
/// Claude Code's refusal. Was scripts_test.exs's check of outbound_gate.py's `|| exit 2`. The mirror holds rust/ alone.
#[dash_test(rule!())]
fn every_send_tool_runs_the_outbound_gate_and_it_fails_closed() {
    let file = rust_root().join("../.claude/settings.json");
    if !rust_root().join("../private/dashboard").is_dir() {
        println!("no private/dashboard beside rust/: the mirror, which holds rust/ alone");
        return;
    }
    let text = std::fs::read_to_string(&file).expect(".claude/settings.json");
    let settings: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let pre = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("PreToolUse");
    let commands_for = |tool: &str| -> Vec<String> {
        (pre.iter())
            .filter(|e| {
                let matcher = e["matcher"].as_str().unwrap_or_default();
                regex::Regex::new(matcher)
                    .expect("a matcher")
                    .is_match(tool)
            })
            .flat_map(|e| e["hooks"].as_array().cloned().unwrap_or_default())
            .filter_map(|h| h["command"].as_str().map(String::from))
            .collect()
    };
    for tool in [
        "mcp__mcp-server-linkedin__send_message",
        "mcp__mcp-server-linkedin__reply_in_thread",
        "mcp__mcp-server-linkedin__message_job_poster",
        "mcp__mcp-server-linkedin__connect_with_person",
        "mcp__mcp-server-linkedin__easy_apply",
        "mcp__claude-in-chrome__form_input",
        "mcp__claude-in-chrome__computer",
        "mcp__claude-in-chrome__browser_batch",
        "mcp__claude-in-chrome__javascript_tool",
        "mcp__claude_ai_Gmail__send_message",
        "mcp__google-workspace__gmail_send_message",
        "mcp__GMAIL__send",
    ] {
        assert!(
            commands_for(tool)
                .iter()
                .any(|c| c.contains("hook outbound-gate")),
            "{tool} skips the outbound gate"
        );
    }
    assert!(!text.contains("outbound_gate.py"), "the .py is gone");
    // the open drafts' nightly check, where `mix drafts.check` ran: the frozen Elixir test that named the step is skipped
    // (ported-tests.txt), so this one holds it (/code-review of PR 1490)
    let nightly = std::fs::read_to_string(rust_root().join("../.github/workflows/private.yml"));
    assert!(
        nightly
            .expect("private.yml")
            .contains("run: private/dashboard/bin/dash outbound drafts"),
        "private.yml no longer checks the open drafts"
    );
    let gate = commands_for("mcp__mcp-server-linkedin__send_message")
        .into_iter()
        .find(|c| c.contains("hook outbound-gate"))
        .expect("the gate's command");
    // run as Claude Code runs it, under a HOME holding no dash, then one whose dash exits 1 as an older dash does, then 0
    let home = TestDir::new("outbound-hook-home");
    let live = home.0.join("repo/site_perso/private/live/rust");
    let run = || {
        std::process::Command::new("sh")
            .args(["-c", &gate])
            .env("HOME", &home.0)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("sh")
    };
    let out = run();
    assert_eq!(out.status.code(), Some(2), "no live dash is a refusal");
    std::fs::create_dir_all(&live).expect("its folder");
    // a link to a binary macOS already knows, never a new script it would check for seconds
    std::os::unix::fs::symlink("/usr/bin/false", live.join("dash")).expect("a dash that fails");
    assert_eq!(run().status.code(), Some(2), "a failing dash is a refusal");
    std::fs::remove_file(live.join("dash")).expect("remove");
    std::os::unix::fs::symlink("/usr/bin/true", live.join("dash")).expect("a dash that passes");
    assert_eq!(
        run().status.code(),
        Some(0),
        "a dash that judged and said nothing lets the call go"
    );
}
