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
                      CREATE TABLE IF NOT EXISTS told (id INTEGER, grp TEXT, PRIMARY KEY (id, grp)) WITHOUT ROWID; \
                      CREATE TABLE IF NOT EXISTS paused (name TEXT PRIMARY KEY) WITHOUT ROWID;";
const READ: Duration = Duration::from_secs(5);
/// His switch's path: `POST /watchers/pause?name=<a watcher or an automation>` pauses it, or resumes it.
pub const PAUSE: &str = "/watchers/pause";
/// The Automations tab (task rust-automations-tab; @nha, 2026-11-10: « I suggest making a tab with a visual for both of us,
/// always dual-use »): each automation's runs as dots on a strip, then its next run.
pub const AUTOMATIONS_TAB: &str = "/watchers?tab=automations";

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

/// The automations, run by `dash orch` (crate::automations): each one's run when due, its one task, its report's push.
pub const AUTOMATIONS: Rust = Rust {
    name: "automations",
    watches: "each automation's schedule: a due one runs its dash command or files its one task, never a second while open",
    every_ms: 60_000,
};

/// The leads' chances, run by `dash orch` (crate::chances): every lead of the older app's store judged each pass, so a lead
/// that lands is judged within 15 s, well inside the minute task rust-judge-on-arrival asks.
pub const CHANCES: Rust = Rust {
    name: "lead chances",
    watches: "the older app's leads: each one's chances out of 10 judged and kept in chances.sqlite, a new lead within a pass",
    every_ms: 15_000,
};

/// The VPS runner's watchdog, run by `dash orch` (crate::watchdog): a job past its limit plus 5 minutes cancelled, a run
/// GitHub left queued with no job named as no hang, each in its line, and the CI queue row (`CI_QUEUE`). Every 2
/// minutes: 3 calls of GitHub's budget a pass, one more a run in progress or queued 15 minutes.
pub const VPS: Rust = Rust {
    name: "vps watchdog",
    watches: "the site-perso-vps runner's runs on GitHub: a job past its timeout-minutes plus 5 cancelled, a run queued 15 \
              minutes with no job named as no hang (GitHub refuses to end it), each named here",
    every_ms: 120_000,
};

/// The CI queue row (crate::watchdog's /status/ci-queue), recorded by the VPS watchdog's pass from the jobs it read: no
/// watcher of its own, so not in `RUST`; its beat is the row, its `said` what the row says.
pub const CI_QUEUE: Rust = Rust {
    name: "ci queue",
    watches: "the runs waiting for a runner, each judged by its jobs: per workflow the count and the oldest wait, red past 30 \
              minutes",
    every_ms: VPS.every_ms,
};

/// Every watcher Rust runs: each next one ported joins here.
const RUST: [&Rust; 5] = [&COMMENTS, &ALERTS, &AUTOMATIONS, &CHANCES, &VPS];

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

/// Whether his switch on /watchers pauses `name`, a watcher of `RUST` or an automation's key. A table that does not read
/// pauses nothing: the pass runs and says what failed.
pub fn is_paused(dir: &Path, name: &str) -> bool {
    let sql = "SELECT 1 FROM paused WHERE name = ?1";
    open(dir).is_ok_and(|db| {
        db.prepare(sql)
            .and_then(|mut q| q.exists([name]))
            .unwrap_or(false)
    })
}

/// His switch: pauses `name`, or resumes it; whether it is paused now. A name that is neither a watcher of `RUST` nor an
/// automation is refused.
pub fn toggle(dir: &Path, name: &str, also: &[&str]) -> Result<bool, String> {
    let known = RUST.iter().any(|w| w.name == name) || also.contains(&name);
    if !known {
        return Err(format!("no watcher or automation {name:?}"));
    }
    // a delete that removed nothing pauses: no read before the write, so two clicks at once never meet a UNIQUE error
    let db = open(dir)?;
    let write = |sql: &str| db.execute(sql, [name]).map_err(|e| e.to_string());
    if write("DELETE FROM paused WHERE name = ?1")? > 0 {
        return Ok(false);
    }
    write("INSERT OR IGNORE INTO paused (name) VALUES (?1)")?;
    Ok(true)
}

