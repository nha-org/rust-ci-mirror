//! The code map /deps draws (@nha, 2026-11-09: « Line counts vary a lot: dash 14.8k, surface 7k »): lines as dash-gate counts
//! them, tests and uses from rust-analyzer's SCIP index; `dash code map` writes it per commit, make rust/deploy runs it.
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
        text(&map["sha"]),
        map["lines"]
    ))
}

/// Each file's inline `mod tests` lines, and the modules of its crate it uses outside them, as dash-gate's graph reads.
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
    let first = doc.split_inclusive(". ").next();
    first.unwrap_or_default().trim().to_string()
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
    let tokens = dash_gate::graph::token_count(&dash_gate::rust_files(&rust.join("crates")));
    json!({"sha": sha, "at": at, "lines": lines, "tokens": tokens, "target": dash_gate::TOKEN_TARGET, "crates": crates})
}

/// The layers of section 3 in plain words, outermost first: what the page calls each of dash-gate's lists.
const LAYERS: [(&str, &str); 6] = [
    ("Binary", "The app (the one dash binary)"),
    ("Plugins", "Plugins"),
    ("Task layers", "Task runner"),
    ("Rings", "Adapters around the core"),
    ("Core", "Core model"),
    ("Tooling", "Build and test tooling"),
];

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One row of a level: its name, lines, what it uses among its kind, where a click goes, what it is.
type Item = (String, u64, u64, Vec<String>, String, String);

/// /deps, three levels (@nha, 2026-11-09: « it does not show the deps. And it does not allow digging into the layers »):
/// layers, `?layer=`, `?crate=`; each a treemap, a graph of uses among its boxes, and a table whose rows go one level down.
pub fn page(
    data: &Path,
    layer_asked: Option<&str>,
    crate_asked: Option<&str>,
) -> dash_surface::Drawing {
    let title = "Code map".to_string();
    let read = std::fs::read_to_string(file(data)).ok();
    let map = read
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or(Value::Null);
    let crates = map["crates"].as_array().cloned().unwrap_or_default();
    let n = |c: &Value, k: &str| c[k].as_u64().unwrap_or(0);
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect()
    };
    let label = |k: &str| {
        LAYERS
            .iter()
            .find(|(key, _)| *key == k)
            .map_or(k, |(_, l)| *l)
            .to_string()
    };
    let layer_of = |c: &str| {
        crates
            .iter()
            .find(|x| x["name"] == c)
            .and_then(|x| x["layer"].as_str())
            .unwrap_or("")
            .to_string()
    };
    let item = |name: &str, v: &Value, href: String, what: &Value| -> Item {
        let what = text(what).into();
        (
            name.into(),
            n(v, "code"),
            n(v, "tests"),
            strings(&v["uses"]),
            href,
            what,
        )
    };
    let found = crate_asked.and_then(|name| crates.iter().find(|c| c["name"] == name));
    let (intro, items): (String, Vec<Item>) = if map.is_null() {
        ("No code map in this data folder yet: `dash code map --data <dir>` writes it, as make rust/deploy does.".into(), vec![])
    } else if let Some(c) = found {
        let name = text(&c["name"]);
        let users: Vec<&str> = crates
            .iter()
            .filter(|x| x["uses"].as_array().is_some_and(|u| u.contains(&c["name"])))
            .filter_map(|x| x["name"].as_str())
            .collect();
        let intro = format!(
            "{name}, in {}: {} {} lines of code, {} of tests. It uses {}. Used by: {}. Up: /deps?layer={}",
            label(&layer_of(name)),
            text(&c["what"]),
            thousands(n(c, "code")),
            thousands(n(c, "tests")),
            strings(&c["uses"]).join(", "),
            Some(users.join(", "))
                .filter(|u| !u.is_empty())
                .unwrap_or("no crate".into()),
            text(&c["layer"])
        );
        let modules = c["modules"].as_array().into_iter().flatten();
        let named = |m: &Value| {
            m["name"]
                .as_str()
                .filter(|m| !m.is_empty())
                .unwrap_or("(crate root)")
                .to_string()
        };
        let items = modules.map(|m| item(&named(m), m, String::new(), &Value::Null));
        (intro, items.collect())
    } else if let Some(key) = layer_asked {
        let held = crates.iter().filter(|c| c["layer"] == key);
        let name = |c: &Value| text(&c["name"]).to_string();
        let items = held.map(|c| item(&name(c), c, format!("/deps?crate={}", name(c)), &c["what"]));
        (
            format!(
                "{}: each crate's lines, what it uses, and a row to open it. Up: /deps",
                label(key)
            ),
            items.collect(),
        )
    } else {
        let items = LAYERS
            .iter()
            .filter(|(key, _)| crates.iter().any(|c| c["layer"] == *key))
            .map(|(key, l)| {
                let held: Vec<&Value> = crates.iter().filter(|c| c["layer"] == *key).collect();
                let uses: BTreeSet<String> = held
                    .iter()
                    .flat_map(|c| strings(&c["uses"]))
                    .map(|u: String| label(&layer_of(&u)))
                    .filter(|u| u != l)
                    .collect();
                let names: Vec<&str> = held.iter().filter_map(|c| c["name"].as_str()).collect();
                (
                    (*l).into(),
                    held.iter().map(|c| n(c, "code")).sum(),
                    held.iter().map(|c| n(c, "tests")).sum(),
                    uses.into_iter().collect(),
                    format!("/deps?layer={key}"),
                    names.join(", "),
                )
            });
        let items: Vec<Item> = items.collect();
        let (code, tests) = items.iter().fold((0, 0), |(a, b), i| (a + i.1, b + i.2));
        (
            format!(
                "{} tokens of Rust against the {} token target (50,000 lines' worth; comments and line breaks not counted), \
             {} lines: {} of code and {} of tests, in {} crates. Generated from commit {} at {} by dash code map: layers \
             from dash-gate's crate lists (rfc-rust-unified section 3), test code and module uses from rust-analyzer's \
             index. A row opens its layer.",
                thousands(n(&map, "tokens")),
                thousands(n(&map, "target")),
                thousands(code + tests),
                thousands(code),
                thousands(tests),
                crates.len(),
                text(&map["sha"]),
                text(&map["at"])
            ),
            items,
        )
    };
    let mut items = items;
    items.sort_by_key(|i| std::cmp::Reverse(i.1 + i.2));
    // a treemap label ends at a colon (« name: value »): design::families is drawn design/families
    let leaf = |i: &Item| {
        format!(
            "\"{}\"\n  \"code\": {}\n  \"tests\": {}\n",
            i.0.replace("::", "/"),
            i.1,
            i.2
        )
    };
    let boxes = format!(
        "treemap-beta\n{}",
        items.iter().map(leaf).collect::<String>()
    );
    let at = |name: &String| items.iter().position(|i| i.0 == *name);
    let nodes: String = items
        .iter()
        .enumerate()
        .map(|(k, i)| format!("  n{k}[\"{}\"]\n", i.0))
        .collect();
    let arrows: String = items
        .iter()
        .enumerate()
        .flat_map(|(k, i)| {
            i.3.iter()
                .filter_map(at)
                .map(move |t| format!("  n{k} --> n{t}\n"))
        })
        .collect();
    let rows: Vec<Value> = items.iter().map(|i| json!({"cells": [i.0, i.5, thousands(i.1), thousands(i.2), i.3.join(", ")], "href": (!i.4.is_empty()).then_some(&i.4)})).collect();
    let mut elements = json!({"intro": {"type": "text", "props": {"text": intro}}});
    let mut shown = vec!["intro"];
    if !items.is_empty() {
        elements["boxes"] = json!({"type": "diagram", "props": {"source": boxes}});
        elements["table"] = json!({"type": "table", "props": {"columns": ["Name", "What it is", "Code", "Tests", "Uses"], "rows": rows}});
        shown.extend(["boxes", "table"]);
    }
    if !arrows.is_empty() {
        elements["uses"] = json!({"type": "diagram", "props": {"source": format!("flowchart LR\n{nodes}{arrows}")}});
        shown.insert(2, "uses");
    }
    elements["page"] = json!({"type": "stack", "children": shown});
    dash_surface::Drawing::Tree {
        title,
        tree: json!({"root": "page", "elements": elements}).to_string(),
    }
}

