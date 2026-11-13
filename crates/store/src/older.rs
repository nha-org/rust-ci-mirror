//! The older app's tasks.edn rows and leads for Rust, read from its SQLite file, read-only: no call to the app, so its stalls never hold a
//! pass (it answered no command for 120 s under disk load, 2026-11-09). Deleted once tasks are filed here.
//! ponytail: its schema is read as it stands (`tasks`, `links`, `project_settings`; for the chances `leads`, `events`,
//! `drafts`, `records` and their `data` keys); a change there fails the sync, named,
//! and the store keeps the last list.
use crate::similar::{Text, files_named};
use dash_core::tasks::{BUILDS_NOTHING, Comment, Demand, Filed, FreeKind, Queue, Queued, Rank};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

/// Whether row `t` is in Rust's queue: open, Rust's, neither labelled unplanned nor put off. One fragment for TASKS and `foldable`,
/// so a fold's target is live exactly when the picker can start it (task rust-fold-targets-live).
macro_rules! queued {
    () => {
        "t.collection = 'tasks.edn' AND t.stage = 'open' AND t.owner = 'rust'
    AND NOT EXISTS (SELECT 1 FROM json_each(json_extract(t.data, '$.labels')) WHERE lower(value) = 'unplanned')
    AND trim(coalesce(json_extract(t.data, '$.later'), '')) = ''"
    };
}

/// Open tasks Rust owns, in the list's order: neither labelled unplanned nor put off (`:later`). Its other holds (a project's,
/// the budget's, a file gone) are the older app's own starts', and hid ready Rust tasks (2026-11-09): Rust's plan holds by its
/// own rules. Its waits: each `waits-on` link to a task of the list not done, in the link's order, and `:blocked-by` as is.
const TASKS: &str = concat!("SELECT t.id, coalesce(t.title, ''),
    -- only an ISO datetime with its offset, as `DateTime.from_iso8601` in `Runner.reopened_ms/1`: julianday() takes more
    CASE WHEN json_type(t.data, '$.since') = 'text' AND json_extract(t.data, '$.since') GLOB '????-??-??T*'
      AND (json_extract(t.data, '$.since') GLOB '*Z' OR json_extract(t.data, '$.since') GLOB '*[+-][0-9][0-9]:[0-9][0-9]')
      THEN CAST(round((julianday(json_extract(t.data, '$.since')) - 2440587.5) * 86400000) AS INTEGER) END,
    (SELECT group_concat(substr(l.\"to\", 7), char(10) ORDER BY l.pos) FROM links l
       WHERE l.\"from\" = 'tasks:' || t.id AND l.kind = 'waits-on' AND l.\"to\" LIKE 'tasks:%'
       AND NOT EXISTS (SELECT 1 FROM tasks d WHERE d.collection = 'tasks.edn' AND 'tasks:' || d.id = l.\"to\"
                       AND d.stage = 'done')),
    -- a list stays in data (`Row.fits?/2` keeps only a text in the column): 96 rows on 2026-11-09
    coalesce(t.blocked_by, (SELECT group_concat(value, ',') FROM json_each(t.data, '$.\"blocked-by\"')
                              WHERE json_type(t.data, '$.\"blocked-by\"') = 'array')),
    -- its labels, a line apart: `dash task add --labels Docs` files one
    (SELECT group_concat(value, char(10)) FROM json_each(t.data, '$.labels')
       WHERE json_type(t.data, '$.labels') = 'array'),
    -- what its rank reads (task rust-orch-pick-dynamic): priority 1 to 4, estimate in points, its epic, its own roadmap item
    -- each cast and bounded, so an odd value is none, never a failed sync
    CASE WHEN CAST(t.priority AS INTEGER) BETWEEN 1 AND 4 THEN CAST(t.priority AS INTEGER) END,
    CASE WHEN CAST(json_extract(t.data, '$.estimate') AS INTEGER) BETWEEN 1 AND 1000
      THEN CAST(json_extract(t.data, '$.estimate') AS INTEGER) END,
    t.epic, ltrim(json_extract(t.data, '$.roadmap'), ':'),
    -- its demand (RFC 20h): its own word, else hard when its effort is high
    coalesce(ltrim(json_extract(t.data, '$.run.demand'), ':'),
      CASE WHEN ltrim(json_extract(t.data, '$.run.effort'), ':') = 'high' THEN 'hard' END),
    -- the fields the files it names come from (task task-dedupe)
    coalesce(t.why, ''), coalesce(t.done_when, '')
  FROM tasks t
  WHERE ", queued!(), "
  ORDER BY t.pos");

/// The roadmap's items in its order (`Focus.roadmap/0`): each id, its horizon today, urgent through its `until` day, else its own
/// (urgent stored by hand reads as now), and what it covers, a line apart.
/// A roadmap item's horizon today, `Focus`'s reading: urgent through its `until` day, else its own, urgent stored by hand
/// reading as now. One fragment for ROADMAP and FOCUS, so the rank and the brief never read it two ways.
macro_rules! horizon {
    () => {
        "CASE WHEN json_extract(data, '$.until') >= date('now', 'localtime') THEN 'urgent'
      ELSE replace(ltrim(coalesce(json_extract(data, '$.horizon'), 'now'), ':'), 'urgent', 'now') END"
    };
}

const ROADMAP: &str = concat!(
    "SELECT id, ",
    horizon!(),
    ",
    (SELECT group_concat(value, char(10)) FROM json_each(data, '$.covers') WHERE json_type(data, '$.covers') = 'array')
  FROM records WHERE collection = 'focus-overall.edn' AND id IS NOT NULL ORDER BY pos"
);

/// A roadmap item: its id, horizon and covers.
type Item = (String, String, Vec<String>);

/// The item task `id` serves and its horizon, as `Focus.owner/2` finds it: its own `:roadmap` first (« none » serves none; one
/// the roadmap no longer has falls through, as an item renamed), else the first item covering its id or a prefix of it ending
/// in « * », else the first naming its epic (as id or cover) or a label.
fn item_of(
    items: &[Item],
    id: &str,
    epic: Option<&str>,
    labels: &[String],
    own: Option<&str>,
) -> Option<(String, String)> {
    let found = |item: &Item| Some((item.0.clone(), item.1.clone()));
    if own == Some("none") {
        return None;
    }
    if let Some(item) = own.and_then(|own| items.iter().find(|i| i.0 == own)) {
        return found(item);
    }
    let by_key = |c: &String| c == id || c.strip_suffix('*').is_some_and(|p| id.starts_with(p));
    let by_group = |item: &Item| {
        let names = epic.into_iter().chain(labels.iter().map(String::as_str));
        names
            .into_iter()
            .any(|n| item.0 == n || item.2.iter().any(|c| c == n))
    };
    (items.iter().find(|i| i.2.iter().any(by_key)))
        .or_else(|| items.iter().find(|i| by_group(i)))
        .and_then(found)
}

/// A name of `:blocked-by` the list holds done: met, as `Runner.finished/2` reads a done task.
const DONE: &str =
    "SELECT 1 FROM tasks WHERE collection = 'tasks.edn' AND id = ?1 AND stage = 'done'";

/// A link ref in the older store's text, `\u{E000}@waits-on/0\u{E001}`: its target is read from the links.
const REF: char = '\u{E000}';

/// The names `:blocked-by`'s words hold, as `Runner.names/1` splits them (« a, b and c »), its link refs left out.
fn names(words: &str) -> Vec<String> {
    // `,\s*|\s+and\s+`: any run of blanks around « and » is one
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words.replace(" and ", ",");
    let names = words.split(',').map(str::trim).filter(|n| !n.is_empty());
    names
        .filter(|n| !n.starts_with(REF))
        .map(String::from)
        .collect()
}

/// His seats' parts, as `Dashboard.Orch.Admission.seats/1` reads them: whether it is night (22:00 to 07:00 local), the grant the
/// forecast loop writes (`claude-boost`, « n » or « n unix », 0 to 3, anything else none), whether `dash drain` holds (`Holds.drain/1`,
/// project `*`, field `drain`) and Rust's own cap, `dash project set site_perso rust-seats <n>` (0 to 9, anything else none).
const SEATS: &str = "SELECT
  CAST(strftime('%H', 'now', 'localtime') AS INTEGER) >= 22 OR CAST(strftime('%H', 'now', 'localtime') AS INTEGER) < 7,
  coalesce((SELECT CASE WHEN trim(value) GLOB '[0-3]' OR trim(value) GLOB '[0-3] *' THEN CAST(substr(trim(value), 1, 1) AS INTEGER)
      ELSE 0 END FROM project_settings WHERE project = 'site_perso' AND field = 'claude-boost'), 0),
  EXISTS (SELECT 1 FROM project_settings WHERE project = '*' AND field = 'drain' AND coalesce(value, '') <> ''),
  (SELECT CAST(trim(value) AS INTEGER) FROM project_settings
      WHERE project = 'site_perso' AND field = 'rust-seats' AND trim(value) GLOB '[0-9]')";

/// What a binding rust-seats cap adds to the reason: it never expires, and how to clear it.
pub const CAP_HINT: &str =
    "a setting that never expires (`dash project set site_perso rust-seats` clears it)";

/// His seats and what set them (task rust-seats-follow-room; @nha, 2026-11-11: « Think about the system and what would improve it
/// for the future. Spike, measure. »): Admission's, 5 by day and 8 at night plus the forecast's `boost`, which Rust's `cap`
/// (rust-seats) only lowers. A drain starts nothing, unless Rust has its cap: then the drain holds the older app's runner alone,
/// and Rust runs on (@nha, 2026-11-09: « finish merging. do not start new work. »; 2026-11-10: « continue with the comments tasks.
/// orchestrate via rust and obeserve »). The machine's load and memory are the policy's, read at each pick.
pub fn seats(night: bool, boost: u8, drain: bool, cap: Option<u8>) -> (usize, String) {
    let (base, when) = match night {
        true => (8, "night"),
        false => (5, "day"),
    };
    let admission = base + boost as usize;
    let said = match boost {
        0 => format!("{when} {base}"),
        _ => format!("{when} {base} + boost {boost}"),
    };
    match cap {
        // a cap outlived its reason once: set at 4 while the load had no guard, it held night's 8 back after the guard merged
        // (task rust-seats-follow-room's PM check, 2026-11-12), so the line says it never expires and how to clear it; under a
        // drain clearing it stops Rust, so no hint then (/code-review)
        Some(cap) if (cap as usize) < admission => (
            cap as usize,
            match drain {
                true => format!(
                    "{said}, capped at {cap} by rust-seats, which lets Rust run through dash drain"
                ),
                false => format!("{said}, capped at {cap} by rust-seats, {CAP_HINT}"),
            },
        ),
        Some(_) => (admission, said),
        None if drain => (
            0,
            format!("drain: no start while dash drain holds ({said} otherwise)"),
        ),
        None => (admission, said),
    }
}

pub(crate) fn opened(db: &Path) -> Result<Connection, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut db_ = Connection::open_with_flags(db, flags).map_err(named)?;
    db_.busy_timeout(Duration::from_secs(5)).map_err(named)?;
    crate::timed::trace(&mut db_, db).map_err(named)?;
    Ok(db_)
}

/// The index the older app's task add searches its events by (`Model.task_named?/1`, `json_extract(data, '$.task') IN ...`):
/// without it each add scanned all 57,186 events, 74 to 78 ms warm and 0.49 s cold; with it 0.1 ms (task
/// rust-store-load-measured, on a copy of the live store). Made by Rust, since no Elixir edit passes the pre-commit; the
/// build holds the write lock 1.2 s cold at most, once, under the older app's 2 s busy_timeout, and every later call only reads the
/// schema. No database or no `events` table there (a worktree, CI): nothing made.
pub fn index_events_by_task(db: &Path) -> Result<(), String> {
    if !db.exists() {
        return Ok(());
    }
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(db, flags).map_err(named)?;
    conn.busy_timeout(Duration::from_secs(10)).map_err(named)?;
    crate::timed::trace(&mut conn, db).map_err(named)?;
    let events: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'events')",
            [],
            |r| r.get(0),
        )
        .map_err(named)?;
    if events {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS events_task ON events (json_extract(data, '$.task'))",
        )
        .map_err(named)?;
    }
    Ok(())
}

