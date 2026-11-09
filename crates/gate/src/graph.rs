//! The crate and module graphs, read from the source, for the gate and the /deps page (@nha, 2026-11-06: « graph of
//! dependencies, ideally we forbid cycles and we have an evergreen rust page in the rust dash »). A small lexer, not syn:
//! syn leaves a macro's body opaque, and `assert_eq!(tasks::show(), ..)` is a use like any other. crates/dash/build.rs runs
//! `write_page`, which is why the crate takes no dependency: a build script compiles what it links.
use crate::{
    AGENT_KINDS, BINARY, PLUGINS, RINGS, TASK_LAYERS, TOOLING, dependency_loop, is_tooling_edge,
    rust_files,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub type Edge = (String, String);

/// Identifiers, `::` and each other mark, comments, strings and char literals gone: a brace or a path in those is no code.
pub(crate) fn tokens(source: &str) -> Vec<String> {
    lexed(source).0
}

/// What each string literal of this source holds, raw ones too, escapes as written. Comments are not in it.
pub(crate) fn strings(source: &str) -> Vec<String> {
    lexed(source).1
}

/// Each of `AGENT_KINDS` this source's code holds, strings included and comments left out, in any case and inside a longer
/// name: `ClaudeRun` and `CODEX_HOME` name one too. Here, not in lib.rs: this module reads the crate root, so the root
/// reading it back is a module cycle.
pub fn agents_named(source: &str) -> Vec<&'static str> {
    let (tokens, strings) = lexed(source);
    let code = [tokens, strings].concat().join("\n").to_lowercase();
    AGENT_KINDS
        .into_iter()
        .filter(|a| code.contains(a))
        .collect()
}

