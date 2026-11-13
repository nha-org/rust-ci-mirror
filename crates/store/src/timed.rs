//! Each statement a store runs, timed, and each write lock's hold (task rust-store-load-measured; @nha, 2026-11-12: « my
//! guess is sqlite is plenty for our current situation if we use it right but measures might beat instinct. »). The binary
//! installs the one sink, which sends each to Datadog; the store never hears of statsd. With no sink, no connection is traced.
use rusqlite::Connection;
use rusqlite::trace::{TraceEvent, TraceEventCodes};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

/// One statement as it ran, or one write lock as it was held: `sql` is then the first write of its transaction.
pub struct Ran<'a> {
    /// The database file, as opened.
    pub file: &'a str,
    pub sql: &'a str,
    pub took: Duration,
    pub lock: bool,
}

impl Ran<'_> {
    /// The store's tag: its file's stem, `tasks` for tasks.sqlite.
    pub fn store(&self) -> String {
        tag_safe(
            Path::new(self.file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("memory"),
        )
    }

    /// The statement's tag, a name Datadog keeps: its verb, its first table and a hash of its whole text, as
    /// `select_tasks_5f1c2a`, so two queries of one table stay apart and `dash statements` maps it back to its SQL.
    pub fn statement(&self) -> String {
        statement_tag(self.sql)
    }
}

pub fn statement_tag(sql: &str) -> String {
    let words: Vec<String> = sql
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let verb = words.first().map(String::as_str).unwrap_or("none");
    let table = words
        .windows(2)
        .find(|w| matches!(w[0].as_str(), "from" | "into" | "update" | "table" | "on"))
        .map(|w| w[1].as_str())
        .unwrap_or("");
    // FNV-1a: the same text gets the same tag in every process and every build.
    let hash = sql.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    let name = tag_safe(&format!("{verb}_{table}"));
    format!("{}_{:06x}", &name[..name.len().min(60)], hash & 0xff_ffff)
}

fn tag_safe(text: &str) -> String {
    let safe: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    safe.trim_matches('_').to_string()
}

type Sink = Box<dyn Fn(&Ran) + Send + Sync>;
/// The sink, and the caller's monotonic clock: the store reads no clock of its own (dash-gate's simulation rule).
static SINK: OnceLock<(Sink, fn() -> Duration)> = OnceLock::new();

/// The process's one sink, set once, before its first connection: a connection opened earlier is not traced. `now` is a
/// monotonic clock, the time since any fixed start, which times a write lock's hold.
pub fn on_statement(sink: Sink, now: fn() -> Duration) {
    let _ = SINK.set((sink, now));
}

#[derive(PartialEq)]
enum Verb {
    /// A transaction opens; `true` when it takes the write lock at once.
    Begin(bool),
    End,
    Write,
    Read,
}

fn verb(sql: &str) -> Verb {
    // Words only: a batch's « BEGIN; » is a BEGIN.
    let mut words = sql
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_uppercase);
    match words.next().as_deref() {
        Some("BEGIN") => Begin(matches!(
            words.next().as_deref(),
            Some("IMMEDIATE" | "EXCLUSIVE")
        )),
        Some("COMMIT" | "END" | "ROLLBACK") => End,
        Some("INSERT" | "UPDATE" | "DELETE" | "REPLACE" | "CREATE" | "DROP" | "ALTER") => Write,
        _ => Read,
    }
}
use Verb::{Begin, End, Read, Write};

/// Traces `db`, opened on `path`, into the sink, if one is set. A write lock is held from a BEGIN IMMEDIATE, or else from a
/// transaction's first write, to its COMMIT or ROLLBACK; a write outside a transaction holds it for its own time.
pub fn trace(db: &mut Connection, path: &Path) -> rusqlite::Result<()> {
    if SINK.get().is_none() {
        return Ok(());
    }
    let file = path.display().to_string();
    let mut open = false;
    // when the write lock was taken, and the transaction's first write, which names its hold
    let mut held: Option<(Duration, Option<String>)> = None;
    db.trace_v2(
        TraceEventCodes::SQLITE_TRACE_PROFILE,
        Some(move |event: TraceEvent<'_>| {
            let (TraceEvent::Profile(stmt, took), Some((sink, now))) = (event, SINK.get()) else {
                return;
            };
            // read before the sink runs, so its own send never lengthens a hold
            let now = now();
            let began = now.saturating_sub(took);
            // one text for one statement, however it ran: a batch leaves its spaces and its « ; »
            let sql = stmt.sql();
            let sql = sql.trim().trim_end_matches(';').trim_end();
            sink(&Ran {
                file: &file,
                sql,
                took,
                lock: false,
            });
            match verb(sql) {
                // a BEGIN IMMEDIATE does nothing but wait for the lock (the busy handler runs inside it): its hold starts at its end,
                // or every wait read as a hold (the load test, 10 sessions). A deferred transaction's first write, or a lone
                // write, both waits and runs in one statement, which SQLite's profile does not split: counted from its start, a
                // hold may include a wait but never drops a long write's own time (/code-review of rust-store-load-measured)
                Begin(at_once) => {
                    open = true;
                    held = at_once.then_some((now, None));
                }
                End => {
                    open = false;
                    if let Some((at, first)) = held.take() {
                        sink(&Ran {
                            file: &file,
                            sql: first.as_deref().unwrap_or(sql),
                            took: now.saturating_sub(at),
                            lock: true,
                        });
                    }
                }
                Write if open => {
                    let (_, first) = held.get_or_insert((began, None));
                    first.get_or_insert_with(|| sql.to_string());
                }
                Write => sink(&Ran {
                    file: &file,
                    sql,
                    took,
                    lock: true,
                }),
                Read => {}
            }
        }),
    )
}

