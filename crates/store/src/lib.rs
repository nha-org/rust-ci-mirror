//! The store adapter of section 3: SQLite behind core's store port, with a bounded page cache and a fixture the size of
//! dash's store.
use dash_core::{Scanned, Store};
use rusqlite::{Connection, params};
use std::path::Path;

pub mod oplog;

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
}
