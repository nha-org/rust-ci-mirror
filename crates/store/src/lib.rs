//! The store adapter of section 3: SQLite behind core's store port, with a bounded page cache and a fixture the size of
//! dash's store.
use dash_core::{Hit, Scanned, Store};
use rusqlite::params;
use std::path::Path;

pub mod audit;
pub mod backup;
pub mod keys;
pub mod older;
pub mod older_comments;
pub mod older_drafts;
pub mod older_handoff;
pub mod older_jev;
pub mod older_linkedin;
pub mod oplog;
pub mod similar;
pub mod tasks;
pub mod timed;

/// So a caller making a replica's database never pins a second rusqlite.
pub use rusqlite::Connection;

/// Section 4: « SQLite's page cache is bounded ». Negative: KiB. The heap limit is the memory cap in bytes, for the C heap the
/// cap cannot count: past it a query answers an error. A test holds it to `CAP_MB`, as this crate does not link the meter.
const CACHE_PRAGMA: &str = "PRAGMA cache_size = -8000; PRAGMA journal_mode = WAL; PRAGMA temp_store = FILE; PRAGMA hard_heap_limit = 209715200;";

pub struct SqliteStore(Connection);

/// A connection on `path` with section 4's bounds.
pub fn connect(path: &Path) -> rusqlite::Result<Connection> {
    let mut db = Connection::open(path)?;
    db.execute_batch(CACHE_PRAGMA)?;
    db.busy_handler(Some(busy))?;
    timed::trace(&mut db, path)?;
    Ok(db)
}

/// A write held by another's commit tries again each millisecond, 5000 times, about the 5 s rusqlite sets (more where a sleep
/// overshoots): SQLite's own backs off to 100 ms sleeps, so a write kept losing to a rebuild's back-to-back page commits and
/// waited 470 to 770 ms (rust/CLAUDE.md rule 5, task rust-old-defects; crates/dash/tests/old_defects.rs holds it). A
/// `busy_timeout` set after `connect` replaces this with SQLite's backoff.
fn busy(tries: i32) -> bool {
    let again = tries < 5000;
    if again {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    again
}

pub fn open(dir: &Path) -> rusqlite::Result<SqliteStore> {
    let db = connect(&dir.join("dash.sqlite"))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS facts (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL);",
    )?;
    db.execute_batch(audit::TABLE)?;
    Ok(SqliteStore(db))
}

/// The store in `dir` for reading only, as /search reads it on its connection's thread: no table made, no write possible.
pub fn open_read(dir: &Path) -> rusqlite::Result<SqliteStore> {
    use rusqlite::OpenFlags;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let path = dir.join("dash.sqlite");
    let mut db = Connection::open_with_flags(&path, flags)?;
    db.execute_batch("PRAGMA cache_size = -8000; PRAGMA hard_heap_limit = 209715200;")?;
    db.busy_handler(Some(busy))?;
    timed::trace(&mut db, &path)?;
    Ok(SqliteStore(db))
}

/// What a hit carries of its body: enough for a hook to judge by, and `dash ingest` stores whole files.
const HIT_BODY_CHARS: i64 = 4000;

/// `rows` facts of about 900 bytes. 200,000 is dash's store of 186 MB (section 2). The test writes it; the server only
/// reads: a write route on `dash serve` would be a way to fill the disk from any local process.
pub fn write_fixture(store: &mut SqliteStore, rows: u64) -> rusqlite::Result<()> {
    let body = "lorem ipsum dolor sit amet ".repeat(33);
    let tx = store.0.transaction()?;
    {
        let mut insert = tx.prepare("INSERT INTO facts (kind, title, body) VALUES (?1, ?2, ?3)")?;
        for i in 0..rows {
            insert.execute(params![
                ["task", "mail", "meeting"][(i % 3) as usize],
                format!("fact {i}"),
                body
            ])?;
        }
    }
    tx.commit()
}

