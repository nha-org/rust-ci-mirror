//! The knowledge base's entries in Rust's store, and a document's glossary as a view of them (task rust-doc-parity-rest; the
//! older app's `Dashboard.Knowledge.view/1`, rfc-dashboard R6; @nha, 2026-10-18: « need sources too and ideally cross link to
//! other entries. Which means we should also hold internal knowledge base internally »). An entry is a term, a company, a
//! project, a person or a decision: its term, meaning and source as HTML, and its home, the doc that defined it. They live in
//! `knowledge.sqlite`'s `knowledge` table on the op log, keyed by id; `dash kb move` reads them in from the older app's backup.
//! A glossary row `<tr id="g-<id>">` whose entry the store holds shows the entry, its term linked to /k/<id>.
use dash_core::{FieldWrite, Value as Field};
use dash_store::oplog::Replica;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const TABLE: &str = "knowledge";
const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS knowledge (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";

fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("knowledge.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}
/// The older app's backup of its entries, a JSON line each: the `store` branch `dash store dump` writes daily.
const BACKUP: &str = "origin/store:knowledge.jsonl";

/// The entries the store holds of `ids`, each its record by id.
pub fn entries(dir: &Path, ids: &[String]) -> Result<HashMap<String, Value>, String> {
    let log = log(dir)?;
    let rows = log
        .read(|db| -> Result<Vec<(String, String)>, String> {
            let mut q = (db.prepare("SELECT id, record FROM knowledge WHERE id = ?1"))
                .map_err(|e| e.to_string())?;
            let mut out = vec![];
            for id in ids {
                let rows = (q.query_map([id], |r| Ok((r.get(0)?, r.get(1)?))))
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    out.push(row.map_err(|e| e.to_string())?);
                }
            }
            Ok(out)
        })
        .map_err(|e| e.to_string())??;
    rows.into_iter()
        .map(|(id, r)| {
            let entry = serde_json::from_str(&r).map_err(|e| format!("{id}: {e}"))?;
            Ok((id, entry))
        })
        .collect()
}

/// The older app's entries, a JSON line each, kept in `dir`'s store: only an entry new or changed is written. A line saying how many.
pub fn move_from(dir: &Path, text: &str) -> Result<String, String> {
    let mut read = vec![];
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("the entries: {e}"))?;
        let Some(id) = v["id"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        read.push((id.to_string(), v));
    }
    let ids: Vec<String> = read.iter().map(|(id, _)| id.clone()).collect();
    let kept = entries(dir, &ids)?;
    let writes: Vec<FieldWrite> = (read.iter())
        .filter(|(id, v)| kept.get(id) != Some(v))
        .map(|(id, v)| FieldWrite {
            table: TABLE.into(),
            row: id.clone(),
            field: "record".into(),
            value: Field::Text(v.to_string()),
        })
        .collect();
    if !writes.is_empty() {
        log(dir)?.write(&writes).map_err(|e| e.to_string())?;
    }
    Ok(format!(
        "moved {} entries, {} written",
        read.len(),
        writes.len()
    ))
}

/// `dash kb move --data <dir> [--from <knowledge.jsonl>]`: the older app's entries moved into this folder's store, read from
/// its backup on the `store` branch unless a file is named.
pub fn move_entries(dir: &Path, from: Option<&Path>, mut out: std::fs::File) {
    let moved = (|| {
        let text = match from {
            Some(f) => std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?,
            None => {
                let repo = crate::pages::repo();
                let repo = repo.to_string_lossy();
                let git = |args: &[&str]| {
                    let args: Vec<&str> =
                        ["-C", repo.as_ref()].iter().chain(args).copied().collect();
                    dash_process::run("git", &args, None, Duration::from_secs(120))
                };
                git(&["fetch", "-q", "origin", "store"])?;
                git(&["show", BACKUP])?
            }
        };
        move_from(dir, &text)
    })();
    match moved {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("dash kb move: {e}");
            std::process::exit(1);
        }
    }
}

/// `body` with its glossary as a view of the knowledge base (`Dashboard.Knowledge.view/1`): each row whose entry the store
/// holds shows its term linked to /k/<id>, its meaning and source, as the design system writes them and the doc's reader reads.
pub fn glossary(dir: &Path, body: &mut Value) -> Result<(), String> {
    let ids = dash_ingest::glossary::ids(body);
    if ids.is_empty() {
        return Ok(());
    }
    let found = entries(dir, &ids)?;
    if found.is_empty() {
        return Ok(());
    }
    dash_ingest::glossary::view(body, &dash_surface::glossary::cells(&ids, &found))
}
