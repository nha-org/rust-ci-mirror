//! A plugin's `ARCHITECTURE.md` against its code (rust/ARCHITECTURE.md section 4 point 12): its Keys are what the crate
//! registers (`submit!` of a Command, Post, Page, Element, Channel, ContextProvider, Recurring, Tool), its Database the tables
//! its schema creates, both ways, so the doc cannot drift. Read from the crate's source, outside its `#[cfg(test)]` modules.
use regex::Regex;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

/// A key as the doc and the code both say it: its kind, then its words, path or key.
pub type Key = (String, String);

static SUBMIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"submit!\s*\{\s*(?:[a-z_]+::)*([A-Z][A-Za-z]+)\s*\{([^}]*)\}").expect("regex")
});
static FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(words|path|key|name):\s*(&\[[^\]]*\]|"[^"]*"|[A-Z_][A-Z0-9_]*)"#).expect("regex")
});
static CONST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"const ([A-Z_][A-Z0-9_]*): &str =\s*"([^"]*)""#).expect("regex"));
static TABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)CREATE TABLE (?:IF NOT EXISTS )?([a-z_][a-z0-9_]*)").expect("regex")
});
static TICKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("regex"));

/// The doc's kind for a registration's type: a job is a `Recurring`.
fn kind(shape: &str) -> &str {
    match shape {
        "Recurring" => "Job",
        s => s,
    }
}

/// Each source of `src`, its test module cut off.
fn sources(src: &Path) -> Vec<String> {
    let mut out = vec![];
    let mut dirs = vec![src.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                out.push(
                    text.split("#[cfg(test)]")
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                );
            }
        }
    }
    out
}

/// What the crate at `dir` registers and creates: its keys, and its tables.
pub fn in_code(dir: &Path) -> (BTreeSet<Key>, BTreeSet<String>) {
    let texts = sources(&dir.join("src"));
    let all = texts.join("\n");
    let consts: std::collections::BTreeMap<&str, &str> = (CONST.captures_iter(&all))
        .map(|c| {
            (
                c.get(1).map_or("", |m| m.as_str()),
                c.get(2).map_or("", |m| m.as_str()),
            )
        })
        .collect();
    let mut keys = BTreeSet::new();
    for c in SUBMIT.captures_iter(&all) {
        let Some(field) = FIELD.captures(&c[2]) else {
            continue;
        };
        let value = &field[2];
        let key = if value.starts_with("&[") {
            let words = value.trim_start_matches("&[").trim_end_matches(']');
            let words: Vec<&str> = words
                .split(',')
                .map(|w| w.trim().trim_matches('"'))
                .filter(|w| !w.is_empty())
                .collect();
            words.join(" ")
        } else if let Some(s) = value.strip_prefix('"') {
            s.trim_end_matches('"').to_string()
        } else {
            consts.get(value).copied().unwrap_or(value).to_string()
        };
        keys.insert((kind(&c[1]).to_string(), key));
    }
    let tables = TABLE
        .captures_iter(&all)
        .map(|c| c[1].to_string())
        .collect();
    (keys, tables)
}

/// What the doc `text` names: each `- **Kind, Kind:** `key`, ...` line of Keys (a `src/` path is where, not a key), and
/// Database's Tables line.
pub fn in_doc(text: &str) -> (BTreeSet<Key>, BTreeSet<String>) {
    let (mut keys, mut tables, mut section) = (BTreeSet::new(), BTreeSet::new(), "");
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            section = h.trim();
            continue;
        }
        let Some((label, rest)) = line.strip_prefix("- **").and_then(|l| l.split_once(":**"))
        else {
            continue;
        };
        let ticked = TICKED
            .captures_iter(rest)
            .map(|c| c[1].to_string())
            .filter(|t| !t.starts_with("src/"));
        match (section, label) {
            ("Keys", kinds) => {
                let ticked: Vec<String> = ticked.collect();
                for kind in kinds.split(',').map(str::trim) {
                    keys.extend(ticked.iter().map(|t| (kind.to_string(), t.clone())));
                }
            }
            ("Database", "Tables") => tables.extend(ticked),
            _ => {}
        }
    }
    (keys, tables)
}

/// How the doc at `dir`'s ARCHITECTURE.md and the crate disagree, each way: none when they match.
pub fn drift(dir: &Path) -> Vec<String> {
    let doc = std::fs::read_to_string(dir.join("ARCHITECTURE.md")).unwrap_or_default();
    let ((doc_keys, doc_tables), (keys, tables)) = (in_doc(&doc), in_code(dir));
    let mut said = vec![];
    said.extend(
        keys.difference(&doc_keys)
            .map(|(k, v)| format!("registered, not in its Keys: {k} `{v}`")),
    );
    said.extend(
        doc_keys
            .difference(&keys)
            .map(|(k, v)| format!("in its Keys, not registered: {k} `{v}`")),
    );
    said.extend(
        tables
            .difference(&doc_tables)
            .map(|t| format!("created, not in its Tables: `{t}`")),
    );
    said.extend(
        doc_tables
            .difference(&tables)
            .map(|t| format!("in its Tables, not created: `{t}`")),
    );
    said
}
