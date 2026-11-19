//! Jev's log lines into the older app's `events` table, the rows its readers query (`Readiness`, `Model.draft_readings`,
//! `Model.jev_month`), so it never rereads the log under its store lock (task rust-jev-log-off-store-lock: a 64 MB reread
//! held `Dashboard.Store.Lock` 37 s, twice in an hour on 2026-11-12). It stops reading the log once its file,
//! private/ats/jev.jsonl, is gone, and keeps the rows it holds; each line Rust appends to the log's new file is fed here.
//! ponytail: a write into the older app's own table, its schema read as it stands (`events`, `tasks`, `leads`); gone with
//! its last Jev reader.
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use std::path::Path;
use std::time::Duration;

/// The source the older app files the log's lines under.
pub const SOURCE: &str = "ats/jev.jsonl";

/// A line as `Dashboard.Store` keeps it, and `Dashboard.Model.events/3` its `about` and `at`: JSON as `data` (compact, its
/// keys in the line's order, sorted by every writer), else the line as `body`. ponytail: a copy of that Elixir mapping, which
/// no longer changes (no Elixir edit passes the pre-commit); its readers match strings in `data` with LIKE, and serde_json
/// writes a string as Jason does.
const INSERT: &str = "INSERT INTO events (source, pos, key, about, at, data, body)
  SELECT ?1, coalesce((SELECT max(pos) + 1 FROM events WHERE source = ?1), 0), NULL,
    CASE WHEN json_extract(?2, '$.task') IN (SELECT id FROM tasks WHERE id IS NOT NULL AND collection <> '')
           THEN 'tasks:' || json_extract(?2, '$.task')
         WHEN json_extract(?2, '$.opp') IN (SELECT id FROM leads WHERE id IS NOT NULL AND collection <> '')
           THEN 'leads:' || json_extract(?2, '$.opp') END,
    coalesce(json_extract(?2, '$.at'), json_extract(?2, '$.ts'), json_extract(?2, '$.date')), ?2, ?3";

/// Lines a transaction inserts at most: a feed of the whole log (an older app that holds none) never holds SQLite's write
/// lock past the older app's 2 s busy_timeout (rust/CLAUDE.md rule 5, no lock held across a scan).
const CHUNK: usize = 200;

/// Feeds the older app's database `db` the log's lines it lacks: `after` gets the text of its last row (`data`, else `body`,
/// none when it holds no line) and how many it holds, and gives the lines that follow in the log; it runs outside any
/// transaction. They go in `CHUNK` at a time, each transaction first checking the last row is still the one it follows, so
/// a feed running beside it is never doubled: the first to move on feeds the rest. No database there (a worktree, CI):
/// nothing fed. The count of lines fed.
pub fn feed(
    db: &Path,
    after: impl FnOnce(Option<&str>, usize) -> Result<Vec<String>, String>,
) -> Result<usize, String> {
    if !db.exists() {
        return Ok(0);
    }
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(db, flags).map_err(named)?;
    crate::timed::trace(&mut conn, db).map_err(named)?;
    // the older app writes within 2 s (its busy_timeout): a feed waits that out, never fails on it
    conn.busy_timeout(Duration::from_secs(10)).map_err(named)?;
    let (mut last, held) = last_row(&conn).map_err(named)?;
    let lines = after(last.as_deref(), held)?;
    let mut fed = 0;
    for chunk in lines.chunks(CHUNK) {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(named)?;
        if last_row(&tx).map_err(named)?.0 != last {
            break;
        }
        for line in chunk {
            let (data, body) = match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => (Some(v.to_string()), None),
                Err(_) => (None, Some(line.clone())),
            };
            tx.execute(INSERT, params![SOURCE, data, body])
                .map_err(named)?;
            last = data.or(body);
        }
        tx.commit().map_err(named)?;
        fed += chunk.len();
    }
    Ok(fed)
}

