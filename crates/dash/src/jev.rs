//! Jev (TypeSafe AI), the core tool of crates/jev: the binary hands it its secrets (bin/secret) and its transport (curl, the
//! key in curl's environment, never its command line), and every caller asks through `Registry::call` (`ask`), so the core
//! caches a repeat question and counts, charges and keeps each call (task rust-jev-tool). CI and a test never pay
//! (`paid_refused`). The log of Jev's attempts (ats.py's rows, `dash doc ready`'s) is here too: `log`, `feed`.
use std::fs::File;
use std::io::Write;
use std::path::Path;

use dash_core::registry::{Ports, Registry};
use dash_hooks::outbound::{JEV_LOG, JEV_LOG_BEFORE};

pub use dash_hooks::jev::{fnv, input_tokens, noul, paid_refused};
pub use dash_jev::{MODEL, USD_PER_INPUT_TOKEN};

/// One question to Jev: `questions` (a JSON object of questions) on `state`, its answer whole (`answers`, `usage`, the
/// key's name that paid, `cached` when the cache gave it). `agent` names the caller in its User-Agent. Refused under CI or a
/// test before anything is read. A store that does not open (a sandbox's read-only copy) keeps and caches nothing, says why.
pub fn ask(
    questions: &serde_json::Value,
    state: serde_json::Value,
    agent: &str,
) -> Result<serde_json::Value, String> {
    if let Some(no) = paid_refused() {
        return Err(no);
    }
    let dir = crate::pages::live_data();
    let kept = std::rc::Rc::new(dash_store::kept(&dir));
    let spending = kept.clone();
    let jev = dash_jev::Jev {
        secret: Box::new(|name| dash_hooks::jev::secret(&crate::pages::repo(), name)),
        post: Box::new(dash_hooks::jev::post),
        spent: Box::new(move |key| dash_core::registry::Budget::spent(&*spending, key)),
    };
    let mut registry = Registry::default();
    registry.add("dash-jev", dash_jev::contribution(jev))?;
    let statsd = dash_statsd::Statsd::new(&dir, "jev");
    let told = dash_hooks::jev::Told(&statsd, Default::default());
    // the system clock, as tasks::now_ms reads it: tasks asks Jev through rfc_judge, so this module reads no tasks
    let now_ms = || dash_core::WallClock::now_ms(&dash_store::oplog::SystemClock);
    let (now_ms, telemetry, k) = (&now_ms, &told, &*kept);
    let ports = Ports {
        now_ms,
        audit: k,
        cache: k,
        telemetry,
        budget: k,
    };
    let asked = dash_jev::arguments(questions, state, agent);
    let (answer, kept) = registry
        .call(dash_jev::NAME, &asked, &ports)
        .ok_or("no Jev tool")?;
    if let Err(why) = kept {
        eprintln!("jev: {why}");
    }
    let answer = answer?;
    // a fresh document answer is its line of Jev's log, written on the tool's side, never by its caller
    let row = (!told.1.get())
        .then(|| dash_hooks::jev::log_row(&asked, &answer, &dash_core::iso(now_ms() / 1000)));
    if let Some(Err(why)) = row.flatten().map(|r| log(&r)) {
        eprintln!("jev: the answer not logged: {why}");
    }
    let mut answer: serde_json::Value =
        serde_json::from_str(&answer).map_err(|e| format!("Jev's answer: {e}"))?;
    answer["cached"] = told.1.get().into();
    Ok(answer)
}

/// The older app's database under a repository root (`tasks::older_db`'s, for a root given).
const OLDER_DB: &str = "private/dashboard/dashboard_dev.db";

/// One Jev attempt (ats.py's row) as a line of the main checkout's log (`pages::repo`), never a worktree's: a verdict asked
/// from a task's session lands where the older app reads it (`Host::jev_log`, `dash jev log`).
pub fn log(row: &serde_json::Value) -> Result<(), String> {
    log_at(&crate::pages::repo(), row)
}