// The code map (task rust-deps-view; @nha, 2026-11-09: « Line counts vary a lot ... that's what we need to surface »)
inventory::submit! {
    crate::pages::Page { path: "/deps", step: "route_deps", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        let (layer, krate) = (a.param("layer"), a.param("crate"));
        let data = crate::comments::serve_from_dir().map_or_else(|_| crate::tasks::live_data(), Path::to_path_buf);
        a.form(Some(Ok(page(&data, layer.as_deref(), krate.as_deref()))))
    } }
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
        assert!(text(page(&data.0, None, None)).starts_with("No code map in this data folder yet"));
        std::fs::write(file(&data.0), map.to_string()).expect("write");
        let said = text(page(&data.0, None, None));
        assert!(
            said.contains("token target") && said.contains(", 10 lines: 6 of code and 4 of tests"),
            "{said}"
        );
        // the layers, then a layer's crates, then a crate's modules and their uses, each row a click down
        let layer = "Adapters around the core\tstore\t5\t4\tCore model\t/deps?layer=Rings";
        assert!(
            said.contains("\"Adapters around the core\"\n  \"code\": 5\n") && said.contains(layer),
            "{said}"
        );
        let rings = text(page(&data.0, Some("Rings"), None));
        assert!(
            rings.contains("store\tThe store.\t5\t4\tcore\t/deps?crate=store"),
            "{rings}"
        );
        let store = text(page(&data.0, None, Some("store")));
        assert!(
            store.contains("(crate root)\t\t4\t3\tkeys\t") && store.contains("n0 --> n2"),
            "{store}"
        );
    }
}
