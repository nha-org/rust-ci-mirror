//! The older app's page comments, moved whole into Rust's store (task editor-in-rfcs, done-when 2; @nha, 2026-11-11: « we
//! need to port documents to the new notion rust that is probably a DB reimport in the new system right ? »). The queue keeps
//! only what waits on an agent; here every thread lands, resolved ones too, his words then each reply, so the page they were
//! written on shows them all. A thread keeps its quote and no position: the page anchors it on the first passage holding
//! those words (dash-surface's DocView), so it follows its paragraph wherever an edit moves it.
use crate::{read_table, segment, text, write_table};
use dash_core::OpLog;
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;

pub const TABLE: &str = "page_threads";

/// One reply, or his first words: who (`nicolas`, `claude`), what, the day.
#[derive(Clone, Debug, PartialEq)]
pub struct Said {
    pub by: String,
    pub text: String,
    pub on: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    /// The page it shows on: `/v/<repo path>`, a page path, or a web page's URL.
    pub page: String,
    /// The words it is about; empty: the page as a whole.
    pub quote: String,
    pub said: Vec<Said>,
    /// Not resolved in the older app.
    pub open: bool,
}

/// The page a comment on `file` shows on, as `link` makes it, with no host and no anchor.
pub fn shown_on(file: &str) -> String {
    match file {
        f if f.starts_with("http://") || f.starts_with("https://") => f.to_string(),
        f if f.starts_with('/') => crate::page_of(f),
        f => {
            let segments: Vec<String> = f.split('/').map(segment).collect();
            format!("/v/{}", segments.join("/"))
        }
    }
}

/// Every thread of the older app's export (`dash store text live/comments.edn`), resolved ones too; a record with no id is
/// no thread anyone can open, and is left out.
pub fn moved(export: &Json) -> Result<Vec<Thread>, String> {
    let all = export.as_array().ok_or("comments: not a list")?;
    let read = |c: &Json| {
        let m = c.as_object()?;
        let id = Some(text(m, "id")).filter(|id| !id.is_empty())?;
        let when = |m: &Map<String, Json>| {
            Some(text(m, "at"))
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| text(m, "date"))
        };
        let first = Said {
            by: "nicolas".into(),
            text: text(m, "text"),
            on: when(m),
        };
        let replies = (m
            .get("replies")
            .and_then(Json::as_array)
            .into_iter()
            .flatten())
        .filter_map(Json::as_object)
        .map(|r| Said {
            by: text(r, "by"),
            text: text(r, "text"),
            on: when(r),
        });
        Some(Thread {
            page: shown_on(&text(m, "file")),
            quote: text(m, "quote"),
            said: std::iter::once(first).chain(replies).collect(),
            open: text(m, "status") != "resolved",
            id,
        })
    };
    Ok(all.iter().filter_map(read).collect())
}

fn to_json(t: &Thread) -> Json {
    let said: Vec<Json> = (t.said.iter())
        .map(|s| json!({"by": s.by, "text": s.text, "on": s.on}))
        .collect();
    json!({"id": t.id, "page": t.page, "quote": t.quote, "said": said, "open": t.open})
}

fn from_json(j: &Json) -> Option<Thread> {
    let s = |v: &Json, k: &str| v[k].as_str().map(str::to_string);
    let said = (j["said"].as_array()?.iter())
        .map(|x| {
            Some(Said {
                by: s(x, "by")?,
                text: s(x, "text")?,
                on: s(x, "on")?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Thread {
        id: s(j, "id")?,
        page: s(j, "page")?,
        quote: s(j, "quote")?,
        said,
        open: j["open"].as_bool()?,
    })
}

/// Every moved thread, by id.
pub fn load(log: &dyn OpLog) -> Result<BTreeMap<String, Thread>, String> {
    read_table(log, TABLE, from_json)
}

/// The export's threads written over the kept ones, each changed one once: a move run again writes only what changed since.
pub fn save(
    log: &mut dyn OpLog,
    kept: &BTreeMap<String, Thread>,
    threads: Vec<Thread>,
) -> Result<usize, String> {
    let rows: BTreeMap<String, Thread> = threads.into_iter().map(|t| (t.id.clone(), t)).collect();
    write_table(log, TABLE, kept, &rows, to_json)
}

/// One page's threads, oldest first.
pub fn on_page<'a>(all: &'a BTreeMap<String, Thread>, page: &str) -> Vec<&'a Thread> {
    let mut on: Vec<&Thread> = all.values().filter(|t| t.page == page).collect();
    on.sort_by(|a, b| {
        (a.said.first().map(|s| &s.on), &a.id).cmp(&(b.said.first().map(|s| &s.on), &b.id))
    });
    on
}
