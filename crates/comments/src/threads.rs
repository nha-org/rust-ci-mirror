//! The older app's page comments, moved whole into Rust's store (task editor-in-rfcs, done-when 2; @nha, 2026-11-11: « we
//! need to port documents to the new notion rust that is probably a DB reimport in the new system right ? »). The queue keeps
//! only what waits on an agent; here every thread lands, resolved ones too, his words then each reply, so the page they were
//! written on shows them all. A thread keeps its quote and no position: the page anchors it on the first passage holding
//! those words (dash-surface's DocView), so it follows its paragraph wherever an edit moves it.
use crate::{read_table, segment, text, write_table};
use dash_core::OpLog;
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

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

pub(crate) fn from_json(j: &Json) -> Option<Thread> {
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

/// The comment queue and the moved threads as a log holds them.
#[derive(Clone, Default)]
pub struct Seen {
    pub rows: BTreeMap<String, crate::Row>,
    pub threads: BTreeMap<String, Thread>,
}

/// `Seen`, kept by one cursor following the log (rfc-rust-unified section 6, the views' pattern): each catch-up reads only the
/// ops written since the last, so a page's rail never replays the log (task untangle-serve-no-queue). Stamps are taken past
/// the log's last inside its write lock, so no local commit lands behind the cursor; a log replaced in place starts it over.
/// ponytail: an op `Replica::apply` brings in below it is missed, as the views' (none applies to this file today).
#[derive(Default)]
pub struct Follow {
    cursor: dash_core::Cursor,
    seen: Arc<Seen>,
}

impl Follow {
    /// Caught up with `log`, its snapshot: a reader holds it with no lock, a later write copying it only while one does.
    pub fn catch_up(&mut self, log: &dyn OpLog) -> Result<Arc<Seen>, String> {
        let read = &mut |a: Option<(i64, &str)>, n| log.ops_after(a, n);
        if self.cursor.replaced(read)? {
            self.seen = Arc::default();
        }
        let seen = &mut self.seen;
        self.cursor.tail(read, crate::PAGE, |ops| {
            // a write of another table (the watchers' rows, the asks) copies no snapshot a reader holds (/code-review)
            let ours =
                |op: &&dash_core::Op| [crate::TABLE, TABLE].contains(&op.write.table.as_str());
            for w in ops.iter().filter(ours).map(|op| &op.write) {
                let seen = Arc::make_mut(seen);
                crate::apply(&mut seen.rows, w, crate::TABLE, crate::from_json);
                crate::apply(&mut seen.threads, w, TABLE, from_json);
            }
        })?;
        Ok(Arc::clone(&self.seen))
    }
}

/// A process's one `Follow`, on the log `open` gives for a file: opened once while the file is the same one (its path and an
/// identity, its inode), caught up at each read. A catch-up that fails drops it, so the next read opens it again.
#[derive(Default)]
pub struct Followed(Mutex<Option<Held>>);

type Held = ((PathBuf, u64), Box<dyn OpLog + Send>, Follow);

impl Followed {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// The queue and moved threads of the file `key` names, caught up with the log `open` gives, or why it does not read.
    /// `open` runs only when that file is not the one held. The lock covers the catch-up alone: the snapshot is read without it.
    pub fn read(
        &self,
        key: (PathBuf, u64),
        open: impl FnOnce() -> Result<Box<dyn OpLog + Send>, String>,
    ) -> Result<Arc<Seen>, String> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (_, log, follow) = match held.take() {
            Some(h) if h.0 == key => held.insert(h),
            _ => held.insert((key, open()?, Follow::default())),
        };
        let seen = follow.catch_up(&**log);
        if seen.is_err() {
            *held = None;
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::{MemoryLog, Op};
    use dash_meter::dash_test;

    /// A log the test still writes after the follower took it.
    struct Shared(Arc<Mutex<MemoryLog>>);

    impl OpLog for Shared {
        fn write(&mut self, writes: &[dash_core::FieldWrite]) -> Result<Vec<Op>, String> {
            self.0.lock().unwrap().write(writes)
        }
        fn apply(&mut self, ops: &[Op]) -> Result<usize, String> {
            self.0.lock().unwrap().apply(ops)
        }
        fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String> {
            self.0.lock().unwrap().ops_after(after, limit)
        }
        fn state_hash(&self) -> Result<u64, String> {
            self.0.lock().unwrap().state_hash()
        }
    }

    fn post(log: &Arc<Mutex<MemoryLog>>, id: &str) {
        let row = crate::posted(id, "/p", "q", "words", "2026-11-15T10:00:00Z", 0);
        crate::put(&mut Shared(Arc::clone(log)), row).expect("a comment");
    }

    /// Done-when 2 of untangle-serve-no-queue, and its /code-review: one log opened per file, however many reads; each read
    /// holds what was written since the last; a log replaced in place (a backup restored under it) is read again from its start,
    /// and another file (a new inode) is opened anew.
    #[dash_test]
    fn one_open_a_file_each_read_caught_up_and_a_replaced_log_read_again() {
        let log = Arc::new(Mutex::new(MemoryLog::default()));
        let (followed, opens) = (Followed::new(), std::cell::Cell::new(0));
        let read = |inode: u64| {
            let open = || {
                opens.set(opens.get() + 1);
                Ok(Box::new(Shared(Arc::clone(&log))) as Box<dyn OpLog + Send>)
            };
            let seen = followed
                .read(("comments.sqlite".into(), inode), open)
                .expect("read");
            seen.rows.keys().cloned().collect::<Vec<_>>()
        };
        post(&log, "a");
        assert_eq!(read(1), ["a"]);
        post(&log, "b");
        assert_eq!(read(1), ["a", "b"], "the cursor followed");
        assert_eq!(read(1), ["a", "b"]);
        assert_eq!(opens.get(), 1, "one open for three reads");
        // restored: the log as it stood before b, then a later write
        log.lock().unwrap().0.truncate(1);
        post(&log, "c");
        assert_eq!(read(1), ["a", "c"], "b is gone with the log that held it");
        assert_eq!(read(2), ["a", "c"]);
        assert_eq!(opens.get(), 2, "a new inode is a new file");
    }
}