/// The line under `root`, synced, then fed to the older app: once moved (`dash jev move`), the log never changes under the
/// older app's store, so it never reads it again (task rust-jev-log-off-store-lock). Before the move, the line goes to the
/// file the older app reads itself. A feed that fails is said and caught up by the next, the line being in the log.
pub(crate) fn log_at(root: &Path, row: &serde_json::Value) -> Result<(), String> {
    let mut path = dash_hooks::outbound::jev_log_target(root)?;
    let before = root.join(JEV_LOG_BEFORE);
    // the file before is never created: moved away since it was named (`dash jev move`), the line goes to the log
    let open = |p: &Path| {
        std::fs::OpenOptions::new()
            .create(p != before)
            .append(true)
            .open(p)
    };
    let mut f = match open(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && path == before => {
            path = dash_hooks::outbound::jev_log_target(root)?;
            open(&path)
        }
        f => f,
    }
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let named = |e: std::io::Error| format!("{}: {e}", path.display());
    f.write_all(format!("{row}\n").as_bytes()).map_err(named)?;
    // written: a sync that fails is said, never an error, or ats.py would write the line a second time
    if let Err(e) = f.sync_data() {
        eprintln!("Jev's log: {} not synced: {e}", path.display());
    }
    if let Err(why) = feed(root) {
        eprintln!("Jev's log: the older app not fed ({why}): the next line catches up");
    }
    Ok(())
}

/// Feeds the older app the log's lines it lacks, those after its last one: none while the log's file before the move stands,
/// since the older app reads that one itself. The count fed.
pub fn feed(root: &Path) -> Result<usize, String> {
    let log = root.join(JEV_LOG);
    if root.join(dash_hooks::outbound::JEV_LOG_BEFORE).exists() || !log.exists() {
        return Ok(0);
    }
    dash_store::older_jev::feed(&root.join(OLDER_DB), |last, held| {
        dash_hooks::outbound::lines_after(&log, last, held)
    })
}

const USAGE_FEED: &str =
    "dash jev feed (the older app's model gets the lines of Jev's log it lacks)";
const USAGE_LOG: &str =
    "dash jev log (each line of stdin, one Jev attempt as JSON, logged and fed to the older app)";

fn feed_command(args: &[&str], mut out: File) {
    if !args.is_empty() {
        eprintln!("usage: {USAGE_FEED}");
        std::process::exit(2);
    }
    match feed(&crate::pages::repo()) {
        Ok(n) => writeln!(out, "{n} lines fed").unwrap_or_default(),
        Err(why) => {
            eprintln!("dash jev feed: {why}");
            std::process::exit(1);
        }
    }
}

/// ats.py's writer: exit 1 when a line is not logged, so it appends that one itself.
fn log_command(args: &[&str], _out: File) {
    if !args.is_empty() {
        eprintln!("usage: {USAGE_LOG}");
        std::process::exit(2);
    }
    for line in std::io::stdin().lines() {
        let logged = line
            .map_err(|e| format!("stdin: {e}"))
            .and_then(|l| match l.trim() {
                "" => Ok(()),
                l => log(&serde_json::from_str(l).map_err(|e| format!("not JSON: {e}"))?),
            });
        if let Err(why) = logged {
            eprintln!("dash jev log: {why}");
            std::process::exit(1);
        }
    }
}

inventory::submit! {
    crate::usage::Command { usage: USAGE_FEED, words: &["jev", "feed"], run: Some(feed_command) }
}