impl SqliteStore {
    /// One fact from `dash ingest`, a command on this machine: `dash serve` stays read-only.
    pub fn add_fact(&self, kind: &str, title: &str, body: &str) -> rusqlite::Result<()> {
        self.0
            .execute(
                "INSERT INTO facts (kind, title, body) VALUES (?1, ?2, ?3)",
                params![kind, title, body],
            )
            .map(|_| ())
    }
}

/// The audit port's adapter (task rust-audit-log), in `audit`.
impl dash_core::registry::Audit for SqliteStore {
    fn record(&self, call: &dash_core::registry::Call) -> Result<(), String> {
        audit::record(&self.0, call)
    }
}

impl SqliteStore {
    /// Each tool's sum since `since_ms`, by name; `tool` alone when named.
    pub fn audit_sums(&self, tool: Option<&str>, since_ms: i64) -> Result<Vec<audit::Sum>, String> {
        audit::sums(&self.0, tool, since_ms)
    }

    /// Every call since `since_ms`, oldest first.
    pub fn audit_calls(&self, since_ms: i64) -> Result<Vec<dash_core::registry::Call>, String> {
        audit::calls(&self.0, since_ms)
    }
}

impl Store for SqliteStore {
    fn scan(&self) -> Result<Scanned, String> {
        self.0
            .query_row(
                "SELECT count(*), coalesce(sum(length(body)), 0) FROM facts",
                [],
                |r| {
                    Ok(Scanned {
                        rows: r.get(0)?,
                        bytes: r.get(1)?,
                    })
                },
            )
            .map_err(|e| e.to_string())
    }

