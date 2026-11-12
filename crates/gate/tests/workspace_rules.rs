//! Section 3's gates on the workspace itself: the dependency graph, the crate list and the line count. The 120 s of
//! section 17 is .config/nextest.toml's global-timeout and CI's step clock.
use cargo_metadata::{DependencyKind, MetadataCommand};
use dash_gate::graph::{
    Lexeme, agents_named, crate_graph, layer, lexed, module_cycles, module_graph, module_holes,
};
use dash_gate::html::{
    BARE_REFUSED, BOARD, COMPONENTS, EDITABLE, SHOWCASE, board_markup, editable_outside,
    html_in_strings, html_tags, hub_named_pages, page_local_markup, registrations, reload_promised,
    shared_component_lists,
};
use dash_gate::inline_js::{MARK, inline_scripts, verdict as inline_js_verdict};
use dash_gate::{
    AGENT_KINDS, BINARY, BROWSER_LAUNCH, DENO_RULES_EXCLUDE, JOB_WORDS, LEAVES, NEVER_IN_THE_CORE,
    OLD_CRM, PLUGINS, REACHES_PAST_RUST, RINGS, TASK_LAYERS, TOOLING, apt_in_workflow, bad_edges,
    binary_fixtures, browsers_without_slot, deno_lint, dependency_loop, includes_off_the_mirror,
    inline_runs, is_tooling_edge, old_crm_named, run_time_paths_past_rust, rust_files,
    rustfmt_check, scripts, tools_named, tracked, words_named,
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
        "an edge section 3 forbids: depend on the core and take the other crate through its port, or add the edge to \
         section 3 and NAMED_EDGES (gate/src/lib.rs) first"
    );
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
    assert_eq!(
        tests_out_of_shape(source),
        Vec::<String>::new(),
        "reshape each listed test: #[test] alone on its line, a fn returning nothing, then `let _budget = rule!();`, \
         no other attribute"
    );
    assert!(source.lines().filter(|l| l.trim() == TEST).count() > 10);
    let good = format!("{TEST}\nfn r() {{\n{GUARD}\n}}");
    assert_eq!(
        tests_out_of_shape(&good),
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
    );
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