/// Runs `w`'s `pass` on its own thread for as long as `dash orch` runs, `sleep` after each: a pass that panics is said and the
/// next one runs; while his switch pauses `w`, none runs and its beat says so. A line on stderr each time what it says changes.
pub fn run<F>(dir: std::path::PathBuf, w: &'static Rust, sleep: Duration, pass: F)
where
    F: Fn(&Path) -> Result<String, String> + Send + std::panic::RefUnwindSafe + 'static,
{
    std::thread::spawn(move || {
        let mut said = String::new();
        loop {
            let line = if is_paused(&dir, w.name) {
                let line = "paused by his switch on /watchers".to_string();
                if let Err(e) = beat(&dir, w, crate::tasks::now_ms(), true, &line) {
                    eprintln!("{}: the beat: {e}", w.name);
                }
                line
            } else {
                match std::panic::catch_unwind(|| pass(&dir)) {
                    Ok(Ok(line)) => line,
                    Ok(Err(e)) => e,
                    Err(_) => "a pass panicked, the next one runs".to_string(),
                }
            };
            if line != said {
                eprintln!("{}: {line}", w.name);
                said = line;
            }
            std::thread::sleep(sleep);
        }
    });
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
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) served: &'static str,
    pub(crate) watches: String,
    pub(crate) beat: String,
    pub(crate) woke: String,
    /// What feeds it, when it says: the comment queue's compass and the old app's comments.
    pub(crate) fed: Option<String>,
    /// When it runs next, in words.
    pub(crate) next: String,
    /// The task it keeps open, or why none.
    pub(crate) task: String,
    /// His switch: the name it posts and whether it is paused, or why it has none.
    pub(crate) pause: Result<(String, bool), &'static str>,
    /// An automation's strip on the Automations tab: its runs, then its next. None for a watcher.
    pub(crate) runs: Vec<surface::components::Event>,
}

/// A row of the beat table: name, last pass and whether it worked, its line, its last wake and when.
pub(crate) type Beat = (String, Option<i64>, bool, String, Option<(String, i64)>);

/// Its last beat: a failed pass is NOT OK, a beat past twice its period LATE.
fn beat_line(every: i64, at: Option<i64>, ok: bool, said: &str, now_ms: i64) -> String {
    match at {
        Some(at) if !ok => format!("NOT OK, {}: {said}", stamp(at)),
        Some(at) if now_ms - at > 2 * every => format!("LATE, {}: {said}", stamp(at)),
        Some(at) => format!("ok, {}: {said}", stamp(at)),
        None => "none yet".into(),
    }
}

/// Every row of the beat table.
pub(crate) fn beats(dir: &Path) -> Result<Vec<Beat>, String> {
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
}

/// Each of Rust's watchers in `RUST`, its beat and wake from the table: listed before its first beat, a row of a watcher
/// no longer in `RUST` never shown, and a table that does not read costs its lines only.
fn rust_rows(dir: &Path, now_ms: i64) -> Vec<Row> {
    let read = beats(dir);
    let paused = |name: &str| is_paused(dir, name);
    let row = |w: &Rust| {
        let (beat, woke, at) = match &read {
            Err(e) => (
                format!("its beat did not read: {e}"),
                format!("its wake did not read: {e}"),
                None,
            ),
            Ok(rows) => match rows.iter().find(|r| r.0 == w.name) {
                None => ("none yet".into(), "nothing yet".into(), None),
                Some((_, at, ok, said, woke)) => (
                    beat_line(w.every_ms, *at, *ok, said, now_ms),
                    woke.as_ref().map_or("nothing yet".into(), |(what, at)| {
                        format!("{}: {what}", stamp(*at))
                    }),
                    *at,
                ),
            },
        };
        let paused = paused(w.name);
        let next = match at {
            _ if paused => "paused by his switch".into(),
            Some(at) => format!("by {}", stamp(at + w.every_ms)),
            None => "at its first pass once dash orch runs".into(),
        };
        Row {
            name: w.name.into(),
            served: "Rust",
            watches: w.watches.into(),
            beat,
            woke,
            fed: None,
            next,
            task: "none: a watcher wakes or pushes, it files no task".into(),
            pause: Ok((w.name.into(), paused)),
            runs: vec![],
        }
    };
    RUST.iter().map(|w| row(w)).collect()
}

/// His launchd jobs (`com.nicolasha.*`): the labels `launchctl print gui/<uid>` lists among its services, each with what
/// `launchctl print` says of it. `launchctl list` exits 1 in the sandbox; the domain's print answers.
pub(crate) type Launchd = Result<Vec<(String, String)>, String>;

