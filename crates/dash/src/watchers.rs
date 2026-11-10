//! The page at /watchers (task rust-watchers-port; @nha, 2026-11-09: « the watcher logic can be ported to rust as well one
//! at a time, there should be a UI for it too imo again dual-use »): each watcher, what it watches, its last beat, what it
//! woke, and whether Rust or the old app serves it. Rust's watchers write their beat in `watchers.sqlite` (`beat`, `woke`);
//! the old app's come from its `dash watchers`, and Dashboard.Watch's wakes from its `dash watch wakes`. One tree, its HTML
//! for him and its text for agents (`dash watchers --data <dir>` prints the text).
use crate::fresh::{self, Read};
use crate::tasks::older_app;
use dash_core::stamp;
use dash_surface as surface;
use serde_json::{Map, json};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `told`: the alerts he was pushed that still alert (crate::alerts).
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS beat (name TEXT PRIMARY KEY, watches TEXT, every_ms INTEGER, \
                      at_ms INTEGER, ok INTEGER, said TEXT, woke TEXT, woke_ms INTEGER) WITHOUT ROWID; \
                      CREATE TABLE IF NOT EXISTS told (id INTEGER, grp TEXT, PRIMARY KEY (id, grp)) WITHOUT ROWID;";
const READ: Duration = Duration::from_secs(5);

/// One of Rust's watchers, as each pass records it.
pub struct Rust<'a> {
    pub name: &'a str,
    pub watches: &'a str,
    pub every_ms: i64,
}

/// The comment watcher: Rust's comment queue, run by `dash orch` (crate::comments).
pub const COMMENTS: Rust = Rust {
    name: "comment queue",
    watches: "his comments on every page (the old app's live/comments.edn): each one a row, the orchestrator woken on them",
    // a pass every POLL_MS after the last one ended, its read of the old app at most READ: a healthy gap never reads LATE
    every_ms: dash_comments::POLL_MS + dash_comments::READ_MS,
};

/// Rust's Datadog monitors, run by `dash orch` (crate::alerts): each new alert pushed to his phone. A pass a minute: the
/// monitors look at 5 minutes, so a breach reaches him within about 6.
pub const ALERTS: Rust = Rust {
    name: "datadog monitors",
    watches: "the monitors tagged service:dash-rust in Datadog: each group newly in Alert pushed to his phone, once",
    every_ms: 60_000,
};

/// The morning run, run by `dash orch` (crate::morning): not done by 09:00, the orchestrator woken once that day to run it.
pub const MORNING: Rust = Rust {
    name: "morning run",
    watches: ".claude/run/morning-run.last: older than today at 09:00, one wake to run .claude/hooks/morning-run.sh's prompt",
    every_ms: 60_000,
};

/// Every watcher Rust runs: each next one ported joins here.
const RUST: [&Rust; 3] = [&COMMENTS, &ALERTS, &MORNING];

/// A line of the old app's `dash watchers` this page cannot read: shown as it is, never dropped.
const UNREAD: &str = "a line the page cannot read";