/// The tables a step's ref names (« drafts:x »), and the stages that close a step: `Model.waits/1`'s `@facts` and `@closed`.
const STEP_TABLES: [&str; 10] = [
    "leads",
    "contacts",
    "drafts",
    "blocks",
    "tasks",
    "handoff_steps",
    "mails",
    "comments",
    "docs",
    "records",
];
const CLOSED: &str = "'done', 'dropped', 'rejected', 'moot', 'sent', 'won', 'lost', 'resolved'";

/// What step `from` (« drafts:x ») waits on in `db`, still open, in its links' order: the older app's `Model.waits/1` for one
/// step (task rust-draft-state-edit). A step closed when any of its rows is; one in no table is shown, as there.
pub fn waits(db: &Path, from: &str) -> Result<Vec<String>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let sql = "SELECT \"to\" FROM links WHERE kind = 'waits-on' AND \"from\" = ?1 ORDER BY pos";
    let mut q = db_.prepare(sql).map_err(named)?;
    let targets = q
        .query_map([from], |r| r.get::<_, String>(0))
        .map_err(named)?;
    let mut open = vec![];
    for to in targets {
        let to = to.map_err(named)?;
        let closed = match to.split_once(':') {
            // a table of the list only: its name is written into the query
            Some((table, id)) if STEP_TABLES.contains(&table) => {
                let sql = format!("SELECT 1 FROM {table} WHERE id = ?1 AND stage IN ({CLOSED})");
                db_.prepare(&sql)
                    .and_then(|mut q| q.exists([id]))
                    .map_err(named)?
            }
            _ => false,
        };
        if !closed {
            open.push(to);
        }
    }
    Ok(open)
}

/// A consistent copy of `db` at `to`, read while the older app writes it (task rust-elixir-reference: the frozen app on :7799
/// reads a copy, never the live file). `to` and its journal files are replaced.
pub fn snapshot(db: &Path, to: &Path) -> Result<(), String> {
    for end in ["", "-wal", "-shm"] {
        let mut file = to.as_os_str().to_owned();
        file.push(end);
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {e}", to.display()));
            }
            _ => {}
        }
    }
    let into = to
        .to_str()
        .ok_or_else(|| format!("{}: not UTF-8", to.display()))?;
    let vacuum = |c: Connection| c.execute("VACUUM INTO ?1", [into]).map(drop);
    let said = |e| format!("{} into {}: {e}", db.display(), to.display());
    match vacuum(opened(db)?) {
        // A WAL database no connection holds has no -wal and no -shm, and a read-only open must create the -shm: in a folder
        // the reader may not write, that fails (cannot open under a task's sandbox on the live store, read-only under a
        // chmod; rust-live-copy). With no -wal and no hot -journal the file is whole, so it is read as immutable. Only a read
        // that failed outright takes it. ponytail: a writer opening it during the copy may checkpoint under that read; a
        // copy for checks takes that risk, a backup would not.
        Err(e)
            if matches!(
                e.sqlite_error_code(),
                Some(rusqlite::ErrorCode::CannotOpen | rusqlite::ErrorCode::ReadOnly)
            ) && !journal(db, "-wal")
                && !journal(db, "-journal") =>
        {
            let path = db.display().to_string();
            let path = path
                .replace('%', "%25")
                .replace('?', "%3f")
                .replace('#', "%23");
            let uri = format!("file:{path}?immutable=1");
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
            let mut whole =
                Connection::open_with_flags(uri, flags).map_err(|e| said(e.to_string()))?;
            crate::timed::trace(&mut whole, db).map_err(|e| said(e.to_string()))?;
            vacuum(whole).map_err(|e| said(e.to_string()))
        }
        read => read.map_err(|e| said(e.to_string())),
    }
}

/// Whether `db`'s journal file `end` (« -wal », « -journal ») is there.
fn journal(db: &Path, end: &str) -> bool {
    let mut file = db.as_os_str().to_owned();
    file.push(end);
    Path::new(&file).exists()
}

/// The list in `db`, the older app's database.
pub fn queue(db: &Path) -> Result<Queue, String> {
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// Whether the list in `db` holds task `id`, at any stage: what a comment's answer links is a task filed, never a name made up.
pub fn filed(db: &Path, id: &str) -> Result<bool, String> {
    let sql = "SELECT 1 FROM tasks WHERE collection = 'tasks.edn' AND id = ?1";
    let found = opened(db)?.prepare(sql).and_then(|mut q| q.exists([id]));
    found.map_err(|e| format!("{}: {e}", db.display()))
}

/// Those of `ids` the list in `db` holds at no stage, read on one connection. A list holding no task at all is refused: an
/// empty or swapped database is no proof that a task was never filed.
pub fn unfiled(db: &Path, ids: &[String]) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let any = "SELECT 1 FROM tasks WHERE collection = 'tasks.edn' LIMIT 1";
    if !db_
        .prepare(any)
        .and_then(|mut q| q.exists([]))
        .map_err(named)?
    {
        return Err(format!("{}: the list holds no task", db.display()));
    }
    let mut filed = (db_.prepare("SELECT 1 FROM tasks WHERE collection = 'tasks.edn' AND id = ?1"))
        .map_err(named)?;
    let mut stray = vec![];
    for id in ids {
        if !filed.exists([id]).map_err(named)? {
            stray.push(id.clone());
        }
    }
    Ok(stray)
}

/// Task `id`'s stage in `db` (« open », « done », « dropped », « new », « backlog ») and its `:run :prompt`, none when the list
/// has no such task: what an automation reads before filing its one task (crates/dash/src/automations.rs).
pub fn stage(db: &Path, id: &str) -> Result<Option<(String, Option<String>)>, String> {
    let sql = "SELECT coalesce(stage, 'open'), json_extract(data, '$.run.prompt') FROM tasks
               WHERE collection = 'tasks.edn' AND id = ?1";
    let read = opened(db)?.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?)));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read.map(Some).map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// site_perso's setting `field` in `db` (`project_settings`), none when unset: the handover reads his `claude-auto` switch
/// (`dash claude-accounts auto on|off`) here (task rust-account-handover-port).
pub fn setting(db: &Path, field: &str) -> Result<Option<String>, String> {
    let sql = "SELECT value FROM project_settings WHERE project = 'site_perso' AND field = ?1";
    let read = opened(db)?.query_row(sql, [field], |r| r.get::<_, String>(0));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read.map(Some).map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// Whether task `id` is graal's, and the task that replaces it (its `:duplicate-of`): a tasks.edn row of project graal that
/// Rust does not own, or an issue of graal's board (`issues-graal.edn`). `None` for any other task (task graal-moves-to-rust:
/// graal starts nothing, its work moves to Rust).
pub fn graal_held(db: &Path, id: &str) -> Result<Option<Option<String>>, String> {
    let sql = "SELECT json_extract(data, '$.\"duplicate-of\"') FROM tasks WHERE collection = 'tasks.edn' AND id = ?1
                 AND project = 'graal' AND ltrim(coalesce(owner, ''), ':') <> 'rust'
               UNION ALL SELECT json_extract(data, '$.\"duplicate-of\"') FROM records
                 WHERE collection = 'issues-graal.edn' AND id = ?1
               LIMIT 1";
    let read = opened(db)?.query_row(sql, [id], |r| r.get::<_, Option<String>>(0));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read.map(Some).map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// The pane of task `id`'s claim in `db`, none when it holds none: `dash pane say` takes a task id for its pane, as the push hook
/// names a reviewer by it (/code-review of PR 1044).
pub fn claim_pane(db: &Path, id: &str) -> Result<Option<String>, String> {
    let sql = "SELECT pane FROM task_claims WHERE task = ?1";
    let read = opened(db)?.query_row(sql, [id], |r| r.get::<_, Option<String>>(0));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read.map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// The PM pass's last recorded view in `db` (`orch_views`, task orch-view): when it read, as written (UTC), its age in seconds,
/// and its inconsistency lines. `None` before the first pass.
pub fn orch_view(db: &Path) -> Result<Option<(String, i64, Vec<String>)>, String> {
    let named = |e: String| format!("{}: {e}", db.display());
    let sql = "SELECT at, CAST((julianday('now') - julianday(at)) * 86400 AS INTEGER),
                 coalesce(json_extract(view, '$.inconsistencies'), '[]')
               FROM orch_views ORDER BY day DESC LIMIT 1";
    let read = opened(db)?.query_row(sql, [], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    });
    let (at, age, lines) = match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        read => read.map_err(|e| named(e.to_string()))?,
    };
    let lines = serde_json::from_str(&lines).map_err(|e| named(format!("its view: {e}")))?;
    Ok(Some((at, age, lines)))
}

/// Each claimed pane's task in `db`: what tells the watcher a pane asking a question is a task's (task rust-watch-port).
pub fn claimed_panes(db: &Path) -> Result<Vec<(String, String)>, String> {
    let failed = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db = opened(db)?;
    let mut read = (db.prepare("SELECT pane, task FROM task_claims WHERE pane IS NOT NULL"))
        .map_err(failed)?;
    let rows = read
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(failed)?;
    rows.collect::<Result<_, _>>().map_err(failed)
}

/// Task `id`'s newest report in `db` (`dash task report`): its row id and its record, JSON as written.
pub fn last_report(db: &Path, id: &str) -> Result<Option<(i64, String)>, String> {
    let sql = "SELECT id, record FROM task_reports WHERE task = ?1 ORDER BY id DESC LIMIT 1";
    let read = opened(db)?.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?)));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read.map(Some).map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// The list's labels, Linear's four groups in its order: type, area, agent, plan (rfc-dashboard 5.5; the older app's
/// `Dashboard.Labels.set/0`). A label is a kind of work, never a project: 12, shared by every project.
pub const LABELS: [&str; 12] = [
    "Bug",
    "Feature",
    "Improvement",
    "Research",
    "Docs",
    "Design",
    "Infra",
    "Security",
    "Perf",
    "Claude",
    "vibe",
    "Unplanned",
];

/// The agent's labels: the Linear sync sets them from who did the work, never his menu.
const AGENT_LABELS: [&str; 2] = ["Claude", "vibe"];

/// The labels he sets from an issue's menu (task rust-issue-sidebar-menus): the set but the agent's.
pub fn settable_labels() -> Vec<&'static str> {
    LABELS
        .into_iter()
        .filter(|l| !AGENT_LABELS.contains(l))
        .collect()
}

/// A task's labels as every view shows them, `Labels.of/2`'s: each of the set once, in the set's spelling (a stored « docs » is
/// Docs), any other word left off.
pub fn labels_of<'a>(held: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut shown: Vec<String> = vec![];
    for word in held {
        let word = word.trim().trim_start_matches(':');
        if let Some(l) = LABELS.iter().find(|l| l.eq_ignore_ascii_case(word))
            && !shown.iter().any(|s| s == l)
        {
            shown.push(l.to_string());
        }
    }
    shown
}

/// Who runs a task, Linear's assignee: the older app's `Tasks.owners/0`, its `:owner`; none is claude.
pub const OWNERS: [&str; 4] = ["claude", "nicolas", "vibe", "rust"];

/// What an issue page's sidebar sets of a task, as the older app holds it (task rust-issue-sidebar-menus): Linear's assignee
/// (`:owner`, none is claude), its labels in the set's spelling, its project (`:epic`), its cycle (a Monday) and the day it
/// reminds him (`:remind`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sidebar {
    pub owner: String,
    pub labels: Vec<String>,
    pub epic: Option<String>,
    pub cycle: Option<String>,
    pub remind: Option<String>,
}

