//! The code map /deps draws (@nha, 2026-11-09: « Line counts vary a lot: dash 14.8k, surface 7k »): lines as dash-gate counts
//! them, tests and uses from rust-analyzer's SCIP index; `dash code map` writes it per commit, make rust/deploy runs it.
use dash_gate::graph::{crate_graph, layer};
use dash_surface::components::{Nest, grouped};
use scip::types::Document;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn file(data: &Path) -> PathBuf {
    data.join("codemap.json")
}

inventory::submit! { crate::usage::Command::usage("dash code map --data <dir>") }
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
    let files = read_scip(&scip).map_err(|e| format!("{out}: {e}"))?;
    let sha = dash_process::run("git", &["rev-parse", "--short=9", "HEAD"], Some(repo), long)?;
    let map = map(
        &rust,
        &files,
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

/// Each file's inline `mod tests` lines and the modules of its crate it uses outside them, by its path.
type Files = BTreeMap<String, (usize, BTreeSet<String>)>;

/// The index at `scip`, one document at a time from the file: parsed whole, its 35 MB took the process past the 200 MB cap,
/// and the live map stood at 2026-11-10's while each deploy said only « /deps keeps the last » (task rust-code-health).
fn read_scip(scip: &Path) -> Result<Files, String> {
    /// Index's `documents`, field 2, length-delimited (scip's generated merge_from)
    const DOCUMENT: u32 = 2 << 3 | 2;
    // the stream buffers its reader itself (/code-review)
    let mut read = std::fs::File::open(scip).map_err(|e| e.to_string())?;
    let mut is = protobuf::CodedInputStream::new(&mut read);
    let mut failed = None;
    let docs = std::iter::from_fn(|| {
        let next = loop {
            match is.read_raw_tag_or_eof() {
                Ok(Some(DOCUMENT)) => break is.read_message::<Document>().map(Some),
                Ok(Some(tag)) => {
                    if let Err(e) = protobuf::rt::skip_field_for_tag(tag, &mut is) {
                        break Err(e);
                    }
                }
                Ok(None) => break Ok(None),
                Err(e) => break Err(e),
            }
        };
        match next {
            Ok(doc) => doc,
            Err(e) => {
                failed = Some(e);
                None
            }
        }
    });
    let files = read_index(docs);
    failed.map_or(Ok(files), |e| Err(e.to_string()))
}

/// Each file's inline `mod tests` lines, and the modules of its crate it uses outside them, as dash-gate's graph reads.
fn read_index(docs: impl IntoIterator<Item = Document>) -> Files {
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
    for doc in docs {
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
pub fn map(rust: &Path, files: &Files, sha: &str, at: &str) -> Value {
    let (names, edges) = crate_graph(rust);
    let crates: Vec<Value> = (names.iter()).map(|name| {
        let folder = rust.join("crates").join(name);
        let mut modules: BTreeMap<String, (usize, usize, BTreeSet<String>, Vec<Value>)> = BTreeMap::new();
        let mut paths = dash_gate::rust_files(&folder);
        paths.sort();
        for path in paths {
            let lines = dash_gate::line_count(std::slice::from_ref(&path));
            let state = state_calls(&std::fs::read_to_string(&path).unwrap_or_default());
            let rel = path.strip_prefix(rust).unwrap_or(&path).to_string_lossy().into_owned();
            let (module, tests, uses) = match (file_module(&rel), files.get(&rel)) {
                (Some((_, m)), Some((t, u))) => (m, *t, u.clone()),
                (Some((_, m)), None) => (m, 0, BTreeSet::new()),
                // tests/, benches/ and build.rs: a test folder's file is all tests
                (None, _) => ("(outside src)".into(), if rel.contains("/tests/") { lines } else { 0 }, BTreeSet::new()),
            };
            let entry = modules.entry(module).or_default();
            let (code, tests) = (lines.saturating_sub(tests), tests.min(lines));
            (entry.0, entry.1) = (entry.0 + code, entry.1 + tests);
            // the file's own row, its name from the crate's folder: the boxes /deps draws inside its module's
            let name = rel.strip_prefix(&format!("crates/{name}/")).unwrap_or(&rel);
            entry.3.push(json!({"name": name, "code": code, "tests": tests, "uses": uses, "state": state}));
            entry.2.extend(uses);
        }
        let (code, tests) = modules.values().fold((0, 0), |(c, t), m| (c + m.0, t + m.1));
        let uses: Vec<&String> = edges.iter().filter(|(f, _)| f == name).map(|(_, t)| t).collect();
        let modules: Vec<Value> = (modules.iter())
            .map(|(m, (c, t, u, f))| {
                let state: u64 = f.iter().filter_map(|f| f["state"].as_u64()).sum();
                json!({"name": m, "code": c, "tests": t, "uses": u, "files": f, "state": state})
            })
            .collect();
        json!({"name": name, "layer": layer(name), "what": what(&folder.join("src")), "code": code, "tests": tests, "uses": uses, "modules": modules})
    }).collect();
    let lines: u64 = crates
        .iter()
        .map(|c| c["code"].as_u64().unwrap_or(0) + c["tests"].as_u64().unwrap_or(0))
        .sum();
    let tokens = dash_gate::graph::token_count(&dash_gate::rust_files(&rust.join("crates")));
    let violations: Vec<Value> = (dash_gate::graph::violations(rust).into_iter())
        .map(|(rule, found, fix)| json!({"rule": rule, "found": found, "fix": fix}))
        .collect();
    // the binary's code against rust/binary-held.txt, as dash-gate counts both (task rust-binary-ratchet)
    let (code, _) = dash_gate::binary::count(rust);
    let held = std::fs::read_to_string(rust.join(dash_gate::binary::HELD)).unwrap_or_default();
    let binary = json!({"code": code, "held": dash_gate::binary::held(&held).0});
    json!({"sha": sha, "at": at, "lines": lines, "tokens": tokens, "target": dash_gate::TOKEN_TARGET, "crates": crates, "violations": violations, "binary": binary})
}

/// The `use_signal` and `use_resource` calls in `source`, by dash-gate's lexer: a comment or a string naming one is none, a
/// turbofish call is one (/code-review).
fn state_calls(source: &str) -> usize {
    use dash_gate::graph::Lexeme::Code;
    let lexed = dash_gate::graph::lexed(source);
    let call = |w: &[_]| match w {
        [Code(name), Code(next)] => {
            matches!(name.as_str(), "use_signal" | "use_resource")
                && matches!(next.as_str(), "(" | "::")
        }
        _ => false,
    };
    lexed.windows(2).filter(|w| call(w)).count()
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

fn strings(v: &Value) -> Vec<String> {
    let all = v.as_array().into_iter().flatten();
    all.filter_map(Value::as_str).map(String::from).collect()
}

/// One row of a level: its name, lines, what it uses among its kind, where a click goes, what it is.
type Item = (String, u64, u64, Vec<String>, String, String);

fn joined(at: &str, name: &str) -> String {
    if at.is_empty() {
        name.into()
    } else {
        format!("{at}/{name}")
    }
}

/// A module's name as drawn: the crate's own is « (crate root) ».
fn module_name(m: &str) -> String {
    Some(m)
        .filter(|m| !m.is_empty())
        .unwrap_or("(crate root)")
        .into()
}

/// A crate's modules as boxes, each holding its files, under the crate's box path `at` ("" when they are the top). A module
/// of one file of code is that file's box alone, so most files pay one title line, not two (/code-review: each title line
/// takes area from the lines below it). A module or a file uses the modules it names: hovering it outlines their boxes.
fn module_boxes(c: &Value, at: &str) -> Vec<Nest> {
    let modules: Vec<&Value> = c["modules"].as_array().into_iter().flatten().collect();
    let file = |f: &Value| text(&f["name"]).trim_start_matches("src/").to_string();
    let coded = |m: &Value| -> Vec<Value> {
        let files = m["files"].as_array().into_iter().flatten();
        files
            .filter(|f| f["code"].as_u64().unwrap_or(0) > 0)
            .cloned()
            .collect()
    };
    // a module's box name: its one file's, else its own
    let boxed = |m: &Value| match &coded(m)[..] {
        [one] => file(one),
        _ => module_name(text(&m["name"])),
    };
    let names: BTreeMap<&str, String> = modules
        .iter()
        .map(|m| (text(&m["name"]), boxed(m)))
        .collect();
    let used = |v: &Value| {
        let to = |u: &String| joined(at, names.get(u.as_str()).map_or(&module_name(u), |n| n));
        strings(v).iter().map(to).collect()
    };
    let leaf = |f: &Value| Nest {
        name: file(f),
        size: f["code"].as_u64().unwrap_or(0),
        uses: used(&f["uses"]),
        children: vec![],
    };
    (modules.iter())
        .map(|m| match &coded(m)[..] {
            [one] => leaf(one),
            _ => Nest {
                name: boxed(m),
                size: m["code"].as_u64().unwrap_or(0),
                uses: used(&m["uses"]),
                children: m["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(leaf)
                    .collect(),
            },
        })
        .collect()
}

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
            grouped(n(c, "code")),
            grouped(n(c, "tests")),
            strings(&c["uses"]).join(", "),
            Some(users.join(", "))
                .filter(|u| !u.is_empty())
                .unwrap_or("no crate".into()),
            text(&c["layer"])
        );
        let modules = c["modules"].as_array().into_iter().flatten();
        let items = modules.map(|m| {
            item(
                &module_name(text(&m["name"])),
                m,
                String::new(),
                &Value::Null,
            )
        });
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
             index. A row opens its layer. The binary, crates/dash: {} lines of code against {} held in rust/binary-held.txt, \
             as dash-gate counts them (its tests' modules left out), which only drop; a feature goes in a plugin crate.",
                grouped(n(&map, "tokens")),
                grouped(n(&map, "target")),
                grouped(code + tests),
                grouped(code),
                grouped(tests),
                crates.len(),
                text(&map["sha"]),
                text(&map["at"]),
                grouped(n(&map["binary"], "code")),
                map["binary"]["held"]
                    .as_u64()
                    .map_or("none".into(), grouped)
            ),
            items,
        )
    };
    // the same level as boxes, nested down to files (task rust-deps-files-hover): a crate's box path is its layer's, then its name
    let crate_box = |c: &Value, path: &dyn Fn(&str) -> String| {
        let name = text(&c["name"]);
        let uses = strings(&c["uses"]).iter().map(|u| path(u)).collect();
        Nest {
            name: name.into(),
            size: 0,
            uses,
            children: module_boxes(c, &path(name)),
        }
    };
    let boxes: Vec<Nest> = if map.is_null() {
        vec![]
    } else if let Some(c) = found {
        module_boxes(c, "")
    } else if let Some(key) = layer_asked {
        let held = crates.iter().filter(|c| c["layer"] == key);
        held.map(|c| crate_box(c, &|n: &str| n.to_string()))
            .collect()
    } else {
        // a layer box's uses are its row's, the layers' names
        let in_layer = |n: &str| format!("{}/{n}", label(&layer_of(n)));
        (items.iter())
            .map(|i| {
                let held = crates.iter().filter(|c| label(text(&c["layer"])) == i.0);
                let children = held.map(|c| crate_box(c, &in_layer)).collect();
                Nest {
                    name: i.0.clone(),
                    size: 0,
                    uses: i.3.clone(),
                    children,
                }
            })
            .collect()
    };
    let mut items = items;
    items.sort_by_key(|i| std::cmp::Reverse(i.1 + i.2));
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
    let rows: Vec<Value> = items.iter().map(|i| json!({"cells": [i.0, i.5, grouped(i.1), grouped(i.2), i.3.join(", ")], "href": (!i.4.is_empty()).then_some(&i.4)})).collect();
    let mut elements = json!({"intro": {"type": "text", "props": {"text": intro}}});
    let mut shown = vec!["intro"];
    if !items.is_empty() {
        elements["boxes"] = json!({"type": "treemap", "props": {"boxes": boxes}});
        elements["table"] = json!({"type": "table", "props": {"columns": ["Name", "What it is", "Code", "Tests", "Uses"], "rows": rows}});
        shown.extend(["boxes", "table"]);
    }
    if !arrows.is_empty() {
        elements["uses"] = json!({"type": "diagram", "props": {"source": format!("flowchart LR\n{nodes}{arrows}")}});
        shown.insert(2, "uses");
    }
    if !map.is_null() && found.is_none() && layer_asked.is_none() {
        health(&map, &crates, &mut elements, &mut shown);
    }
    elements["page"] = json!({"type": "stack", "children": shown});
    dash_surface::Drawing::Tree {
        title,
        tree: json!({"root": "page", "elements": elements}).to_string(),
    }
}

/// How many modules each ranking shows: under a table's page, so neither pages.
const RANKED: usize = 15;
/// A module in the rankings: its name, lines of code, use_signal and use_resource calls.
type Ranked = (String, u64, u64);

/// The top level's health section (task rust-code-health; @nha, 2026-11-10: « It will be for the "code" tab. better get
/// this early imo. »): what dash-gate refuses on, each with its fix, and the modules ranked by lines, then by use_signal and
/// use_resource calls. boundary 0.27 and archunit 0.0.2 found no more than the gate (PR body).
fn health(map: &Value, crates: &[Value], elements: &mut Value, shown: &mut Vec<&str>) {
    let Some(found) = map["violations"].as_array() else {
        elements["health"] = json!({"type": "text", "props": {"text": "No health record in this map yet: `dash code map --data <dir>` writes one."}});
        shown.push("health");
        return;
    };
    let rows: Vec<Value> = (found.iter())
        .map(|v| json!({"cells": [text(&v["rule"]), text(&v["found"]), text(&v["fix"])]}))
        .collect();
    let modules: Vec<Ranked> = (crates.iter())
        .flat_map(|c| {
            let modules = c["modules"].as_array().into_iter().flatten();
            modules.map(|m| {
                let name = joined(text(&c["name"]), &module_name(text(&m["name"])));
                (
                    name,
                    m["code"].as_u64().unwrap_or(0),
                    m["state"].as_u64().unwrap_or(0),
                )
            })
        })
        .collect();
    // two short rankings, never a pager: the page's query pages and filters every table on it (/code-review)
    let ranked = |by: &dyn Fn(&Ranked) -> (u64, u64)| -> Vec<Value> {
        let mut held: Vec<&Ranked> = modules.iter().filter(|m| by(m).0 > 0).collect();
        held.sort_by_key(|m| std::cmp::Reverse(by(m)));
        let cells = |(name, code, state): &&Ranked| json!({"cells": [name, code.to_string(), state.to_string()]});
        held.iter().take(RANKED).map(cells).collect()
    };
    let (largest, stateful) = (ranked(&|m| (m.1, m.2)), ranked(&|m| (m.2, m.1)));
    let said = format!(
        "{} gate violations at commit {}, mapped {}: the layer, loop and module rules dash-gate refuses a commit on, each with its fix. \
         Below, the {RANKED} modules with the most lines of code, then the {RANKED} with the most use_signal and use_resource calls.",
        found.len(),
        text(&map["sha"]),
        text(&map["at"])
    );
    elements["health-title"] = json!({"type": "heading", "props": {"text": "Health"}});
    elements["health"] = json!({"type": "text", "props": {"text": said}});
    shown.extend(["health-title", "health"]);
    if !rows.is_empty() {
        elements["violations"] =
            json!({"type": "table", "props": {"columns": ["Rule", "Found", "Fix"], "rows": rows}});
        shown.push("violations");
    }
    let columns = [
        "Module",
        "Lines of code",
        "use_signal and use_resource calls",
    ];
    elements["largest"] = json!({"type": "table", "props": {"columns": columns, "rows": largest}});
    elements["stateful"] =
        json!({"type": "table", "props": {"columns": columns, "rows": stateful}});
    shown.extend(["largest", "stateful"]);
}

// The code map (task rust-deps-view; @nha, 2026-11-09: « Line counts vary a lot ... that's what we need to surface »)
inventory::submit! {
    crate::pages::Page { path: "/deps", step: "route_deps", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        let (layer, krate) = (a.param("layer"), a.param("crate"));
        let data = a.data();
        a.form(Some(Ok(page(&data, layer.as_deref(), krate.as_deref()))))
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use protobuf::Message;
    use scip::types::{Index, Occurrence};

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
    #[dash_meter::dash_test]
    fn the_map_splits_code_from_tests_and_the_page_draws_it_in_both_forms() {
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
        put("binary-held.txt", "7\n");
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
            // a field before the documents, which the read skips
            metadata: Some(scip::types::Metadata::default()).into(),
            ..Default::default()
        };
        // read from the file, one document at a time, as `dash code map` reads it
        let scip = dir.0.join("index.scip");
        std::fs::write(&scip, index.write_to_bytes().expect("bytes")).expect("write");
        let files = read_scip(&scip).expect("the index reads");
        let map = map(&dir.0, &files, "abc", "2026-11-09T20:00:00Z");
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
            json!({"name": "", "code": 4, "tests": 3, "uses": ["keys"], "state": 0,
                "files": [{"name": "src/lib.rs", "code": 4, "tests": 3, "uses": ["keys"], "state": 0}]})
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
        // done-when 3 of rust-binary-ratchet: the binary's count against its held number
        assert!(
            said.contains(
                "The binary, crates/dash: 0 lines of code against 7 held in rust/binary-held.txt"
            ),
            "{said}"
        );
        // the layers, then a layer's crates, then a crate's modules and their uses, each row a click down
        let layer = "Adapters around the core\tstore\t5\t4\tCore model\t/deps?layer=Rings";
        // nested boxes down to files, area the code lines, each box's uses as box paths (task rust-deps-files-hover)
        // a module of one file of code is that file's box, a use of it outlines the file
        let nested = "Adapters around the core: 5 lines; uses Core model\n  store: 5 lines; uses Core model/core\n    \
                      lib.rs: 4 lines; uses keys.rs\n    keys.rs: 1 line\nCore model: 1 line\n";
        assert!(said.contains(nested) && said.contains(layer), "{said}");
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

    /// Task rust-code-health: the health section's rows are the map's records, the same in the text form and the HTML.
    #[dash_meter::dash_test]
    fn the_health_section_draws_each_violation_with_its_fix_and_ranks_modules() {
        let dir = TestDir::new("codemap-health");
        let put = |path: &str, text: &str| {
            let path = dir.0.join(path);
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
            std::fs::write(path, text).expect("write");
        };
        // the core taking a ring is the edge the gate refuses, and the ring taking the core back closes a loop
        put(
            "crates/core/Cargo.toml",
            "[dependencies]\ndash-ui.workspace = true\n",
        );
        put("crates/core/src/lib.rs", "//! The core.\n");
        put(
            "crates/ui/Cargo.toml",
            "[dependencies]\ndash-core.workspace = true\n",
        );
        put(
            "crates/ui/src/lib.rs",
            "mod page;\npub fn a() {}\npub fn b() {}\npub fn c() {}\npub fn d() {}\npub fn e() {}\npub fn f() {}\npub fn g() {}\n",
        );
        put(
            "crates/ui/src/page.rs",
            "fn p() {\n    let a = use_signal::<u8>(|| 0);\n    let b = use_resource(f);\n    // use_signal( said\n    let s = \"use_resource(\";\n}\n",
        );
        let map = map(&dir.0, &Files::new(), "abc", "2026-11-12T09:00:00Z");
        let (fix, loop_fix) = (dash_gate::graph::EDGE_FIX, dash_gate::graph::CRATE_LOOP_FIX);
        assert_eq!(
            map["violations"],
            json!([{"rule": "layer edge", "found": "core -> ui", "fix": fix},
                {"rule": "crate loop", "found": "core -> ui -> core", "fix": loop_fix}])
        );
        let data = TestDir::new("codemap-health-data");
        std::fs::write(file(&data.0), map.to_string()).expect("write");
        let dash_surface::Drawing::Tree { tree, .. } = page(&data.0, None, None) else {
            panic!("a tree")
        };
        let said = dash_surface::text(&tree).expect("a text");
        let html =
            dash_surface::markup(&dash_surface::screen::page("Code map", &tree).expect("html"));
        let violations = [
            format!("layer edge\tcore -> ui\t{fix}"),
            format!("crate loop\tcore -> ui -> core\t{loop_fix}"),
        ];
        // ranked by lines: ui's root (8) before its page (6); then by calls, the page alone, its two calls counted and the
        // comment and the string naming one left out
        let largest = "ui/(crate root)\t8\t0\t\nui/page\t6\t2\t\ncore/(crate root)\t1\t0\t\n";
        let stateful = "calls\tLink\nui/page\t6\t2\t\n";
        assert!(
            said.contains("2 gate violations at commit abc, mapped 2026-11-12T09:00:00Z")
                && violations.iter().all(|v| said.contains(v)),
            "{said}"
        );
        assert!(said.contains(largest) && said.contains(stateful), "{said}");
        assert!(
            html.contains(">Health<") && html.contains("core -&#62; ui") && html.contains(fix),
            "{html}"
        );
        // a layer's page and a crate's draw no health section
        let dash_surface::Drawing::Tree { tree, .. } = page(&data.0, Some("Rings"), None) else {
            panic!("a tree")
        };
        assert!(
            !dash_surface::text(&tree)
                .expect("a text")
                .contains("gate violations")
        );
        // a map from before the health record says how to get one
        std::fs::write(file(&data.0), json!({"sha": "x", "crates": []}).to_string())
            .expect("write");
        let dash_surface::Drawing::Tree { tree, .. } = page(&data.0, None, None) else {
            panic!("a tree")
        };
        assert!(
            dash_surface::text(&tree)
                .expect("a text")
                .contains("No health record")
        );
    }
}