/// ponytail: a connection and the schema's DDL per call, two or three every 30 s; one per thread if a pass's time shows it.
/// `connect`'s busy handler stays: a busy_timeout here would replace it with SQLite's backoff (/code-review of rust-old-defects).
pub(crate) fn open(dir: &Path) -> Result<dash_store::Connection, String> {
    let db = dash_store::connect(&dir.join("watchers.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    Ok(db)
}

/// A pass of `w` ended at `now_ms`, `ok` or failed, and says `said`. Runtime state, never the op log: a beat every 30 s is
/// no fact to replicate.
pub fn beat(dir: &Path, w: &Rust, now_ms: i64, ok: bool, said: &str) -> Result<(), String> {
    let sql = "INSERT INTO beat (name, watches, every_ms, at_ms, ok, said) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(name) \
               DO UPDATE SET watches = ?2, every_ms = ?3, at_ms = ?4, ok = ?5, said = ?6";
    let args = (w.name, w.watches, w.every_ms, now_ms, ok, said);
    open(dir)?
        .execute(sql, args)
        .map(drop)
        .map_err(|e| e.to_string())
}

/// `w` woke `what` at `now_ms`: kept until its next wake.
pub fn woke(dir: &Path, w: &Rust, now_ms: i64, what: &str) -> Result<(), String> {
    let sql = "INSERT INTO beat (name, watches, every_ms, woke, woke_ms) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(name) DO \
               UPDATE SET woke = ?4, woke_ms = ?5";
    let args = (w.name, w.watches, w.every_ms, what, now_ms);
    open(dir)?
        .execute(sql, args)
        .map(drop)
        .map_err(|e| e.to_string())
}

/// A row of the page.
#[derive(Debug, PartialEq)]
struct Row {
    name: String,
    served: &'static str,
    watches: String,
    beat: String,
    woke: String,
    /// What feeds it, when it says: the comment queue's compass and the old app's comments.
    fed: Option<String>,
}

/// A row of the beat table: name, last pass and whether it worked, its line, its last wake and when.
type Beat = (String, Option<i64>, bool, String, Option<(String, i64)>);

/// Its last beat: a failed pass is NOT OK, a beat past twice its period LATE.
fn beat_line(every: i64, at: Option<i64>, ok: bool, said: &str, now_ms: i64) -> String {
    match at {
        Some(at) if !ok => format!("NOT OK, {}: {said}", stamp(at)),
        Some(at) if now_ms - at > 2 * every => format!("LATE, {}: {said}", stamp(at)),
        Some(at) => format!("ok, {}: {said}", stamp(at)),
        None => "none yet".into(),
    }
}

/// Each of Rust's watchers in `RUST`, its beat and wake from the table: listed before its first beat, a row of a watcher
/// no longer in `RUST` never shown, and a table that does not read costs its lines only.
fn rust_rows(dir: &Path, now_ms: i64) -> Vec<Row> {
    let read = || -> Result<Vec<Beat>, String> {
        let db = open(dir)?;
        let mut q = db
            .prepare("SELECT name, at_ms, ok, said, woke, woke_ms FROM beat")
            .map_err(|e| e.to_string())?;
        let rows = q.query_map([], |r| {
            let woke: Option<String> = r.get(4)?;
            let woke_ms: Option<i64> = r.get(5)?;
            let ok: Option<bool> = r.get(2)?;
            let said: Option<String> = r.get(3)?;
            Ok((
                r.get(0)?,
                r.get(1)?,
                ok.unwrap_or(true),
                said.unwrap_or_default(),
                woke.zip(woke_ms),
            ))
        });
        rows.and_then(Iterator::collect).map_err(|e| e.to_string())
    };
    let read = read();
    let row = |w: &Rust| {
        let (beat, woke) = match &read {
            Err(e) => (
                format!("its beat did not read: {e}"),
                format!("its wake did not read: {e}"),
            ),
            Ok(rows) => match rows.iter().find(|r| r.0 == w.name) {
                None => ("none yet".into(), "nothing yet".into()),
                Some((_, at, ok, said, woke)) => (
                    beat_line(w.every_ms, *at, *ok, said, now_ms),
                    woke.as_ref().map_or("nothing yet".into(), |(what, at)| {
                        format!("{}: {what}", stamp(*at))
                    }),
                ),
            },
        };
        Row {
            name: w.name.into(),
            served: "Rust",
            watches: w.watches.into(),
            beat,
            woke,
            fed: None,
        }
    };
    RUST.iter().map(|w| row(w)).collect()
}

/// Each of Rust's watchers whose beat says it stopped, LATE, NOT OK or unread, as /watchers shows it: the next wake names them
/// (rust/CLAUDE.md rule 5: a watcher that stops says so where it is read; Elixir's wake-wait was down 5 hours, 2026-11-08).
/// One before its first beat has not stopped.
pub fn stopped(dir: &Path, now_ms: i64) -> Vec<String> {
    let rows = rust_rows(dir, now_ms).into_iter();
    rows.filter(|r| !r.beat.starts_with("ok, ") && r.beat != "none yet")
        .map(|r| format!("{}: {}", r.name, r.beat))
        .collect()
}

/// What each of the old app's watchers watches, by the name its `dash watchers` gives; any other, its own words. The old
/// app says no such thing; each row goes when its watcher is ported and says its own (`Rust::watches`).
fn watches(name: &str) -> Option<&'static str> {
    Some(match name {
        "wake-wait" => "the orchestrator's wake file: hands each wake to its session",
        "comment-wake" => "the old app's silence: wakes wake-wait when the app is quiet 5 minutes",
        "PM loop" => "open tasks, their PRs and CI: starts, wakes and merges",
        "mail poll" => "nicolas@klar.im, every 5 minutes",
        "orchestrator-job" => "launchd's orchestrator job",
        "Datadog agent" => "the Datadog agent's OTLP port",
        "Rust tool" => "Rust's front door on 7788",
        n if n.starts_with("CI runner") => "its CI jobs and its disk",
        _ => return None,
    })
}

/// The old app's `dash watchers`: « Watchers: <state> », then a line a watcher, « <ok|NOT OK|WAITING> <name>: <detail>
/// (<stamp>) », a red one's probe on the line under it.
fn old_rows(said: &str) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![];
    for line in said.lines().skip(1) {
        let line = line.trim();
        if let Some(probe) = line.strip_prefix("probe: ") {
            if let Some(r) = rows.last_mut() {
                r.beat = format!("{}; probe: {probe}", r.beat);
            }
            continue;
        }
        let read = ["NOT OK ", "WAITING ", "ok "]
            .iter()
            .find_map(|s| line.strip_prefix(s).map(|rest| (s.trim(), rest)))
            .and_then(|(state, rest)| Some((state, rest.split_once(": ")?)));
        let Some((state, (name, detail))) = read else {
            // a shape the old app adds later is shown as it is: a red watcher never hides behind a parse
            if !line.is_empty() {
                rows.push(Row {
                    name: UNREAD.into(),
                    served: "the old app",
                    watches: "unknown".into(),
                    beat: line.into(),
                    woke: "not recorded by the old app".into(),
                    fed: None,
                });
            }
            continue;
        };
        let (detail, at) = match detail.rsplit_once(" (") {
            Some((d, at)) if at.ends_with(" UTC)") => (d, at.trim_end_matches(')')),
            _ => (detail, "no time"),
        };
        rows.push(Row {
            name: name.into(),
            served: "the old app",
            watches: watches(name).map_or(detail.into(), str::to_string),
            beat: format!("{state}, {at}: {detail}"),
            woke: "not recorded by the old app".into(),
            fed: None,
        });
    }
    rows
}

