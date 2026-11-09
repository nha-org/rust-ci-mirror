//! The code map /deps draws (@nha, 2026-11-09: « Line counts vary a lot: dash 14.8k, surface 7k: that's what we need to
//! surface »): each crate's layer, what it does, its lines split into code and tests, what it uses, and its modules. Lines are
//! counted as dash-gate counts the line cap; tests and module uses come from rust-analyzer's SCIP index. `dash code map`
//! writes it once per commit (make rust/deploy runs it); a page only reads it.
use dash_gate::graph::{crate_graph, layer};
use protobuf::Message;
use scip::types::Index;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn file(data: &Path) -> PathBuf {
    data.join("codemap.json")
}

/// `dash code map --data <dir>`: indexes `<repo>/rust` (about 80 s and 3 GB), then writes the map.
pub fn run(data: &Path, repo: &Path) -> Result<String, String> {
    let (rust, scip) = (repo.join("rust"), data.join("index.scip"));
    let out = scip.to_string_lossy();
    let long = Duration::from_secs(900);
    dash_process::run(
        "rust-analyzer",
        &["scip", ".", "--output", &out],
        Some(&rust),
        long,
    )?;
    let bytes = std::fs::read(&scip).map_err(|e| format!("{out}: {e}"))?;
    let index = Index::parse_from_bytes(&bytes).map_err(|e| format!("{out}: {e}"))?;
    let sha = dash_process::run("git", &["rev-parse", "--short=9", "HEAD"], Some(repo), long)?;
    let map = map(
        &rust,
        &index,
        sha.trim(),
        &crate::comments::iso(crate::tasks::now_ms()),
    );
    std::fs::write(file(data), map.to_string())
        .map_err(|e| format!("{}: {e}", file(data).display()))?;
    let _ = std::fs::remove_file(&scip);
    Ok(format!(
        "code map of {}: {} lines\n",
        map["sha"].as_str().unwrap_or_default(),
        map["lines"]
    ))
}

/// What the index says of each file of `rust/`: the lines of its inline `mod tests`, and the modules of its crate it uses
/// outside them (a module's own items, macro-made modules and test code left out, as dash-gate's module graph reads).
fn read_index(index: &Index) -> BTreeMap<String, (usize, BTreeSet<String>)> {
    let ours = |symbol: &str| -> Option<(String, String)> {
        let mut parts = symbol.splitn(5, ' ').skip(2);
        let package = parts.next()?;
        let krate = if package == "dash" {
            "dash"
        } else {
            package.strip_prefix("dash-")?
        };
        let path: Vec<&str> = parts
            .nth(1)?
            .split_inclusive(['/', '#', '.', '(', '[', '!'])
            .collect();
        let modules = path
            .iter()
            .map_while(|s| s.strip_suffix('/'))
            .filter(|m| *m != "crate");
        Some((krate.into(), modules.collect::<Vec<_>>().join("::")))
    };
    let mut files = BTreeMap::new();
    for doc in &index.documents {
        let Some((krate, module)) = file_module(&doc.relative_path) else {
            continue;
        };
        // SCIP's range: [line, char, end line, end char], or [line, char, end char] on one line
        let span = |o: &scip::types::Occurrence| {
            let r = &o.enclosing_range;
            (r[0], if r.len() == 4 { r[2] } else { r[0] })
        };
        let tests: Vec<(i32, i32)> = (doc.occurrences.iter())
            .filter(|o| {
                o.symbol_roles & 1 == 1
                    && o.symbol.ends_with("tests/")
                    && o.enclosing_range.len() >= 3
            })
            .map(span)
            .collect();
        let in_tests = |line: i32| tests.iter().any(|(a, b)| (*a..=*b).contains(&line));
        let uses = (doc.occurrences.iter())
            .filter(|o| {
                o.symbol_roles & 1 == 0 && !o.symbol.ends_with('/') && !in_tests(o.range[0])
            })
            .filter_map(|o| ours(&o.symbol))
            .filter(|(k, m)| *k == krate && *m != module && !m.ends_with("_completions"))
            .map(|(_, m)| m)
            .collect();
        let lines = tests.iter().map(|(a, b)| (b - a + 1) as usize).sum();
        files.insert(doc.relative_path.clone(), (lines, uses));
    }
    files
}