/// Each tasks.edn row's sidebar; `cycle` is a column, the older app's `:remind` and labels sit in its data. A column's field is
/// read from its column alone: its data holds the model's marker there, U+E000 « col » U+E001 (`Model.Row`'s `@col`), and a
/// task with no cycle showed « col » (rust-issue-sidebar-menus' live check). A reminder that is no text is none: a number
/// there would fail every row's read.
const SIDEBAR: &str = "SELECT id, ltrim(coalesce(nullif(owner, ''), 'claude'), ':'),
    (SELECT group_concat(value, char(10)) FROM json_each(data, '$.labels') WHERE json_type(data, '$.labels') = 'array'),
    nullif(ltrim(coalesce(epic, ''), ':'), ''),
    nullif(cycle, ''),
    CASE json_type(data, '$.remind') WHEN 'text' THEN nullif(json_extract(data, '$.remind'), '') END
  FROM tasks WHERE collection = 'tasks.edn' AND id IS NOT NULL";

fn sidebar_row(r: &rusqlite::Row) -> rusqlite::Result<(String, Sidebar)> {
    let labels: Option<String> = r.get(2)?;
    let side = Sidebar {
        owner: r.get(1)?,
        labels: labels_of(labels.as_deref().unwrap_or("").lines()),
        epic: r.get(3)?,
        cycle: r.get(4)?,
        remind: r.get(5)?,
    };
    Ok((r.get(0)?, side))
}

/// Every task's sidebar in `db`, by id: what the board's reader keeps for the issue pages, one read a minute.
pub fn sidebars(db: &Path) -> Result<std::collections::BTreeMap<String, Sidebar>, String> {
    let read = |db: &Connection| -> rusqlite::Result<_> {
        db.prepare(SIDEBAR)?.query_map([], sidebar_row)?.collect()
    };
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// What an imported project's issue says of who holds it (`Dashboard.Board.Backlog`'s `issues-<project>.edn`): its assignees,
/// and the day it was last written, updated else created.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IssueMarks {
    pub assignees: Vec<String>,
    pub day: Option<String>,
}

/// Each issue of `project` in `db` by its number, as `dash task ls` names it (klar-101 is « 101 »): what the board's In
/// Progress truth reads (task rust-board-in-progress-truth). The store, never the Backlog.md files: an imported project's
/// truth is here.
pub fn issue_marks(
    db: &Path,
    project: &str,
) -> Result<std::collections::BTreeMap<String, IssueMarks>, String> {
    let sql = "SELECT id, json_extract(data, '$.assignee'),
        coalesce(json_extract(data, '$.updated'), json_extract(data, '$.created'))
      FROM records WHERE collection = ?1 AND id IS NOT NULL";
    let prefix = format!("{project}-");
    let row = |r: &rusqlite::Row| -> rusqlite::Result<(String, IssueMarks)> {
        let id: String = r.get(0)?;
        let who: Option<String> = r.get(1)?;
        // a list as JSON, or one name
        let assignees = match who.as_deref().map(serde_json::from_str::<Vec<String>>) {
            Some(Ok(names)) => names,
            _ => who.into_iter().collect(),
        };
        let day = r.get(2)?;
        let id = id.strip_prefix(&prefix).unwrap_or(&id).to_string();
        Ok((id, IssueMarks { assignees, day }))
    };
    let collection = format!("issues-{project}.edn");
    let read = |db: &Connection| -> rusqlite::Result<_> {
        db.prepare(sql)?.query_map([&collection], row)?.collect()
    };
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// Task `id`'s sidebar in `db`, none when the list has no such task: what a menu's write starts from, read as it stands.
pub fn sidebar(db: &Path, id: &str) -> Result<Option<Sidebar>, String> {
    let sql = format!("{SIDEBAR} AND id = ?1");
    match opened(db)?.query_row(&sql, [id], sidebar_row) {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read
            .map(|(_, s)| Some(s))
            .map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// The projects a task may join, the epics tasks.edn names at any stage (`Write.property("epic")` takes no other), in
/// order of name: the issue page's « Add to project » menu.
pub fn epics(db: &Path) -> Result<Vec<String>, String> {
    let sql = "SELECT ltrim(epic, ':') AS e FROM tasks WHERE collection = 'tasks.edn' AND trim(coalesce(epic, '')) NOT IN ('', ':')
               GROUP BY e ORDER BY e";
    let read = |db: &Connection| -> rusqlite::Result<Vec<String>> {
        db.prepare(sql)?.query_map([], |r| r.get(0))?.collect()
    };
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// A task's words, as `Launch.argv/2` read them: its run's kind before its own, plan-first on the row or its run.
const FILED: &str = "SELECT coalesce(title, ''), coalesce(why, ''), coalesce(done_when, ''),
    coalesce(json_extract(data, '$.run.prompt'), ''),
    ltrim(coalesce(json_extract(data, '$.run.kind'), json_extract(data, '$.kind'), ''), ':'), coalesce(epic, ''),
    -- a path absent is NULL, and NULL OR false is NULL: none is false
    coalesce(plan_first = 1 OR json_extract(data, '$.\"plan-first\"') = 1 OR json_extract(data, '$.run.\"plan-first\"') = 1, 0),
    CASE WHEN CAST(json_extract(data, '$.run.turns') AS INTEGER) > 0 THEN CAST(json_extract(data, '$.run.turns') AS INTEGER) END
  FROM tasks WHERE collection = 'tasks.edn' AND id = ?1";
/// His comments still waiting on Claude, in his order, as `Comments.untreated_comments/0` reads them: not resolved, and no
/// Claude reply after his last word. A reviewer's run (review-<id>) gets those on the task it reviews too (`Watch.owners/1`).
const COMMENTS: &str =
    "SELECT coalesce(id, ''), coalesce(file, ''), coalesce(quote, ''), coalesce(text, '')
  FROM comments WHERE (task = ?1 OR 'review-' || task = ?1) AND coalesce(stage, '') <> 'resolved'
    AND coalesce(json_extract(data, '$.replies[#-1].by'), '') <> 'claude'
  ORDER BY pos";
/// Its Linear key, as `dash linear sync` linked it: a row without a Linear address is none (`Dashboard.Linear.links/0`).
const LINEAR: &str = "SELECT json_extract(data, '$.identifier') FROM records
  WHERE collection = 'linear.edn' AND id = ?1 AND json_extract(data, '$.url') LIKE 'https://linear.app/%'";
/// The overall focuses, `Focus.overall/0`: the roadmap's Urgent items, then its Now ones, horizons read as ROADMAP reads them.
const FOCUS: &str = concat!(
    "SELECT id, coalesce(title, id) FROM (SELECT id, title, pos, ",
    horizon!(),
    " AS h FROM records WHERE collection = 'focus-overall.edn' AND id IS NOT NULL)
  WHERE h IN ('urgent', 'now') ORDER BY h = 'now', pos"
);

/// What task `id`'s brief is written from, none when the list has no such task (task brief-tests-prepush-skip: Rust writes
/// the brief, never `dash task brief`).
pub fn brief(db: &Path, id: &str) -> Result<Option<Filed>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let row = db_.query_row(FILED, [id], |r| {
        Ok(Filed {
            id: id.to_string(),
            title: r.get(0)?,
            why: r.get(1)?,
            done_when: r.get(2)?,
            prompt: r.get(3)?,
            kind: r.get(4)?,
            epic: r.get(5)?,
            plan_first: r.get(6)?,
            turns: r.get(7)?,
            ..Filed::default()
        })
    });
    let mut filed = match row {
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        row => row.map_err(named)?,
    };
    let mut q = db_.prepare(COMMENTS).map_err(named)?;
    let comments = q.query_map([id], |r| {
        let (id, file, quote, text) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        Ok(Comment {
            id,
            file,
            quote,
            text,
        })
    });
    filed.comments = comments.and_then(Iterator::collect).map_err(named)?;
    let linear = db_.query_row(LINEAR, [id], |r| r.get::<_, Option<String>>(0));
    filed.linear = match linear {
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        key => key.map_err(named)?.filter(|k| !k.is_empty()),
    };
    let mut q = db_.prepare(FOCUS).map_err(named)?;
    let focus = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)));
    filed.focus = focus.and_then(Iterator::collect).map_err(named)?;
    Ok(Some(filed))
}

/// The open tasks of `project` as `dash task add`'s duplicate check reads them (`dash_store::similar`, task task-dedupe): the
/// rows `TaskStyle.overlaps/3` compares, each with the three fields its words come from.
const OPEN_TEXTS: &str = "SELECT id, coalesce(title, ''), coalesce(why, ''), coalesce(done_when, '') FROM tasks
  WHERE collection = 'tasks.edn' AND stage = 'open' AND id IS NOT NULL
    -- a row with no project is the older app's own: site_perso, as `task add` files one (/code-review: 88 open rows)
    AND (?1 IS NULL OR coalesce(project, 'site_perso') = ?1) ORDER BY pos";

/// The open tasks of `project` in `db`, the older app's database; every project's with none.
pub fn open_texts(db: &Path, project: Option<&str>) -> Result<Vec<Text>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let mut q = db_.prepare(OPEN_TEXTS).map_err(named)?;
    let rows = q.query_map([project], |r| {
        Ok(Text {
            id: r.get(0)?,
            title: r.get(1)?,
            why: r.get(2)?,
            done_when: r.get(3)?,
        })
    });
    rows.and_then(Iterator::collect).map_err(named)
}

/// A lead as the older app files it, what its chances read (crates/dash/src/chances.rs): its columns, its `data` as JSON,
/// whether a human answered on its timeline (a call or a LinkedIn entry not ours, `Fields.human?/1`) and whether one of its
/// drafts holds screening answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Lead {
    pub id: String,
    pub posted: Option<String>,
    pub remote: Option<String>,
    pub onsite: Option<String>,
    pub data: String,
    pub human: bool,
    pub answered: bool,
}

const LEADS: &str = "SELECT l.id, l.posted, l.remote, l.onsite_expectation, l.data,
    EXISTS (SELECT 1 FROM events e WHERE e.source = 'timeline:' || l.id
            AND json_extract(e.data, '$.channel') IN ('~:call', '~:linkedin', 'call', 'linkedin')
            AND coalesce(json_type(e.data, '$.by'), 'null') IN ('null', 'false')),
    -- `List.wrap(answers) != []`: any value but none or an empty list
    EXISTS (SELECT 1 FROM drafts d WHERE d.lead = l.id AND coalesce(json_type(d.data, '$.answers'), 'null') <> 'null'
            AND (json_type(d.data, '$.answers') <> 'array' OR json_array_length(d.data, '$.answers') > 0))
  FROM leads l WHERE l.collection = 'opportunities' AND l.id IS NOT NULL ORDER BY l.pos";

/// Every lead in `db`, the older app's database.
pub fn leads(db: &Path) -> Result<Vec<Lead>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let mut q = db_.prepare(LEADS).map_err(named)?;
    let rows = q.query_map([], |r| {
        Ok(Lead {
            id: r.get(0)?,
            posted: r.get(1)?,
            remote: r.get(2)?,
            onsite: r.get(3)?,
            data: r.get(4)?,
            human: r.get(5)?,
            answered: r.get(6)?,
        })
    });
    rows.and_then(Iterator::collect).map_err(named)
}

/// A lead or a task as /search lists them (crates/dash/src/search.rs, FindLive's port, task rust-search-port): its kind, id,
/// title, the line under it and the words beside those it is found by, as `Dashboard.Search`'s rows were.
#[derive(Debug, Clone, PartialEq)]
pub struct Searchable {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub line: String,
    pub words: String,
}