/// The domain's services, a line each « <pid> <status> <label> »: its labels of ours, in its order.
fn our_labels(domain: &str) -> Vec<String> {
    let label = |l: &str| l.split_whitespace().nth(2).map(str::to_string);
    let services = domain
        .lines()
        .filter(|l| !l.contains("=>"))
        .filter_map(label);
    services
        .filter(|l| l.starts_with("com.nicolasha."))
        .collect()
}

fn ask_launchd() -> Launchd {
    let ctl = "/bin/launchctl";
    // SAFETY: getuid has no failure and touches no memory.
    let uid = unsafe { libc::getuid() };
    let domain = dash_process::run(ctl, &["print", &format!("gui/{uid}")], None, READ)?;
    let ours = our_labels(&domain);
    let ours = ours.iter().map(String::as_str);
    let print = |label: &str| {
        let said = dash_process::run(ctl, &["print", &format!("gui/{uid}/{label}")], None, READ);
        (
            label.to_string(),
            said.unwrap_or_else(|e| format!("\tunread = {e}")),
        )
    };
    Ok(ours.map(print).collect())
}

/// A row a launchd job: its state, runs and last exit as `launchctl print` gives them at its top level (one tab in). launchd
/// keeps no clock time of a run: its next is said from its interval.
fn launchd_rows(read: &Launchd) -> Vec<Row> {
    let jobs = match read {
        Ok(jobs) => jobs.clone(),
        Err(e) => vec![("launchd".into(), format!("\tunread = {e}"))],
    };
    let row = |(label, said): (String, String)| {
        let field = |name: &str| {
            let top = said
                .lines()
                .filter(|l| l.starts_with('\t') && !l.starts_with("\t\t"));
            top.filter_map(|l| l.trim().split_once(" = "))
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        };
        let interval = field("run interval");
        let mut beat = format!(
            "{}, runs {}, last exit {}",
            field("state").unwrap_or("unknown".into()),
            field("runs").unwrap_or("unknown".into()),
            field("last exit code").unwrap_or("none".into())
        );
        if let Some(e) = field("unread") {
            beat = format!("did not read: {e}");
        }
        Row {
            watches: interval
                .as_ref()
                .map_or("on demand, or kept alive, as its plist says".into(), |i| {
                    format!("every {i}")
                }),
            next: interval.map_or("when launchd starts it: no interval".into(), |i| {
                format!("within {i} of its last start")
            }),
            name: label,
            served: "launchd",
            beat,
            woke: "not recorded by launchd".into(),
            fed: None,
            task: "none: a launchd job files no task".into(),
            pause: Err("launchd's own: bin/sandbox-apply loads and unloads it"),
            runs: vec![],
        }
    };
    jobs.into_iter().map(row).collect()
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
                    next: "on the old app's own period".into(),
                    task: "none: the old app's watchers file none".into(),
                    pause: Err("the old app's own: no switch here"),
                    runs: vec![],
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
            next: "on the old app's own period".into(),
            task: "none: the old app's watchers file none".into(),
            pause: Err("the old app's own: no switch here"),
            runs: vec![],
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
        next: "on the old app's own period".into(),
        task: "none: the old app's watchers file none".into(),
        pause: Err("the old app's own: no switch here"),
        runs: vec![],
    }
}

/// His switch on a row: the design system's one-button form, its text « Pause: POST /watchers/pause?name=… » for agents.
/// `back`: the query that sends him back to the tab he flipped it on, « &tab=automations », or none.
fn switch(pause: &Result<(String, bool), &'static str>, back: &str) -> serde_json::Value {
    match pause {
        Ok((name, paused)) => json!({"type": "action", "props": {
            "action": format!("{PAUSE}?name={}{back}", name.replace(' ', "+")),
            "label": if *paused { "Resume" } else { "Pause" },
        }}),
        Err(why) => json!({"type": "text", "props": {"text": format!("Pause: no switch, {why}")}}),
    }
}

/// The page's two tabs, the one shown marked.
fn tabs(automations: bool) -> serde_json::Value {
    json!({"type": "links", "props": {"links": [
        {"name": "Watchers", "href": "/watchers", "current": !automations},
        {"name": "Automations", "href": AUTOMATIONS_TAB, "current": automations},
    ]}})
}