/// Dashboard.Watch, the old app's one watcher, its wakes today from `dash watch wakes`: it records no beat.
fn old_watch(wakes: Result<String, String>) -> Row {
    Row {
        name: "Dashboard.Watch".into(),
        served: "the old app",
        watches: "his comments (each routed to a task card), task events, panes, the Mac's load, other repos' requests"
            .into(),
        beat: "not recorded by the old app".into(),
        woke: match wakes {
            Ok(w) => format!("the orchestrator, by day: {}", w.trim()),
            Err(e) => format!("the old app did not answer: {e}"),
        },
        fed: None,
    }
}

/// `note`: how many watchers and how old the read is; a skeleton until the old app's watchers are read (task rust-skeleton-live).
fn tree(rows: &[Row], note: Option<&str>) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut elements = Map::new();
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Watchers"}}),
    );
    let note = note.map_or_else(
        || surface::skeleton("The old app's watchers"),
        |n| text(n.into()),
    );
    elements.insert("note".into(), note);
    let mut children = vec!["title".to_string(), "note".to_string()];
    for (i, r) in rows.iter().enumerate() {
        let fed = r.fed.as_ref().map(|f| ("fed", format!("Fed by: {f}")));
        let lines = [
            ("served", format!("Served by: {}", r.served)),
            ("watches", format!("Watches: {}", r.watches)),
            ("beat", format!("Last beat: {}", r.beat)),
            ("woke", format!("Woke: {}", r.woke)),
        ]
        .into_iter()
        .chain(fed);
        let mut names = vec![];
        for (kind, line) in lines {
            let name = format!("{kind}-{i}");
            elements.insert(name.clone(), text(line));
            names.push(name);
        }
        let name = format!("watcher-{i}");
        elements.insert(
            name.clone(),
            json!({"type": "section", "props": {"title": r.name}, "children": names}),
        );
        children.push(name);
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// The old app's answers: its `dash watchers`, then its `dash watch wakes 1`, asked at once.
type Old = (Result<String, String>, Result<String, String>);

fn ask_old() -> Old {
    let cli = older_app().bin;
    let ask = |args: &[&str]| dash_process::run(&cli, args, None, READ);
    std::thread::scope(|s| {
        let said = s.spawn(|| ask(&["watchers"]));
        let wakes = s.spawn(|| ask(&["watch", "wakes", "1"]));
        let joined = |h: std::thread::ScopedJoinHandle<'_, _>| {
            h.join().unwrap_or_else(|_| Err("panicked".into()))
        };
        (joined(said), joined(wakes))
    })
}

