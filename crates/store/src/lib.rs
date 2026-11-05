//! The store adapter of section 3: SQLite behind core's store port, with a bounded page cache and a fixture the size of
//! dash's store.
use dash_core::{Hit, Scanned, Store};
use rusqlite::params;
use std::path::Path;

pub mod oplog;

/// So a caller making a replica's database never pins a second rusqlite.
pub use rusqlite::Connection;

/// Section 4: « SQLite's page cache is bounded ». Negative: KiB.
const CACHE_PRAGMA: &str =
    "PRAGMA cache_size = -8000; PRAGMA journal_mode = WAL; PRAGMA temp_store = FILE;";

pub struct SqliteStore(Connection);

/// A connection on `path` with section 4's bounds.
pub fn connect(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.execute_batch(CACHE_PRAGMA)?;
    Ok(db)
}

pub fn open(dir: &Path) -> rusqlite::Result<SqliteStore> {
    let db = connect(&dir.join("dash.sqlite"))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS facts (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL);",
    )?;
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
    use dash_meter::memory::TestBudget;

    #[test]
    fn an_empty_store_scans_to_zero() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("store-empty");
        let scanned = open(&dir.0).expect("open").scan().expect("scan");
        assert_eq!((scanned.rows, scanned.bytes), (0, 0));
    }

    #[test]
    fn the_fixture_reads_back_what_it_wrote() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("store");
        let mut db = open(&dir.0).expect("open");
        write_fixture(&mut db, 1_000).expect("write");
        let scanned = db.scan().expect("scan");
        assert_eq!(scanned.rows, 1_000);
        assert_eq!(scanned.bytes, 1_000 * 891);
    }

    #[test]
    fn search_finds_a_fact_by_title_or_body_and_takes_like_s_wildcards_literally() {
        let _budget = TestBudget::new();
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
}