/// Done-when 1 of rust-gate-says-fix: every gate test that refuses on a found list says what to do (e6d2f1b61 and ba36501d2
/// each cost a task a round, their gate naming only the offence).
#[test]
fn every_list_a_gate_refuses_on_says_the_fix() {
    let _budget = rule!();
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
        assert!(
            bad_edges(&[edge(good.0, good.1)]).is_empty(),
            "an allowed edge is refused, fix bad_edges: {good:?}"
        );
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
    // The message says what to do, not only where: design-stage-notion-toggle's test in families.rs called its parent's
    // page and spent a fix round on the loop it made (lesson, 2026-11-10). Tests count: their uses are read like the rest.
    let cycles = module_cycles(&rust_root());
    assert!(
        cycles.is_empty(),
        "a module loop, each path leading back to its start: move what both sides use into a module below them; a test that \
         needs its parent's item goes in the parent's tests or the crate's tests/ folder. Found: {cycles:?}"
    );
    assert_eq!(
        module_holes(&rust_root()),
        Vec::<String>::new(),
        "code the module walk cannot read: give each module its own file (no #[path], no include!), declare it \
         with `mod` from its parent, one crate root"
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

#[test]
fn the_core_names_no_tool_or_provider() {
    let _budget = rule!();
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

#[test]
fn the_dash_crate_holds_no_html() {
    let _budget = rule!();
    // The message says what to do, not only where: rust-one-doc-component spent turns finding it (lesson, 2026-11-10).
    let found = html_in_strings(&rust_root());
    assert!(
        found.is_empty(),
        "HTML is dash_surface's to draw: build the page from its components, and in a dash test assert on the text form \
         (Accept: text/plain) or the surface tree, never a tag literal. Found: {found:?}"
    );
}

/// Task rust-task-page-edits, done-when 2 (@nha, 2026-11-11: « I can edit the description but it is lost after a refresh »):
/// the design system's editable field saves or is read only. Text is editable only in the editor's field, inside the form
/// that posts it, and on the Notion tab's specimen: a page that makes its own text editable is refused.
#[test]
fn text_is_editable_only_in_the_editor_that_saves_it() {
    let _budget = rule!();
    let found = editable_outside(&rust_root());
    assert!(
        found.is_empty(),
        "a page's words are edited in the design system's Composer, whose Save posts them (`words` opens it on the saved \
         text; a task's description posts to /tasks/<id>/set); anything else is DocView, read only. Allowed: {EDITABLE:?}. \
         Found: {found:?}"
    );
}

/// The rule above on a planted tree: an editable element a page draws is named, a comment, a string or a test is not.
#[test]
fn a_planted_editable_field_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn a_tag_with_attributes_fails_the_gate() {
    let _budget = rule!();
    let found = gate_on("gate-attrs", "<svg width=\"10\">");
    assert!(found.iter().any(|f| f.ends_with(": <svg")), "{found:?}");
}

/// Done-when 1 of rust-gate-placeholders: a closing tag is HTML, the bare opening beside it a placeholder.
#[test]
fn a_closing_tag_fails_the_gate() {
    let _budget = rule!();
    let found = gate_on("gate-closing", "<p>text</p>");
    assert!(found.iter().any(|f| f.ends_with(": </p")), "{found:?}");
}

/// Done-when 1 of rust-gate-placeholders: a self-closing tag is HTML, with or without a space.
#[test]
fn a_self_closing_tag_fails_the_gate() {
    let _budget = rule!();
    let found = gate_on("gate-self-closing", "line<br/>next<hr />");
    assert!(found.iter().any(|f| f.ends_with(": <br")), "{found:?}");
    assert!(found.iter().any(|f| f.ends_with(": <hr")), "{found:?}");
}

/// Done-when 1 of rust-gate-placeholders: a bare `<path>` in usage text is a placeholder (three tasks lost a commit to it,
/// 2026-11-11).
#[test]
fn a_bare_path_placeholder_passes_the_gate() {
    let _budget = rule!();
    let found = gate_on("gate-path", "usage: dash store text <path> [--json]");
    assert_eq!(
        found,
        Vec::<String>::new(),
        "a bare <path> read as an SVG tag"
    );
}

/// Done-when 1 of rust-gate-placeholders: a bare `<id>` in usage text is a placeholder.
#[test]
fn a_bare_id_placeholder_passes_the_gate() {
    let _budget = rule!();
    let found = gate_on("gate-id", "usage: dash task show <id>");
    assert_eq!(found, Vec::<String>::new(), "a bare <id> read as a tag");
}

/// Done-when 2 of rust-gate-placeholders: a real `<path d=…>` and a `<button>`, bare too (the orchestrator's answer: « Bare
/// <button> refused »), are still refused; BARE_REFUSED holds every tag a component registers.
#[test]
fn a_real_path_or_button_still_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn no_page_draws_what_a_component_covers() {
    let _budget = rule!();
    assert_eq!(
        page_local_markup(&rust_root()),
        Vec::<String>::new(),
        "a page draws what a component covers: use the component from crates/surface/src/components.rs, \
         adding it there first (HTML and text forms) when it is missing"
    );
}

/// The rule above on a planted tree: a page's own button or card fails it, the component's file, the showcase and plain
/// Rust that merely names a field `input` pass.
#[test]
fn a_planted_button_or_card_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn each_component_registers_from_its_own_module() {
    let _budget = rule!();
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

/// Done-when 2 of rust-components-register: no list outside a component's module names a component (@nha, 2026-11-10:
/// « this then finally unblocks adding parallelism and unblocks velocity development »).
#[test]
fn no_shared_list_names_the_components() {
    let _budget = rule!();
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
#[test]
fn a_shared_list_of_components_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn two_branches_each_adding_a_component_merge_clean() {
    let _budget = rule!();
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
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
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
#[test]
fn the_board_draws_only_components() {
    let _budget = rule!();
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

/// `mix lint`'s browser slot rule, held on rust/ by the gate (task brief-tests-prepush-skip): a commit touching only rust/
/// skips the app's lint.
#[test]
fn every_rust_file_that_starts_a_browser_takes_a_slot() {
    let _budget = rule!();
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
#[test]
fn a_planted_browser_launch_without_a_slot_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn no_rust_file_names_the_old_crm() {
    let _budget = rule!();
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
    assert_eq!(
        scripts(&rust_root(), &files),
        Vec::<String>::new(),
        "new logic is Rust: port the script to a dash command with its test, then delete the script"
    );
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

#[test]
fn no_binary_fixture_is_tracked_under_crates() {
    let _budget = rule!();
    let files = tracked(&rust_root());
    assert_eq!(
        binary_fixtures(&rust_root(), &files),
        Vec::<String>::new(),
        "a binary test file is fetched, never tracked: list `<sha256> <name> <url>` in its crate's fixtures.txt, read it \
         with dash_meter::fixtures::fixture, then git rm it"
    );
}

#[test]
fn a_planted_binary_fixture_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn ci_steps_are_one_line() {
    let _budget = rule!();
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
#[test]
fn apt_runs_after_held_debs() {
    let _budget = rule!();
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
#[test]
fn no_group_is_ended_by_a_shelled_kill() {
    let _budget = rule!();
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
#[test]
fn a_shelled_group_kill_is_found_in_each_form() {
    let _budget = rule!();
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
#[test]
fn the_active_workflows_run_on_the_vps_and_build_nothing_there() {
    let _budget = rule!();
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

/// The script ratchet (dash_gate::ratchet, task rust-scripts-port) runs at both doors: the pre-commit against HEAD's number,
/// the PR gate against main's side of the merge ref, so a number raised to let a script in fails at either.
#[test]
fn the_precommit_and_the_pr_gate_run_the_script_count() {
    let _budget = rule!();
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
            && step.contains(r#"crate::ratchet::run(&ctx.top, true, Some("HEAD"))"#),
        "the pre-commit no longer runs dash scripts count --check --base HEAD: keep the scripts-count step in \
         dash_gate::hook::pre_commit::run, and .githooks/pre-commit calling dash hook pre-commit"
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
    assert_eq!(
        deno_lint(&rust_root(), &files),
        Vec::<String>::new(),
        "fix each line deno names; when deno did not run, install the pinned one with bin/ci/lint-tools.sh"
    );
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
        Vec::<String>::new(),
        "a clean plant trips the rule: narrow its matcher so it refuses only the planted offence"
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
#[test]
fn no_page_promises_the_next_reload() {
    let _budget = rule!();
    assert_eq!(
        reload_promised(&rust_root()),
        Vec::<String>::new(),
        "a page never promises the next reload: show the state it holds now, or have it refresh itself"
    );
}

/// The rule above on a planted tree: the phrase in any case or over a continued line fails it; a comment, a test's own
/// words and the gate's list pass.
#[test]
fn a_planted_reload_promise_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn a_new_page_edits_no_hub() {
    let _budget = rule!();
    assert_eq!(
        hub_named_pages(&rust_root()),
        Vec::<String>::new(),
        "register the page's route in its own module (crates/dash/src/pages.rs), never in a hub"
    );
}

/// The rule above on a planted tree: a page module registering its paths passes, and so do a hub's tests naming them; a
/// route for it in main.rs, by its path or its prefix, in tasks.rs, or registered in main.rs, fails it. The registrations
/// here spell `step` before `path`: a page is read whatever its field order, or a hub naming it would pass.
#[test]
fn a_planted_hub_route_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn every_crate_sits_in_a_layer_of_section_3() {
    let _budget = rule!();
    let (crates, _) = crate_graph(&rust_root());
    let unplaced: Vec<&String> = crates.iter().filter(|c| layer(c) == "Other").collect();
    assert!(
        unplaced.is_empty(),
        "{unplaced:?}: in no layer of section 3, name each in dash-gate's lists"
    );
}

/// Task design-cards-not-default: a page lists things as a `list` (rows) or a `table`; a `cards` container only where a card
/// was approved (@nha, 2026-11-09: « Cards are not the default UI element », « I am tempted to ban them as a UI element
/// unless explicitly approved »). The design system itself (crates/surface) may draw and show one.
#[test]
fn cards_only_where_approved() {
    let _budget = rule!();
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
#[test]
fn no_elixir_lets_a_branch_take_its_edit_back_out() {
    let _budget = rule!();
    use dash_gate::hook::{Ctx, no_elixir};
    // SAFETY: before any thread. Run by the pre-commit, its GIT_INDEX_FILE would aim the step's git at the real index, and
    // his global config (signing, fsmonitor) stays out of the fixture's commits (/code-review).
    unsafe {
        std::env::remove_var("GIT_INDEX_FILE");
        std::env::remove_var("GIT_DIR");
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    }
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
#[test]
fn a_gate_step_runs_a_command_mains_dash_answers() {
    let _budget = rule!();
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
#[test]
fn every_included_file_rides_the_mirror() {
    let _budget = rule!();
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
#[test]
fn every_run_time_path_past_rust_goes_through_the_monorepo_helper() {
    let _budget = rule!();
    assert_eq!(
        run_time_paths_past_rust(&rust_root(), &tracked(&rust_root())),
        Vec::<String>::new(),
        "reach the monorepo through dash_meter::monorepo(), which stops the test on the mirror"
    );
}

/// Claude Code's hooks are `dash hook` commands: .claude/hooks keeps only the two scripts frozen Elixir still runs, each
/// naming the task that retires it (task rust-hooks-rest-port, its check 4). outbound_gate.py, the third, went with task
/// outbound-gate.
#[test]
fn claude_hooks_keep_only_the_two_scripts_elixir_runs_each_naming_its_retiring_task() {
    let _budget = rule!();
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
#[test]
fn a_planted_run_time_path_past_rust_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn a_planted_include_off_the_mirror_fails_the_gate() {
    let _budget = rule!();
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
#[test]
fn inline_js_only_goes_down() {
    let _budget = rule!();
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

#[test]
fn a_planted_script_is_counted_and_a_bare_one_refused() {
    let _budget = rule!();
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
#[test]
fn every_send_tool_runs_the_outbound_gate_and_it_fails_closed() {
    let _budget = rule!();
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