fn lexed(source: &str) -> (Vec<String>, Vec<String>) {
    let s: Vec<char> = source.chars().collect();
    let (mut out, mut strings, mut i) = (vec![], vec![], 0);
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    while i < s.len() {
        let next = s.get(i + 1).copied();
        match s[i] {
            '/' if next == Some('/') => {
                while i < s.len() && s[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                let mut depth = 1;
                i += 2;
                while i < s.len() && depth > 0 {
                    match (s[i], s.get(i + 1)) {
                        ('/', Some('*')) => (depth, i) = (depth + 1, i + 2),
                        ('*', Some('/')) => (depth, i) = (depth - 1, i + 2),
                        _ => i += 1,
                    }
                }
            }
            '"' => {
                i += 1;
                let start = i;
                while i < s.len() && s[i] != '"' {
                    i += if s[i] == '\\' { 2 } else { 1 };
                }
                strings.push(s[start..i.min(s.len())].iter().collect());
                i += 1;
            }
            // '\n' and '{' are literals; the quote of a lifetime is dropped and its name is a word like any other.
            '\'' if next == Some('\\') => {
                i += 3; // past the escaped character, a quote included
                while i < s.len() && s[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            '\'' if s.get(i + 2) == Some(&'\'') => i += 3,
            // `..` is its own token: `0..crate::a::N` starts a path, where `v.b::<u8>()` does not.
            c @ (':' | '.') if next == Some(c) => {
                out.push(format!("{c}{c}"));
                i += 2;
            }
            c if word_char(c) => {
                let start = i;
                while i < s.len() && word_char(s[i]) {
                    i += 1;
                }
                let word: String = s[start..i].iter().collect();
                let hashes = s[i..].iter().take_while(|c| **c == '#').count();
                // A raw identifier is its name: `mod r#type;` is type.rs.
                if word == "r" && hashes == 1 && s.get(i + 1).is_some_and(|c| word_char(*c)) {
                    i += 1;
                    continue;
                }
                if matches!(word.as_str(), "r" | "br" | "cr") && s.get(i + hashes) == Some(&'"') {
                    let end = ["\"".to_string(), "#".repeat(hashes)].concat();
                    let end: Vec<char> = end.chars().collect();
                    i += hashes + 1;
                    let start = i;
                    while i < s.len() && !s[i..].starts_with(&end) {
                        i += 1;
                    }
                    strings.push(s[start..i.min(s.len())].iter().collect());
                    i += end.len();
                }
                out.push(word);
            }
            c if c.is_whitespace() => i += 1,
            c => {
                out.push(c.to_string());
                i += 1;
            }
        }
    }
    (out, strings)
}

/// Each path that starts at `i` with the name it goes by here, a `use` group's branches apart, and the token after them.
/// Only a `use` has an alias: in code `as` ends the path, and `X as crate::c::T` starts the next one.
fn read_path(
    t: &[String],
    mut i: usize,
    in_use: bool,
    mut path: Vec<String>,
    out: &mut Vec<(Vec<String>, String)>,
) -> usize {
    let mut alias = None;
    while let Some(token) = t.get(i) {
        if token == "{" {
            i += 1;
            while let Some(inner) = t.get(i)
                && inner != "}"
            {
                i = match inner.as_str() {
                    "," => i + 1,
                    _ => read_path(t, i, in_use, path.clone(), out),
                };
            }
            return i + 1;
        }
        path.push(token.clone());
        i += 1;
        match t.get(i).map(String::as_str) {
            Some("::") => i += 1,
            Some("as") if in_use => {
                alias = t.get(i + 1).cloned();
                i += 2;
                break;
            }
            _ => break,
        }
    }
    if path.len() > 1 && path.last().is_some_and(|word| word == "self") {
        path.pop();
    }
    let name = alias.or(path.last().cloned()).unwrap_or_default();
    out.push((path, name));
    i
}

#[derive(Default)]
struct Found {
    modules: BTreeSet<String>,
    uses: Vec<(String, Vec<String>)>,
    /// What a `use` brought in: (module, name) to its path.
    imports: BTreeMap<(String, String), Vec<String>>,
    files: BTreeSet<PathBuf>,
    /// What the walk could not read: the gate refuses it, the page names it, the build goes on.
    holes: Vec<String>,
}

/// The file of a `mod name;`, under the folder its module path names.
// ponytail: no #[path] attribute (the workspace has none): `walk` names one as a hole the gate refuses, never a silent one.
fn module_file(src: &Path, module: &str) -> Option<PathBuf> {
    let folder = module
        .split("::")
        .skip(1)
        .fold(src.to_path_buf(), |p, s| p.join(s));
    let (flat, nested) = (folder.with_extension("rs"), folder.join("mod.rs"));
    [flat, nested].into_iter().find(|file| file.exists())
}

/// A path whose first word is what a `use` in an open block brought in, written out in full.
fn through_local(local: &[(usize, String, Vec<String>)], path: Vec<String>) -> Vec<String> {
    let first = path.first();
    match local.iter().rev().find(|(_, name, _)| Some(name) == first) {
        Some((_, _, full)) => [full.as_slice(), &path[1..]].concat(),
        None => path,
    }
}

/// One module's tokens from `i`, to its closing brace or the end of its file. An inline module is a module, so a test
/// module's `use super::*` is that module's edge, never its file's.
fn walk(src: &Path, module: &str, t: &[String], mut i: usize, found: &mut Found) -> usize {
    found.modules.insert(module.to_string());
    let is_word = |at: usize| {
        t.get(at)
            .is_some_and(|w| w.starts_with(|c: char| c.is_alphabetic() || c == '_'))
    };
    let mut depth = 0;
    // A `use` in a block holds to that block's end: (depth, name, path).
    let mut local: Vec<(usize, String, Vec<String>)> = vec![];
    while let Some(token) = t.get(i) {
        let after = t.get(i + 2).map(String::as_str);
        match token.as_str() {
            "{" => {
                depth += 1;
                i += 1;
            }
            "}" if depth == 0 => return i + 1,
            "}" => {
                depth -= 1;
                local.retain(|(at, ..)| *at <= depth);
                i += 1;
            }
            "mod" if is_word(i + 1) && after == Some("{") => {
                i = walk(src, &format!("{module}::{}", t[i + 1]), t, i + 3, found);
            }
            "mod" if is_word(i + 1) && after == Some(";") => {
                let child = format!("{module}::{}", t[i + 1]);
                match module_file(src, &child) {
                    Some(file) => read_module(src, &child, &file, found),
                    None => {
                        found.holes.push(format!("{child} has no file"));
                        found.modules.insert(child);
                    }
                }
                i += 3;
            }
            // Code the lexer cannot follow, wherever it lands: a macro's string argument is gone from the tokens.
            "include" if t.get(i + 1).is_some_and(|n| n == "!") => {
                found.holes.push(format!("{module} has an include!"));
                i += 1;
            }
            "path" if i >= 2 && t[i - 1] == "[" && t[i - 2] == "#" => {
                found.holes.push(format!("{module} has a #[path]"));
                i += 1;
            }
            // A visibility is no use: `pub(in crate::b)` depends on nothing.
            "pub"
                if t.get(i + 1).is_some_and(|n| n == "(")
                    && t.get(i + 2).is_some_and(|n| n == "in") =>
            {
                while t.get(i).is_some_and(|n| n != ")") {
                    i += 1;
                }
            }
            "use" => {
                let mut paths = vec![];
                i = read_path(t, i + 1, true, vec![], &mut paths);
                for (path, name) in paths {
                    let path = through_local(&local, path);
                    found.uses.push((module.to_string(), path.clone()));
                    if depth == 0 {
                        found.imports.insert((module.to_string(), name), path);
                    } else {
                        local.push((depth, name, path));
                    }
                }
            }
            _ if is_word(i)
                && t.get(i + 1).is_some_and(|n| n == "::")
                && (i == 0 || !matches!(t[i - 1].as_str(), "::" | ".")) =>
            {
                let mut paths = vec![];
                i = read_path(t, i, false, vec![], &mut paths);
                // `crate::log!()` is an exported macro, wherever it is written: the root holds no code of it.
                let called =
                    t.get(i).is_some_and(|n| n == "!") && t.get(i + 1).is_none_or(|n| n != "=");
                for (path, _) in paths {
                    if called && path.len() == 2 && path[0] == "crate" {
                        continue;
                    }
                    found
                        .uses
                        .push((module.to_string(), through_local(&local, path)));
                }
            }
            _ => i += 1,
        }
    }
    i
}

fn read_module(src: &Path, module: &str, file: &Path, found: &mut Found) {
    found.files.insert(file.to_path_buf());
    match std::fs::read_to_string(file) {
        Ok(text) => {
            walk(src, module, &tokens(&text), 0, found);
        }
        // rustc may never load it: a #[cfg] module in another encoding builds.
        Err(e) => {
            found.modules.insert(module.to_string());
            found.holes.push(format!("{module} cannot be read: {e}"));
        }
    }
}

/// Where a path starts: its `crate`, `self` and `super` walked off from `module`, and what is left of it.
fn anchor<'a>(module: &str, path: &'a [String]) -> (String, &'a [String]) {
    let mut at = module.to_string();
    let mut rest = path;
    while let Some((first, tail)) = rest.split_first() {
        match first.as_str() {
            "crate" => at = "crate".to_string(),
            "self" => {}
            "super" => {
                at = at
                    .rsplit_once("::")
                    .map_or("crate", |(up, _)| up)
                    .to_string()
            }
            _ => break,
        }
        rest = tail;
    }
    (at, rest)
}

/// The module a path lands in, its longest prefix that is one. A bare first word counts only as a child of this module,
/// else it is another crate's or a type.
// ponytail: a type named as a child module reads as that child; a name from `use super::*` or through a chain of module-level `use`s is no edge, and
// a macro's body counts where it is written, not where it is called, and `use crate::log;` of a macro reads as the root. Enough while the crates hold a few modules each;
// rust-analyzer's resolver if that ever misleads the gate.
fn resolve(module: &str, path: &[String], modules: &BTreeSet<String>) -> String {
    let (mut at, rest) = anchor(module, path);
    for word in rest {
        let inner = format!("{at}::{word}");
        if !modules.contains(&inner) {
            break;
        }
        at = inner;
    }
    at
}

/// A path whose first word, after its `crate::` or `super::`, is what a `use` there brought in: written out in full, with the
/// module to read it from. `crate::store::X` under the root's `use dash_store as store;` is another crate's, so no edge.
fn through_import(module: &str, path: &[String], found: &Found) -> (String, Vec<String>) {
    let (at, rest) = anchor(module, path);
    if let Some((first, tail)) = rest.split_first()
        && !found.modules.contains(&format!("{at}::{first}"))
        && let Some(import) = found.imports.get(&(at.clone(), first.clone()))
    {
        let bare = anchor(&at, import).1;
        let ours = bare.len() < import.len()
            || bare
                .first()
                .is_some_and(|word| found.modules.contains(&format!("{at}::{word}")));
        return if ours {
            (at, [import.as_slice(), tail].concat())
        } else {
            (module.to_string(), vec![])
        };
    }
    (module.to_string(), path.to_vec())
}

/// A crate's `src`, read from its one root. Two roots are two `crate::` namespaces this reads as one; a file no module reaches
/// (src/bin, `include!`, `#[path]`) is code the graph does not hold. Either is a hole.
fn read_crate(src: &Path) -> Found {
    let mut found = Found::default();
    let roots: Vec<PathBuf> = ["lib.rs", "main.rs"]
        .iter()
        .map(|root| src.join(root))
        .filter(|file| file.exists())
        .collect();
    for root in &roots {
        read_module(src, "crate", root, &mut found);
    }
    if roots.len() != 1 {
        found.holes.push(format!("{} crate roots", roots.len()));
    }
    if src.is_dir() {
        for file in rust_files(src) {
            if !found.files.contains(&file) {
                let name = file.strip_prefix(src).unwrap_or(&file).display();
                found.holes.push(format!("{name} is read by no module"));
            }
        }
    }
    found.holes.sort();
    found
}

/// A crate's modules and which names which, from its `src`: `crate` is its root. Declaring a child is no edge, naming one is:
/// a parent that only declares a child runs none of its code, and a test module's `use super::*` would loop on every parent.
pub fn module_graph(src: &Path) -> (Vec<String>, Vec<Edge>) {
    graph_of(read_crate(src))
}

fn graph_of(found: Found) -> (Vec<String>, Vec<Edge>) {
    let edges: BTreeSet<Edge> = found
        .uses
        .iter()
        .map(|(from, path)| {
            let (base, path) = through_import(from, path, &found);
            (from.clone(), resolve(&base, &path, &found.modules))
        })
        .filter(|(from, to)| from != to)
        .collect();
    (
        found.modules.into_iter().collect(),
        edges.into_iter().collect(),
    )
}

/// The workspace's crates under `root`, by folder.
fn crate_names(root: &Path) -> Vec<String> {
    let entries = std::fs::read_dir(root.join("crates")).expect("read crates");
    let mut names: Vec<String> = entries
        .map(|e| e.expect("entry").path())
        .filter(|p| p.join("Cargo.toml").exists())
        .map(|p| {
            p.file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// `from -> to` between the workspace's crates, by their folders, read from each Cargo.toml as the gate reads `cargo metadata`.
/// `the_graph_keeps_to_section_3` holds the two equal, so a form this does not read (`[dependencies.dash-x]`, a renamed key) fails there.
pub fn crate_graph(root: &Path) -> (Vec<String>, Vec<Edge>) {
    let names = crate_names(root);
    let mut edges = BTreeSet::new();
    for name in &names {
        let manifest = root.join("crates").join(name).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest).expect("read a manifest");
        let (mut in_deps, mut in_dev) = (false, false);
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.starts_with('[') {
                in_deps = line.ends_with("dependencies]");
                in_dev = line.ends_with("dev-dependencies]");
            } else if in_deps
                && let Some(key) = line.split(['.', ' ', '=']).next()
                && let Some(to) = key.strip_prefix("dash-")
                && names.iter().any(|n| n == to)
                && !is_tooling_edge(in_dev, to)
            {
                edges.insert((name.clone(), to.to_string()));
            }
        }
    }
    (names, edges.into_iter().collect())
}

/// The module loop of each crate under `root`, as `crate: path`.
pub fn module_cycles(root: &Path) -> Vec<String> {
    let in_crate = |name: &String| {
        let (_, edges) = module_graph(&root.join("crates").join(name).join("src"));
        dependency_loop(&edges).map(|path| format!("{name}: {}", path.join(" ")))
    };
    crate_names(root).iter().filter_map(in_crate).collect()
}

/// What each crate under `root` holds that the walk could not read, as `crate: what`.
pub fn module_holes(root: &Path) -> Vec<String> {
    let in_crate = |name: &String| {
        let found = read_crate(&root.join("crates").join(name).join("src"));
        let named = found
            .holes
            .into_iter()
            .map(|hole| format!("{name}: {hole}"));
        named.collect::<Vec<_>>()
    };
    crate_names(root).iter().flat_map(in_crate).collect()
}

/// What each node names, one line each, tab-separated as /search answers: `from<TAB>to, to`.
fn table(names: &[String], edges: &[Edge]) -> String {
    let row = |name: &String| {
        let to: Vec<&str> = edges
            .iter()
            .filter(|(f, _)| f == name)
            .map(|(_, t)| t.as_str())
            .collect();
        format!("{name}\t{}\n", to.join(", "))
    };
    let rows: String = names.iter().map(row).collect();
    format!("This\tDepends on\n{rows}")
}

/// A hole can hold a file's name: on one line, so it cannot fake a row or rewrite what is shown. The copy of crates/dash's
/// `tasks::one_line`, which links this crate only to build.
fn one_line(words: &str) -> String {
    let hidden = |c: char| {
        c.is_control()
            || matches!(c, '\u{ad}' | '\u{61c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
    };
    words
        .chars()
        .map(|c| if hidden(c) { ' ' } else { c })
        .collect()
}

/// The page, from the code alone, as plain text: no screen is drawn until rust-ui-tree settles the rendering.
pub fn page(root: &Path) -> String {
    let (crates, crate_edges) = crate_graph(root);
    let mut modules = String::new();
    let mut flat = vec![];
    let mut holes = vec![];
    for name in &crates {
        let found = read_crate(&root.join("crates").join(name).join("src"));
        holes.extend(found.holes.iter().map(|hole| format!("{name}: {hole}")));
        let (nodes, edges) = graph_of(found);
        // A test module is a leaf on its parent and would be most of the page.
        let shown = |m: &String| !m.split("::").any(|word| word == "tests");
        let nodes: Vec<String> = nodes.into_iter().filter(shown).collect();
        let edges: Vec<Edge> = edges
            .into_iter()
            .filter(|(f, t)| shown(f) && shown(t))
            .collect();
        if nodes.len() > 1 {
            modules += &format!("{name}\n{}\n", table(&nodes, &edges));
        } else {
            flat.push(name.as_str());
        }
    }
    let holes = if holes.is_empty() {
        String::new()
    } else {
        format!(
            "\nNot read, so not in these graphs: {}.\n",
            one_line(&holes.join("; "))
        )
    };
    format!(
        "Rust dependencies\nTasks: /tasks\nGenerated from the code at each build. The gate fails on a loop in either graph.\n\nCrates\n{}\nModules\n{modules}One module each: {}.\n{holes}",
        table(&crates, &crate_edges),
        flat.join(", ")
    )
}

/// Section 3's layer of a crate, the first list that names it; a crate none names is « Other », which the gate refuses
/// (`the_diagram_nests_each_crate_in_its_layer_with_every_edge`) and a build still draws.
pub fn layer(name: &str) -> &'static str {
    match name {
        BINARY => "Binary",
        "core" => "Core",
        _ if PLUGINS.contains(&name) => "Plugins",
        _ if TASK_LAYERS.contains(&name) => "Task layers",
        _ if RINGS.contains(&name) => "Rings",
        _ if TOOLING.contains(&name) => "Tooling",
        _ => "Other",
    }
}

/// The layers in the order the diagram stacks them.
const LAYERS: [&str; 7] = [
    "Binary",
    "Plugins",
    "Task layers",
    "Rings",
    "Core",
    "Tooling",
    "Other",
];

/// The crate graph as Mermaid, each layer a subgraph holding its crates: the diagram /deps draws (task rust-deps-view;
/// @nha, 2026-11-09: « that tab is unreadable as-is », « there has to be off-the shelf techniques or even libs we can reuse
/// here »). Ids carry a `c_` so a crate named like a Mermaid word (`end`, `graph`) stays a node.
pub fn mermaid(root: &Path) -> String {
    let (crates, edges) = crate_graph(root);
    let mut out = String::from("flowchart LR\n");
    for (i, title) in LAYERS.iter().enumerate() {
        let held: Vec<&String> = crates.iter().filter(|c| layer(c) == *title).collect();
        if held.is_empty() {
            continue;
        }
        out += &format!("  subgraph l{i}[\"{title}\"]\n");
        for name in held {
            out += &format!("    c_{name}[\"{name}\"]\n");
        }
        out += "  end\n";
    }
    for (from, to) in edges {
        out += &format!("  c_{from} --> c_{to}\n");
    }
    out
}

/// What the binary's build script runs: the page into `out` and its diagram beside it (`deps.mmd`), and cargo told to run it
/// again when the graphs can change: a crate's manifest or source, or the lockfile, which a new crate changes. A test's or an
/// extension's file rebuilds nothing.
pub fn write_page(root: &Path, out: &Path) {
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.lock").display()
    );
    for name in crate_names(root) {
        let folder = root.join("crates").join(name);
        println!(
            "cargo:rerun-if-changed={}",
            folder.join("Cargo.toml").display()
        );
        println!("cargo:rerun-if-changed={}", folder.join("src").display());
    }
    std::fs::write(out, page(root)).expect("write the page");
    std::fs::write(out.with_file_name("deps.mmd"), mermaid(root)).expect("write the diagram");
}
