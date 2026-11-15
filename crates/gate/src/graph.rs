//! The crate and module graphs, read from the source, for the gate and the /deps page (@nha, 2026-11-06: « graph of
//! dependencies, ideally we forbid cycles and we have an evergreen rust page in the rust dash »). A small lexer, not syn:
//! syn leaves a macro's body opaque, and `assert_eq!(tasks::show(), ..)` is a use like any other. /deps reads the code map
//! dash builds from these lists and rust-analyzer's index (crates/dash/src/codemap.rs).
use crate::{
    AGENT_KINDS, BINARY, LEAVES, PLUGINS, RINGS, SDK_CRATE, TASK_LAYERS, TOOLING, bad_edges,
    dependency_loop, is_tooling_edge, rust_files,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub type Edge = (String, String);

/// The size the cap holds: tokens, comments and blank lines left out, so neither a formatter's line breaks nor a comment moves
/// it (@nha, 2026-11-09: « we are not trying to artificially cut lines », « newlines vs no newlines does not matter at all »).
pub fn token_count(files: &[PathBuf]) -> usize {
    let read = |f: &PathBuf| std::fs::read_to_string(f).expect("read");
    files.iter().map(|f| tokens(&read(f)).len()).sum()
}

/// Identifiers, `::` and each other mark, comments, strings and char literals gone: a brace or a path in those is no code.
pub(crate) fn tokens(source: &str) -> Vec<String> {
    let code = |l: Lexeme| match l {
        Lexeme::Code(c) => Some(c),
        Lexeme::Str(_) => None,
    };
    lexed(source).into_iter().filter_map(code).collect()
}

/// What each string literal of this source holds, raw ones too, escapes as written. Comments are not in it.
pub(crate) fn strings(source: &str) -> Vec<String> {
    let string = |l: Lexeme| match l {
        Lexeme::Str(s) => Some(s),
        Lexeme::Code(_) => None,
    };
    lexed(source).into_iter().filter_map(string).collect()
}

/// Each of `AGENT_KINDS` this source's code holds, strings included and comments left out, in any case and inside a longer
/// name: `ClaudeRun` and `CODEX_HOME` name one too. Here, not in lib.rs: this module reads the crate root, so the root
/// reading it back is a module cycle.
pub fn agents_named(source: &str) -> Vec<&'static str> {
    let code = [tokens(source), strings(source)]
        .concat()
        .join("\n")
        .to_lowercase();
    AGENT_KINDS
        .into_iter()
        .filter(|a| code.contains(a))
        .collect()
}

pub use crate::lex::{Lexeme, lexed};

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
        for to in manifest_edges(name, &text, &names) {
            edges.insert((name.clone(), to));
        }
    }
    (names, edges.into_iter().collect())
}

/// The crates of `names` that crate `name`'s Cargo.toml `text` depends on, tooling edges left out: [`crate_graph`]'s reader,
/// which binary.rs's placement check reads a branch's manifests with.
pub fn manifest_edges(name: &str, text: &str, names: &[String]) -> Vec<String> {
    let (mut in_deps, mut in_dev) = (false, false);
    let mut edges = vec![];
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_deps = line.ends_with("dependencies]");
            in_dev = line.ends_with("dev-dependencies]");
        } else if in_deps
            && let Some(key) = line.split(['.', ' ', '=']).next()
            && let Some(to) = key.strip_prefix("dash-")
            && names.iter().any(|n| n == to)
            && !is_tooling_edge(name, in_dev, to)
        {
            edges.push(to.to_string());
        }
    }
    edges
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

/// Section 3's layer of a crate, the first list that names it; a crate none names is « Other », which the gate refuses
/// (`the_diagram_nests_each_crate_in_its_layer_with_every_edge`) and a build still draws.
pub fn layer(name: &str) -> &'static str {
    match name {
        BINARY => "Binary",
        "core" | SDK_CRATE => "Core",
        _ if PLUGINS.contains(&name) => "Plugins",
        _ if TASK_LAYERS.contains(&name) => "Task layers",
        // a leaf is its ring's heavy part, in a crate of its own
        _ if RINGS.contains(&name) || LEAVES.contains(&name) => "Rings",
        _ if TOOLING.contains(&name) => "Tooling",
        _ => "Other",
    }
}

