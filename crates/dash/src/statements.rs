//! The stores' statements, measured (task rust-store-load-measured): each one's time and each write lock's hold go to Datadog
//! per statement, and the slow ones to a file beside the stores, so `dash statements --data <dir>` names the ten slowest with
//! their query plans.
use dash_statsd::Statsd;
use dash_store::timed::{Ran, on_statement};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// A statement this slow, or a lock held this long, is kept with its SQL. `DASH_STATEMENTS_SLOW_MS` lowers it for a measure
/// on a copy, which keeps every statement at 0.
const SLOW: Duration = Duration::from_millis(20);
const SLOW_KNOB: &str = "DASH_STATEMENTS_SLOW_MS";
/// What SQLite's profile clock tells from nothing on this Mac.
const SENT: Duration = Duration::from_millis(1);
const SLOW_FILE: &str = "statements-slow.jsonl";
/// Past this the file starts again, its last lines kept as `.prev`: two files at most, whatever the traffic.
const SLOW_FILE_BYTES: u64 = 1 << 20;
const SHOWN: usize = 10;

inventory::submit! { crate::usage::Command::usage("dash statements --data <dir>") }

/// The process's sink: `switch` is the statsd address as last read, None while off.
pub fn sink(
    dir: &Path,
    report: Arc<Statsd>,
    switch: impl Fn() -> Option<SocketAddr> + Send + Sync + 'static,
) {
    let file = dir.join(SLOW_FILE);
    let slow = std::env::var(SLOW_KNOB)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(SLOW, Duration::from_millis);
    on_statement(
        Box::new(move |ran| {
            // SQLite's clock on this Mac counts whole milliseconds, so a faster statement reads 0: sent, it would be a datagram
            // and a series point saying nothing (/code-review of rust-store-load-measured, on the datagrams per statement)
            if let Some(to) = switch().filter(|_| ran.took >= SENT) {
                report.statement(to, &ran.store(), &ran.statement(), ran.lock, ran.took);
            }
            if ran.took >= slow {
                keep(&file, ran);
            }
        }),
        since_start,
    );
}

/// The store's clock: monotonic, from this process's first read of it.
fn since_start() -> Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// One line, written whole by one call, so two threads' lines never interleave. A failed write costs the line only.
fn keep(file: &Path, ran: &Ran) {
    // one thread at a time from the size check to the append: two that both saw a full file would rename twice, the second
    // putting a fresh file over the full `.prev`. Slow statements only, so the wait is rare.
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if std::fs::metadata(file).is_ok_and(|m| m.len() > SLOW_FILE_BYTES) {
        let _ = std::fs::rename(file, file.with_extension("jsonl.prev"));
    }
    let line = serde_json::json!({
        // the clock read here: tasks.rs reaches this module through reporting.rs, so it may not be named back
        "at_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()),
        "file": ran.file,
        "sql": ran.sql,
        "ms": ran.took.as_secs_f64() * 1000.0,
        "lock": ran.lock,
    });
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file);
    if let Ok(mut f) = opened {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

#[derive(Default)]
struct Seen {
    max_ms: f64,
    total_ms: f64,
    count: u64,
}

/// `dash statements --data <dir>`: the ten slowest statements and lock holds kept, each with its query plan read from its
/// store now. A statement is grouped by its file and text, ranked by its slowest run.
pub fn slowest(dir: &Path) -> String {
    let mut seen: std::collections::BTreeMap<(String, String, bool), Seen> = Default::default();
    for name in [format!("{SLOW_FILE}.prev"), SLOW_FILE.to_string()] {
        let text = std::fs::read_to_string(dir.join(&name)).unwrap_or_default();
        for v in text
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        {
            let key = (
                v["file"].as_str().unwrap_or_default().to_string(),
                v["sql"].as_str().unwrap_or_default().to_string(),
                v["lock"].as_bool().unwrap_or_default(),
            );
            let ms = v["ms"].as_f64().unwrap_or_default();
            let s = seen.entry(key).or_default();
            s.max_ms = s.max_ms.max(ms);
            s.total_ms += ms;
            s.count += 1;
        }
    }
    let mut ranked: Vec<_> = seen.into_iter().collect();
    ranked.sort_by(|a, b| b.1.max_ms.total_cmp(&a.1.max_ms));
    if ranked.is_empty() {
        return format!("no statement kept in {}\n", dir.join(SLOW_FILE).display());
    }
    let mut out = String::new();
    for (i, ((file, sql, lock), s)) in ranked.iter().take(SHOWN).enumerate() {
        let ran = Ran {
            file,
            sql,
            took: Duration::ZERO,
            lock: *lock,
        };
        let kind = if *lock { "lock held" } else { "statement" };
        let one_line = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        out += &format!(
            "{}. {} {:.1} ms max, {:.1} ms mean, {} runs: store {}, {}\n   {}\n",
            i + 1,
            kind,
            s.max_ms,
            s.total_ms / s.count as f64,
            s.count,
            ran.store(),
            ran.statement(),
            one_line,
        );
        for step in dash_store::timed::plan(Path::new(file), sql) {
            out += &format!("   plan: {step}\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    #[dash_test]
    fn the_slowest_are_ranked_with_their_plan_and_a_full_scan_shows() {
        let dir = std::env::temp_dir().join(format!("statements-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("tasks.sqlite");
        let conn = dash_store::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, data TEXT)")
            .unwrap();
        let file = db.display().to_string();
        let scan = "SELECT 1 FROM events WHERE json_extract(data, '$.task') = ?1";
        let fast = "SELECT data FROM events WHERE id = ?1";
        for (sql, ms) in [(scan, 490.0), (fast, 25.0), (scan, 300.0)] {
            let took = Duration::from_secs_f64(ms / 1000.0);
            keep(
                &dir.join(SLOW_FILE),
                &Ran {
                    file: &file,
                    sql,
                    took,
                    lock: false,
                },
            );
        }
        let text = slowest(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[0].starts_with(
                "1. statement 490.0 ms max, 395.0 ms mean, 2 runs: store tasks, select_events_"
            ),
            "{text}"
        );
        assert!(lines[2].contains("SCAN events"), "{text}");
        assert!(
            text.contains("2. statement 25.0 ms max")
                && text.contains("SEARCH events USING INTEGER PRIMARY KEY"),
            "{text}"
        );
    }
}