/// The leads in their order (a key starting with _ is their template), then the tasks of the list in theirs. A lead by its
/// company, where it came from, its stage, place, length and stack; a task by its id, stage and why.
const SEARCHABLE: &str = "SELECT kind, id, title, line, words FROM (
    SELECT 'lead' AS kind, 0 AS n, pos, id, coalesce(nullif(title, ''), id) AS title,
      concat_ws(' · ', nullif(company, ''), nullif(via, ''), nullif(stage, '')) AS line,
      concat_ws(' ', location, duration, (SELECT group_concat(value, ' ') FROM json_each(data, '$.stack')
                                          WHERE json_type(data, '$.stack') = 'array')) AS words
    FROM leads WHERE collection = 'opportunities' AND id IS NOT NULL AND coalesce(key, '') NOT LIKE '\\_%' ESCAPE '\\'
    UNION ALL
    SELECT 'task', 1, pos, id, coalesce(nullif(title, ''), id), concat_ws(' · ', id, stage), coalesce(why, '')
    FROM tasks WHERE collection = 'tasks.edn' AND id IS NOT NULL)
  ORDER BY n, pos";

/// Every lead and task in `db`, the older app's database.
pub fn searchable(db: &Path) -> Result<Vec<Searchable>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let mut q = db_.prepare(SEARCHABLE).map_err(named)?;
    let rows = q.query_map([], |r| {
        Ok(Searchable {
            kind: r.get(0)?,
            id: r.get(1)?,
            title: r.get(2)?,
            line: r.get(3)?,
            words: r.get(4)?,
        })
    });
    rows.and_then(Iterator::collect).map_err(named)
}

/// A map source of the model (`profile.edn`, `skills.edn`): each record's key and its data, JSON.
pub fn records(db: &Path, collection: &str) -> Result<Vec<(String, String)>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let sql = "SELECT coalesce(id, ltrim(key, ':')), data FROM records WHERE collection = ?1 ORDER BY pos";
    let db_ = opened(db)?;
    let mut q = db_.prepare(sql).map_err(named)?;
    let rows = q.query_map([collection], |r| Ok((r.get(0)?, r.get(1)?)));
    rows.and_then(Iterator::collect).map_err(named)
}

fn roadmap(db: &Connection) -> rusqlite::Result<Vec<Item>> {
    let mut items = db.prepare(ROADMAP)?;
    let items = items.query_map([], |r| {
        let covers: Option<String> = r.get(2)?;
        let covers = covers
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        Ok((r.get(0)?, r.get(1)?, covers))
    })?;
    items.collect()
}

/// A roadmap item as /plan draws it (task linear-projects-view): its id, title, horizon today and definition of done, then
/// its numbers from its tasks, each task counted under the item `item_of` finds for it. `points`: its live tasks' estimates
/// (1 for none, a dropped one left out), Linear's project progress; `done`: those of the done ones. `first`: the day its
/// first live task was added; `last`: the day its last one was done, both ISO days.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadmapItem {
    pub id: String,
    pub title: String,
    pub horizon: String,
    pub dod: Option<String>,
    pub points: i64,
    pub done: i64,
    pub first: Option<String>,
    pub last: Option<String>,
    /// Its page and its RFC as the focus row holds them, and its live tasks counted.
    pub link: Option<String>,
    pub rfc: Option<String>,
    pub tasks: i64,
}

const ROADMAP_ITEMS: &str = concat!(
    "SELECT id, coalesce(title, id), ",
    horizon!(),
    ",
    (SELECT group_concat(value, char(10)) FROM json_each(data, '$.covers') WHERE json_type(data, '$.covers') = 'array'),
    json_extract(data, '$.dod'), json_extract(data, '$.link'), json_extract(data, '$.rfc')
  FROM records WHERE collection = 'focus-overall.edn' AND id IS NOT NULL ORDER BY pos"
);

/// Every live task with what places it and what it counts: its points, whether done, the days it was added and done (the
/// columns open with an ISO day: « 2026-11-10: merged »).
const ROADMAP_TASKS: &str = "SELECT t.id, t.epic,
    (SELECT group_concat(value, char(10)) FROM json_each(t.data, '$.labels') WHERE json_type(t.data, '$.labels') = 'array'),
    ltrim(json_extract(t.data, '$.roadmap'), ':'),
    CASE WHEN CAST(json_extract(t.data, '$.estimate') AS INTEGER) BETWEEN 1 AND 1000
      THEN CAST(json_extract(t.data, '$.estimate') AS INTEGER) ELSE 1 END,
    coalesce(t.stage, 'open') = 'done', substr(t.added, 1, 10), substr(t.done, 1, 10)
  FROM tasks t WHERE t.collection = 'tasks.edn' AND coalesce(t.stage, 'open') <> 'dropped'";

/// Every roadmap item in its order with its numbers (task linear-projects-view: « progress comes from its tasks »).
pub fn roadmap_items(db: &Path) -> Result<Vec<RoadmapItem>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let read = || -> rusqlite::Result<Vec<RoadmapItem>> {
        let mut found = vec![];
        let mut q = db_.prepare(ROADMAP_ITEMS)?;
        for row in q.query_map([], |r| {
            let covers: Option<String> = r.get(3)?;
            let item = RoadmapItem {
                id: r.get(0)?,
                title: r.get(1)?,
                horizon: r.get(2)?,
                dod: r.get(4)?,
                link: r.get(5)?,
                rfc: r.get(6)?,
                ..RoadmapItem::default()
            };
            Ok((item, covers.unwrap_or_default()))
        })? {
            found.push(row?);
        }
        let items: Vec<Item> = (found.iter())
            .map(|(i, c)| {
                (
                    i.id.clone(),
                    i.horizon.clone(),
                    c.lines().map(String::from).collect(),
                )
            })
            .collect();
        let mut q = db_.prepare(ROADMAP_TASKS)?;
        let tasks = q.query_map([], |r| {
            let labels: Option<String> = r.get(2)?;
            let labels: Vec<String> = labels
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect();
            let (id, epic, own): (String, Option<String>, Option<String>) =
                (r.get(0)?, r.get(1)?, r.get(3)?);
            let item = item_of(&items, &id, epic.as_deref(), &labels, own.as_deref());
            let days = |s: Option<String>| s.filter(|d| dash_core::is_day(d));
            Ok((
                item.map(|i| i.0),
                r.get::<_, i64>(4)?,
                r.get::<_, bool>(5)?,
                days(r.get(6)?),
                days(r.get(7)?),
            ))
        })?;
        for task in tasks {
            let (Some(item), points, done, added, closed) = task? else {
                continue;
            };
            let Some((i, _)) = found.iter_mut().find(|(i, _)| i.id == item) else {
                continue;
            };
            i.points += points;
            i.tasks += 1;
            if done {
                i.done += points;
                i.last = i.last.take().max(closed);
            }
            i.first = match (i.first.take(), added) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        Ok(found.into_iter().map(|(i, _)| i).collect())
    };
    read().map_err(named)
}

/// A PR the board links to a task (`pr_issues` over `pr_links`): `kind` closing or contributing, `state` open, merged or closed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkedPr {
    pub url: String,
    pub number: Option<i64>,
    pub state: String,
    pub kind: String,
    pub draft: bool,
}

/// What `dash linear sync` writes of a task (`Dashboard.Board.Issues`' fields the Linear sync read): every tasks.edn row, its
/// stage and fields, the roadmap item that covers it (`item_of`), whether the older app's claims hold it, and its PRs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinearTask {
    pub id: String,
    pub title: String,
    pub why: Option<String>,
    pub done_when: Option<String>,
    /// new, backlog, open, done or dropped
    pub stage: String,
    /// 1 urgent to 4 low
    pub priority: Option<i64>,
    pub epic: Option<String>,
    pub owner: String,
    pub parent: Option<String>,
    /// tasks.edn's `added` and `done` (« 2026-10-22: merged »), each opening with its day
    pub added: Option<String>,
    pub done: Option<String>,
    /// Linear's points, 1, 2, 3, 5 or 8, else its run's effort word
    pub estimate: Option<i64>,
    pub effort: Option<String>,
    /// its run's agent (vibe's work is labelled vibe whatever its owner)
    pub agent: Option<String>,
    pub labels: Vec<String>,
    pub due: Option<String>,
    pub every: Option<String>,
    pub item: Option<String>,
    pub claimed: bool,
    pub prs: Vec<LinkedPr>,
}

const LINEAR_TASKS: &str = "SELECT t.id, coalesce(t.title, t.id), t.why, t.done_when, coalesce(t.stage, 'open'), t.priority,
    nullif(ltrim(coalesce(t.epic, ''), ':'), ''), ltrim(coalesce(nullif(t.owner, ''), 'claude'), ':'), t.parent, t.added,
    t.done,
    CASE WHEN json_extract(t.data, '$.estimate') IN (1, 2, 3, 5, 8) THEN json_extract(t.data, '$.estimate') END,
    CASE json_type(t.data, '$.run.effort') WHEN 'text' THEN ltrim(json_extract(t.data, '$.run.effort'), ':') END,
    CASE json_type(t.data, '$.run.agent') WHEN 'text' THEN ltrim(json_extract(t.data, '$.run.agent'), ':') END,
    (SELECT group_concat(value, char(10)) FROM json_each(t.data, '$.labels') WHERE json_type(t.data, '$.labels') = 'array'),
    CASE json_type(t.data, '$.due') WHEN 'text' THEN nullif(json_extract(t.data, '$.due'), '') END,
    CASE json_type(t.data, '$.every') WHEN 'text' THEN nullif(json_extract(t.data, '$.every'), '') END,
    ltrim(json_extract(t.data, '$.roadmap'), ':'),
    EXISTS (SELECT 1 FROM task_claims c WHERE c.task = t.id)
  FROM tasks t WHERE t.collection = 'tasks.edn' AND t.id IS NOT NULL ORDER BY t.pos";

const LINKED_PRS: &str = "SELECT i.issue, l.url, l.number, l.state, i.kind, l.draft
  FROM pr_issues i JOIN pr_links l ON l.url = i.pr ORDER BY l.number DESC";

/// A text with each mention, « \u{E000}m:<table>:<id>\u{E001} », as the name `names` holds for « <table>:<id> », else its id.
fn unmentioned(s: &str, names: &std::collections::HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("\u{E000}m:") {
        let inside = &rest[at + "\u{E000}m:".len()..];
        let Some(end) = inside.find('\u{E001}') else {
            break;
        };
        out.push_str(&rest[..at]);
        let r = &inside[..end];
        out.push_str(
            names
                .get(r)
                .map_or_else(|| r.rsplit(':').next().unwrap_or(r), String::as_str),
        );
        rest = &inside[end + '\u{E001}'.len_utf8()..];
    }
    out + rest
}