/// What to do about each rule's finding, the one text the gate's asserts and /deps's health section both print (task
/// rust-code-health; @nha, 2026-11-10: « It will be for the "code" tab. »).
pub const EDGE_FIX: &str = "depend on the core and take the other crate through its port, or add the edge to section 3 \
     and NAMED_EDGES (gate/src/lib.rs) first";
pub const CRATE_LOOP_FIX: &str =
    "move what both crates need into the lower one, or behind a core port the upper one implements";
pub const MODULE_LOOP_FIX: &str = "move what both sides use into a module below them; a test that needs its parent's \
     item goes in the parent's tests or the crate's tests/ folder";
pub const PLACE_FIX: &str =
    "name it in one of dash-gate's lists (RINGS, LEAVES, TOOLING, TASK_LAYERS, PLUGINS)";
pub const PORT_FIX: &str =
    "add dash-core to its dependencies and take what it needs through a core port";
pub const HOLE_FIX: &str = "give each module its own file (no #[path], no include!), declare it with `mod` from its \
     parent, one crate root";

/// One finding of the gate's graph rules under `root`: the rule's name, what it found, its fix.
pub type Violation = (&'static str, String, &'static str);

/// Every finding of the gate's graph rules under `root` (layers, ports, crate and module loops, unread modules), the
/// workspace as `crate_graph` reads it: the list the gate refuses on, with its fix, for the health section of /deps.
pub fn violations(root: &Path) -> Vec<Violation> {
    let (names, edges) = crate_graph(root);
    let unplaced = (names.iter())
        .filter(|c| layer(c) == "Other")
        .map(|c| ("unplaced crate", c.clone(), PLACE_FIX));
    let on_port = |c: &&String| {
        (edges.iter()).any(|(from, to)| from == *c && (to == "core" || to == SDK_CRATE))
    };
    let portless = (names.iter())
        .filter(|c| TASK_LAYERS.contains(&c.as_str()) || PLUGINS.contains(&c.as_str()))
        .filter(|c| !on_port(c))
        .map(|c| ("no core port", c.clone(), PORT_FIX));
    let edge = bad_edges(&edges)
        .into_iter()
        .map(|e| ("layer edge", e, EDGE_FIX));
    let crate_loop =
        dependency_loop(&edges).map(|p| ("crate loop", p.join(" -> "), CRATE_LOOP_FIX));
    let modules = module_cycles(root)
        .into_iter()
        .map(|c| ("module loop", c, MODULE_LOOP_FIX));
    let holes = module_holes(root)
        .into_iter()
        .map(|h| ("unread module", h, HOLE_FIX));
    let found = unplaced.chain(portless).chain(edge).chain(crate_loop);
    found.chain(modules).chain(holes).collect()
}

/// dash-core, whose root a plugin not yet moved to dash-sdk ([`SDK_CRATE`], task untangle-sdk-crate) still takes: both
/// roots pass the reach count until each plugin's Cargo.toml lists dash-sdk alone (`a_plugin_takes_dash_sdk_alone_held_per_crate`).
pub const SDK: &str = "core";