inventory::submit! {
    crate::usage::Command { usage: USAGE_LOG, words: &["jev", "log"], run: Some(log_command) }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dash_meter::TestDir;
    use serde_json::json;
    use std::time::Duration;

    /// The older app's tables, as much of them as the feed reads, its log's rows `held`.
    fn older(root: &Path, held: &[&str]) -> dash_store::Connection {
        std::fs::create_dir_all(root.join("private/dashboard")).expect("dir");
        let db = dash_store::Connection::open(root.join(OLDER_DB)).expect("db");
        db.execute_batch(
            "CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, at TEXT, data TEXT, body TEXT,
               PRIMARY KEY (source, pos));
             CREATE TABLE tasks (collection TEXT, id TEXT);
             CREATE TABLE leads (collection TEXT, id TEXT);",
        )
        .expect("tables");
        for (pos, line) in held.iter().enumerate() {
            let data = serde_json::from_str::<serde_json::Value>(line)
                .expect("json")
                .to_string();
            db.execute(
                "INSERT INTO events (source, pos, data) VALUES ('ats/jev.jsonl', ?1, ?2)",
                (pos as i64, data),
            )
            .expect("row");
        }
        db
    }

    /// The log's rows as the older app holds them, oldest first.
    fn held(db: &dash_store::Connection) -> Vec<serde_json::Value> {
        let mut q = db
            .prepare("SELECT data FROM events WHERE source = 'ats/jev.jsonl' ORDER BY pos")
            .expect("query");
        q.query_map([], |r| r.get::<_, String>(0))
            .expect("rows")
            .map(|d| serde_json::from_str(&d.expect("row")).expect("json"))
            .collect()
    }

    /// The bytes this process has read through any file so far: Linux's `rchar`, every read call, the page cache's included.
    /// `None` on the Mac, which counts only reads that reached the disk.
    fn bytes_read() -> Option<u64> {
        let io = std::fs::read_to_string("/proc/self/io").ok()?;
        let line = io.lines().find(|l| l.starts_with("rchar:"))?;
        line["rchar:".len()..].trim().parse().ok()
    }

    /// Done-when 1: a Jev call's line costs no read of the whole log. On a 64 MB log the older app holds up to its last line,
    /// three calls' lines are synced and fed reading under 2 MB each, lines_after's 1 MB window and the older app's pages,
    /// counted on Linux (CI's mirror), where a missing count fails the test; a read of the whole log into memory fails its
    /// 50 MB budget on the Mac too. The wall clock is a loose ceiling, 5 s a call, as
    /// a shared runner's fsync took 1.4 s (mirror run 43306836185 failed the 200 ms bound this replaced, task
    /// rust-store-rebuild-write-flake); the older app's reread was 37 s, 2026-11-12.
    #[dash_meter::dash_test]
    fn a_jev_line_on_a_64_mb_log_reads_its_tail_alone() {
        let dir = TestDir::new("jev-log-64mb");
        let path = dir.0.join(JEV_LOG);
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        let mut w = std::io::BufWriter::new(File::create(&path).expect("log"));
        let state = "x".repeat(16 * 1024);
        let mut last = String::new();
        for i in 0..4096 {
            // ats.py's form: sorted keys, a space after each separator
            last = format!(
                "{{\"key\": \"k{i}\", \"request\": {{\"body\": {{\"state\": \"{state}\"}}}}, \"status\": 200}}"
            );
            writeln!(w, "{last}").expect("line");
        }
        drop(w);
        assert!(std::fs::metadata(&path).expect("size").len() >= 64 << 20);
        let db = older(&dir.0, &[&last]);
        let rows: Vec<_> = (0..3)
            .map(|i| json!({"key": format!("new{i}"), "status": 200, "ts": "2026-11-12T18:00:00"}))
            .collect();
        let before = bytes_read();
        assert!(
            before.is_some() || !cfg!(target_os = "linux"),
            "/proc/self/io unread: the bytes read go uncounted"
        );
        for row in &rows {
            let began = std::time::Instant::now();
            log_at(&dir.0, row).expect("logged");
            let took = began.elapsed();
            assert!(took < Duration::from_secs(5), "a call took {took:?}");
        }
        if let (Some(before), Some(after)) = (before, bytes_read()) {
            let read = after - before;
            assert!(
                read < 3 * (2 << 20),
                "three calls read {read} bytes, past 2 MB each"
            );
        }
        assert_eq!(held(&db)[1..], rows[..]);
    }

    /// Before the move a line goes to the file the older app reads itself, nothing fed. `dash jev move` moves it and feeds the
    /// older app what it lacks: here the line its last reread missed and the one after. A full copy of the old file back
    /// beside the log (a rebase's autostash, review of PR 1520) adds nothing to the log or the older app; a copy holding a line
    /// the log lacks adds that line alone; a link left by a move that stopped halfway adds nothing either.
    #[dash_meter::dash_test]
    fn the_move_feeds_what_the_older_app_lacks_and_a_copy_back_adds_only_new_lines() {
        let dir = TestDir::new("jev-log-move");
        let line = |k: &str| json!({"key": k, "status": 200});
        let [a, b, c, d, x, e, g] = ["a", "b", "c", "d", "x", "e", "g"].map(line);
        let (before, log) = (dir.0.join(JEV_LOG_BEFORE), dir.0.join(JEV_LOG));
        std::fs::create_dir_all(dir.0.join("private/ats")).expect("dir");
        std::fs::write(&before, format!("{a}\n{b}\n")).expect("before");
        let db = older(&dir.0, &[&a.to_string()]);
        log_at(&dir.0, &c).expect("logged");
        assert_eq!(held(&db), vec![a.clone()], "the older app reads it itself");
        assert!(!log.exists());
        dash_hooks::outbound::jev_log_move(&dir.0).expect("moved");
        assert_eq!(feed(&dir.0), Ok(2));
        assert!(!before.exists());
        let text = |want: &[&serde_json::Value]| {
            let lines: Vec<String> = want.iter().map(|v| format!("{v}\n")).collect();
            assert_eq!(std::fs::read_to_string(&log).expect("log"), lines.concat());
        };
        text(&[&a, &b, &c]);
        std::fs::write(&before, format!("{a}\n{b}\n")).expect("a full copy back");
        log_at(&dir.0, &d).expect("logged");
        assert!(!before.exists());
        text(&[&a, &b, &c, &d]);
        std::fs::write(&before, format!("{a}\n{x}")).expect("a copy with a new line");
        log_at(&dir.0, &e).expect("logged");
        text(&[&a, &b, &c, &d, &x, &e]);
        std::fs::hard_link(&log, &before).expect("a move stopped halfway");
        log_at(&dir.0, &g).expect("logged");
        assert!(!before.exists());
        text(&[&a, &b, &c, &d, &x, &e, &g]);
        assert_eq!(held(&db), vec![a, b, c, d, x, e, g]);
        assert_eq!(
            dash_hooks::outbound::jev_log_move(&dir.0),
            Ok(log.clone()),
            "moved once"
        );
        let mut half = std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .expect("log");
        write!(half, "{{\"key\": \"h\"").expect("half a line");
        assert_eq!(feed(&dir.0), Ok(0), "half a line waits");
        writeln!(half, "}}").expect("its end");
        assert_eq!(feed(&dir.0), Ok(1));
    }

    /// The older app's last row missing from the log (a union merge that reordered it, a hand edit): the lines past the count
    /// it holds are fed, so it catches up; a log shorter than that count feeds nothing, and says so.
    #[dash_meter::dash_test]
    fn a_last_row_the_log_lacks_falls_back_on_the_count() {
        let dir = TestDir::new("jev-log-lost");
        let path = dir.0.join(JEV_LOG);
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        std::fs::write(&path, "{\"key\": \"a\"}\n{\"key\": \"b\"}\n").expect("log");
        let db = older(&dir.0, &["{\"key\": \"gone\"}"]);
        assert_eq!(feed(&dir.0), Ok(1));
        assert_eq!(held(&db)[1], json!({"key": "b"}));
        let short = TestDir::new("jev-log-short");
        let path = short.0.join(JEV_LOG);
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        std::fs::write(&path, "{\"key\": \"a\"}\n").expect("log");
        let db = older(&short.0, &["{\"key\": \"x\"}", "{\"key\": \"y\"}"]);
        let said = feed(&short.0).expect_err("refused");
        assert!(said.contains("holds 2 Jev rows"), "{said}");
        assert_eq!(held(&db).len(), 2);
    }

    /// A test never pays: the call is refused before the key is read, nextest setting NEXTEST.
    #[dash_meter::dash_test]
    fn a_test_never_pays_jev() {
        let said = ask(&serde_json::json!({}), serde_json::json!({}), "test").expect_err("refused");
        assert!(said.contains("is set: a check never pays Jev"), "{said}");
    }
}