/// linear.edn as the store holds it: each task's Linear identifier and url, by its id.
pub fn linear_links(
    db: &Path,
) -> Result<std::collections::BTreeMap<String, (String, String)>, String> {
    let sql =
        "SELECT id, json_extract(data, '$.identifier'), json_extract(data, '$.url') FROM records
      WHERE collection = 'linear.edn' AND id IS NOT NULL";
    let read = |db: &Connection| -> rusqlite::Result<_> {
        db.prepare(sql)?
            .query_map([], |r| {
                let (id, idf, url): (String, Option<String>, Option<String>) =
                    (r.get(0)?, r.get(1)?, r.get(2)?);
                Ok((id, (idf.unwrap_or_default(), url.unwrap_or_default())))
            })?
            .collect()
    };
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// Every task as `dash linear sync` writes it, in the list's order.
pub fn linear_tasks(db: &Path) -> Result<Vec<LinearTask>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let read = || -> rusqlite::Result<Vec<LinearTask>> {
        let items = roadmap(&db_)?;
        let mut prs: std::collections::HashMap<String, Vec<LinkedPr>> = Default::default();
        let mut q = db_.prepare(LINKED_PRS)?;
        for row in q.query_map([], |r| {
            let pr = LinkedPr {
                url: r.get(1)?,
                number: r.get(2)?,
                state: r.get(3)?,
                kind: r.get(4)?,
                draft: r.get::<_, Option<bool>>(5)?.unwrap_or(false),
            };
            Ok((r.get::<_, String>(0)?, pr))
        })? {
            let (issue, pr) = row?;
            prs.entry(issue).or_default().push(pr);
        }
        // a mention (« \u{E000}m:companies:comet\u{E001} ») reads as its name, as the older app's pages and the issues show it
        let mut q = db_.prepare(
            "SELECT 'companies:' || id, name FROM companies WHERE name IS NOT NULL
             UNION ALL SELECT 'contacts:' || id, name FROM contacts WHERE name IS NOT NULL",
        )?;
        let names: std::collections::HashMap<String, String> = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let named = |s: Option<String>| s.map(|s| unmentioned(&s, &names));
        let mut q = db_.prepare(LINEAR_TASKS)?;
        let tasks = q.query_map([], |r| {
            let labels: Option<String> = r.get(14)?;
            let own: Option<String> = r.get(17)?;
            Ok(LinearTask {
                id: r.get(0)?,
                title: r.get(1)?,
                why: r.get(2)?,
                done_when: r.get(3)?,
                stage: r.get(4)?,
                priority: r.get(5)?,
                epic: r.get(6)?,
                owner: r.get(7)?,
                parent: r.get(8)?,
                added: r.get(9)?,
                done: r.get(10)?,
                estimate: r.get(11)?,
                effort: r.get(12)?,
                agent: r.get(13)?,
                labels: labels_of(labels.as_deref().unwrap_or("").lines()),
                due: r.get(15)?,
                every: r.get(16)?,
                item: own,
                claimed: r.get(18)?,
                prs: vec![],
            })
        })?;
        let mut all = vec![];
        for task in tasks {
            let mut t = task?;
            t.title = unmentioned(&t.title, &names);
            (t.why, t.done_when) = (named(t.why.take()), named(t.done_when.take()));
            let own = t.item.take();
            t.item =
                item_of(&items, &t.id, t.epic.as_deref(), &t.labels, own.as_deref()).map(|i| i.0);
            t.prs = prs.remove(&t.id).unwrap_or_default();
            all.push(t);
        }
        Ok(all)
    };
    read().map_err(named)
}

/// The tasks of the list named in ?1, a line apart, at any stage: each id, whether the older app closed it (`done` or `dropped`:
/// it left the board) and its roadmap horizon (urgent, now, next, later) as the plan finds it, none on no item. What /comments
/// draws a filed comment's card from (task comments-board-pill).
const PLACES: &str = "SELECT t.id, coalesce(t.stage, 'open') IN ('done', 'dropped'),
    (SELECT group_concat(value, char(10)) FROM json_each(t.data, '$.labels') WHERE json_type(t.data, '$.labels') = 'array'),
    t.epic, ltrim(json_extract(t.data, '$.roadmap'), ':')
  FROM tasks t WHERE t.collection = 'tasks.edn'
    AND instr(char(10) || ?1 || char(10), char(10) || t.id || char(10)) > 0";

/// A task's place in the list: closed, and its roadmap horizon.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub closed: bool,
    pub horizon: Option<String>,
}

fn places_in(db: &Connection, ids: &[String]) -> rusqlite::Result<Vec<(String, Place)>> {
    let items = roadmap(db)?;
    let mut rows = db.prepare(PLACES)?;
    let rows = rows.query_map([ids.join("\n")], |r| {
        let id: String = r.get(0)?;
        let labels: Option<String> = r.get(2)?;
        let labels: Vec<String> = labels
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        let (epic, own): (Option<String>, Option<String>) = (r.get(3)?, r.get(4)?);
        let horizon = item_of(&items, &id, epic.as_deref(), &labels, own.as_deref()).map(|i| i.1);
        let closed = r.get(1)?;
        Ok((id, Place { closed, horizon }))
    })?;
    rows.collect()
}

/// The place of each task of `ids` in `db`, the older app's database; a task it does not hold has none.
pub fn places(db: &Path, ids: &[String]) -> Result<Vec<(String, Place)>, String> {
    places_in(&opened(db)?, ids).map_err(|e| format!("{}: {e}", db.display()))
}

/// Whether task `id`'s row in `db`, the older app's database, says `review true` (`dash store set tasks.edn <id> --review true`):
/// its PR then lands only after a reviewer's verdict (task rust-land-waits-review). A task it does not hold says no.
pub fn review(db: &Path, id: &str) -> Result<bool, String> {
    let sql = "SELECT coalesce(json_extract(data, '$.review') = 1, 0) FROM tasks WHERE collection = 'tasks.edn' AND id = ?1";
    let read = opened(db)?.query_row(sql, [id], |r| r.get(0));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        read => read.map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// A task a lesson's fold may name: every project's, any stage (task rust-fold-port).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foldable {
    pub id: String,
    /// Its project, empty for site_perso's oldest rows.
    pub project: String,
    pub stage: String,
    pub epic: Option<String>,
    /// The lesson turns its raises summed so far.
    pub lesson_turns: i64,
    /// Its owner, empty when it has none.
    pub owner: String,
    /// Whether Rust's picker can start it: in its queue (`queued!`).
    pub queued: bool,
    /// The task it was merged into (`:duplicate-of`).
    pub duplicate_of: Option<String>,
    /// Its `:run :project` and `:run :prompt`, empty when unset: what a start reads (task run-fields-at-add).
    pub run_project: String,
    pub prompt: String,
    /// Its layer, kept in Rust's store, never in this list: none here, its reader fills it.
    pub layer: Option<String>,
}

impl Foldable {
    /// The fields a start of it lacks, every one at once (task run-fields-at-add: nanolathe-local-ai failed two starts, one
    /// missing field each): a Rust task's layer (Rust's picker runs site_perso's alone), an agent's run project (the older app
    /// reads no owner as claude, `Tasks.owner/1`), and a run prompt. A task he owns (« nicolas ») starts no session: it lacks
    /// nothing. Here, below dash's `folds` and `layers`, so both read it with no module loop between them.
    pub fn missing(&self) -> Vec<&'static str> {
        let blank = |s: &str| s.trim().is_empty();
        let mut lacks = vec![];
        match self.owner.as_str() {
            "rust" if self.layer.is_none() => lacks.push("layer"),
            "rust" => {}
            "" | "claude" | "vibe" if blank(&self.run_project) => lacks.push("run project"),
            "" | "claude" | "vibe" => {}
            _ => return lacks,
        }
        if blank(&self.prompt) {
            lacks.push("run prompt");
        }
        lacks
    }
}