/// The text of the source's last row and its count of rows.
fn last_row(db: &Connection) -> rusqlite::Result<(Option<String>, usize)> {
    db.query_row(
        "SELECT (SELECT coalesce(data, body) FROM events WHERE source = ?1 ORDER BY pos DESC LIMIT 1),
                (SELECT count(*) FROM events WHERE source = ?1)",
        [SOURCE],
        |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// The older app's tables, as much of them as the feed reads.
    const OLDER: &str = "
      CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, at TEXT, data TEXT, body TEXT,
        PRIMARY KEY (source, pos));
      CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT);
      CREATE TABLE leads (collection TEXT, pos INTEGER, id TEXT);
      INSERT INTO leads VALUES ('opportunities', 0, 'acme');
      INSERT INTO events VALUES ('ats/jev.jsonl', 0, NULL, NULL, '2026-11-01', '{\"ts\":\"2026-11-01\"}', NULL);
      INSERT INTO events VALUES ('live/task-journal.jsonl', 0, NULL, NULL, NULL, '{}', NULL);";

    /// A row of the log's source: pos, about, at, data, body.
    type Row = (
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );

    fn rows(db: &Path) -> Vec<Row> {
        let c = Connection::open(db).expect("db");
        let mut q = c
            .prepare("SELECT pos, about, at, data, body FROM events WHERE source = ?1 ORDER BY pos")
            .expect("query");
        q.query_map([SOURCE], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("rows")
    }

    /// Each line lands after the last, as the older app would have read it: compact JSON, its lead and its time; a line that
    /// is no JSON as its text. The feed's reader is shown the last row, and a second feed with nothing new adds nothing.
    #[dash_test]
    fn the_lines_land_as_the_older_app_reads_them() {
        let dir = TestDir::new("older-jev-feed");
        let db = dir.0.join("older.db");
        Connection::open(&db)
            .expect("db")
            .execute_batch(OLDER)
            .expect("tables");
        let fed = feed(&db, |last, held| {
            assert_eq!((last, held), (Some("{\"ts\":\"2026-11-01\"}"), 1));
            Ok(vec![
                "{\"opp\": \"acme\", \"ts\": \"2026-11-12T10:00:00\"}".into(),
                "half a li".into(),
            ])
        })
        .expect("fed");
        assert_eq!(fed, 2);
        let got = rows(&db);
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(
            got[1],
            (
                1,
                Some("leads:acme".into()),
                Some("2026-11-12T10:00:00".into()),
                Some("{\"opp\":\"acme\",\"ts\":\"2026-11-12T10:00:00\"}".into()),
                None
            )
        );
        assert_eq!(got[2], (2, None, None, None, Some("half a li".into())));
        let again = feed(&db, |last, held| {
            assert_eq!((last, held), (Some("half a li"), 3));
            Ok(vec![])
        });
        assert_eq!(again, Ok(0));
    }

    /// A whole log goes in `CHUNK` lines a transaction, all of them; a feed that another moved past meanwhile stops, so no line
    /// lands twice.
    #[dash_test]
    fn a_long_feed_goes_by_chunks_and_a_feed_beside_it_is_never_doubled() {
        let dir = TestDir::new("older-jev-chunks");
        let db = dir.0.join("older.db");
        Connection::open(&db)
            .expect("db")
            .execute_batch(OLDER)
            .expect("tables");
        let lines: Vec<String> = (0..CHUNK * 2 + 50)
            .map(|i| format!("{{\"n\":{i}}}"))
            .collect();
        assert_eq!(feed(&db, |_, _| Ok(lines.clone())), Ok(CHUNK * 2 + 50));
        assert_eq!(rows(&db).len(), CHUNK * 2 + 51);
        let beside = feed(&db, |_, _| {
            feed(&db, |_, _| Ok(vec!["{\"n\":\"first\"}".into()])).expect("the other feed");
            Ok(vec!["{\"n\":\"first\"}".into()])
        });
        assert_eq!(beside, Ok(0));
        assert_eq!(rows(&db).len(), CHUNK * 2 + 52);
    }

    /// No older database there (a worktree, CI): nothing fed, and none made.
    #[dash_test]
    fn no_database_is_no_feed() {
        let dir = TestDir::new("older-jev-none");
        let db = dir.0.join("none.db");
        assert_eq!(feed(&db, |_, _| Ok(vec!["{}".into()])), Ok(0));
        assert!(!db.exists());
    }
}
