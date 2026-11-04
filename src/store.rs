//! SQLite for the memory harness: a bounded page cache and a fixture the size of dash's store. The store crate of
//! section 3 grows from here.
use rusqlite::{Connection, params};
use std::path::Path;

/// Section 4: « SQLite's page cache is bounded ». Negative: KiB.
const CACHE_PRAGMA: &str =
    "PRAGMA cache_size = -8000; PRAGMA journal_mode = WAL; PRAGMA temp_store = FILE;";

pub fn open(dir: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open(dir.join("dash.sqlite"))?;
    db.execute_batch(CACHE_PRAGMA)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS facts (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL);",
    )?;
    Ok(db)
}

pub struct Scanned {
    pub rows: i64,
    pub bytes: i64,
}

/// `rows` facts of about 900 bytes. 200,000 is dash's store of 186 MB (section 2). The test writes it; the server only
/// reads: a write route on `dash serve` would be a way to fill the disk from any local process.
pub fn write_fixture(db: &mut Connection, rows: u64) -> rusqlite::Result<()> {
    let body = "lorem ipsum dolor sit amet ".repeat(33);
    let tx = db.transaction()?;
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

/// One full read, so the whole file passes through SQLite's cache.
pub fn scan(db: &Connection) -> rusqlite::Result<Scanned> {
    db.query_row("SELECT count(*), sum(length(body)) FROM facts", [], |r| {
        Ok(Scanned {
            rows: r.get(0)?,
            bytes: r.get(1)?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::TestBudget;

    #[test]
    fn the_fixture_reads_back_what_it_wrote() {
        let _budget = TestBudget::new();
        let dir = crate::TestDir::new("store");
        let mut db = open(&dir.0).expect("open");
        write_fixture(&mut db, 1_000).expect("write");
        let scanned = scan(&db).expect("scan");
        assert_eq!(scanned.rows, 1_000);
        assert_eq!(scanned.bytes, 1_000 * 891);
    }
}