/// The SDK's public API: the names its crate root declares or re-exports with `pub`, never its `pub mod`s. A path through
/// a module (`dash_core::tasks::Status`) reaches past it, whatever the compiler lets through: the root is the one small
/// surface a plugin may hold to (ARCHITECTURE.md section 3), so a name leaves a module for it by a `pub use` there.
// ponytail: an exported macro (`#[macro_export]`) and a glob re-export at the root are not read; neither is in the core.
fn root_api(src: &Path) -> BTreeSet<String> {
    const ITEM_KINDS: [&str; 8] = [
        "fn",
        "struct",
        "enum",
        "trait",
        "type",
        "union",
        "crate",
        "macro_rules",
    ];
    let root = ["lib.rs", "main.rs"].map(|r| src.join(r));
    let Some(text) = root.iter().find_map(|f| std::fs::read_to_string(f).ok()) else {
        return BTreeSet::new();
    };
    let t = tokens(&text);
    let (mut api, mut depth, mut i) = (BTreeSet::new(), 0, 0);
    while let Some(token) = t.get(i) {
        match token.as_str() {
            "{" | "(" | "[" => depth += 1,
            "}" | ")" | "]" => depth -= 1,
            "pub" if depth == 0 && t.get(i + 1).is_some_and(|n| n != "(") => {
                let mut at = i + 1;
                while t.get(at).is_some_and(|w| {
                    matches!(
                        w.as_str(),
                        "async" | "unsafe" | "extern" | "const" | "static" | "mut"
                    )
                }) {
                    at += 1;
                }
                match t.get(at).map(String::as_str) {
                    Some("use") => {
                        let mut paths = vec![];
                        i = read_path(&t, at + 1, true, vec![], &mut paths);
                        api.extend(paths.into_iter().map(|(_, name)| name));
                        continue;
                    }
                    Some("mod") | None => {}
                    // `pub const X`, `pub static mut X`: the qualifier was the keyword, the name is here
                    Some(name) if !ITEM_KINDS.contains(&name) => {
                        api.insert(name.to_string());
                    }
                    Some(_) => api.extend(t.get(at + 1).cloned()),
                }
            }
            _ => {}
        }
        i += 1;
    }
    api
}

/// Each path a crate's `src` writes into a crate of `ours` (folder names), as `(module, ["dash_core", "tasks", ..])`: a
/// name a `use` brought in is written out in full, so `use dash_core as sdk;` then `sdk::tasks::X` is `dash_core::tasks::X`.
// ponytail: a path written with a leading `::` is not read, and an alias is followed one `use` deep, from its own module
// (`use crate::sdk;` of a root's `use dash_core as sdk;` is not); the workspace writes neither. rust-analyzer's resolver
// is the upgrade if one appears.
pub fn reaches(src: &Path, ours: &[String]) -> Vec<(String, Vec<String>)> {
    let found = read_crate(src);
    let ours: BTreeSet<String> = ours
        .iter()
        .map(|c| format!("dash_{}", c.replace('-', "_")))
        .collect();
    let mut out = BTreeSet::new();
    for (module, path) in &found.uses {
        let (at, rest) = anchor(module, path);
        let full = match rest.split_first() {
            Some((first, tail)) => match found.imports.get(&(at, first.clone())) {
                Some(import) if import.first().is_some_and(|w| ours.contains(w)) => {
                    [import.as_slice(), tail].concat()
                }
                _ => path.clone(),
            },
            None => path.clone(),
        };
        if full.first().is_some_and(|w| ours.contains(w)) {
            out.insert((module.clone(), full));
        }
    }
    out.into_iter().collect()
}

/// What `plugin`'s paths reach past the SDK's public `api`: anything in another crate of ours, and in the SDK anything but
/// its root's names. Its own crate and a test's tooling (the meter, a plugin's design system: cargo's graph holds them to
/// dev-dependencies) are no reach. Each as `plugin: dash_x::path`, cut after its first capitalised word (a type or a
/// constant: `Screen::Keys` is `Screen`), only the longest of a path and its prefixes.
pub fn past_the_sdk(
    plugin: &str,
    paths: &[(String, Vec<String>)],
    api: &BTreeSet<String>,
) -> Vec<String> {
    let item = |p: &Vec<String>| {
        let cut = p
            .iter()
            .skip(1)
            .position(|w| w.starts_with(char::is_uppercase));
        p[..cut.map_or(p.len(), |c| c + 2)].join("::")
    };
    let reach = |path: &&Vec<String>| {
        let to = path[0].trim_start_matches("dash_").replace('_', "-");
        // a glob hides which names it takes, its root's modules among them: it counts
        let root = path
            .get(1)
            .is_none_or(|n| api.contains(&format!("{to}::{n}")));
        let sdk = to == SDK || to == SDK_CRATE;
        to != plugin && !is_tooling_edge(plugin, true, &to) && !(sdk && root)
    };
    let found: BTreeSet<String> = paths
        .iter()
        .map(|(_, p)| p)
        .filter(reach)
        .map(item)
        .collect();
    let longest = |p: &&String| {
        !found
            .iter()
            .any(|q| q.len() > p.len() && q.starts_with(&format!("{p}::")))
    };
    found
        .iter()
        .filter(longest)
        .map(|p| format!("{plugin}: {p}"))
        .collect()
}