/// The page's tree from `dir`'s beats, read now, and the old app's answers with a note on their age.
/// `fed`: what feeds the comment queue (crate::comments::fed), handed in so this module never reads the queue itself.
fn tree_of(dir: &Path, fed: &str, old: Option<Old>, age: &str) -> String {
    let mut rows = rust_rows(dir, crate::tasks::now_ms());
    if let Some(r) = rows.iter_mut().find(|r| r.name == COMMENTS.name) {
        r.fed = Some(fed.into());
    }
    let note = match old {
        Some((said, wakes)) => {
            rows.push(old_watch(wakes));
            match said {
                Ok(said) => rows.extend(old_rows(&said)),
                Err(e) => rows.push(Row {
                    name: "the old app's watchers".into(),
                    served: "the old app",
                    watches: "its dash watchers".into(),
                    beat: format!("the old app did not answer: {e}"),
                    woke: "not recorded by the old app".into(),
                    fed: None,
                }),
            }
            // Dashboard.Watch and each line read: a placeholder for a silent app or an unread line is no watcher
            let old = rows
                .iter()
                .filter(|r| r.served == "the old app" && r.name != UNREAD);
            let old = old
                .filter(|r| !r.beat.starts_with("the old app did not answer"))
                .count();
            Some(format!(
                "{} watchers: {} served by Rust, {old} by the old app. {age}",
                RUST.len() + old,
                RUST.len()
            ))
        }
        None => None,
    };
    tree(&rows, note.as_deref().map(str::trim_end))
}

static OLD: Mutex<Read<Old>> = Mutex::new(Read::new());

/// The page at /watchers. The server routes on one thread and the old app's reads took 2.7 s at a load of 91: they show
/// from their last read, as /agents' do (crate::fresh); Rust's beats are read at each view.
pub fn page(dir: &Path, fed: &str) -> Result<surface::Drawing, String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let now = now.map_err(|e| e.to_string())?.as_secs();
    let tree = match fresh::last(&OLD, now, "watchers", ask_old)? {
        Some((old, at)) => tree_of(dir, fed, Some(old), &fresh::ago(now, at)),
        None => tree_of(dir, fed, None, ""),
    };
    Ok(surface::Drawing::Tree {
        title: "Watchers".into(),
        tree,
    })
}