/// `crates/store/src/a/b.rs` -> (store, a::b); lib.rs, main.rs and mod.rs name their folder's module, the crate's own as "".
fn file_module(path: &str) -> Option<(String, String)> {
    let (krate, rest) = path.strip_prefix("crates/")?.split_once('/')?;
    let mut parts: Vec<&str> = rest
        .strip_prefix("src/")?
        .strip_suffix(".rs")?
        .split('/')
        .collect();
    if matches!(parts.last(), Some(&("lib" | "main" | "mod"))) {
        parts.pop();
    }
    Some((krate.into(), parts.join("::")))
}

/// What a crate's doc says it is: the first sentence of its root's `//!` lines.
fn what(src: &Path) -> String {
    let root = ["lib.rs", "main.rs"]
        .iter()
        .find_map(|f| std::fs::read_to_string(src.join(f)).ok());
    let doc: Vec<String> = (root.unwrap_or_default().lines())
        .map_while(|l| l.strip_prefix("//!").map(|t| t.trim().to_string()))
        .collect();
    let doc = doc.join(" ");
    doc.split_inclusive(". ")
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The map of `rust` (the workspace root) from its files and its index, as `dash code map` writes it.
pub fn map(rust: &Path, index: &Index, sha: &str, at: &str) -> Value {
    let files = read_index(index);
    let (names, edges) = crate_graph(rust);
    let crates: Vec<Value> = (names.iter()).map(|name| {
        let folder = rust.join("crates").join(name);
        let mut modules: BTreeMap<String, (usize, usize, BTreeSet<String>)> = BTreeMap::new();
        for path in dash_gate::rust_files(&folder) {
            let lines = dash_gate::line_count(std::slice::from_ref(&path));
            let rel = path.strip_prefix(rust).unwrap_or(&path).to_string_lossy().into_owned();
            let (module, tests, uses) = match (file_module(&rel), files.get(&rel)) {
                (Some((_, m)), Some((t, u))) => (m, *t, u.clone()),
                (Some((_, m)), None) => (m, 0, BTreeSet::new()),
                // tests/, benches/ and build.rs: a test folder's file is all tests
                (None, _) => ("(outside src)".into(), if rel.contains("/tests/") { lines } else { 0 }, BTreeSet::new()),
            };
            let entry = modules.entry(module).or_default();
            (entry.0, entry.1) = (entry.0 + lines.saturating_sub(tests), entry.1 + tests.min(lines));
            entry.2.extend(uses);
        }
        let (code, tests) = modules.values().fold((0, 0), |(c, t), m| (c + m.0, t + m.1));
        let uses: Vec<&String> = edges.iter().filter(|(f, _)| f == name).map(|(_, t)| t).collect();
        let modules: Vec<Value> = (modules.iter())
            .map(|(m, (c, t, u))| json!({"name": m, "code": c, "tests": t, "uses": u}))
            .collect();
        json!({"name": name, "layer": layer(name), "what": what(&folder.join("src")), "code": code, "tests": tests, "uses": uses, "modules": modules})
    }).collect();
    let lines: u64 = crates
        .iter()
        .map(|c| c["code"].as_u64().unwrap_or(0) + c["tests"].as_u64().unwrap_or(0))
        .sum();
    json!({"sha": sha, "at": at, "lines": lines, "limit": dash_gate::LINE_LIMIT, "crates": crates})
}

/// The layers of section 3 in plain words, outermost first: what the page calls each of dash-gate's lists.
const LAYERS: [(&str, &str); 7] = [
    ("Binary", "The app (the one dash binary)"),
    ("Plugins", "Plugins"),
    ("Task layers", "Task runner"),
    ("Rings", "Adapters around the core"),
    ("Core", "Core model"),
    ("Tooling", "Build and test tooling"),
    ("Other", "In no layer"),
];

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let (head, rest) = digits.split_at(digits.len() % 3);
    let groups = rest
        .as_bytes()
        .chunks(3)
        .map(|g| std::str::from_utf8(g).unwrap_or_default());
    [head]
        .into_iter()
        .chain(groups)
        .filter(|g| !g.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

/// /deps: the treemap of where the lines are, the crates' table, then their modules'. One tree: its text form is the agent's.
pub fn page(data: &Path) -> dash_surface::Drawing {
    let title = "Code map".to_string();
    let read = std::fs::read_to_string(file(data))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let Some(map) = read else {
        let text = "No code map in this data folder yet: `dash code map --data <dir>` writes it, as make rust/deploy does.";
        let tree =
            json!({"root": "p", "elements": {"p": {"type": "text", "props": {"text": text}}}});
        return dash_surface::Drawing::Tree {
            title,
            tree: tree.to_string(),
        };
    };
    let crates = map["crates"].as_array().cloned().unwrap_or_default();
    let n = |c: &Value, k: &str| c[k].as_u64().unwrap_or(0);
    let mut by_size = crates.clone();
    by_size.sort_by_key(|c| std::cmp::Reverse(n(c, "code") + n(c, "tests")));
    let mut source = String::from("treemap-beta\n");
    for (key, label) in LAYERS {
        let held: Vec<&Value> = by_size.iter().filter(|c| c["layer"] == key).collect();
        if held.is_empty() {
            continue;
        }
        source += &format!("\"{label}\"\n");
        for c in held {
            source += &format!(
                "  \"{}\"\n    \"code\": {}\n    \"tests\": {}\n",
                c["name"].as_str().unwrap_or_default(),
                n(c, "code"),
                n(c, "tests")
            );
        }
    }
    let (code, tests) = crates
        .iter()
        .fold((0, 0), |(a, b), c| (a + n(c, "code"), b + n(c, "tests")));
    let words = |v: &Value| {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    let layer = |k: &Value| {
        LAYERS
            .iter()
            .find(|(key, _)| k == key)
            .map_or("", |(_, l)| l)
    };
    let rows: Vec<Value> = (by_size.iter())
        .map(|c| json!({"cells": [c["name"], layer(&c["layer"]), c["what"], thousands(n(c, "code")), thousands(n(c, "tests")), words(&c["uses"])]}))
        .collect();
    let mut modules = vec![];
    for c in &by_size {
        for m in c["modules"].as_array().into_iter().flatten() {
            let name = if m["name"] == "" {
                "(crate root)"
            } else {
                m["name"].as_str().unwrap_or_default()
            };
            modules.push(json!({"cells": [c["name"], name, thousands(n(m, "code")), thousands(n(m, "tests")), words(&m["uses"])]}));
        }
    }
    let text = format!(
        "{} lines of Rust against the {} line cap (dash-gate's LINE_LIMIT): {} of code and {} of tests, in {} crates. \
         Generated from commit {} at {} by dash code map: layers from dash-gate's crate lists (rfc-rust-unified section 3), \
         lines as the gate counts them, test code and module uses from rust-analyzer's index.",
        thousands(code + tests),
        thousands(n(&map, "limit")),
        thousands(code),
        thousands(tests),
        crates.len(),
        map["sha"].as_str().unwrap_or_default(),
        map["at"].as_str().unwrap_or_default(),
    );
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["intro", "crates", "table", "modules"]},
        "intro": {"type": "text", "props": {"text": text}},
        "crates": {"type": "diagram", "props": {"source": source}},
        "table": {"type": "table", "props": {"columns": ["Crate", "Layer", "What it does", "Code", "Tests", "Uses"], "rows": rows}},
        "modules": {"type": "section", "props": {"title": "Modules"}, "children": ["module-table"]},
        "module-table": {"type": "table", "props": {"columns": ["Crate", "Module", "Code", "Tests", "Uses"], "rows": modules}},
    }});
    dash_surface::Drawing::Tree {
        title,
        tree: tree.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, memory::TestBudget};
    use scip::types::{Document, Occurrence};

    fn at(symbol: &str, roles: i32, range: Vec<i32>, enclosing: Vec<i32>) -> Occurrence {
        let (symbol, symbol_roles) = (format!("rust-analyzer cargo {symbol}"), roles);
        Occurrence {
            symbol,
            symbol_roles,
            range,
            enclosing_range: enclosing,
            ..Default::default()
        }
    }

    /// Lines as the gate counts them, split by the index's `mod tests` and test folders; a use from a test or of a module
    /// itself is none; what a crate does is its doc's first sentence; the page's two forms say it, and no map says how to make one.
    #[test]
    fn the_map_splits_code_from_tests_and_the_page_draws_it_in_both_forms() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("codemap");
        let put = |path: &str, text: &str| {
            let path = dir.0.join(path);
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
            std::fs::write(path, text).expect("write");
        };
        put(
            "crates/store/Cargo.toml",
            "[dependencies]\ndash-core.workspace = true\n",
        );
        put(
            "crates/store/src/lib.rs",
            "//! The store. More words.\nmod keys;\n\n#[cfg(test)]\nmod tests {\n    fn a() {}\n}\n",
        );
        put("crates/store/src/keys.rs", "pub fn k() {}\n");
        put("crates/store/tests/t.rs", "fn t() {}\n");
        put("crates/core/Cargo.toml", "");
        put("crates/core/src/lib.rs", "//! The core.\n");
        let lib = Document {
            relative_path: "crates/store/src/lib.rs".into(),
            occurrences: vec![
                at(
                    "dash-store 0.1.0 crate/tests/",
                    1,
                    vec![4, 4, 9],
                    vec![4, 0, 6, 1],
                ),
                at("dash-store 0.1.0 crate/keys/k().", 0, vec![1, 4, 7], vec![]),
                at("dash-store 0.1.0 crate/keys/", 0, vec![1, 4, 8], vec![]),
                at(
                    "dash-store 0.1.0 crate/other/x().",
                    0,
                    vec![5, 4, 7],
                    vec![],
                ),
            ],
            ..Default::default()
        };
        let index = Index {
            documents: vec![lib],
            ..Default::default()
        };
        let map = map(&dir.0, &index, "abc", "2026-11-09T20:00:00Z");
        let store = &map["crates"]
            .as_array()
            .expect("crates")
            .iter()
            .find(|c| c["name"] == "store")
            .expect("store")
            .clone();
        assert_eq!(
            (store["code"].as_u64(), store["tests"].as_u64()),
            (Some(5), Some(4)),
            "{store}"
        );
        assert_eq!(
            (store["layer"].as_str(), store["what"].as_str()),
            (Some("Rings"), Some("The store."))
        );
        assert_eq!(store["uses"], json!(["core"]));
        assert_eq!(
            store["modules"][0],
            json!({"name": "", "code": 4, "tests": 3, "uses": ["keys"]})
        );
        assert_eq!(map["lines"], 10);
        let data = TestDir::new("codemap-data");
        let text = |d: dash_surface::Drawing| match d {
            dash_surface::Drawing::Tree { tree, .. } => dash_surface::text(&tree).expect("a text"),
            dash_surface::Drawing::Page(_) => panic!("a tree"),
        };
        assert!(text(page(&data.0)).starts_with("No code map in this data folder yet"));
        std::fs::write(file(&data.0), map.to_string()).expect("write");
        let said = text(page(&data.0));
        assert!(
            said.starts_with("10 lines of Rust against the 55,000 line cap"),
            "{said}"
        );
        assert!(
            said.contains("  \"store\"\n    \"code\": 5\n    \"tests\": 4\n"),
            "{said}"
        );
        assert!(
            said.contains("store\tAdapters around the core\tThe store.\t5\t4\tcore"),
            "{said}"
        );
        assert!(said.contains("store\t(crate root)\t4\t3\tkeys"), "{said}");
    }
}