/// SQLite's plan for `sql` on `file`, read-only, its parameters unbound: one line a step, indented by depth.
pub fn plan(file: &Path, sql: &str) -> Vec<String> {
    use rusqlite::OpenFlags;
    let opened = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY);
    let explained = opened.and_then(|db| {
        let mut stmt = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let mut rows = stmt.raw_query();
        let mut steps = Vec::new();
        let mut depth: std::collections::HashMap<i64, usize> = Default::default();
        while let Some(row) = rows.next()? {
            let (id, parent, detail): (i64, i64, String) = (row.get(0)?, row.get(1)?, row.get(3)?);
            let d = depth.get(&parent).map_or(0, |d| d + 1);
            depth.insert(id, d);
            steps.push(format!("{}{detail}", "  ".repeat(d)));
        }
        Ok(steps)
    });
    explained.unwrap_or_else(|e| vec![format!("none: {e}")])
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::sync::Mutex;

    static SEEN: Mutex<Vec<(String, String, bool, Duration)>> = Mutex::new(Vec::new());
    static TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(100);

    #[dash_test]
    fn each_statement_and_each_lock_hold_reach_the_sink() {
        on_statement(
            Box::new(|ran| {
                SEEN.lock().unwrap().push((
                    ran.store(),
                    ran.sql.trim().trim_end_matches(';').to_string(),
                    ran.lock,
                    ran.took,
                ));
            }),
            // a clock one millisecond on at each read: the transaction's hold spans its statements' reads
            || Duration::from_millis(TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
        );
        let dir = std::env::temp_dir().join(format!("timed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tasks.sqlite");
        let mut db = Connection::open(&path).unwrap();
        trace(&mut db, &path).unwrap();
        db.execute_batch("CREATE TABLE t (x)").unwrap();
        db.execute_batch(
            "BEGIN; SELECT 1; INSERT INTO t VALUES (1); INSERT INTO t VALUES (2); COMMIT;",
        )
        .unwrap();
        db.execute_batch("BEGIN IMMEDIATE; SELECT 1; INSERT INTO t VALUES (3); COMMIT;")
            .unwrap();
        db.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
            .unwrap();
        let seen = SEEN.lock().unwrap().clone();
        let _ = std::fs::remove_dir_all(&dir);
        let locks: Vec<&str> = seen.iter().filter(|s| s.2).map(|s| s.1.as_str()).collect();
        // The CREATE alone, then each transaction once, named by its first write: never the SELECT before it, nor the BEGIN.
        assert_eq!(
            locks,
            [
                "CREATE TABLE t (x)",
                "INSERT INTO t VALUES (1)",
                "INSERT INTO t VALUES (3)"
            ],
            "{seen:?}"
        );
        // The hold runs from the first write's start to the COMMIT: two clock reads later at least.
        let hold = seen.iter().filter(|s| s.2).nth(1).map(|s| s.3);
        assert!(hold >= Some(Duration::from_millis(2)), "{seen:?}");
        assert!(seen.iter().all(|s| s.0 == "tasks"), "{seen:?}");
        assert!(
            seen.iter().any(|s| !s.2 && s.1 == "SELECT count(*) FROM t"),
            "{seen:?}"
        );
    }

    #[dash_test]
    fn a_statement_tag_names_its_verb_and_table_and_keeps_two_queries_apart() {
        let one = statement_tag("SELECT id FROM tasks WHERE id = ?1");
        let two = statement_tag("SELECT stage FROM tasks WHERE id = ?1");
        assert!(one.starts_with("select_tasks_"), "{one}");
        assert_ne!(one, two);
        assert_eq!(one, statement_tag("SELECT id FROM tasks WHERE id = ?1"));
        assert!(
            statement_tag("SELECT x FROM \"weird table\"")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        );
    }
}