/// The Automations tab: a row an automation, its schedule, its strip of runs then its next, drawn with the design system's
/// Timeline, and its switch on the row. The text form: a record a run, then the next, under the automation's name.
fn automations_tree(rows: &[Row]) -> String {
    let mut elements = Map::new();
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Watchers"}}),
    );
    elements.insert("tabs".into(), tabs(true));
    let ours = rows;
    let note = format!(
        "Automations: {}. A dot a run of the last 7 days, failed in red, a run opens its task or its error; the hollow dot is \
         the next run.",
        ours.len()
    );
    elements.insert(
        "note".into(),
        json!({"type": "text", "props": {"text": note}}),
    );
    let mut children = vec!["title".to_string(), "tabs".into(), "note".into()];
    for (i, r) in ours.iter().enumerate() {
        let schedule =
            json!({"type": "text", "props": {"text": format!("Schedule: {}", r.watches)}});
        let strip = json!({"type": "timeline", "props": {"events": r.runs, "strip": true}});
        let parts = [
            ("schedule", schedule),
            ("runs", strip),
            ("pause", switch(&r.pause, "&tab=automations")),
        ];
        let mut names = vec![];
        for (kind, element) in parts {
            let name = format!("{kind}-{i}");
            elements.insert(name.clone(), element);
            names.push(name);
        }
        let name = format!("automation-{i}");
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

/// The day's runs that only met another's fix and pushed nothing, since 00:00 UTC (task rust-pr-one-owner, done-when 4; @nha,
/// 2026-11-10: « How will you fix systematically the wasted effort time and tokens of duplicate work ? »): a summary with their
/// tokens, then a line each. Rust's task store in `dir`, read only; one that does not read says so.
fn duplicate_lines(dir: &Path, now_ms: i64) -> Vec<String> {
    let day = now_ms - now_ms.rem_euclid(86_400_000);
    // No store yet: no run ended, and a page never makes one (/code-review of a76563791).
    if !dir.join("tasks.sqlite").is_file() {
        return vec![
            "None today: of 0 runs ended since 00:00 UTC, none only met another's fix.".into(),
        ];
    }
    let ends = dash_store::tasks::open(dir, dir).and_then(|tasks| tasks.ends_since(day));
    let ends = match ends {
        Ok(ends) => ends,
        Err(e) => return vec![format!("The runs' ends did not read: {e}")],
    };
    let dups: Vec<_> = ends.iter().filter(|e| e.end.duplicate).collect();
    let cost: u64 = dups.iter().filter_map(|e| e.end.tokens).sum();
    let (n, all) = (dups.len(), ends.len());
    let mut lines = vec![match n {
        0 => {
            format!("None today: of {all} runs ended since 00:00 UTC, none only met another's fix.")
        }
        n => format!(
            "{n} of {all} runs ended since 00:00 UTC only met another's fix and pushed nothing: {cost} tokens, cache reads left out."
        ),
    }];
    let tokens = |t: Option<u64>| t.map_or("its tokens unread".into(), |t| format!("{t} tokens"));
    lines.extend(dups.iter().map(|e| {
        format!(
            "{}, ended {}: {}",
            e.task,
            stamp(e.at_ms),
            tokens(e.end.tokens)
        )
    }));
    lines
}

/// `note`: how many watchers and how old the read is; a skeleton until the old app's watchers are read (task rust-skeleton-live).
/// `duplicates`: `duplicate_lines`, its own section under the note.
fn tree(rows: &[Row], note: Option<&str>, duplicates: &[String]) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut elements = Map::new();
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Watchers"}}),
    );
    elements.insert("tabs".into(), tabs(false));
    let note = note.map_or_else(
        || surface::skeleton("The old app's watchers"),
        |n| text(n.into()),
    );
    elements.insert("note".into(), note);
    let mut children = vec!["title".to_string(), "tabs".into(), "note".to_string()];
    if !duplicates.is_empty() {
        let mut names = vec![];
        for (i, line) in duplicates.iter().enumerate() {
            let name = format!("duplicate-{i}");
            elements.insert(name.clone(), text(line.clone()));
            names.push(name);
        }
        let title = "Duplicate runs today";
        let section = json!({"type": "section", "props": {"title": title}, "children": names});
        elements.insert("duplicates".into(), section);
        children.push("duplicates".into());
    }
    for (i, r) in rows.iter().enumerate() {
        let fed = r.fed.as_ref().map(|f| ("fed", format!("Fed by: {f}")));
        let lines = [
            ("served", format!("Served by: {}", r.served)),
            ("watches", format!("Watches: {}", r.watches)),
            ("beat", format!("Last run: {}", r.beat)),
            ("next", format!("Next run: {}", r.next)),
            ("task", format!("Open task: {}", r.task)),
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
        elements.insert(format!("pause-{i}"), switch(&r.pause, ""));
        names.push(format!("pause-{i}"));
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
pub(crate) type Old = (Result<String, String>, Result<String, String>, Launchd);

fn ask_old() -> Old {
    let cli = older_app().bin;
    let ask = |args: &[&str]| dash_process::run(&cli, args, None, READ);
    std::thread::scope(|s| {
        let said = s.spawn(|| ask(&["watchers"]));
        let wakes = s.spawn(|| ask(&["watch", "wakes", "1"]));
        let launchd = s.spawn(ask_launchd);
        let joined = |h: std::thread::ScopedJoinHandle<'_, _>| {
            h.join().unwrap_or_else(|_| Err("panicked".into()))
        };
        let launchd = launchd.join().unwrap_or_else(|_| Err("panicked".into()));
        (joined(said), joined(wakes), launchd)
    })
}

/// The page's tree from `dir`'s beats, read now, and the old app's answers with a note on their age.
/// `fed`: what feeds the comment queue (crate::comments::fed), handed in so this module never reads the queue itself.
/// `stage`: an automation's task's stage in the list (dash_store::older::stage on the live app, a stand-in in a test).
/// `automations`: crate::automations' rows, handed in so this module never reads them itself (it sits below them).
pub(crate) fn tree_of(
    dir: &Path,
    fed: &str,
    old: Option<Old>,
    age: &str,
    automations: Vec<Row>,
) -> String {
    let now_ms = crate::tasks::now_ms();
    let mut rows = rust_rows(dir, now_ms);
    if let Some(r) = rows.iter_mut().find(|r| r.name == COMMENTS.name) {
        r.fed = Some(fed.into());
    }
    let ours = automations.len();
    rows.extend(automations);
    let note = match old {
        Some((said, wakes, launchd)) => {
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
                    next: "on the old app's own period".into(),
                    task: "none: the old app's watchers file none".into(),
                    pause: Err("the old app's own: no switch here"),
                    runs: vec![],
                }),
            }
            // Dashboard.Watch and each line read: a placeholder for a silent app or an unread line is no watcher
            let old = rows
                .iter()
                .filter(|r| r.served == "the old app" && r.name != UNREAD);
            let old = old
                .filter(|r| !r.beat.starts_with("the old app did not answer"))
                .count();
            let jobs = launchd.as_ref().map_or(0, Vec::len);
            rows.extend(launchd_rows(&launchd));
            Some(format!(
                "{} schedules: {} watchers served by Rust, {ours} automations, {old} watchers by the old app, {jobs} launchd \
                 jobs. {age}",
                RUST.len() + ours + old + jobs,
                RUST.len()
            ))
        }
        None => None,
    };
    tree(
        &rows,
        note.as_deref().map(str::trim_end),
        &duplicate_lines(dir, now_ms),
    )
}

