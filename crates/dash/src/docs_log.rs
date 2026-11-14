//! The docs table and Rust's store of documents on it (task editor-in-rfcs), below every reader: docs.rs (/v/ and the docs
//! pass) and the comment agent, which rewrites a section of one (task rust-doc-cowrite), write it, the views tasks.rs starts
//! read it, and none reaches another (dash-gate's module-cycle rule). `docs.sqlite`'s `docs` table on the op log, keyed by a
//! page's repo path, read in from its HTML file by `dash doc import` and written back as its export by `dash doc save`.
//! doc_store.rs's half joined it, as a module new to the binary fails rust-binary-ratchet; both leave for the docs plugin
//! (task rust-move-docs).
use dash_core::{FieldWrite, Value as Field};
use dash_store::oplog::Replica;
use dash_surface as surface;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const TABLE: &str = "docs";
const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS docs (id TEXT PRIMARY KEY, record TEXT, body TEXT) WITHOUT ROWID;";

/// The docs table's op log, `body` added to a table made before it (the schema is judged at open, so before the replica).
pub fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("docs.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    let has_body = (db.prepare("SELECT 1 FROM pragma_table_info('docs') WHERE name = 'body'"))
        .and_then(|mut q| q.exists([]))
        .map_err(|e| e.to_string())?;
    // two processes opening an older file at once both add it: the second one's duplicate is the first one's column (/code-review)
    if !has_body
        && let Err(e) = db.execute_batch("ALTER TABLE docs ADD COLUMN body TEXT")
        && !e.to_string().contains("duplicate column name")
    {
        return Err(e.to_string());
    }
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// The folder whose pages Rust serves; the older app keeps every other /v/ path.
pub const FOLDER: &str = "private/docs/";

/// The repo path a /v/ request names, when it is a page of private/docs: decoded, with no way out of the folder.
pub fn doc_path(request: &str) -> Option<String> {
    let path = crate::pages::segment_decoded(request.strip_prefix("/v/")?);
    let name = path.strip_prefix(FOLDER)?;
    let plain =
        !name.is_empty() && !name.contains('/') && !name.contains('\\') && !name.starts_with('.');
    (plain && name.ends_with(".html")).then_some(path)
}

/// A document as the store keeps it, and its file's fingerprint when the store last read or wrote that file: the file is
/// still edited by hand and by agents (the rfc skill), so one that changed since is read in again, never shadowed.
pub struct Kept {
    pub doc: Value,
    pub file: String,
}

/// A file's bytes as a fingerprint under the importer's reader: a doc kept by an older reader reads in again.
pub(crate) use dash_ingest::doc::print_of;

/// The document kept for `path`, with its file's fingerprint then. ponytail: one indexed row read, never the whole op log
/// (dash_comments' read_table scans every op, fine for its few thousand rows, not for 71 documents of up to 500 KB).
pub fn stored(dir: &Path, path: &str) -> Result<Option<Kept>, String> {
    let log = log(dir)?;
    let records = log
        .read(|db| -> Result<Vec<String>, String> {
            let mut q = db
                .prepare("SELECT record FROM docs WHERE id = ?1")
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([path], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())??;
    let kept = |r: &String| -> Result<Kept, String> {
        let v: Value = serde_json::from_str(r).map_err(|e| format!("{path}: {e}"))?;
        let file = v["file"].as_str().unwrap_or_default().to_string();
        Ok(Kept {
            doc: v["doc"].clone(),
            file,
        })
    };
    records.first().map(kept).transpose()
}

/// `doc` (none: its record left as it was) and the file's `body` kept as `path`'s, in one commit; `file` the print of `body`.
fn keep(dir: &Path, path: &str, doc: Option<&Value>, file: &str, body: &str) -> Result<(), String> {
    keep_if(&mut log(dir)?, path, None, doc, file, body).map(drop)
}

/// `keep` on an open log, only while the row's record print is still `base` when given (`Some("")`: no record): a save that
/// landed meanwhile is never written over by an older read. Whether it wrote.
pub(crate) fn keep_if(
    log: &mut Replica,
    path: &str,
    base: Option<&str>,
    doc: Option<&Value>,
    file: &str,
    body: &str,
) -> Result<bool, String> {
    let write = |field: &str, value: String| FieldWrite {
        table: TABLE.into(),
        row: path.into(),
        field: field.into(),
        value: Field::Text(value),
    };
    let record = doc.map(|d| write("record", json!({"doc": d, "file": file}).to_string()));
    let writes: Vec<FieldWrite> = record
        .into_iter()
        .chain([write("body", body.into())])
        .collect();
    // a record print that does not read holds nothing: no write, the doc named behind, read next pass
    let holds = |db: &dash_store::Connection| {
        Ok(base.is_none_or(|base| record_print(db, path).is_ok_and(|p| p == base)))
    };
    let wrote = log.write_if(holds, &writes).map_err(|e| e.to_string())?;
    Ok(wrote.is_some())
}

/// The print of the file a row's record was read from, `""` with no record. ponytail: SQLite's json_extract parses the record
/// in C, one row at a time; a column of its own if a pass's time shows it.
pub(crate) fn record_print(db: &dash_store::Connection, path: &str) -> Result<String, String> {
    let sql = "SELECT json_extract(record, '$.file') FROM docs WHERE id = ?1";
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let rows = q
        .query_map([path], |r| r.get::<_, Option<String>>(0))
        .map_err(|e| e.to_string())?;
    let all = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(all.into_iter().next().flatten().unwrap_or_default())
}

/// The document for `path`: the store's while its file is as the store last saw it, else the file read in again by `dash doc
/// import` and kept (its first read, or an edit made to the file since). `None`: neither holds it.
pub fn opened(dir: &Path, repo: &Path, path: &str) -> Result<Option<Value>, String> {
    let file = repo.join(path);
    let bytes = std::fs::read(&file).ok();
    let now = bytes.as_deref().map(print_of);
    match (stored(dir, path)?, &now) {
        (Some(k), Some(now)) if &k.file == now => return Ok(Some(k.doc)),
        (Some(k), None) => return Ok(Some(k.doc)),
        (None, None) => return Ok(None),
        _ => {}
    }
    let doc = read_in(dir, repo, path, now.as_deref().unwrap_or_default())?;
    let body = String::from_utf8_lossy(bytes.as_deref().unwrap_or_default());
    keep(
        dir,
        path,
        Some(&doc),
        now.as_deref().unwrap_or_default(),
        &body,
    )?;
    Ok(Some(doc))
}

/// `file` read in, else an older READER's doc of these bytes (/code-review of PR 1592).
pub(crate) fn read_in(dir: &Path, repo: &Path, path: &str, now: &str) -> Result<Value, String> {
    let kept = stored(dir, path)?.map(|k| (k.doc, k.file));
    dash_ingest::doc::read_in(&repo.join(path), surface::documents::loads, kept, now)
}

/// `dash doc save`: `doc` kept as the document of `path`, then its HTML file in `repo` written again as its export. A
/// document the Notion view does not read is refused, and so is a save over a file changed since the store last read it (an
/// edit by hand would be lost): nothing is written.
pub fn save(dir: &Path, repo: &Path, path: &str, doc: &Value) -> Result<PathBuf, String> {
    surface::documents::loads(doc)?;
    let file = repo.join(path);
    let seen = stored(dir, path)?.map(|k| k.file);
    if let Some(now) = std::fs::read(&file).ok().map(|b| print_of(&b))
        && seen.as_deref() != Some(now.as_str())
    {
        return Err(format!(
            "{path} changed on disk since the store read it: open /v/{path} again, then save"
        ));
    }
    let export = surface::documents::export(doc.clone());
    std::fs::write(&file, &export).map_err(|e| format!("{}: {e}", file.display()))?;
    keep(dir, path, Some(doc), &print_of(export.as_bytes()), &export)?;
    Ok(file)
}

/// A comment on a document's words, the section the comment agent may rewrite (task rust-doc-cowrite; @nha, 2026-11-12, the
/// done-when: « A comment asking a change makes the agent edit that section, naming the diff »): its page's path, its
/// heading's words and each passage's field number and words, as `passages` numbers them.
pub struct Section {
    pub path: String,
    pub name: String,
    pub fields: Vec<(usize, String)>,
}

/// The section a comment on `page` quoting `quote` is in, when `page` is a document of private/docs and the quote is found.
pub fn section(dir: &Path, repo: &Path, page: &str, quote: &str) -> Option<Section> {
    let path = doc_path(page.split(['?', '#']).next()?)?;
    let doc = opened(dir, repo, &path).ok()??;
    let (name, fields) = surface::documents::section_of(&doc, quote)?;
    Some(Section { path, name, fields })
}

/// Where the agent's reply carries its edit: a fenced block opened by this, one JSON object.
const EDIT_FENCE: &str = "```edit";

impl Section {
    /// What the agent's prompt adds: the section's passages and how to change them. It writes no file: `applied` does.
    pub fn asked(&self) -> String {
        let fields: serde_json::Map<String, Value> = (self.fields.iter())
            .map(|(n, w)| (format!("b{n}"), Value::String(w.clone())))
            .collect();
        format!(
            " This page is a document, and his comment is on its section « {} ». A change he asks to that section's words is \
             yours to make, not a task: end your reply with a block opened by the line {EDIT_FENCE} and closed by ```, holding \
             one JSON object whose keys are the passages you change, each with its new words in full, only among these: {}. \
             Say in one line what you changed; the diff is added under your reply. Work he asks beyond the document's words \
             (code, a feature) is still filed as a task, as above; a question gets no block.",
            self.name,
            Value::Object(fields)
        )
    }

    /// The reply posted under his comment: the agent's words without its edit block, then the edit applied to the document,
    /// kept and its file written (`save`), and its diff; or why it was not applied. A reply with no block is posted as it is.
    pub fn applied(&self, dir: &Path, repo: &Path, reply: &str) -> String {
        let Some(at) = reply.find(EDIT_FENCE) else {
            return reply.to_string();
        };
        let after = &reply[at + EDIT_FENCE.len()..];
        // one JSON value read whole, so a fence inside its words never cuts it short (/code-review)
        let mut read =
            serde_json::Deserializer::from_str(after).into_iter::<serde_json::Map<String, Value>>();
        let (done, rest) = match read.next() {
            Some(Ok(asked)) => {
                let rest = after[read.byte_offset()..].trim_start();
                (
                    self.edit(dir, repo, &asked),
                    rest.strip_prefix("```").unwrap_or(rest),
                )
            }
            Some(Err(e)) => (
                Err(format!("the edit is not one JSON object ({e})")),
                after.split_once("```").map_or("", |(_, r)| r),
            ),
            None => (Err("the edit block holds nothing".into()), ""),
        };
        let said = format!("{}{}", &reply[..at], rest).trim().to_string();
        format!(
            "{said}\n\n{}",
            done.unwrap_or_else(|why| format!("Not applied: {why}"))
        )
    }

    fn edit(
        &self,
        dir: &Path,
        repo: &Path,
        asked: &serde_json::Map<String, Value>,
    ) -> Result<String, String> {
        let mut words: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
        for (key, value) in asked {
            let n = (key.strip_prefix('b').and_then(|n| n.parse().ok()))
                .filter(|n| self.fields.iter().any(|(f, _)| f == n))
                .ok_or(format!("{key} is not a passage of « {} »", self.name))?;
            let new = value
                .as_str()
                .ok_or(format!("{key}'s words are not a string"))?;
            words.insert(n, new.to_string());
        }
        let kept = opened(dir, repo, &self.path)?.ok_or(format!("{} is gone", self.path))?;
        // the passages it was shown, still where they were: else a number names another passage now
        let now = surface::documents::passages(&kept);
        if self
            .fields
            .iter()
            .any(|(n, w)| now.get(*n).map(|p| &p.1) != Some(w))
        {
            return Err(format!(
                "« {} » changed while the agent wrote: comment again",
                self.name
            ));
        }
        let (doc, changed) = surface::documents::edited(&kept, &|n| words.get(&n).cloned());
        if changed == 0 {
            return Ok("No change: the words asked were the words there.".into());
        }
        save(dir, repo, &self.path, &doc)?;
        let mut diff = vec![];
        for (n, old) in &self.fields {
            if let Some(new) = words.get(n).filter(|new| *new != old) {
                diff.extend(old.lines().map(|l| format!("- {l}")));
                diff.extend(new.lines().map(|l| format!("+ {l}")));
            }
        }
        Ok(format!(
            "Edited « {} » in /v/{}:\n````diff\n{}\n````",
            self.name,
            self.path,
            diff.join("\n")
        ))
    }
}