    // ponytail: LIKE scans every row, and past the limit it keeps title matches, then the newest, so a hook never sees an
    // older body match. FTS5 and the vectors of section 8 replace it in the index crate's phase.
    fn search(&self, query: &str, limit: u32) -> Result<Vec<Hit>, String> {
        if query.is_empty() {
            return Ok(vec![]);
        }
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let mut statement = self
            .0
            .prepare(
                "SELECT id, title, substr(body, 1, ?3) FROM facts WHERE id IN (
                   SELECT id FROM facts WHERE title LIKE ?1 ESCAPE '\\' OR body LIKE ?1 ESCAPE '\\'
                   ORDER BY title LIKE ?1 ESCAPE '\\' DESC, id DESC LIMIT ?2)
                 ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![pattern, limit, HIT_BODY_CHARS], |r| {
                Ok(Hit {
                    id: r.get::<_, i64>(0)?.to_string(),
                    title: r.get(1)?,
                    body: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::cap::CAP_MB;
    use dash_meter::dash_test;

    #[dash_test]
    fn a_connection_holds_sqlite_s_heap_to_the_memory_cap() {
        let dir = TestDir::new("store-heap-limit");
        let db = connect(&dir.0.join("limit.sqlite")).expect("connect");
        let limit: i64 = db
            .query_row("PRAGMA hard_heap_limit", [], |row| row.get(0))
            .expect("the limit");
        assert_eq!(limit, CAP_MB as i64 * 1024 * 1024);
    }

    #[dash_test]
    fn an_empty_store_scans_to_zero() {
        let dir = TestDir::new("store-empty");
        let scanned = open(&dir.0).expect("open").scan().expect("scan");
        assert_eq!((scanned.rows, scanned.bytes), (0, 0));
    }

    #[dash_test]
    fn the_fixture_reads_back_what_it_wrote() {
        let dir = TestDir::new("store");
        let mut db = open(&dir.0).expect("open");
        write_fixture(&mut db, 1_000).expect("write");
        let scanned = db.scan().expect("scan");
        assert_eq!(scanned.rows, 1_000);
        assert_eq!(scanned.bytes, 1_000 * 891);
    }

    #[dash_test]
    fn search_finds_a_fact_by_title_or_body_and_takes_like_s_wildcards_literally() {
        let dir = TestDir::new("store-search");
        let db = open(&dir.0).expect("open");
        db.add_fact("file", "alpha", "the body").expect("add");
        db.add_fact("file", "other", "alpha inside").expect("add");
        db.add_fact("file", "100% done", "beta").expect("add");
        let titles = |query: &str, limit| -> Vec<String> {
            let hits = db.search(query, limit).expect("search");
            hits.into_iter().map(|h| h.title).collect()
        };
        assert_eq!(titles("alpha", 10), ["alpha", "other"]);
        assert_eq!(titles("alpha", 1), ["alpha"]);
        assert_eq!(titles("0%", 10), ["100% done"]);
        assert!(titles("b_ta", 10).is_empty(), "_ is no wildcard");
        assert!(titles("a%a", 10).is_empty(), "% is no wildcard");
        assert!(titles("", 10).is_empty(), "no query, no hits");
        db.add_fact("file", "gamma title", "old").expect("add");
        for i in 0..5 {
            db.add_fact("file", &format!("new {i}"), "gamma")
                .expect("add");
        }
        assert_eq!(
            titles("gamma", 3),
            ["gamma title", "new 3", "new 4"],
            "the limit keeps a title match and the newest, in the store's order"
        );
        db.add_fact("file", "long", &"x".repeat(9000)).expect("add");
        let hits = db.search("xxx", 10).expect("search");
        assert_eq!(
            hits[0].body.len(),
            HIT_BODY_CHARS as usize,
            "a hit's body is bounded"
        );
    }

    use dash_core::registry::Audit;

    fn call(
        tool: &str,
        answer: Result<&str, &str>,
        at_ms: i64,
        cost: f64,
    ) -> dash_core::registry::Call {
        dash_core::registry::Call {
            tool: tool.into(),
            origin: "plugin".into(),
            asked: "{}".into(),
            answer: answer.map(str::to_string).map_err(str::to_string),
            at_ms,
            ms: 10,
            cost,
        }
    }

    /// Task rust-audit-log, part 2's store side: the calls kept sum by tool over a period, the last failure named.
    #[dash_test]
    fn calls_kept_sum_what_worked_and_failed_by_tool_over_a_period() {
        let dir = TestDir::new("audit-sums");
        assert_eq!(open_read(&dir.0).err().map(drop), Some(()), "no file yet");
        let store = open(&dir.0).unwrap();
        assert_eq!(store.audit_sums(None, 0), Ok(vec![]));
        for c in [
            call("hostman", Ok("old"), 50, 9.0),
            call("hostman", Ok("a"), 100, 1.5),
            call("hostman", Err("first"), 110, 0.0),
            call("hostman", Err("last"), 120, 0.0),
            call("jobs", Ok("b"), 130, 2.0),
        ] {
            store.record(&c).unwrap();
        }
        let long = format!("a{}", "é".repeat(audit::KEPT_BYTES));
        store.record(&call("jobs", Ok(&long), 140, 0.0)).unwrap();
        let read = open_read(&dir.0).unwrap();
        let sum = |tool: &str, worked, failed, cost, ms, last: Option<&str>| audit::Sum {
            tool: tool.into(),
            worked,
            failed,
            cost,
            ms,
            last_failure: last.map(str::to_string),
        };
        assert_eq!(
            read.audit_sums(None, 100).unwrap(),
            [
                sum("hostman", 1, 2, 1.5, 30, Some("last")),
                sum("jobs", 2, 0, 2.0, 20, None)
            ]
        );
        assert_eq!(
            read.audit_sums(Some("hostman"), 0).unwrap(),
            [sum("hostman", 2, 2, 10.5, 40, Some("last"))]
        );
        let calls = read.audit_calls(120).unwrap();
        assert_eq!(calls[0], call("hostman", Err("last"), 120, 0.0));
        let kept = calls[2].answer.as_ref().unwrap();
        assert_eq!(
            (calls.len(), kept.len()),
            (3, audit::KEPT_BYTES - 1),
            "cut on a char"
        );
    }
}