static OLD: Mutex<Read<Old>> = Mutex::new(Read::new());

/// The page at /watchers. The server routes on one thread and the old app's reads took 2.7 s at a load of 91: they show
/// from their last read, as /agents' do (crate::fresh); Rust's beats are read at each view.
/// `tab` « automations »: the Automations tab, from the automations' rows alone; any other, the watchers.
pub fn page(
    dir: &Path,
    fed: &str,
    automations: Vec<Row>,
    tab: &str,
) -> Result<surface::Drawing, String> {
    if tab == "automations" {
        return Ok(surface::Drawing::Tree {
            title: "Automations".into(),
            tree: automations_tree(&automations),
        });
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let now = now.map_err(|e| e.to_string())?.as_secs();
    let tree = match fresh::last(&OLD, now, "watchers", ask_old)? {
        Some((old, at)) => tree_of(dir, fed, Some(old), &fresh::ago(now, at), automations),
        None => tree_of(dir, fed, None, "", automations),
    };
    Ok(surface::Drawing::Tree {
        title: "Watchers".into(),
        tree,
    })
}

inventory::submit! { crate::usage::Command::usage("dash watchers --data <dir> [--tab automations]") }
/// `dash watchers --data <dir> [--tab automations]`: the page's text, for an agent with no port.
pub fn list(dir: &Path, fed: &str, automations: Vec<Row>, tab: &str, mut out: std::fs::File) {
    use std::io::Write;
    let tree = match tab {
        "automations" => automations_tree(&automations),
        _ => tree_of(dir, fed, Some(ask_old()), "", automations),
    };
    let text = surface::text(&tree);
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
    use dash_meter::dash_test;

    /// The old app's `dash watchers` of 2026-11-09 01:15, cut to three rows, one of them red.
    const OLD: &str = "Watchers: operational
  ok wake-wait: beats every 2 h; wake-wait: pid 10405 waits for supervisor 71778's wakes (2026-11-09 00:05 UTC)
  NOT OK mail poll: beats every 5 min; Last poll: stale past 10 min (2026-11-08 23:11 UTC)
    probe: bin/klar-mail poll; every: 5 min
  ok CI runner 2: beats every 4 days; disk 5.4 GB of 8.0 GB (2026-11-08 22:53 UTC)
";

    #[dash_test]
    fn each_watcher_is_one_row_in_both_forms() {
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
        assert_eq!(rows.pop().map(|r| r.name), Some(VPS.name.into()));
        assert_eq!(rows.pop().map(|r| r.name), Some(CHANCES.name.into()));
        assert_eq!(rows.pop().map(|r| r.name), Some(AUTOMATIONS.name.into()));
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
                next: "by 2026-11-09 01:16 UTC".into(),
                task: "none: a watcher wakes or pushes, it files no task".into(),
                pause: Ok(("comment queue".into(), false)),
                runs: vec![],
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
        let tree = tree(&rows, Some("5 watchers"), &[]);
        let text = surface::text(&tree).expect("text");
        let html = surface::markup(&surface::screen::page("Watchers", &tree).expect("html"));
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

    /// `launchctl print` of 2026-11-10, cut to its top level and one nested line.
    const SWEEP: &str = "gui/501/com.nicolasha.freework-sweep = {
\tactive count = 0
\tstate = not running
\truns = 4
\tlast exit code = 0
\tendpoints = {
\t\tstate = active
\t}
\trun interval = 86400 seconds
}";

    #[dash_test]
    fn every_schedule_shows_its_last_and_next_run_open_task_and_switch_in_both_forms() {
        let dir = TestDir::new("watchers-schedules");
        let launchd = Ok(vec![
            (
                "com.nicolasha.freework-sweep".to_string(),
                SWEEP.to_string(),
            ),
            (
                "com.nicolasha.rust-serve".to_string(),
                "\tstate = running\n\truns = 1\n".to_string(),
            ),
        ]);
        let old = (Ok(OLD.to_string()), Ok("1 wakes".to_string()), launchd);
        let tree = tree_of(&dir.0, "the feed", Some(old), "", vec![]);
        let text = surface::text(&tree).expect("text");
        // Rust's watchers have a switch, the old app's and launchd's say why none
        assert!(
            text.contains("Pause: POST /watchers/pause?name=comment+queue"),
            "{text}"
        );
        assert!(
            text.contains("Pause: no switch, the old app's own"),
            "{text}"
        );
        let at = text
            .find("com.nicolasha.freework-sweep\n")
            .expect("a launchd row");
        let row = &text[at..];
        for line in [
            "Served by: launchd",
            "Watches: every 86400 seconds",
            "Last run: not running, runs 4, last exit 0",
            "Next run: within 86400 seconds of its last start",
            "Open task: none: a launchd job files no task",
            "Pause: no switch, launchd's own",
        ] {
            assert!(row.contains(line), "lacks {line}: {row}");
        }
        assert!(
            text.contains("Last run: running, runs 1, last exit none"),
            "{text}"
        );
        assert!(text.contains("2 launchd jobs."), "{text}");
        let domain = "\tservices = {\n\t\t   79706      2 \tcom.nicolasha.rust-serve\n\t\t       0      0 \tcom.apple.x\n\t}\n\t\t\"com.nicolasha.MailExtensions.night-training\" => enabled\n";
        assert_eq!(our_labels(domain), ["com.nicolasha.rust-serve"]);
        // his switch flips it: the button says Resume, the watcher says paused, the HTML holds the form
        assert!(toggle(&dir.0, "comment queue", &[]).expect("pauses"));
        let tree = tree_of(&dir.0, "the feed", None, "", vec![]);
        let html = surface::markup(&surface::screen::page("Watchers", &tree).expect("html"));
        assert!(
            html.contains("/watchers/pause?name=comment+queue"),
            "{html}"
        );
        assert!(html.contains("Resume"), "{html}");
        let text = surface::text(&tree).expect("text");
        assert!(text.contains("Next run: paused by his switch"), "{text}");
        assert!(!toggle(&dir.0, "comment queue", &[]).expect("resumes"));
        // a name it is told of is known too; any other is refused
        assert!(toggle(&dir.0, "nothing", &[]).is_err());
        assert!(toggle(&dir.0, "nothing", &["nothing"]).expect("pauses"));
    }

    #[dash_test]
    fn a_silent_old_app_or_an_unread_line_costs_only_its_rows() {
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
        assert_eq!(
            (rows.len(), rows[0].name.as_str()),
            (RUST.len(), "comment queue")
        );
        assert_eq!(
            (rows[0].beat.as_str(), rows[0].woke.as_str()),
            ("none yet", "nothing yet")
        );
        let text = surface::text(&tree_of(&dir.0, "the feed", None, "", vec![])).expect("text");
        assert!(
            text.contains("\nFed by: the feed\n"),
            "the comment queue says what feeds it: {text}"
        );
        assert!(
            text.contains("comment queue") && text.contains("The old app's watchers: being read."),
            "{text}"
        );
        // a silent old app: its placeholder is no watcher in the count, Dashboard.Watch still is
        let old = (
            Err("down".to_string()),
            Err("down".to_string()),
            Err("down".to_string()),
        );
        let text = surface::text(&tree_of(
            &dir.0,
            "the feed",
            Some(old),
            "Read 0 min ago.",
            vec![],
        ))
        .expect("text");
        assert!(text.contains("the old app did not answer: down"), "{text}");
        assert!(
            // each watcher ported joins RUST: the count follows it, never a literal to edit
            text.contains(&format!(
                "{} schedules: {} watchers served by Rust, 0 automations, 1 watchers by the old app, 0 launchd jobs. Read 0 min \
                 ago.",
                RUST.len() + 1,
                RUST.len()
            )),
            "{text}"
        );
        let old = (
            Ok(format!("{OLD}  STALE x: y\n")),
            Ok("1 wakes".to_string()),
            Ok(vec![]),
        );
        let text =
            surface::text(&tree_of(&dir.0, "the feed", Some(old), "", vec![])).expect("text");
        assert!(
            text.contains(&format!(
                "{} schedules: {} watchers served by Rust, 0 automations, 4 watchers by the old app, 0 launchd jobs.",
                RUST.len() + 4,
                RUST.len()
            )),
            "{text}"
        );

        // Task rust-pr-one-owner, done-when 4: the day's duplicate runs and their tokens; yesterday's and a run that pushed are none
        let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("tasks");
        let now = crate::tasks::now_ms();
        let day = now - now.rem_euclid(86_400_000);
        let dup = |tokens| dash_core::tasks::End {
            pushed: Some(false),
            duplicate: true,
            tokens,
            nudges: 0,
        };
        tasks
            .end_at(day - 1, "yesterday", &dup(Some(9)))
            .expect("end");
        tasks
            .end_at(day, "t-pushed", &dash_core::tasks::End::default())
            .expect("end");
        tasks
            .end_at(day + 1, "t-dup", &dup(Some(40_000)))
            .expect("end");
        tasks.end_at(day + 2, "t-dup2", &dup(None)).expect("end");
        let text = surface::text(&tree_of(&dir.0, "the feed", None, "", vec![])).expect("text");
        let said = "Duplicate runs today\n2 of 3 runs ended since 00:00 UTC only met another's fix and pushed nothing: 40000 \
                    tokens, cache reads left out.\n";
        assert!(text.contains(said), "{text}");
        let lines = [
            format!("t-dup2, ended {}: its tokens unread", stamp(day + 2)),
            format!("t-dup, ended {}: 40000 tokens", stamp(day + 1)),
        ];
        assert!(text.contains(&lines.join("\n")), "{text}");
        assert!(!text.contains("yesterday,"), "{text}");
        let none = TestDir::new("watchers-none");
        let text = surface::text(&tree_of(&none.0, "the feed", None, "", vec![])).expect("text");
        assert!(
            text.contains(
                "None today: of 0 runs ended since 00:00 UTC, none only met another's fix."
            ),
            "{text}"
        );
        assert!(
            !none.0.join("tasks.sqlite").exists(),
            "the page made no store"
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