/// The parts of a plugin crate (ARCHITECTURE.md section 2 point 5): `crate::tool` and below is its tool, `crate::ui` and
/// below its UI components, the rest is the rest.
fn part(module: &str) -> &'static str {
    if under(module, "crate::tool") {
        "tool"
    } else if under(module, "crate::ui") {
        "ui"
    } else {
        "rest"
    }
}

/// `module` is `path` or one of its children.
fn under(module: &str, path: &str) -> bool {
    module == path || module.starts_with(&format!("{path}::"))
}

/// Where `plugin`'s tool and UI components reach outside their part: a module of the plugin's other parts or root, or
/// past the SDK's public `api`. The rest may take its own tool, its components and the SDK, which [`past_the_sdk`] holds.
fn split_breaks(
    plugin: &str,
    src: &Path,
    reaches: &[(String, Vec<String>)],
    api: &BTreeSet<String>,
) -> Vec<String> {
    let (_, edges) = module_graph(src);
    let inside = edges
        .iter()
        .filter(|(from, to)| part(from) != "rest" && part(from) != part(to))
        .map(|(from, to)| format!("{plugin}: {from} -> {to}"));
    let paths: Vec<_> = reaches
        .iter()
        .filter(|(m, _)| part(m) != "rest")
        .cloned()
        .collect();
    let outside = past_the_sdk(plugin, &paths, api);
    inside.chain(outside).collect()
}

/// Each loop among a crate's top-level modules, its root `crate` among them, as the modules in it: the module graph with
/// `crate::a::b` read as `crate::a`. A module loop is refused at any depth (`module_cycles`); this one is the tangle that
/// keeps a feature from leaving for a plugin crate, where its parts must go together.
pub fn feature_loops(src: &Path) -> Vec<Vec<String>> {
    let top = |m: &str| m.splitn(3, "::").take(2).collect::<Vec<_>>().join("::");
    let (modules, edges) = module_graph(src);
    let mut next: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (from, to) in &edges {
        let (from, to) = (top(from), top(to));
        if from != to {
            next.entry(from).or_default().insert(to);
        }
    }
    let reached = |start: &str| {
        let (mut seen, mut stack) = (BTreeSet::new(), vec![start.to_string()]);
        while let Some(at) = stack.pop() {
            for to in next.get(&at).into_iter().flatten() {
                if seen.insert(to.clone()) {
                    stack.push(to.clone());
                }
            }
        }
        seen
    };
    let tops: BTreeSet<String> = modules.iter().map(|m| top(m)).collect();
    let reach: BTreeMap<&String, BTreeSet<String>> = tops.iter().map(|m| (m, reached(m))).collect();
    let mut loops = BTreeSet::new();
    for (m, from_m) in &reach {
        if from_m.contains(*m) {
            let mates = from_m.iter().filter(|o| reach[o].contains(*m)).cloned();
            loops.insert(mates.collect::<Vec<_>>());
        }
    }
    loops.into_iter().collect()
}