/// Every task in `db`, every project's, for a fold to name (`Dashboard.Tracker.all/0`'s ids and stages).
pub fn foldable(db: &Path) -> Result<Vec<Foldable>, String> {
    let sql = concat!(
        "SELECT t.id, coalesce(t.project, ''), coalesce(t.stage, 'open'), t.epic,
        coalesce(CAST(json_extract(t.data, '$.\"lesson-turns\"') AS INTEGER), 0), coalesce(t.owner, ''),
        coalesce((",
        // a NULL stage or owner leaves the test NULL: not queued
        queued!(),
        "), 0),
        -- text only: a number or a list there failed every fold's read (/code-review)
        CASE json_type(t.data, '$.\"duplicate-of\"') WHEN 'text' THEN json_extract(t.data, '$.\"duplicate-of\"') END,
        CASE json_type(t.data, '$.run.project') WHEN 'text' THEN json_extract(t.data, '$.run.project') ELSE '' END,
        CASE json_type(t.data, '$.run.prompt') WHEN 'text' THEN json_extract(t.data, '$.run.prompt') ELSE '' END
      FROM tasks t WHERE t.collection = 'tasks.edn' AND t.id IS NOT NULL ORDER BY t.pos"
    );
    let db_ = opened(db)?;
    let read = db_.prepare(sql).and_then(|mut q| {
        q.query_map([], |r| {
            Ok(Foldable {
                id: r.get(0)?,
                project: r.get(1)?,
                stage: r.get(2)?,
                epic: r.get(3)?,
                lesson_turns: r.get(4)?,
                owner: r.get(5)?,
                queued: r.get(6)?,
                duplicate_of: r
                    .get::<_, Option<String>>(7)?
                    .map(|d| d.trim_start_matches(':').to_string()),
                run_project: r.get(8)?,
                prompt: r.get(9)?,
                layer: None,
            })
        })?
        .collect()
    });
    read.map_err(|e| format!("{}: {e}", db.display()))
}

/// His feedback before `before` (local time, as the table keeps it): his comments and queue entries, and of his chats only
/// the corrections, as `Dashboard.Orch.Folds` reads them: each row's time, source, ref and words.
pub fn his_feedback(db: &Path, before: &str) -> Result<Vec<[String; 4]>, String> {
    let sql = "SELECT at, source, coalesce(ref, ''), text FROM feedback
      WHERE at <> '' AND at < ?1 AND (source <> 'chat' OR kind = 'correction') AND text IS NOT NULL ORDER BY at";
    let db_ = opened(db)?;
    let read = db_.prepare(sql).and_then(|mut q| {
        q.query_map([before], |r| {
            Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
        })?
        .collect()
    });
    read.map_err(|e| format!("{}: {e}", db.display()))
}

fn read(db: &Connection) -> rusqlite::Result<Queue> {
    let items = roadmap(db)?;
    let mut rows = db.prepare(TASKS)?;
    let rows = rows.query_map([], |r| {
        let links: Option<String> = r.get(3)?;
        let links = links.unwrap_or_default();
        let labels: Option<String> = r.get(5)?;
        let labels: Vec<String> = labels
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        let light = |l: &String| BUILDS_NOTHING.iter().any(|b| b.eq_ignore_ascii_case(l));
        let id: String = r.get(0)?;
        let (epic, own): (Option<String>, Option<String>) = (r.get(8)?, r.get(9)?);
        let item = item_of(&items, &id, epic.as_deref(), &labels, own.as_deref());
        let title: String = r.get(1)?;
        let files = files_named(&Text {
            id: String::new(),
            title: title.clone(),
            why: r.get(11)?,
            done_when: r.get(12)?,
        });
        let task = Queued {
            held: None,
            files,
            title,
            reopened_ms: r.get(2)?,
            waits: links.lines().map(String::from).collect(),
            // the older app keeps no layer: Rust's store holds it, by `dash task add --layer`
            layer: None,
            builds_nothing: labels.iter().any(light),
            // its deadline is Rust's, kept by `dash task add --deadline`
            rank: Rank {
                item: item.as_ref().map(|i| i.0.clone()),
                horizon: item.map(|i| i.1),
                deadline: None,
                priority: r.get(6)?,
                estimate: r.get(7)?,
                // its impact is Rust's, kept by `dash task add --impact`
                impact: None,
            },
            free: FreeKind::of(&labels),
            demand: Demand::of(r.get::<_, Option<String>>(10)?.as_deref()),
            epic: epic.filter(|e| !e.is_empty()),
            labels,
            // Rust's store resolves it at each read of its queue
            route: None,
            id,
        };
        Ok((task, r.get::<_, Option<String>>(4)?))
    })?;
    let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut done = db.prepare(DONE)?;
    let mut tasks = vec![];
    // A name in words that is no task of the list holds it, as the older app holds it, until the words are taken off.
    for (mut task, words) in rows {
        for name in names(&words.unwrap_or_default()) {
            if !done.exists([&name])? && !task.waits.contains(&name) {
                task.waits.push(name);
            }
        }
        tasks.push(task);
    }
    let (seats, seats_why) = db.query_row(SEATS, [], |r| {
        Ok(seats(r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })?;
    Ok(Queue {
        seats,
        seats_why,
        tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// Task rust-store-load-measured, done-when 2: the older app's task-add check (`Model.task_named?/1`'s events part,
    /// verbatim) scans every event before the index and searches by it after; a second call is a no-op, and no database there
    /// makes none.
    #[dash_test]
    fn the_task_add_check_searches_events_by_their_task_index() {
        let dir = TestDir::new("older-events-task");
        let db = dir.0.join("older.db");
        let conn = Connection::open(&db).expect("db");
        conn.execute_batch(
            "CREATE TABLE events (source TEXT, pos INTEGER, about TEXT, data TEXT, PRIMARY KEY (source, pos));
             INSERT INTO events VALUES ('live/task-journal.jsonl', 0, NULL, '{\"task\":\"a\"}');",
        )
        .expect("table");
        let check = "EXPLAIN QUERY PLAN SELECT EXISTS (SELECT 1 FROM events WHERE (about IS NULL OR about NOT LIKE 'tasks:%')
                     AND source NOT LIKE 'timeline:%'
                     AND json_extract(data, '$.task') IN (SELECT value FROM json_each(?1)))";
        let plan = |c: &Connection| -> String {
            let mut q = c.prepare(check).expect("plan");
            q.query_map(["[\"a\"]"], |r| r.get::<_, String>(3))
                .expect("rows")
                .map(|r| r.expect("row"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(plan(&conn).contains("SCAN events"), "{}", plan(&conn));
        assert_eq!(index_events_by_task(&db), Ok(()));
        assert_eq!(index_events_by_task(&db), Ok(()));
        let after = plan(&Connection::open(&db).expect("db"));
        assert!(
            after.contains("SEARCH events USING INDEX events_task"),
            "{after}"
        );
        let none = dir.0.join("none.db");
        assert_eq!(index_events_by_task(&none), Ok(()));
        assert!(!none.exists());
    }

    /// Task rust-seats-follow-room, done-when 1, a test per case: Rust's seats are Admission's, 5 by day and 8 at night plus the
    /// forecast's boost; rust-seats only lowers them, never raises them; a drain starts nothing unless Rust has its cap.
    #[dash_test]
    fn seats_by_day_are_five() {
        assert_eq!(seats(false, 0, false, None), (5, "day 5".into()));
    }

    #[dash_test]
    fn seats_at_night_are_eight() {
        assert_eq!(seats(true, 0, false, None), (8, "night 8".into()));
    }

    #[dash_test]
    fn the_forecast_boost_adds_seats_by_day_and_at_night() {
        assert_eq!(seats(false, 2, false, None), (7, "day 5 + boost 2".into()));
        assert_eq!(
            seats(true, 3, false, None),
            (11, "night 8 + boost 3".into())
        );
    }

    #[dash_test]
    fn rust_seats_caps_them_saying_it_never_expires_and_never_raises_them() {
        assert_eq!(
            seats(true, 2, false, Some(4)),
            (
                4,
                format!("night 8 + boost 2, capped at 4 by rust-seats, {CAP_HINT}")
            ),
        );
        assert_eq!(
            seats(false, 0, false, Some(9)),
            (5, "day 5".into()),
            "never raises them"
        );
    }

    #[dash_test]
    fn a_drain_starts_nothing_unless_rust_has_its_cap() {
        assert_eq!(
            seats(false, 1, true, None),
            (
                0,
                "drain: no start while dash drain holds (day 5 + boost 1 otherwise)".into()
            )
        );
        assert_eq!(
            seats(false, 1, true, Some(9)),
            (6, "day 5 + boost 1".into()),
            "with its cap, Rust runs through a drain"
        );
        assert_eq!(
            seats(false, 1, true, Some(4)),
            (4, "day 5 + boost 1, capped at 4 by rust-seats, which lets Rust run through dash drain".into()),
            "no hint to clear a cap a drain needs (/code-review)"
        );
    }

    /// The older app's tables, as much of them as the read takes.
    const OLDER: &str =
        "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL,
        title TEXT, blocked_by TEXT, owner TEXT, priority INTEGER, epic TEXT, cycle TEXT, why TEXT, done_when TEXT);
      CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, data TEXT NOT NULL);
      CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
      CREATE TABLE project_settings (project TEXT, field TEXT, value TEXT NOT NULL);";

    fn task(
        db: &Connection,
        pos: i64,
        id: &str,
        stage: &str,
        owner: &str,
        data: &str,
        by: Option<&str>,
    ) {
        let sql = "INSERT INTO tasks (collection, pos, id, stage, data, title, blocked_by, owner) VALUES ('tasks.edn', ?1, ?2, ?3, ?4, ?2, ?5, ?6)";
        db.execute(sql, rusqlite::params![pos, id, stage, data, by, owner])
            .expect("a task");
    }

    /// Task linear-projects-view, done-when 1: an item's progress comes from its tasks, each counted under the item that
    /// covers it: points its estimates (1 for none), a dropped task left out; its first day the earliest added, its last the
    /// latest done.
    #[dash_test]
    fn a_roadmap_item_s_progress_comes_from_its_tasks() {
        let dir = dash_meter::TestDir::new("older-roadmap");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, epic TEXT,
               added TEXT, done TEXT);
             CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, title TEXT, data TEXT NOT NULL);
             INSERT INTO records VALUES ('focus-overall.edn', 0, 'rust', 'Rust', '{\"dod\":\"It builds itself\",\"link\":\"/v/rfc-x.html\"}');
             INSERT INTO records VALUES ('focus-overall.edn', 1, 'jobs', 'Jobs',
               '{\"horizon\":\"next\",\"covers\":[\"apply-*\"]}');
             INSERT INTO tasks VALUES ('tasks.edn', 0, 'a', 'done', '{\"estimate\":3}', 'rust', '2026-11-02', '2026-11-05: merged');
             INSERT INTO tasks VALUES ('tasks.edn', 1, 'b', 'open', '{}', 'rust', '2026-11-01', NULL);
             INSERT INTO tasks VALUES ('tasks.edn', 2, 'c', 'dropped', '{\"estimate\":8}', 'rust', '2026-10-01', NULL);
             INSERT INTO tasks VALUES ('tasks.edn', 3, 'apply-x', 'done', '{}', NULL, '2026-11-03', '2026-11-04');
             INSERT INTO tasks VALUES ('tasks.edn', 4, 'other', 'open', '{}', NULL, '2026-01-01', NULL);",
        )
        .expect("tables");
        let mut items = roadmap_items(&path).expect("read");
        assert_eq!(
            (items[0].link.take().as_deref(), items[0].rfc.as_deref()),
            (Some("/v/rfc-x.html"), None)
        );
        let item = |id, title: &str, horizon: &str, dod: Option<&str>, n, d, f: &str, l: &str| {
            RoadmapItem {
                id: String::from(id),
                title: title.into(),
                horizon: horizon.into(),
                dod: dod.map(String::from),
                points: n,
                done: d,
                first: Some(f.into()),
                last: Some(l.into()),
                tasks: if id == "rust" { 2 } else { 1 },
                ..RoadmapItem::default()
            }
        };
        assert_eq!(
            items,
            [
                item(
                    "rust",
                    "Rust",
                    "now",
                    Some("It builds itself"),
                    4,
                    3,
                    "2026-11-01",
                    "2026-11-05"
                ),
                item(
                    "jobs",
                    "Jobs",
                    "next",
                    None,
                    1,
                    1,
                    "2026-11-03",
                    "2026-11-04"
                ),
            ]
        );
    }

    /// Task rust-issue-sidebar-menus, done-when 1: the label list his menu offers is the set but the agent's, and a task's
    /// labels read in the set's spelling, once, any other word left off.
    #[dash_test]
    fn the_label_list_is_the_set_s_and_a_task_s_labels_read_in_its_spelling() {
        assert_eq!(
            settable_labels(),
            [
                "Bug",
                "Feature",
                "Improvement",
                "Research",
                "Docs",
                "Design",
                "Infra",
                "Security",
                "Perf",
                "Unplanned"
            ]
        );
        assert_eq!(
            labels_of(["docs", ":bug", "Docs", "site_perso", "claude"]),
            ["Docs", "Bug", "Claude"]
        );
    }

    /// Task rust-issue-sidebar-menus, done-when 1: each task's sidebar is read off its row, an owner none being claude, its
    /// labels in the set's spelling, its cycle from the column; the epic list is every stage's epics, once, by name.
    #[dash_test]
    fn a_task_s_sidebar_and_the_epic_list_are_read_off_the_list() {
        let dir = dash_meter::TestDir::new("older-sidebar");
        let file = dir.0.join("older.db");
        let db = Connection::open(&file).expect("db");
        db.execute_batch(OLDER).expect("tables");
        task(
            &db,
            1,
            "a",
            "open",
            "rust",
            r#"{"labels":["docs","site_perso"],"remind":"2026-11-13"}"#,
            None,
        );
        // b's reminder is no text: none, every other row still read
        task(&db, 2, "b", "done", "", r#"{"remind":1}"#, None);
        // c's cycle was cleared: the model leaves its column's marker in its data, its column empty
        task(
            &db,
            3,
            "c",
            "open",
            "nicolas",
            "{\"cycle\":\"\u{E000}col\u{E001}\"}",
            None,
        );
        db.execute(
            "UPDATE tasks SET epic = 'rust-poc', cycle = '2026-11-09' WHERE id = 'a'",
            [],
        )
        .expect("a's epic");
        db.execute("UPDATE tasks SET epic = ':crm' WHERE id = 'b'", [])
            .expect("b's epic");
        db.execute("UPDATE tasks SET epic = 'rust-poc' WHERE id = 'c'", [])
            .expect("c's epic");
        let a = Sidebar {
            owner: "rust".into(),
            labels: vec!["Docs".into()],
            epic: Some("rust-poc".into()),
            cycle: Some("2026-11-09".into()),
            remind: Some("2026-11-13".into()),
        };
        assert_eq!(sidebar(&file, "a"), Ok(Some(a.clone())));
        assert_eq!(sidebar(&file, "z"), Ok(None));
        let all = sidebars(&file).expect("every sidebar");
        assert_eq!(all.get("a"), Some(&a));
        assert_eq!(all["b"].owner, "claude", "none is claude");
        assert_eq!(all["b"].epic.as_deref(), Some("crm"));
        assert_eq!(all["c"].cycle, None, "the column's marker is no cycle");
        assert_eq!(all["b"].remind, None, "a number is no reminder");
        assert_eq!(epics(&file), Ok(vec!["crm".to_string(), "rust-poc".into()]));
    }

    /// Task rust-elixir-reference: the copy the frozen app reads holds the rows, and a second copy replaces the first.
    #[dash_test]
    fn a_snapshot_copies_the_database_and_replaces_the_last_one() {
        let dir = dash_meter::TestDir::new("snapshot");
        let (db, to) = (dir.0.join("live.db"), dir.0.join("copy.db"));
        let live = Connection::open(&db).expect("db");
        live.execute_batch(OLDER).expect("tables");
        task(&live, 1, "a", "open", "rust", "{}", None);
        snapshot(&db, &to).expect("the first copy");
        task(&live, 2, "b", "open", "rust", "{}", None);
        snapshot(&db, &to).expect("a copy over the first");
        let copy = Connection::open(&to).expect("the copy");
        let n: i64 = copy
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .expect("its rows");
        assert_eq!(n, 2);
    }

    /// Task task-dedupe, done-when 3's input: each open task carries the files its title, why and done-when name, and
    /// `open_texts` reads a project's open tasks as the duplicate check compares them.
    #[dash_test]
    fn a_task_carries_the_files_it_names_and_a_project_s_open_texts_are_read() {
        let dir = dash_meter::TestDir::new("older-files");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(OLDER).expect("tables");
        db.execute_batch("ALTER TABLE tasks ADD COLUMN project TEXT")
            .expect("project");
        task(&db, 0, "a", "open", "rust", "{}", None);
        task(&db, 1, "shut", "done", "rust", "{}", None);
        task(&db, 2, "graal-one", "open", "rust", "{}", None);
        db.execute_batch(
            "UPDATE tasks SET why = 'Fix rust/crates/dash/src/main.rs', done_when = '1. bin/site-check passes: a test',
               project = 'site_perso' WHERE id IN ('a', 'shut');
             UPDATE tasks SET project = 'graal' WHERE id = 'graal-one';
             INSERT INTO tasks (collection, pos, id, stage, data, title) VALUES ('tasks.edn', 3, 'no-project', 'open', '{}', 'N');",
        )
        .expect("fields");
        let q = queue(&path).expect("queue");
        let a = q.tasks.iter().find(|t| t.id == "a").expect("a");
        assert_eq!(a.files, ["rust/crates/dash/src/main.rs", "bin/site-check"]);
        let texts = open_texts(&path, Some("site_perso")).expect("texts");
        let ids: Vec<&str> = texts.iter().map(|t| t.id.as_str()).collect();
        // a row with no project is site_perso's, as the older app's add files one
        assert_eq!(ids, ["a", "no-project"]);
        assert_eq!(texts[0].why, "Fix rust/crates/dash/src/main.rs");
    }

    #[dash_test]
    fn rust_s_open_tasks_come_in_list_order_with_their_unmet_waits_and_reopen_time() {
        let db = Connection::open_in_memory().expect("db");
        db.execute_batch(OLDER).expect("tables");
        task(
            &db,
            5,
            "b",
            "open",
            "rust",
            r#"{"since":"2026-11-08T21:26:52.543184Z","labels":["Feature","docs"]}"#,
            Some("\u{E000}@waits-on/0\u{E001} and \u{E000}@waits-on/1\u{E001}"),
        );
        task(&db, 1, "a", "open", "rust", "{}", None);
        task(&db, 2, "his", "open", "claude", "{}", None);
        task(
            &db,
            3,
            "later",
            "open",
            "rust",
            r#"{"later":"after a"}"#,
            None,
        );
        task(
            &db,
            4,
            "unplanned",
            "open",
            "rust",
            r#"{"labels":["Unplanned"]}"#,
            None,
        );
        task(&db, 6, "shut", "done", "rust", "{}", None);
        let words = "\u{E000}@waits-on/0\u{E001} and shut, his DNS steps and a";
        task(&db, 7, "words", "open", "rust", "{}", Some(words));
        db.execute(
            "INSERT INTO links VALUES ('tasks:words', 'waits-on', 'drafts:x', 0)",
            [],
        )
        .expect("a link to no task");
        task(&db, 8, "old", "dropped", "claude", "{}", None);
        let link = "INSERT INTO links VALUES ('tasks:b', 'waits-on', ?1, ?2)";
        for (to, pos) in [
            ("tasks:his", 1),
            ("tasks:shut", 0),
            ("tasks:old", 2),
            ("tasks:a", 3),
        ] {
            db.execute(link, rusqlite::params![to, pos])
                .expect("a link");
        }
        // task rust-orch-pick-dynamic, done-when 5: what the rank reads, from the task and the roadmap (focus-overall.edn)
        let item = "INSERT INTO records VALUES ('focus-overall.edn', ?1, ?2, ?3)";
        for (pos, id, data) in [
            (
                0,
                "dash-sqlite",
                r#"{"horizon":"later","covers":["dashboard","b*"]}"#,
            ),
            (1, "rust-poc", r#"{"covers":["rust"]}"#),
            (2, "trip", r#"{"horizon":"next","until":"2999-01-01"}"#),
        ] {
            db.execute(item, rusqlite::params![pos, id, data])
                .expect("an item");
        }
        db.execute("UPDATE tasks SET priority = 1, epic = 'rust', data = json_set(data, '$.estimate', 5) WHERE id = 'a'", [])
            .expect("a's fields");
        db.execute("UPDATE tasks SET epic = 'rust', data = json_set(data, '$.roadmap', ':trip') WHERE id = 'words'", [])
            .expect("its own item");
        // odd values are none or read as the number, never a failed sync; an own item gone falls through to its prefix
        let odd = "UPDATE tasks SET priority = 1.0, data = json_set(data, '$.estimate', 99999999999, '$.roadmap', ':gone')
          WHERE id = 'b'";
        db.execute(odd, []).expect("b's odd fields");
        // RFC 20h's demand: an effort high is hard, a task's own word wins over its effort (task rust-pools-fable, done-when 2)
        let high = "UPDATE tasks SET data = json_set(data, '$.run', json('{\"effort\":\":high\"}')) WHERE id = 'a'";
        db.execute(high, []).expect("a's effort");
        let own = "UPDATE tasks SET data = json_set(data, '$.run', json('{\"effort\":\"high\",\"demand\":\"normal\"}'))
          WHERE id = 'words'";
        db.execute(own, []).expect("words' demand");
        let queue = read(&db).expect("the list");
        let ids: Vec<&str> = queue.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "words"]);
        let rank = |at: usize| {
            let r = &queue.tasks[at].rank;
            (
                r.item.as_deref(),
                r.horizon.as_deref(),
                r.priority,
                r.estimate,
            )
        };
        assert_eq!(
            rank(0),
            (Some("rust-poc"), Some("now"), Some(1), Some(5)),
            "its epic covered, no horizon is now"
        );
        assert_eq!(
            rank(1),
            (Some("dash-sqlite"), Some("later"), Some(1), None),
            "its own item gone, its id by a prefix; a real 1.0 is priority 1, an estimate past 1000 none"
        );
        assert_eq!(
            rank(2),
            (Some("trip"), Some("urgent"), None, None),
            "its own item first, urgent until its day"
        );
        let kinds: Vec<_> = queue.tasks.iter().map(|t| t.free).collect();
        assert_eq!(
            kinds,
            [None, Some(FreeKind::Planning), None],
            "a Docs label is planning work for big-pickle"
        );
        let light: Vec<bool> = queue.tasks.iter().map(|t| t.builds_nothing).collect();
        assert_eq!(light, [false, true, false], "a Docs label builds nothing");
        // the graph's selectors read each task's epic and labels (task rust-graph-why-these)
        let epics: Vec<Option<&str>> = queue.tasks.iter().map(|t| t.epic.as_deref()).collect();
        assert_eq!(epics, [Some("rust"), None, Some("rust")]);
        assert_eq!(queue.tasks[1].labels, ["Feature", "docs"]);
        let demands: Vec<Demand> = queue.tasks.iter().map(|t| t.demand).collect();
        assert_eq!(demands, [Demand::Hard, Demand::Normal, Demand::Normal]);
        assert_eq!(
            queue.tasks[1].waits,
            ["his", "old", "a"],
            "a done wait is met, a dropped one is not, as Runner.finished/2"
        );
        assert_eq!(queue.tasks[1].reopened_ms, Some(1_794_173_212_543));
        assert_eq!(queue.tasks[0].reopened_ms, None);
        assert_eq!(
            queue.tasks[2].waits,
            ["his DNS steps", "a"],
            "each name of its words, a done one met"
        );
        assert!([5, 8].contains(&queue.seats), "{}", queue.seats);
        db.execute(
            "INSERT INTO project_settings VALUES ('site_perso', 'claude-boost', '2 1794100000')",
            [],
        )
        .expect("a boost");
        assert!([7, 10].contains(&read(&db).expect("the list").seats));
        db.execute("UPDATE project_settings SET value = '2x'", [])
            .expect("a malformed boost");
        assert!(
            [5, 8].contains(&read(&db).expect("the list").seats),
            "2x grants none"
        );
        let drain = "INSERT INTO project_settings VALUES ('*', 'drain', '2026-11-09T13:26:27Z')";
        db.execute(drain, []).expect("a drain");
        assert_eq!(
            read(&db).expect("the list").seats,
            0,
            "a drain starts nothing"
        );
        db.execute("DELETE FROM project_settings WHERE field = 'drain'", [])
            .expect("a resume");
        assert!(
            [5, 8].contains(&read(&db).expect("the list").seats),
            "a resume gives them back"
        );
        // Rust's own cap, whatever the drain: the older app's runner stays held, Rust runs the lesser of its cap and Admission's
        db.execute(drain, []).expect("a drain again");
        db.execute(
            "INSERT INTO project_settings VALUES ('site_perso', 'rust-seats', '2')",
            [],
        )
        .expect("rust's seats");
        let capped = read(&db).expect("the list");
        assert_eq!(
            capped.seats, 2,
            "rust's own seats, under the night's or the day's"
        );
        assert!(
            capped
                .seats_why
                .ends_with(", capped at 2 by rust-seats, which lets Rust run through dash drain"),
            "under a drain, no hint to clear it: clearing would stop Rust (/code-review): {}",
            capped.seats_why
        );
        db.execute("DELETE FROM project_settings WHERE field = 'drain'", [])
            .expect("no drain");
        let why = read(&db).expect("the list").seats_why;
        assert!(
            why.ends_with(&format!(", capped at 2 by rust-seats, {CAP_HINT}")),
            "the query's parts and the clear command reach the reason: {why}"
        );
        db.execute(drain, []).expect("the drain back");
        db.execute(
            "UPDATE project_settings SET value = '9' WHERE field = 'rust-seats'",
            [],
        )
        .expect("a cap over Admission's");
        assert!(
            [5, 8].contains(&read(&db).expect("the list").seats),
            "rust-seats 9 never raises the day's or the night's, through the query (PR 1463's /code-review)"
        );
        db.execute(
            "UPDATE project_settings SET value = 'x' WHERE field = 'rust-seats'",
            [],
        )
        .expect("a malformed count");
        assert_eq!(
            read(&db).expect("the list").seats,
            0,
            "a malformed count falls back to the drain"
        );
        db.execute(
            "DELETE FROM project_settings WHERE field IN ('rust-seats', 'drain')",
            [],
        )
        .expect("back");
        // /code-review of rust-own-queue: a list kept in data, blanks around « and », a since that is only a date
        let data = r#"{"blocked-by":["shut","a"],"since":"2026-11-08"}"#;
        task(&db, 9, "listed", "open", "rust", data, None);
        task(&db, 10, "spaced", "open", "rust", "{}", Some("x  and\ny"));
        let data = r#"{"since":"2026-11-08T21:26:52"}"#;
        task(&db, 11, "no-offset", "open", "rust", data, Some("a and a"));
        let queue = read(&db).expect("the list");
        assert_eq!(
            queue.tasks[3].waits,
            ["a"],
            "a list's names, a done one met"
        );
        assert_eq!(
            queue.tasks[3].reopened_ms, None,
            "a date alone is no reopen time"
        );
        assert_eq!(queue.tasks[4].waits, ["x", "y"]);
        assert_eq!(
            queue.tasks[5].reopened_ms, None,
            "no offset, as from_iso8601"
        );
    }

    #[dash_test]
    fn a_task_s_stage_and_newest_report_read_none_when_absent() {
        let dir = dash_meter::TestDir::new("older-stage");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(OLDER).expect("schema");
        let reports = "CREATE TABLE task_reports (id INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT NOT NULL, at TEXT NOT NULL, record TEXT NOT NULL);";
        db.execute_batch(reports).expect("reports");
        task(
            &db,
            0,
            "morning-run",
            "done",
            "",
            r#"{"run":{"prompt":"Morning run"}}"#,
            None,
        );
        task(&db, 1, "bare", "open", "", "{}", None);
        assert_eq!(
            stage(&path, "morning-run").unwrap(),
            Some(("done".into(), Some("Morning run".into())))
        );
        assert_eq!(stage(&path, "bare").unwrap(), Some(("open".into(), None)));
        assert_eq!(stage(&path, "gone").unwrap(), None);
        assert_eq!(last_report(&path, "morning-run").unwrap(), None);
        for r in ["{\"n\":1}", "{\"n\":2}"] {
            db.execute(
                "INSERT INTO task_reports (task, at, record) VALUES ('morning-run', 'now', ?1)",
                [r],
            )
            .expect("report");
        }
        assert_eq!(
            last_report(&path, "morning-run").unwrap(),
            Some((2, "{\"n\":2}".into()))
        );
    }

    /// Task comments-board-pill: a stand-in list's tasks named, at any owner and stage, each closed once done or dropped, its
    /// horizon its roadmap item's (its own, else a cover, else its epic), none on no item; a task not named is not read.
    #[dash_test]
    fn every_task_s_place_is_its_stage_and_its_item_s_horizon() {
        let db = Connection::open_in_memory().expect("db");
        db.execute_batch(OLDER).expect("tables");
        task(
            &db,
            0,
            "shipped",
            "done",
            "claude",
            r#"{"roadmap":":trip"}"#,
            None,
        );
        task(&db, 1, "filed", "open", "rust", "{}", None);
        task(&db, 2, "loose", "dropped", "claude", "{}", None);
        task(&db, 3, "unnamed", "open", "claude", "{}", None);
        db.execute("UPDATE tasks SET epic = 'rust' WHERE id = 'filed'", [])
            .expect("its epic");
        let item = "INSERT INTO records VALUES ('focus-overall.edn', ?1, ?2, ?3)";
        for (pos, id, data) in [
            (0, "rust-poc", r#"{"covers":["rust"]}"#),
            (1, "trip", r#"{"horizon":":later"}"#),
        ] {
            db.execute(item, rusqlite::params![pos, id, data])
                .expect("an item");
        }
        let place = |closed: bool, horizon: Option<&str>| Place {
            closed,
            horizon: horizon.map(String::from),
        };
        let named = ["shipped", "filed", "loose", "gone"].map(String::from);
        let mut got = places_in(&db, &named).expect("the places");
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            [
                ("filed".to_string(), place(false, Some("now"))),
                ("loose".to_string(), place(true, None)),
                ("shipped".to_string(), place(true, Some("later"))),
            ]
        );
    }

    /// Done-when 3 of rust-board-in-progress-truth: an imported issue's assignees and last day, by its number, from the store.
    #[dash_test]
    fn an_issue_s_marks_are_its_assignees_and_last_day() {
        let dir = dash_meter::TestDir::new("older-marks");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, data TEXT NOT NULL);
             INSERT INTO records VALUES ('issues-klar.edn', 0, 'klar-15',
               '{\"assignee\":[\"@nha\"],\"updated\":\"2026-06-28 10:00\",\"created\":\"2026-05-19\"}');
             INSERT INTO records VALUES ('issues-klar.edn', 1, 'klar-2', '{\"created\":\"2026-05-19 08:30\"}');
             INSERT INTO records VALUES ('issues-klar.edn', 2, 'klar-3', '{\"assignee\":\"claude\"}');
             INSERT INTO records VALUES ('issues-graal.edn', 0, 'graal-15', '{\"assignee\":[\"x\"]}');",
        )
        .expect("tables");
        let marks = issue_marks(&path, "klar").expect("marks");
        let mark = |who: &[&str], day: Option<&str>| IssueMarks {
            assignees: who.iter().map(|w| w.to_string()).collect(),
            day: day.map(String::from),
        };
        assert_eq!(marks["15"], mark(&["@nha"], Some("2026-06-28 10:00")));
        assert_eq!(marks["2"], mark(&[], Some("2026-05-19 08:30")));
        assert_eq!(marks["3"], mark(&["claude"], None));
        assert_eq!(marks.len(), 3, "graal's stay graal's: {marks:?}");
    }

    /// What `dash linear sync` reads of each task (task rust-linear-sync-port): its fields, a mention as its name, the roadmap item
    /// covering its epic, its PRs newest first, the older app's claim; linear.edn's links as held.
    #[dash_test]
    fn a_task_reads_as_linear_writes_it() {
        let dir = dash_meter::TestDir::new("older-linear");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT, why TEXT,
               done_when TEXT, epic TEXT, owner TEXT, parent TEXT, added TEXT, done TEXT, priority INTEGER);
             CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, title TEXT, data TEXT NOT NULL);
             CREATE TABLE pr_issues (pr TEXT, issue TEXT, rule TEXT, kind TEXT);
             CREATE TABLE pr_links (url TEXT, number INTEGER, state TEXT, draft INTEGER);
             CREATE TABLE task_claims (task TEXT);
             CREATE TABLE companies (id TEXT, name TEXT);
             CREATE TABLE contacts (id TEXT, name TEXT);
             INSERT INTO companies VALUES ('comet', 'Comet');
             INSERT INTO records VALUES ('focus-overall.edn', 0, 'rust-poc', 'The Rust tool', '{}');
             INSERT INTO records VALUES ('linear.edn', 0, 'a', NULL, '{\"identifier\":\"AAA-1\",\"url\":\"https://linear.app/x\"}');
             INSERT INTO tasks VALUES ('tasks.edn', 0, 'a', 'open',
               '{\"estimate\":3,\"labels\":[\"bug\"],\"run\":{\"effort\":\"high\",\"agent\":\"vibe\"},\"every\":\"1 week\",\"due\":\"2026-12-01\"}',
               'Call \u{E000}m:companies:comet\u{E001}', 'Why', '1. Done', ':rust-poc', ':claude', NULL, '2026-11-01', NULL, 2);
             INSERT INTO tasks VALUES ('tasks.edn', 1, 'b', 'done', '{\"estimate\":4}', 'B', NULL, NULL, NULL, '', 'a',
               '2026-11-02', '2026-11-03: merged', NULL);
             INSERT INTO pr_links VALUES ('https://g/pull/1', 1, 'merged', 0), ('https://g/pull/2', 2, 'open', 1);
             INSERT INTO pr_issues VALUES ('https://g/pull/1', 'a', 'branch', 'closing'), ('https://g/pull/2', 'a', 'hand', 'contributing');
             INSERT INTO task_claims VALUES ('b');",
        )
        .expect("tables");
        let tasks = linear_tasks(&path).expect("tasks");
        let a = &tasks[0];
        assert_eq!(a.title, "Call Comet", "a mention reads as its name");
        assert_eq!(
            (a.stage.as_str(), a.priority, a.estimate),
            ("open", Some(2), Some(3))
        );
        assert_eq!(
            (a.effort.as_deref(), a.agent.as_deref(), a.owner.as_str()),
            (Some("high"), Some("vibe"), "claude")
        );
        assert_eq!(
            (a.every.as_deref(), a.due.as_deref()),
            (Some("1 week"), Some("2026-12-01"))
        );
        assert_eq!(
            (a.labels.clone(), a.item.as_deref()),
            (vec!["Bug".to_string()], Some("rust-poc"))
        );
        let prs: Vec<(i64, &str, &str, bool)> = a
            .prs
            .iter()
            .map(|p| {
                (
                    p.number.unwrap(),
                    p.state.as_str(),
                    p.kind.as_str(),
                    p.draft,
                )
            })
            .collect();
        assert_eq!(
            prs,
            vec![
                (2, "open", "contributing", true),
                (1, "merged", "closing", false)
            ]
        );
        let b = &tasks[1];
        assert_eq!(
            (b.estimate, b.owner.as_str(), b.parent.as_deref(), b.claimed),
            (None, "claude", Some("a"), true)
        );
        assert_eq!(b.done.as_deref(), Some("2026-11-03: merged"));
        let links = linear_links(&path).expect("links");
        assert_eq!(
            links["a"],
            ("AAA-1".to_string(), "https://linear.app/x".to_string())
        );
    }

    /// Task rust-land-waits-review: `review true` on a row holds its PR for a reviewer; false, absent or no row holds none.
    #[dash_test]
    fn review_true_is_read_off_its_row() {
        let dir = dash_meter::TestDir::new("older-review");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL);
             INSERT INTO tasks VALUES ('tasks.edn', 0, 'a', 'open', '{\"review\":true}');
             INSERT INTO tasks VALUES ('tasks.edn', 1, 'b', 'open', '{\"review\":false}');
             INSERT INTO tasks VALUES ('tasks.edn', 2, 'c', 'open', '{}');",
        )
        .expect("schema");
        let read = |id: &str| review(&path, id).expect("read");
        assert_eq!(
            [read("a"), read("b"), read("c"), read("none")],
            [true, false, false, false]
        );
    }

    /// Done-when 1 of brief-tests-prepush-skip: a brief is written from the task's row, his comments waiting on it, its Linear key
    /// and his Urgent then Now focuses, read here, never asked of the older app.
    #[dash_test]
    fn a_brief_reads_its_row_comments_linear_key_and_focuses() {
        let dir = dash_meter::TestDir::new("older-brief");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT,
               why TEXT, done_when TEXT, epic TEXT, plan_first INTEGER);
             CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, title TEXT, data TEXT NOT NULL);
             CREATE TABLE comments (pos INTEGER, id TEXT, stage TEXT, task TEXT, file TEXT, quote TEXT, text TEXT,
               data TEXT NOT NULL DEFAULT '{}');
             INSERT INTO tasks VALUES ('tasks.edn', 0, 'a', 'open',
               '{\"run\":{\"prompt\":\"Do it\",\"kind\":\"writing\",\"turns\":12,\"plan-first\":true}}',
               'A', 'Why', '1. It holds', 'speed', NULL);
             INSERT INTO tasks VALUES ('tasks.edn', 1, 'b', 'open', '{\"kind\":\"data\"}', 'B', NULL, NULL, NULL, 0);
             INSERT INTO comments VALUES (1, 'c2', 'open', 'a', '/p', NULL, 'second', '{}');
             INSERT INTO comments VALUES (0, 'c1', 'open', 'a', '/p', 'here', 'first', '{}');
             INSERT INTO comments VALUES (2, 'c3', 'resolved', 'a', '/p', NULL, 'done', '{}');
             INSERT INTO comments VALUES (3, 'c4', 'open', 'a', '/p', NULL, 'answered',
               '{\"replies\":[{\"by\":\"claude\",\"text\":\"Done\"}]}');
             INSERT INTO comments VALUES (4, 'c5', 'open', 'a', '/p', NULL, 'again',
               '{\"replies\":[{\"by\":\"claude\",\"text\":\"Done\"},{\"by\":\"nha\",\"text\":\"No\"}]}');
             INSERT INTO tasks VALUES ('tasks.edn', 2, 'review-a', 'open', '{}', 'R', NULL, NULL, NULL, 0);
             INSERT INTO records VALUES ('linear.edn', 0, 'a', NULL,
               '{\"identifier\":\"AAA-1\",\"url\":\"https://linear.app/x/issue/AAA-1\"}');
             INSERT INTO records VALUES ('linear.edn', 1, 'b', NULL, '{\"identifier\":\"AAA-2\",\"url\":\"hand-made\"}');
             INSERT INTO records VALUES ('focus-overall.edn', 0, 'now-one', 'Now one', '{}');
             INSERT INTO records VALUES ('focus-overall.edn', 1, 'later', 'Later', '{\"horizon\":\"~:later\"}');
             INSERT INTO records VALUES ('focus-overall.edn', 2, 'fire', 'Fire', '{\"until\":\"2999-01-01\"}');",
        )
        .expect("tables");
        let a = brief(&path, "a").unwrap().expect("a");
        let comment = |id: &str, quote: &str, text: &str| Comment {
            id: id.into(),
            file: "/p".into(),
            quote: quote.into(),
            text: text.into(),
        };
        let focus = vec![
            ("fire".into(), "Fire".into()),
            ("now-one".into(), "Now one".into()),
        ];
        let want = Filed {
            id: "a".into(),
            title: "A".into(),
            why: "Why".into(),
            done_when: "1. It holds".into(),
            prompt: "Do it".into(),
            kind: "writing".into(),
            epic: "speed".into(),
            plan_first: true,
            turns: Some(12),
            linear: Some("AAA-1".into()),
            // c3 resolved and c4 answered last by Claude are not waiting; c5, answered again by him, is
            comments: vec![
                comment("c1", "here", "first"),
                comment("c2", "", "second"),
                comment("c5", "", "again"),
            ],
            focus: focus.clone(),
        };
        assert_eq!(a, want);
        let b = brief(&path, "b").unwrap().expect("b");
        assert_eq!(
            (b.kind.as_str(), b.plan_first, b.linear, b.turns),
            ("data", false, None, None)
        );
        assert!(b.comments.is_empty() && b.focus == focus);
        assert_eq!(brief(&path, "gone").unwrap(), None);
        // a reviewer's run reads the comments on the task it reviews (`Watch.owners/1`)
        assert_eq!(
            brief(&path, "review-a")
                .unwrap()
                .expect("review-a")
                .comments,
            want.comments
        );
    }
}