/// `dash watchers --data <dir>`: the page's text, for an agent with no port.
pub fn list(dir: &Path, fed: &str, mut out: std::fs::File) {
    use std::io::Write;
    let text = surface::text(&tree_of(dir, fed, Some(ask_old()), ""));
    match text {
        Ok(text) => write!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("watchers: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    /// The old app's `dash watchers` of 2026-11-09 01:15, cut to three rows, one of them red.
    const OLD: &str = "Watchers: operational
  ok wake-wait: beats every 2 h; wake-wait: pid 10405 waits for supervisor 71778's wakes (2026-11-09 00:05 UTC)
  NOT OK mail poll: beats every 5 min; Last poll: stale past 10 min (2026-11-08 23:11 UTC)
    probe: bin/klar-mail poll; every: 5 min
  ok CI runner 2: beats every 4 days; disk 5.4 GB of 8.0 GB (2026-11-08 22:53 UTC)
";

    #[test]
    fn each_watcher_is_one_row_in_both_forms() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("watchers");
        let at = 1_794_186_900_000; // 2026-11-09 01:15 UTC
        beat(&dir.0, &COMMENTS, at, true, "1 open, 0 written").expect("beats");
        woke(
            &dir.0,
            &COMMENTS,
            at - 60_000,
            "the orchestrator with f99c7dcc",
        )
        .expect("wakes");
        beat(&dir.0, &COMMENTS, at + 30_000, true, "0 open, 1 written").expect("beats again");
        // a watcher no longer in RUST, renamed or gone, leaves its row in the table: never shown
        let gone = Rust {
            name: "comments",
            watches: "old name",
            every_ms: 1,
        };
        beat(&dir.0, &gone, at, true, "renamed since").expect("beats");
        let mut rows = rust_rows(&dir.0, at + 30_000);
        assert_eq!(rows.pop().map(|r| r.name), Some(MORNING.name.into()));
        assert_eq!(rows.pop().map(|r| r.name), Some(ALERTS.name.into()));
        assert_eq!(
            rows,
            [Row {
                name: "comment queue".into(),
                served: "Rust",
                watches: COMMENTS.watches.into(),
                beat: "ok, 2026-11-09 01:15 UTC: 0 open, 1 written".into(),
                woke: "2026-11-09 01:14 UTC: the orchestrator with f99c7dcc".into(),
                fed: None,
            }],
            "a wake stays until the next, a beat replaces the last"
        );
        let late = rust_rows(&dir.0, at + 30_000 + 2 * COMMENTS.every_ms + 1);
        assert!(
            late[0].beat.starts_with("LATE, 2026-11-09 01:15 UTC"),
            "{}",
            late[0].beat
        );
        // a pass that fails beats too, NOT OK, so a queue that routes nothing never looks healthy
        beat(
            &dir.0,
            &COMMENTS,
            at + 60_000,
            false,
            "comment: the old app: timed out",
        )
        .expect("beats");
        let failed = rust_rows(&dir.0, at + 60_000);
        assert_eq!(
            failed[0].beat,
            "NOT OK, 2026-11-09 01:16 UTC: comment: the old app: timed out"
        );

        rows.push(old_watch(Ok(
            "2026-11-09 1 wakes (the dashboard 1)\n".into()
        )));
        rows.extend(old_rows(OLD));
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "comment queue",
                "Dashboard.Watch",
                "wake-wait",
                "mail poll",
                "CI runner 2"
            ]
        );
        assert_eq!(
            rows[3].beat,
            "NOT OK, 2026-11-08 23:11 UTC: beats every 5 min; Last poll: stale past 10 min; probe: bin/klar-mail poll; every: 5 min"
        );
        assert_eq!(rows[4].watches, "its CI jobs and its disk");

        // Dual-use: one tree, its text for agents and its HTML for him, each holding every field of every row.
        let tree = tree(&rows, Some("5 watchers"));
        let text = surface::text(&tree).expect("text");
        let html = surface::screen::page("Watchers", &tree).expect("html");
        for r in &rows {
            for field in [r.name.as_str(), r.served, &r.watches, &r.beat, &r.woke] {
                assert!(text.contains(field), "text lacks {field}:\n{text}");
                let escaped = field
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('\'', "&#39;");
                assert!(html.contains(&escaped), "html lacks {field}");
            }
        }
        assert!(
            text.contains("Served by: Rust") && text.contains("Served by: the old app"),
            "{text}"
        );
    }

    #[test]
    fn a_silent_old_app_or_an_unread_line_costs_only_its_rows() {
        let _budget = TestBudget::new();
        let row = old_watch(Err("timed out".into()));
        assert_eq!(row.woke, "the old app did not answer: timed out");
        assert!(old_rows("").is_empty());
        // a line in a shape the page cannot read is shown as it is, never dropped
        let unread = old_rows("Watchers: operational\n  STALE mail poll: 20 min\n");
        assert_eq!(
            (unread[0].name.as_str(), unread[0].beat.as_str()),
            (UNREAD, "STALE mail poll: 20 min")
        );

        // Rust's watchers are listed before their first beat, and the first view says the old app's read is on its way.
        let dir = TestDir::new("watchers-empty");
        let rows = rust_rows(&dir.0, 0);
        assert_eq!((rows.len(), rows[0].name.as_str()), (3, "comment queue"));
        assert_eq!(
            (rows[0].beat.as_str(), rows[0].woke.as_str()),
            ("none yet", "nothing yet")
        );
        let text = surface::text(&tree_of(&dir.0, "the feed", None, "")).expect("text");
        assert!(
            text.contains("\nFed by: the feed\n"),
            "the comment queue says what feeds it: {text}"
        );
        assert!(
            text.contains("comment queue") && text.contains("The old app's watchers: being read."),
            "{text}"
        );
        // a silent old app: its placeholder is no watcher in the count, Dashboard.Watch still is
        let old = (Err("down".to_string()), Err("down".to_string()));
        let text = surface::text(&tree_of(&dir.0, "the feed", Some(old), "Read 0 min ago."))
            .expect("text");
        assert!(text.contains("the old app did not answer: down"), "{text}");
        assert!(
            text.contains("4 watchers: 3 served by Rust, 1 by the old app. Read 0 min ago."),
            "{text}"
        );
        let old = (
            Ok(format!("{OLD}  STALE x: y\n")),
            Ok("1 wakes".to_string()),
        );
        let text = surface::text(&tree_of(&dir.0, "the feed", Some(old), "")).expect("text");
        assert!(
            text.contains("7 watchers: 3 served by Rust, 4 by the old app."),
            "{text}"
        );

        // a beat table that does not read costs the Rust rows' lines, never the page
        let file = TestDir::new("watchers-file");
        std::fs::write(file.0.join("watchers.sqlite"), "not a database").expect("a bad file");
        let rows = rust_rows(&file.0, 0);
        assert!(
            rows[0].beat.starts_with("its beat did not read: "),
            "{}",
            rows[0].beat
        );
    }
}