/// The modules in a loop of [`feature_loops`], over every crate under `root`, and each loop as `crate: a b c`.
pub fn feature_loop_count(root: &Path) -> (usize, Vec<String>) {
    let each: Vec<(String, Vec<String>)> = crate_names(root)
        .into_iter()
        .flat_map(|name| {
            let loops = feature_loops(&root.join("crates").join(&name).join("src"));
            loops.into_iter().map(move |l| (name.clone(), l))
        })
        .collect();
    let count = each.iter().map(|(_, l)| l.len()).sum();
    (
        count,
        each.iter()
            .map(|(c, l)| format!("{c}: {}", l.join(" ")))
            .collect(),
    )
}

/// Every plugin and task layer's reaches past the SDK under `root` ([`past_the_sdk`]) and its split's breaks
/// ([`split_breaks`]).
pub fn plugin_reaches(root: &Path) -> (Vec<String>, Vec<String>) {
    let ours = crate_names(root);
    // dash-sdk's root, and dash-core's while a plugin still takes it (task untangle-sdk-crate), each name as `<crate>::<name>`
    let api: BTreeSet<String> = [SDK, SDK_CRATE]
        .into_iter()
        .flat_map(|c| {
            root_api(&root.join("crates").join(c).join("src"))
                .into_iter()
                .map(move |n| format!("{c}::{n}"))
        })
        .collect();
    let (mut past, mut breaks) = (vec![], vec![]);
    for plugin in PLUGINS
        .iter()
        .chain(&TASK_LAYERS)
        .filter(|p| ours.iter().any(|c| c == *p))
    {
        let src = root.join("crates").join(plugin).join("src");
        let paths = reaches(&src, &ours);
        past.extend(past_the_sdk(plugin, &paths, &api));
        breaks.extend(split_breaks(plugin, &src, &paths, &api));
    }
    (past, breaks)
}

/// The binary's helpers its feature modules reach (ARCHITECTURE.md section 5 rule 17): each leaves for the SDK or the Host.
const BINARY_HELPERS: [&str; 3] = ["crate::pages", "crate::usage", "crate::tasks"];

/// What the binary's `src` registers itself, rule 16: each `inventory::submit!`, as `file: Page`, `file: Command` or its
/// other type (a `Warmer`), and each `dash_core::pane!`, a menu entry with its page or alone, as `file: Pane` (task
/// rust-plugin-settings: the macro holds no `submit` token of the binary's, so it is named here); a move takes its own to its
/// plugin crate.
pub fn binary_registrations(src: &Path) -> Vec<String> {
    let mut found = vec![];
    for file in rust_files(src) {
        let t = tokens(&std::fs::read_to_string(&file).expect("read"));
        let name = file
            .strip_prefix(src)
            .unwrap_or(&file)
            .display()
            .to_string();
        let panes =
            (0..t.len()).filter(|&i| t[i] == "pane" && t.get(i + 1).is_some_and(|n| n == "!"));
        found.extend(panes.map(|_| format!("{name}: Pane")));
        for at in
            (0..t.len()).filter(|&i| t[i] == "submit" && t.get(i + 1).is_some_and(|n| n == "!"))
        {
            let path = t
                .iter()
                .skip(at + 3)
                .take_while(|w| !matches!(w.as_str(), "{" | "(" | "}"));
            // every registration counts, by its type's name, or the static's or call's it names (`submit! { PAGE }`)
            let kind = path.filter(|w| w.starts_with(char::is_uppercase)).last();
            found.push(format!("{name}: {}", kind.map_or("submit", String::as_str)));
        }
    }
    found.sort();
    found
}

/// The binary's modules naming one of [`BINARY_HELPERS`] from outside it, rule 17: each `from -> helper` once.
pub fn binary_helper_uses(src: &Path) -> Vec<String> {
    let (_, edges) = module_graph(src);
    let helper = |m: &str| BINARY_HELPERS.into_iter().find(|h| under(m, h));
    edges
        .iter()
        .filter_map(|(from, to)| {
            let h = helper(to)?;
            (helper(from) != Some(h)).then(|| format!("{from} -> {h}"))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
