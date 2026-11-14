//! Core's TaskStore on the op log, the one write path: a run's whole history replays and merges as any table does.
//! ponytail: its own file, since `facts` has a NOT NULL column the log refuses; one file once dash's model moves here.
//!
//! A claim is a row and a lock: the row says who is claimed, the lock that its run still lives. The system frees a lock when
//! its process ends, however it ends, so no pid is ever read back or mistaken for a later process's. The lock is the worktree's
//! as much as the row's, so its file sits beside that worktree: a second `--data` folder has its own rows, and still no second
//! run starts in that folder.
use crate::oplog::{OpError, Replica};
use dash_core::tasks::{
    Claim, Demand, End, FreeKind, HOLD_MS, Hold, Impact, LANE, LANE_HOLD_MS, LANE_PARKED,
    ORCHESTRATOR, Outcome, Queue, Queued, Rank, Run, Standing, TaskStore, over_cap, refused,
    waits_unmet,
};
use dash_core::{FieldWrite, Op, Value};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, status TEXT, attempts INTEGER,
  pane TEXT, session TEXT, outcome TEXT, said TEXT, told TEXT, waits TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS agents (id TEXT PRIMARY KEY, folder TEXT, state TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS listed (id TEXT PRIMARY KEY, title TEXT, pos INTEGER, since_ms INTEGER, waits TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS settings (id TEXT PRIMARY KEY, value INTEGER) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS moved (id TEXT PRIMARY KEY, column TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS holds (id TEXT PRIMARY KEY, holder TEXT, why TEXT, touched_ms INTEGER) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS ends (id TEXT PRIMARY KEY, task TEXT, at_ms INTEGER, pushed INTEGER, duplicate INTEGER,
    tokens INTEGER, nudges INTEGER) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS routes (id TEXT PRIMARY KEY, pools TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS plans (id TEXT PRIMARY KEY, start TEXT, target TEXT, after TEXT, milestones TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS copies (id TEXT PRIMARY KEY, task TEXT, verb TEXT, form TEXT, at_ms INTEGER, state TEXT)
    WITHOUT ROWID;";
/// A folder from before a column (`waits`, `handed`, `rejudged`, `deployed`, listed's `layer`) keeps its rows: the column is
/// added. Two processes opening an old folder after a deploy can race here: only the column's absence afterwards is a failure.
fn add_column(db: &Connection, table: &str, name: &str) -> Result<(), String> {
    let sql = format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name = ?1");
    let has = || {
        db.query_row(&sql, [name], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())
    };
    if has()? == 0 {
        let added = db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {name} TEXT"));
        if has()? == 0 {
            added.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// A roadmap item's plan, as `dash roadmap set` filed it (task linear-projects-view): its start and target days, the items
/// it waits on, its milestones (day, name) in their order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub start: Option<String>,
    pub target: Option<String>,
    pub after: Vec<String>,
    pub milestones: Vec<(String, String)>,
}

fn plans_in(db: &Connection) -> rusqlite::Result<HashMap<String, Plan>> {
    let sql = "SELECT id, coalesce(start, ''), coalesce(target, ''), coalesce(after, ''), coalesce(milestones, '') FROM plans";
    let mut rows = db.prepare(sql)?;
    let rows = rows.query_map([], |r| {
        let some = |s: String| (!s.is_empty()).then_some(s);
        let (after, milestones): (String, String) = (r.get(3)?, r.get(4)?);
        let milestones = milestones.lines().filter_map(|l| l.split_once(' '));
        let plan = Plan {
            start: some(r.get(1)?),
            target: some(r.get(2)?),
            after: after.split_whitespace().map(String::from).collect(),
            milestones: milestones
                .map(|(d, n)| (d.into(), n.trim().into()))
                .collect(),
        };
        Ok((r.get(0)?, plan))
    })?;
    rows.collect()
}

/// The plans of the store in `dir`, read only, as a page reads them with no lock held: none before the store has any.
pub fn read_plans(dir: &Path) -> Result<HashMap<String, Plan>, String> {
    use rusqlite::OpenFlags;
    let path = dir.join("tasks.sqlite");
    let named = |e: rusqlite::Error| format!("{}: {e}", path.display());
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut db = match Connection::open_with_flags(&path, flags) {
        Err(_) if !path.exists() => return Ok(HashMap::new()),
        db => db.map_err(named)?,
    };
    crate::timed::trace(&mut db, &path).map_err(named)?;
    let has = db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'plans'",
        [],
        |r| r.get::<_, i64>(0),
    );
    match has.map_err(named)? {
        0 => Ok(HashMap::new()),
        _ => plans_in(&db).map_err(named),
    }
}

/// A row as a page shows it. `last_ms`: when its run's newest write was stamped, Unix milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub status: String,
    pub attempts: u32,
    pub outcome: Option<String>,
    pub said: Option<String>,
    pub last_ms: i64,
    /// Claimed, and no run holds its lock: a crash nobody freed yet, or nothing left running to free it.
    pub run_gone: bool,
}

/// One write of a row's status. `attempts`: the try a claim counted; `outcome`: how a run ended, written with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub at_ms: i64,
    pub status: String,
    pub attempts: Option<u32>,
    pub outcome: Option<String>,
}

/// A task write filed for the older app (`SqliteTasks::file_copy`), not yet sent through.
#[derive(Clone, Debug, PartialEq)]
pub struct Copy {
    pub row: String,
    pub task: String,
    pub verb: String,
    pub form: String,
    /// when it was filed, Unix milliseconds
    pub at_ms: i64,
}

/// An agent `dash claude` started, by its pane (rust-dash-claude). `state`: « running », « ended <code> » or « refused: <why> ».
#[derive(Clone, Debug, PartialEq)]
pub struct AgentRow {
    pub pane: String,
    pub folder: String,
    pub state: String,
    pub last_ms: i64,
}

pub struct SqliteTasks {
    log: Replica,
    locks: PathBuf,
    held: HashMap<String, File>,
}

/// `locks`: the folder the task worktrees sit in.
pub fn open(dir: &Path, locks: &Path) -> Result<SqliteTasks, String> {
    let db = crate::connect(&dir.join("tasks.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    for column in [
        "waits", "handed", "rejudged", "deployed", "account", "back", "model", "note",
    ] {
        add_column(&db, "tasks", column)?;
    }
    for column in [
        "layer",
        "builds_nothing",
        "deadline",
        "priority",
        "estimate",
        "horizon",
        "item",
        "free",
        "demand",
        "epic",
        "labels",
        "gone",
        "files",
        "impact",
    ] {
        add_column(&db, "listed", column)?;
    }
    add_column(&db, "ends", "nudges")?;
    let locks = locks.to_path_buf();
    std::fs::create_dir_all(&locks).map_err(|e| e.to_string())?;
    let held = HashMap::new();
    let log = Replica::new(db, "dash").map_err(said)?;
    Ok(SqliteTasks { log, locks, held })
}

/// The writes that remove each of `ids` from `listed`, its layer, deadline and impact with it, so a row the list later holds comes
/// back with neither.
fn forgotten(ids: &[String]) -> Vec<FieldWrite> {
    let write = |id: &String, field: &str, value: Value| FieldWrite {
        table: "listed".into(),
        row: id.clone(),
        field: field.into(),
        value,
    };
    (ids.iter())
        .flat_map(|id| {
            [
                write(id, "gone", Value::Int(1)),
                write(id, "layer", Value::Null),
                write(id, "deadline", Value::Null),
                write(id, "impact", Value::Null),
            ]
        })
        .collect()
}

fn said(e: OpError) -> String {
    format!("{e:?}")
}

/// An id is a lock file's name, and comes from outside.
fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
}

/// Every row. `waits` is one field of ids, a space between, so a later write replaces the whole list, on any replica. A row with
/// no status yet (a replica's page can split a batch) is free, as a claim reads it: open.
fn standing_in(db: &Connection) -> rusqlite::Result<Vec<Standing>> {
    let sql = "SELECT id, status, outcome, waits, attempts, pane, session, handed, rejudged, back, deployed FROM tasks ORDER BY id";
    let mut rows = db.prepare(sql)?;
    let rows = rows.query_map([], |r| {
        let waits: Option<String> = r.get(3)?;
        Ok(Standing {
            id: r.get(0)?,
            status: r
                .get::<_, Option<String>>(1)?
                .unwrap_or_else(|| "open".into()),
            attempts: r.get::<_, Option<u32>>(4)?.unwrap_or_default(),
            pane: r.get(5)?,
            session: r.get(6)?,
            handed: r.get(7)?,
            rejudged: r.get(8)?,
            back: r.get::<_, Option<String>>(9)?.is_some(),
            deployed: r.get(10)?,
            outcome: r.get(2)?,
            waits: waits
                .unwrap_or_default()
                .split_whitespace()
                .map(String::from)
                .collect(),
        })
    })?;
    rows.collect()
}

fn write(id: &str, name: &str, value: Value) -> FieldWrite {
    let (table, row, field) = ("tasks".into(), id.into(), name.into());
    FieldWrite {
        table,
        row,
        field,
        value,
    }
}

fn field(id: &str, name: &str, value: Option<&str>) -> FieldWrite {
    write(
        id,
        name,
        value.map_or(Value::Null, |v| Value::Text(v.into())),
    )
}

/// A run's end is news to the list: `told` goes with it, so a later end (a merge judged again) is told too.
fn run_writes(id: &str, status: &str, run: &Run) -> [FieldWrite; 6] {
    let (kind, words) = run.outcome.parts();
    [
        field(id, "told", None),
        field(id, "status", Some(status)),
        field(id, "pane", run.pane.as_deref()),
        field(id, "session", run.session.as_deref()),
        field(id, "outcome", Some(kind)),
        field(id, "said", Some(words)),
    ]
}

/// The list as the last sync wrote it, every row it ever listed: `pos` null once it no longer does. `waits` a line apart, as the
/// list may name a wait in words. `held`: his drag to Backlog or In Review on /tasks, which no pick passes (`plan`).
fn listed_in(db: &Connection) -> rusqlite::Result<Vec<(Option<i64>, Queued)>> {
    let sql =
        "SELECT l.id, l.title, l.pos, l.since_ms, l.waits, m.\"column\", l.layer, l.builds_nothing,
          l.deadline, l.priority, l.estimate, l.horizon, l.item, l.free, l.demand,
          l.epic, l.labels, l.files, l.impact FROM listed l
        LEFT JOIN moved m ON m.id = l.id WHERE l.gone IS NULL ORDER BY l.pos";
    let mut rows = db.prepare(sql)?;
    let rows = rows.query_map([], |r| {
        let waits: Option<String> = r.get(4)?;
        let held = match r.get::<_, Option<String>>(5)?.as_deref() {
            Some("backlog") => Some("he moved it to Backlog".to_string()),
            Some("in-review") => Some("he moved it to In Review".to_string()),
            _ => None,
        };
        let task = Queued {
            held,
            id: r.get(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            reopened_ms: r.get(3)?,
            waits: waits
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect(),
            layer: r.get(6)?,
            builds_nothing: r.get::<_, Option<String>>(7)?.is_some(),
            rank: Rank {
                deadline: r.get(8)?,
                priority: r.get::<_, Option<String>>(9)?.and_then(|p| p.parse().ok()),
                estimate: r.get::<_, Option<String>>(10)?.and_then(|e| e.parse().ok()),
                horizon: r.get(11)?,
                item: r.get(12)?,
                impact: (r.get::<_, Option<String>>(18)?.as_deref()).and_then(Impact::of),
            },
            free: r
                .get::<_, Option<String>>(13)?
                .and_then(|k| FreeKind::of(&[k])),
            demand: Demand::of(r.get::<_, Option<String>>(14)?.as_deref()),
            route: None,
            epic: r.get(15)?,
            labels: (r.get::<_, Option<String>>(16)?.unwrap_or_default().lines())
                .map(String::from)
                .collect(),
            files: (r.get::<_, Option<String>>(17)?.unwrap_or_default().lines())
                .map(String::from)
                .collect(),
        };
        Ok((r.get(2)?, task))
    })?;
    rows.collect()
}

fn seats_in(db: &Connection) -> rusqlite::Result<Option<i64>> {
    let sql = "SELECT value FROM settings WHERE id = 'seats'";
    db.query_row(sql, [], |r| r.get(0))
        .optional()
        .map(Option::flatten)
}

/// What set the seats, as the last sync read it; none before a sync wrote it.
fn seats_why_in(db: &Connection) -> rusqlite::Result<String> {
    let sql = "SELECT value FROM settings WHERE id = 'seats-why'";
    let why: Option<Option<String>> = db.query_row(sql, [], |r| r.get(0)).optional()?;
    Ok(why.flatten().unwrap_or_default())
}

type Listed = (Vec<(Option<i64>, Queued)>, Option<i64>, String);

/// The live holds at `now_ms`: touched within `HOLD_MS`, or a run's whose task is claimed, which its run's life touches, or the
/// PR lane's within `LANE_HOLD_MS`.
const LIVE_HOLDS: &str =
    "SELECT h.id, h.holder, coalesce(h.why, ''), coalesce(h.touched_ms, 0) FROM holds h
    LEFT JOIN tasks t ON h.holder = 'run ' || t.id
    WHERE h.holder IS NOT NULL AND (h.touched_ms > ?1 OR t.status = 'claimed'
      OR (h.holder IN (?3, ?4) AND h.touched_ms > ?2)) ORDER BY h.id";

fn holds_in(db: &Connection, now_ms: i64) -> rusqlite::Result<Vec<Hold>> {
    let mut rows = db.prepare(LIVE_HOLDS)?;
    let args = rusqlite::params![now_ms - HOLD_MS, now_ms - LANE_HOLD_MS, LANE, LANE_PARKED];
    let rows = rows.query_map(args, |r| {
        Ok(Hold {
            pr: r.get::<_, String>(0)?.parse().unwrap_or_default(),
            holder: r.get(1)?,
            why: r.get(2)?,
            touched_ms: r.get(3)?,
        })
    })?;
    rows.collect()
}

fn hold_write(pr: u64, name: &str, value: Value) -> FieldWrite {
    FieldWrite {
        table: "holds".into(),
        row: pr.to_string(),
        field: name.into(),
        value,
    }
}

/// A run's end as /watchers reads it: its task, when, and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Ended {
    pub task: String,
    pub at_ms: i64,
    pub end: End,
}

impl SqliteTasks {
    /// The layer task `id` names, whether the list still offers it or not (a task in review or merged keeps its row): what
    /// `dash task rfc-check <pr>` judges against (task rust-task-rfc-judge). `None` for no layer or no row.
    pub fn layer_named(&self, id: &str) -> Result<Option<String>, String> {
        let (listed, ..) = self.listed()?;
        Ok(listed
            .into_iter()
            .find(|(_, t)| t.id == id)
            .and_then(|(_, t)| t.layer))
    }

    fn listed(&self) -> Result<Listed, String> {
        let read = |db: &Connection| -> rusqlite::Result<Listed> {
            Ok((listed_in(db)?, seats_in(db)?, seats_why_in(db)?))
        };
        self.log
            .read(read)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// The list's tasks for Rust and his seats, copied here: a field written only when it changed, so a pass a minute grows the
    /// log only by what moved. A task the list no longer offers keeps its row, unlisted. How many fields were written.
    /// ponytail: every row ever listed is read and searched linearly, and a task filed at the head rewrites each `pos`: tens of
    /// Rust tasks, a few ops a filing. A rank between neighbours and a pruned table if the list reaches thousands.
    pub fn sync(&mut self, queue: &Queue) -> Result<usize, String> {
        let (before, seats, seats_why) = self.listed()?;
        let text = |id: &str, name: &str, v: &str| FieldWrite {
            table: "listed".into(),
            row: id.into(),
            field: name.into(),
            value: Value::Text(v.into()),
        };
        let int = |id: &str, name: &str, v: Option<i64>| FieldWrite {
            table: "listed".into(),
            row: id.into(),
            field: name.into(),
            value: v.map_or(Value::Null, Value::Int),
        };
        let mut writes = vec![];
        for (pos, task) in queue.tasks.iter().enumerate() {
            let old = before.iter().find(|(_, t)| t.id == task.id);
            let id = task.id.as_str();
            if old.is_none_or(|(_, t)| t.title != task.title) {
                writes.push(text(id, "title", &task.title));
            }
            // a row `forget` removed comes back once the list holds its id
            if old.is_none() {
                writes.push(int(id, "gone", None));
            }
            if old.is_none_or(|(p, _)| *p != Some(pos as i64)) {
                writes.push(int(id, "pos", Some(pos as i64)));
            }
            if old.is_none_or(|(_, t)| t.reopened_ms != task.reopened_ms) {
                writes.push(int(id, "since_ms", task.reopened_ms));
            }
            if old.is_none_or(|(_, t)| t.waits != task.waits) {
                writes.push(text(id, "waits", &task.waits.join("\n")));
            }
            // what its rank reads, the deadline aside: the front door keeps that one
            let (r, was) = (&task.rank, old.map(|(_, t)| &t.rank));
            let fields = [
                (
                    "priority",
                    r.priority.map(|p| p.to_string()),
                    was.map(|w| w.priority.map(|p| p.to_string())),
                ),
                (
                    "estimate",
                    r.estimate.map(|e| e.to_string()),
                    was.map(|w| w.estimate.map(|e| e.to_string())),
                ),
                ("horizon", r.horizon.clone(), was.map(|w| w.horizon.clone())),
                ("item", r.item.clone(), was.map(|w| w.item.clone())),
                (
                    "free",
                    task.free.map(|k| k.label().into()),
                    old.map(|(_, t)| t.free.map(|k| k.label().into())),
                ),
                (
                    "demand",
                    Some(task.demand.name().into()),
                    old.map(|(_, t)| Some(t.demand.name().into())),
                ),
                ("epic", task.epic.clone(), old.map(|(_, t)| t.epic.clone())),
                (
                    "labels",
                    Some(task.labels.join("\n")).filter(|l| !l.is_empty()),
                    old.map(|(_, t)| Some(t.labels.join("\n")).filter(|l| !l.is_empty())),
                ),
                (
                    "files",
                    Some(task.files.join("\n")).filter(|f| !f.is_empty()),
                    old.map(|(_, t)| Some(t.files.join("\n")).filter(|f| !f.is_empty())),
                ),
            ];
            for (name, now, before) in fields {
                // a new row writes only what it holds
                if before.flatten() != now {
                    let mut write = text(id, name, now.as_deref().unwrap_or_default());
                    if now.is_none() {
                        write.value = Value::Null;
                    }
                    writes.push(write);
                }
            }
            if old.is_some_and(|(_, t)| t.builds_nothing) != task.builds_nothing {
                let mut light = text(id, "builds_nothing", "yes");
                if !task.builds_nothing {
                    light.value = Value::Null;
                }
                writes.push(light);
            }
        }
        let gone = before
            .iter()
            .filter(|(p, t)| p.is_some() && !queue.tasks.iter().any(|q| q.id == t.id));
        writes.extend(gone.map(|(_, t)| int(&t.id, "pos", None)));
        if seats != Some(queue.seats as i64) {
            writes.push(FieldWrite {
                table: "settings".into(),
                row: "seats".into(),
                field: "value".into(),
                value: Value::Int(queue.seats as i64),
            });
        }
        if seats_why != queue.seats_why {
            writes.push(FieldWrite {
                table: "settings".into(),
                row: "seats-why".into(),
                field: "value".into(),
                value: Value::Text(queue.seats_why.clone()),
            });
        }
        if !writes.is_empty() {
            self.log.write(&writes).map_err(said)?;
        }
        Ok(writes.len())
    }

    /// The layer a task changes, as `dash task add --layer` filed it: kept beside its listed row, which a sync never writes over,
    /// so the picker holds every other task naming it while one runs (task rust-task-names-layer).
    pub fn layer(&mut self, id: &str, layer: &str) -> Result<(), String> {
        self.kept(id, "layer", layer)
    }

    /// The day an urgent task is due by, as `dash task add --deadline` filed it, kept as its layer is (task rust-orch-pick-dynamic):
    /// the older app has no such field.
    pub fn deadline(&mut self, id: &str, day: &str) -> Result<(), String> {
        self.kept(id, "deadline", day)
    }

    /// What a Rust task retires and meets, as `dash task add --impact` counted it, kept as its layer is (task rust-task-impact):
    /// the older app has no such field.
    pub fn impact(&mut self, id: &str, impact: &Impact) -> Result<(), String> {
        self.kept(id, "impact", &impact.stored())
    }

    /// Each task's impact as `impact` kept it, by id: what `dash task style` reads to name the open Rust tasks with none.
    pub fn impacts(&self) -> Result<HashMap<String, Impact>, String> {
        let rows = self.by_id("SELECT id, impact FROM listed WHERE impact IS NOT NULL")?;
        let read = rows
            .into_iter()
            .filter_map(|(id, i)| Some((id, Impact::of(&i)?)));
        Ok(read.collect())
    }

    /// Each task's layer as `layer` kept it, by id (task run-fields-at-add: a Rust task no start could launch lacks one).
    pub fn layers(&self) -> Result<HashMap<String, String>, String> {
        self.by_id("SELECT id, layer FROM listed WHERE trim(coalesce(layer, '')) <> ''")
    }

    /// One kept column by id: `sql` selects the id, then the value.
    fn by_id(&self, sql: &str) -> Result<HashMap<String, String>, String> {
        let read = |db: &Connection| -> rusqlite::Result<HashMap<String, String>> {
            db.prepare(sql)?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        };
        self.log
            .read(read)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// Each demand's route as `dash orch route` set it, its pools a comma apart, best first (RFC 20h rule 4: a new constraint is a
    /// row, not code). A demand with no row takes its policy's own route.
    pub fn routes(&self) -> Result<Vec<(String, Vec<String>)>, String> {
        let read = |db: &Connection| -> rusqlite::Result<Vec<(String, Vec<String>)>> {
            let mut rows = db.prepare("SELECT id, coalesce(pools, '') FROM routes ORDER BY id")?;
            let rows = rows.query_map([], |r| {
                let pools: String = r.get(1)?;
                let pools = pools.split(',').map(str::trim).filter(|p| !p.is_empty());
                Ok((r.get(0)?, pools.map(String::from).collect()))
            })?;
            rows.collect()
        };
        self.log
            .read(read)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// `demand`'s route from now on: its pools, best first; none leaves it no pool, the login's own model.
    pub fn route(&mut self, demand: Demand, pools: &[String]) -> Result<(), String> {
        let named =
            |p: &String| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if let Some(odd) = pools.iter().find(|p| !named(p)) {
            return Err(format!(
                "{odd:?}: not a pool name, as claude --model takes one"
            ));
        }
        let write = FieldWrite {
            table: "routes".into(),
            row: demand.name().into(),
            field: "pools".into(),
            value: Value::Text(pools.join(",")),
        };
        self.log.write(&[write]).map(drop).map_err(said)
    }

    /// Fields of roadmap item `id`'s plan (task linear-projects-view), Rust's own: the older app's `dash focus set` rewrites its
    /// items whole from the fields it knows, so a field kept there would go at its next write. `start`, `target`: an ISO day or
    /// empty; `after`: the items it waits on, a space apart; `milestones`: one a line, « <day> <name> ». Every field is read
    /// first, and the start never after the target, the plan's own and the ones given together: one that does not read refuses
    /// them all, named, and nothing is written (/code-review: a bad later field had left the earlier ones saved).
    pub fn plan(&mut self, id: &str, fields: &[(&str, &str)]) -> Result<(), String> {
        if !is_id(id) {
            return Err(format!("{id}: not a roadmap item id"));
        }
        let mut now = self.plans()?.remove(id).unwrap_or_default();
        for (field, value) in fields {
            let value = value.trim();
            let odd = match *field {
                "start" | "target" => {
                    (!value.is_empty() && !dash_core::is_day(value)).then(|| value.to_string())
                }
                "after" => (value.split_whitespace())
                    .find(|a| !is_id(a) || *a == id)
                    .map(String::from),
                "milestones" => value
                    .lines()
                    .find(|l| {
                        l.split_once(' ')
                            .is_none_or(|(d, n)| !dash_core::is_day(d) || n.trim().is_empty())
                    })
                    .map(String::from),
                _ => {
                    return Err(format!(
                        "{field}: a plan holds start, target, after and milestones"
                    ));
                }
            };
            if let Some(odd) = odd {
                return Err(format!("{id} {field}: « {odd} » does not read"));
            }
            let day = (!value.is_empty()).then(|| value.to_string());
            match *field {
                "start" => now.start = day,
                "target" => now.target = day,
                _ => {}
            }
        }
        if let (Some(s), Some(t)) = (&now.start, &now.target)
            && s > t
        {
            return Err(format!("{id}: its start {s} is after its target {t}"));
        }
        let writes: Vec<FieldWrite> = (fields.iter())
            .map(|(field, value)| FieldWrite {
                table: "plans".into(),
                row: id.into(),
                field: field.to_string(),
                value: Value::Text(value.trim().into()),
            })
            .collect();
        self.log.write(&writes).map(drop).map_err(said)
    }

    /// Every roadmap item's plan, by id.
    pub fn plans(&self) -> Result<HashMap<String, Plan>, String> {
        self.log
            .read(plans_in)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// A field the front door keeps beside a listed row, which a sync never writes over.
    fn kept(&mut self, id: &str, field: &str, value: &str) -> Result<(), String> {
        if !is_id(id) {
            return Err(format!("{id}: not a task id"));
        }
        let write = |field: &str, value: Value| FieldWrite {
            table: "listed".into(),
            row: id.into(),
            field: field.into(),
            value,
        };
        let writes = [
            write(field, Value::Text(value.into())),
            write("gone", Value::Null),
        ];
        self.log.write(&writes).map(drop).map_err(said)
    }

    /// Once per store, the rows only a kept layer or deadline made that `unfiled` says the list never filed, forgotten: those
    /// left before the front door checked an id (no-such-task-xyz's probe, adds the older app refused). Some(their ids) the
    /// time it runs; a failure writes nothing, so the next call tries again.
    pub fn sweep_once(
        &mut self,
        unfiled: impl FnOnce(&[String]) -> Result<Vec<String>, String>,
    ) -> Result<Option<Vec<String>>, String> {
        let sql = "SELECT 1 FROM settings WHERE id = 'strays_swept'";
        let done = |db: &Connection| db.query_row(sql, [], |_| Ok(())).optional();
        if self
            .log
            .read(done)
            .map_err(said)?
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Ok(None);
        }
        let stray = unfiled(&self.never_listed()?)?;
        // one write: the rows and the mark together, or neither
        let mut writes = forgotten(&stray);
        writes.push(FieldWrite {
            table: "settings".into(),
            row: "strays_swept".into(),
            field: "value".into(),
            value: Value::Int(1),
        });
        self.log.write(&writes).map_err(said)?;
        Ok(Some(stray))
    }

    /// The rows only a kept layer or deadline made, the list never holding them: what `forget` asks the list about.
    fn never_listed(&self) -> Result<Vec<String>, String> {
        let sql = "SELECT id FROM listed WHERE gone IS NULL AND pos IS NULL AND title IS NULL ORDER BY id";
        let read = |db: &Connection| -> rusqlite::Result<Vec<String>> {
            db.prepare(sql)?.query_map([], |r| r.get(0))?.collect()
        };
        self.log
            .read(read)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// Each of `ids` removed, its layer and deadline with it: a task the list never filed (task rust-front-door-known-id: a probe
    /// update of no-such-task-xyz left a deadline row). A row the list later holds comes back empty.
    pub fn forget(&mut self, ids: &[String]) -> Result<(), String> {
        let writes = forgotten(ids);
        match writes.is_empty() {
            true => Ok(()),
            false => self.log.write(&writes).map(drop).map_err(said),
        }
    }

    /// A new open row, naming what it waits on. Refused with a line saying why: it exists, past the cap, a name no row has, a cycle.
    pub fn add(&mut self, id: &str, waits: &[String]) -> Result<(), String> {
        self.set_waits(id, waits, false)
    }

    /// The whole list an existing row waits on replaced, its run left as it is. Refused as `add` is.
    pub fn waits(&mut self, id: &str, waits: &[String]) -> Result<(), String> {
        self.set_waits(id, waits, true)
    }

    /// `exists`, and all the rest, read under the write lock: a claim landing meanwhile cannot have its status written over.
    fn set_waits(&mut self, id: &str, waits: &[String], exists: bool) -> Result<(), String> {
        if !is_id(id) {
            return Err(format!("{id}: not a task id"));
        }
        let mut names: Vec<String> = vec![];
        for wait in waits {
            if !names.contains(wait) {
                names.push(wait.clone());
            }
        }
        let mut writes = vec![field(id, "waits", Some(&names.join(" ")))];
        if !exists {
            writes.push(field(id, "status", Some("open")));
            writes.push(write(id, "attempts", Value::Int(0)));
        }
        let mut refusal = None;
        let holds = |db: &Connection| {
            let rows = standing_in(db)?;
            refusal = match (rows.iter().any(|r| r.id == id), exists) {
                (true, false) => Some(format!(
                    "{id}: refused, it exists: dash task waits changes what it waits on"
                )),
                (false, true) => Some(format!("{id}: refused, no such task")),
                _ => refused(&rows, id, &names),
            };
            Ok(refusal.is_none())
        };
        let written = self.log.write_if(holds, &writes).map_err(said)?;
        match (written, refusal) {
            (Some(_), _) => Ok(()),
            (None, why) => Err(why.unwrap_or(format!("{id}: refused"))),
        }
    }

    /// An open row given up on, so it holds no place under the cap: what `add` made and nothing will ever run. A claimed row is its
    /// run's. Refused while an unfinished row waits on it: no merge would ever free that one. Untold, as any ended run: `next` tells
    /// the list only of what it offers, so a task it still offers does not sit there with neither side moving it.
    pub fn close(&mut self, id: &str, why: &str) -> Result<(), String> {
        let writes = [
            field(id, "status", Some("finished")),
            field(id, "outcome", Some("ended")),
            field(id, "said", Some(why)),
        ];
        let sql = "SELECT 1 FROM tasks WHERE id = ?1 AND status = 'open'";
        let mut refusal = None;
        let open = |db: &Connection| {
            let row = db.query_row(sql, [id], |_| Ok(())).optional()?;
            let rows = standing_in(db)?;
            let waiting = rows
                .iter()
                .filter(|r| r.status != "finished" && r.waits.iter().any(|w| w == id));
            let waiting: Vec<&str> = waiting.map(|r| r.id.as_str()).collect();
            if row.is_some() && !waiting.is_empty() {
                let names = waiting.join(", ");
                refusal = Some(format!(
                    "{id}: refused, {names} waits on it: dash task waits changes that first"
                ));
            }
            Ok(row.is_some() && refusal.is_none())
        };
        let written = self.log.write_if(open, &writes).map_err(said)?;
        if let Some(line) = refusal {
            return Err(line);
        }
        written
            .map(drop)
            .ok_or(format!("{id}: refused, not an open task"))
    }

    /// `lock`: its claim's, held by its run. `close`: held by a `take_over` until its `record`.
    fn lock_path(&self, id: &str, kind: &str) -> PathBuf {
        self.locks.join(format!("rust-{id}.{kind}"))
    }

    /// The column he dragged `id`'s card to on /tasks (task rust-board-linear-order): the board view shows it there until a run
    /// claims it. The caller names a column the board has; no run, list or lock is touched.
    pub fn move_to(&mut self, id: &str, column: &str) -> Result<(), String> {
        if !is_id(id) {
            return Err(format!("{id}: not a task id"));
        }
        let to = FieldWrite {
            table: "moved".into(),
            row: id.into(),
            field: "column".into(),
            value: Value::Text(column.into()),
        };
        self.log.write(&[to]).map(drop).map_err(said)
    }

    /// The lock of `id`'s claim, when no run holds it.
    fn take_lock(&self, id: &str) -> Result<Option<File>, String> {
        self.take_named(id, "lock")
    }

    /// Whether a `take_over` holds `id`: a look then neither reads its claim as a crash nor frees it, though its run is gone.
    fn closing(&self, id: &str) -> Result<bool, String> {
        Ok(self.take_named(id, "close")?.is_none())
    }

    fn take_named(&self, id: &str, kind: &str) -> Result<Option<File>, String> {
        if !is_id(id) {
            return Err(format!("{id}: not a task id"));
        }
        // Never truncated on open: a look that finds it held must leave its run's pid there (`take_over`).
        let path = self.lock_path(id, kind);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// This file's op log after `after`, a page at a time: what the views read (section 6).
    pub fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String> {
        self.log.ops_after(after, limit).map_err(said)
    }

    /// `hold_pr` at `now_ms`: the clock handed in, so a test reads a hold past `HOLD_MS`.
    pub fn hold_at(
        &mut self,
        now_ms: i64,
        pr: u64,
        holder: &str,
        why: &str,
    ) -> Result<Option<Hold>, String> {
        let mut found: Option<Hold> = None;
        // Read under the write lock: of two takes at once, one.
        let holds = |db: &Connection| {
            found = holds_in(db, now_ms)?.into_iter().find(|h| h.pr == pr);
            let Some(other) = found.as_ref().filter(|h| h.holder != holder) else {
                return Ok(true);
            };
            // A run's hold whose task is not claimed yet: a hand-back waiting on a seat, which the orchestrator may take for an
            // urgent fix (@nha, 2026-11-10: « blocking commits from the main is not a good solution - urgent tasks may require
            // that »). A run at work keeps its own: `dash task close` ends it first.
            let run = other.holder.strip_prefix("run ");
            let claimed = |id: &str| {
                db.query_row("SELECT status FROM tasks WHERE id = ?1", [id], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .optional()
                .map(|s| s.flatten().as_deref() == Some("claimed"))
            };
            // The lane's parked hold is its skip mark, never a lock: anyone takes it. Its live one the
            // orchestrator takes, as a crashed lane would hold it 6 hours; the lane parks it in one write.
            let lane = match other.holder.as_str() {
                LANE_PARKED => true,
                LANE => [ORCHESTRATOR, LANE_PARKED].contains(&holder),
                _ => false,
            };
            let run = run.is_some_and(|id| !claimed(id).unwrap_or(true));
            Ok(lane || holder == ORCHESTRATOR && run)
        };
        let writes = [
            hold_write(pr, "holder", Value::Text(holder.into())),
            hold_write(pr, "why", Value::Text(why.into())),
            hold_write(pr, "touched_ms", Value::Int(now_ms)),
        ];
        let written = self.log.write_if(holds, &writes).map_err(said)?;
        let other = found.filter(|h| h.holder != holder);
        match (written, other) {
            (Some(_), other) => Ok(other),
            (None, Some(h)) => Err(format!("#{pr} is held by {}: {}", h.holder, h.why)),
            (None, None) => Err(format!("#{pr}: not taken")),
        }
    }

    /// Each live hold at `now_ms`.
    pub fn holds_at(&self, now_ms: i64) -> Result<Vec<Hold>, String> {
        let read = |db: &Connection| holds_in(db, now_ms);
        self.log
            .read(read)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// Each run's end since `since_ms`, the newest first.
    pub fn ends_since(&self, since_ms: i64) -> Result<Vec<Ended>, String> {
        // nudges: a store from before it has the column added as TEXT (`add_column`), read back as the count it holds
        let sql = "SELECT task, at_ms, pushed, duplicate, tokens, CAST(nudges AS INTEGER) FROM ends WHERE at_ms >= ?1 \
                   ORDER BY at_ms DESC";
        let read = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([since_ms], |r| {
                Ok(Ended {
                    task: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    at_ms: r.get::<_, Option<i64>>(1)?.unwrap_or_default(),
                    end: End {
                        pushed: r.get::<_, Option<i64>>(2)?.map(|p| p != 0),
                        duplicate: r.get::<_, Option<i64>>(3)? == Some(1),
                        tokens: r.get::<_, Option<i64>>(4)?.map(|t| t as u64),
                        nudges: r.get::<_, Option<i64>>(5)?.unwrap_or_default() as u32,
                    },
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        };
        self.log
            .read(read)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// `run_end` at `now_ms`.
    pub fn end_at(&mut self, now_ms: i64, id: &str, end: &End) -> Result<(), String> {
        let row = format!("{id} {now_ms}");
        let int = |name: &str, value: Option<i64>| FieldWrite {
            table: "ends".into(),
            row: row.clone(),
            field: name.into(),
            value: value.map_or(Value::Null, Value::Int),
        };
        let writes = [
            FieldWrite {
                table: "ends".into(),
                row: row.clone(),
                field: "task".into(),
                value: Value::Text(id.into()),
            },
            int("at_ms", Some(now_ms)),
            int("pushed", end.pushed.map(i64::from)),
            int("duplicate", Some(i64::from(end.duplicate))),
            int("tokens", end.tokens.map(|t| t as i64)),
            int("nudges", Some(i64::from(end.nudges))),
        ];
        self.log.write(&writes).map(drop).map_err(said)
    }

    /// The row, one `field: value` a line, as a command prints it.
    pub fn show(&self, id: &str) -> Result<Option<String>, String> {
        let fields = [
            "status", "attempts", "pane", "session", "account", "outcome", "said", "waits",
            "rejudged", "deployed", "note",
        ];
        let sql = format!("SELECT {} FROM tasks WHERE id = ?1", fields.join(", "));
        let row = |db: &Connection| {
            db.query_row(&sql, [id], |r| {
                let text = |(i, name)| {
                    let value = match r.get_ref(i)? {
                        rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                        _ => r.get::<_, Option<String>>(i)?.unwrap_or_default(),
                    };
                    Ok(format!("{name}: {value}"))
                };
                let lines = fields.iter().enumerate().map(text);
                lines.collect::<rusqlite::Result<Vec<_>>>()
            })
            .optional()
        };
        let row = self
            .log
            .read(row)
            .map_err(said)?
            .map_err(|e| e.to_string())?;
        Ok(row.map(|lines| lines.join("\n")))
    }

    /// Every row, the newest write of a run first; `told` and `deployed`, written after a merge, `waits`, written by hand, and a
    /// `note` are no run. A stamp holds its write's time above its low 16 bits (`Clock::tick`).
    pub fn rows(&self) -> Result<Vec<Row>, String> {
        let sql = "SELECT id, status, attempts, outcome, said,
            (SELECT max(stamp) FROM ops WHERE tbl = 'tasks' AND row_id = id AND field NOT IN ('told', 'deployed', 'waits', 'note')) AS last
            FROM tasks ORDER BY last DESC";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    status: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    attempts: r.get::<_, Option<u32>>(2)?.unwrap_or(0),
                    outcome: r.get(3)?,
                    said: r.get(4)?,
                    last_ms: r.get::<_, Option<i64>>(5)?.unwrap_or(0) >> 16,
                    run_gone: false,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        };
        let mut rows = self
            .log
            .read(rows)
            .map_err(said)?
            .map_err(|e| e.to_string())?;
        for row in rows.iter_mut().filter(|r| r.status == "claimed") {
            // A lock this page cannot look at says nothing: only one it took is free.
            row.run_gone = matches!(self.take_lock(&row.id), Ok(Some(_)));
        }
        Ok(rows)
    }

    /// Its row's story, oldest first: each write of its status, with the try a claim counted and the outcome a run ended on, at
    /// that write's time (task rust-task-page-linear: the old issue page's Activity). Every write names its status first, and a
    /// commit's ops take one writer's consecutive stamps, each field once: what follows a status that way is its own commit's,
    /// never a later write in the same millisecond (/code-review of rust-task-page-linear).
    pub fn history(&self, id: &str) -> Result<Vec<Event>, String> {
        let sql = "SELECT stamp, writer, field, CAST(value AS TEXT) FROM ops
            WHERE tbl = 'tasks' AND row_id = ?1 ORDER BY stamp, writer";
        let ops = |db: &Connection| {
            let mut ops = db.prepare(sql)?;
            let ops = ops.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            ops.collect::<rusqlite::Result<Vec<(i64, String, String, Option<String>)>>>()
        };
        let ops = self
            .log
            .read(ops)
            .map_err(said)?
            .map_err(|e| e.to_string())?;
        let mut events: Vec<Event> = vec![];
        // the last status write's commit as far as read: its last stamp, its writer, the fields it named
        let mut commit: Option<(i64, String, Vec<String>)> = None;
        for (stamp, writer, field, value) in ops {
            // a note is its own commit and its own line (`TaskStore::note`), its words the event's outcome
            if field == "note" {
                events.push(Event {
                    at_ms: stamp >> 16,
                    status: "note".into(),
                    attempts: None,
                    outcome: value,
                });
                commit = None;
                continue;
            }
            if field == "status" {
                events.push(Event {
                    at_ms: stamp >> 16,
                    status: value.unwrap_or_default(),
                    attempts: None,
                    outcome: None,
                });
                commit = Some((stamp, writer, vec![]));
                continue;
            }
            let follows = commit.as_ref().is_some_and(|(last, by, named)| {
                stamp == last + 1 && writer == *by && !named.contains(&field)
            });
            let (Some((last, _, named)), Some(event)) =
                (commit.as_mut().filter(|_| follows), events.last_mut())
            else {
                commit = None;
                continue;
            };
            *last = stamp;
            named.push(field.clone());
            match field.as_str() {
                "attempts" => event.attempts = value.and_then(|v| v.parse().ok()),
                "outcome" => event.outcome = value.filter(|v| !v.is_empty()),
                _ => {}
            }
        }
        Ok(events)
    }

    /// The agent in `pane`: where it works and how it stands.
    pub fn agent(&mut self, pane: &str, folder: &str, state: &str) -> Result<(), String> {
        let text = |name: &str, value: &str| FieldWrite {
            table: "agents".into(),
            row: pane.into(),
            field: name.into(),
            value: Value::Text(value.into()),
        };
        let writes = [text("folder", folder), text("state", state)];
        self.log.write(&writes).map(drop).map_err(said)
    }

    /// A task write a session asked (`dash task add|update|done`, a fold's raise), filed here first, the older app's copy still to
    /// send (task rust-task-writes-port): `task`, the tasks it names (space-separated), `form`, the command's form as the older
    /// app reads it. Its row's id, which orders it.
    pub fn file_copy(&mut self, task: &str, verb: &str, form: &str) -> Result<String, String> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let at_ms = self.log.now_ms();
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let row = format!("{at_ms:013}-{}-{seq:06}", std::process::id());
        let write = |name: &str, value: Value| FieldWrite {
            table: "copies".into(),
            row: row.clone(),
            field: name.into(),
            value,
        };
        let writes = [
            write("task", Value::Text(task.into())),
            write("verb", Value::Text(verb.into())),
            write("form", Value::Text(form.into())),
            write("at_ms", Value::Int(at_ms)),
        ];
        self.log.write(&writes).map_err(said)?;
        Ok(row)
    }

    /// Each copy the older app has not taken nor refused, oldest first: its row, task, verb and form.
    pub fn uncopied(&self) -> Result<Vec<Copy>, String> {
        let sql =
            "SELECT id, task, verb, form, at_ms FROM copies WHERE state IS NULL ORDER BY at_ms, id";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([], |r| {
                Ok(Copy {
                    row: r.get(0)?,
                    task: r.get(1)?,
                    verb: r.get(2)?,
                    form: r.get(3)?,
                    at_ms: r.get(4)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        };
        self.log
            .read(rows)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// Every add filed with a form opening `prefix` (« cwd=<a folder> »: the adds a session ran there), oldest first, those
    /// the older app refused left out (task move-names-task: a part moved to a task the session filed).
    pub fn adds_from(&self, prefix: &str) -> Result<Vec<Copy>, String> {
        // substr, not LIKE: an encoded folder holds « % »
        let sql = "SELECT id, task, verb, form, at_ms FROM copies WHERE verb = 'add' \
                   AND (state IS NULL OR state NOT LIKE 'refused%') AND substr(form, 1, length(?1)) = ?1 ORDER BY at_ms, id";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([prefix], |r| {
                Ok(Copy {
                    row: r.get(0)?,
                    task: r.get(1)?,
                    verb: r.get(2)?,
                    form: r.get(3)?,
                    at_ms: r.get(4)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        };
        self.log
            .read(rows)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// Whether an add of `task` waits for its copy: the task is ours already, though the older app's list holds none yet.
    pub fn adding(&self, task: &str) -> Result<bool, String> {
        let sql = "SELECT 1 FROM copies WHERE state IS NULL AND verb = 'add' AND task = ?1";
        let found = |db: &Connection| db.prepare(sql).and_then(|mut q| q.exists([task]));
        self.log
            .read(found)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// How copy `row` ended: « copied », or « refused: <the older app's words> ».
    pub fn copy_ended(&mut self, row: &str, state: &str) -> Result<(), String> {
        let write = FieldWrite {
            table: "copies".into(),
            row: row.into(),
            field: "state".into(),
            value: Value::Text(state.into()),
        };
        self.log.write(&[write]).map(drop).map_err(said)
    }

    /// Every agent, the newest write first.
    pub fn agents(&self) -> Result<Vec<AgentRow>, String> {
        let sql = "SELECT id, folder, state, (SELECT max(stamp) FROM ops WHERE tbl = 'agents' AND row_id = id) AS last
            FROM agents ORDER BY last DESC";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([], |r| {
                Ok(AgentRow {
                    pane: r.get(0)?,
                    folder: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    state: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    last_ms: r.get::<_, Option<i64>>(3)?.unwrap_or(0) >> 16,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        };
        self.log
            .read(rows)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    /// `status` set, this run's claim let go.
    fn finish(&mut self, id: &str, status: &str, run: &Run) -> Result<(), String> {
        let mut writes = run_writes(id, status, run).to_vec();
        // A run on it finished, however: no hand-back is live, so a later reopen ranks as any task (`handed_back`). A release
        // keeps it, as a retry is the same hand-back.
        if status == "finished" {
            writes.push(field(id, "back", None));
        }
        let written = self.log.write(&writes);
        self.held.remove(id);
        self.held.remove(&format!("{id} close"));
        written.map(drop).map_err(said)
    }

    fn claimed(&self) -> Result<Vec<Claim>, String> {
        let sql =
            "SELECT id, pane, session, attempts, NULLIF(account, ''), NULLIF(model, '') FROM tasks
          WHERE status = 'claimed'";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([], |r| {
                Ok(Claim {
                    id: r.get(0)?,
                    pane: r.get(1)?,
                    session: r.get(2)?,
                    attempts: r.get(3)?,
                    account: r.get(4)?,
                    model: r.get(5)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        };
        self.log
            .read(rows)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }
}

impl TaskStore for SqliteTasks {
    fn claim(&mut self, id: &str) -> Result<u32, String> {
        // The lock first: a free_crashed looking between the row and the lock would read a run gone.
        let lock = self.take_lock(id)?.ok_or(format!("{id}: a run holds it"))?;
        let sql = "SELECT 1 FROM tasks WHERE id = ?1 AND status != 'open'";
        // A claim is how a task the list offers gets its row: the cap holds here as at `add`. Its waits hold too, whoever claims:
        // `dash task run` names any id, and a wait may land after a pick and before its run's claim.
        // ponytail: reads every row, finished ones too; one person's tasks stay small, a count and a row read if they do not.
        let mut refusal = None;
        let free = |db: &Connection| {
            let row = db.query_row(sql, [id], |_| Ok(())).optional()?;
            let rows = standing_in(db)?;
            if row.is_none() {
                refusal = match rows.iter().find(|r| r.id == id) {
                    None => over_cap(&rows, id),
                    Some(own) => {
                        waits_unmet(&rows, &own.waits).map(|w| format!("{id}: refused, it {w}"))
                    }
                };
            }
            Ok(row.is_none() && refusal.is_none())
        };
        let attempts = self.attempts(id)? + 1;
        let writes = [
            field(id, "status", Some("claimed")),
            write(id, "attempts", Value::Int(attempts.into())),
            field(id, "pane", None),
            field(id, "session", None),
            // a run claimed it: his drag is spent, so a run released later shows its own column, not a move made before it
            FieldWrite {
                table: "moved".into(),
                row: id.into(),
                field: "column".into(),
                value: Value::Null,
            },
            field(id, "account", None),
        ];
        let written = self.log.write_if(free, &writes);
        let written = written.map_err(said)?;
        if let Some(line) = refusal {
            return Err(line);
        }
        written.ok_or(format!("{id}: not an open task"))?;
        // Its run's pid, read only while this lock is held, so never a later process's: what `take_over` stops.
        let pid = std::process::id().to_string();
        let wrote = lock
            .set_len(0)
            .and_then(|()| (&lock).write_all(pid.as_bytes()));
        wrote.map_err(|e| format!("{}: {e}", self.lock_path(id, "lock").display()))?;
        self.held.insert(id.to_string(), lock);
        Ok(attempts)
    }

    fn hold(&mut self, id: &str) -> Result<Claim, String> {
        // The lock first, as a claim: `free` and `claim` take it too, so the row read below stays claimed while it is held.
        let lock = self.take_lock(id)?.ok_or(format!("{id}: a run holds it"))?;
        let claimed = self.claimed()?.into_iter().find(|c| c.id == id);
        let claim = claimed.ok_or(format!("{id}: not claimed"))?;
        let attempts = claim.attempts + 1;
        let writes = [write(id, "attempts", Value::Int(attempts.into()))];
        self.log.write(&writes).map_err(said)?;
        self.held.insert(id.to_string(), lock);
        Ok(Claim { attempts, ..claim })
    }

    fn opened(&mut self, id: &str, pane: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "pane", Some(pane))])
            .map(drop)
            .map_err(said)
    }

    fn adopted(&mut self, id: &str, session: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "session", Some(session))])
            .map(drop)
            .map_err(said)
    }

    fn left(&mut self, id: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "pane", None)])
            .map(drop)
            .map_err(said)
    }

    fn login(&mut self, id: &str, account: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "account", Some(account))])
            .map(drop)
            .map_err(said)
    }

    fn model(&mut self, id: &str, model: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "model", Some(model))])
            .map(drop)
            .map_err(said)
    }

    fn record(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.finish(id, "finished", run)
    }

    fn release(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.finish(id, "open", run)
    }

    fn waiting(&mut self, id: &str, on: Option<&Outcome>) -> Result<(), String> {
        let (kind, words) = on.map(Outcome::parts).unzip();
        let writes = [field(id, "outcome", kind), field(id, "said", words)];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn crashed(&self) -> Result<Vec<Claim>, String> {
        let mut crashed = vec![];
        for claim in self.claimed()? {
            // Taken here, then dropped: only a run that lives holds it. One that cannot be looked at says so, never reads as alive.
            if self.take_lock(&claim.id)?.is_some() && !self.closing(&claim.id)? {
                crashed.push(claim);
            }
        }
        Ok(crashed)
    }

    fn free(&mut self, id: &str) -> Result<bool, String> {
        let Some(_lock) = self.take_lock(id)? else {
            return Ok(false);
        };
        // Its run stopped by a close that has not recorded it yet (/code-review of baa197583)
        if self.closing(id)? {
            return Ok(false);
        }
        let writes = [
            field(id, "status", Some("open")),
            field(id, "outcome", Some("failed")),
            field(id, "said", Some("its run crashed")),
        ];
        // Still claimed, now its lock is held: a run that ended since the read above has said how, and that stands.
        let sql = "SELECT 1 FROM tasks WHERE id = ?1 AND status = 'claimed'";
        let still = |db: &Connection| {
            let row = db.query_row(sql, [id], |_| Ok(())).optional()?;
            Ok(row.is_some())
        };
        Ok(self.log.write_if(still, &writes).map_err(said)?.is_some())
    }

    // Its run stopped by the pid its claim wrote, its process group first: a run started by `dash orch` leads one, and the git or
    // gh it waits on goes with it. A term, then a kill if it lingers; its lock then held here until `record`. The close lock first,
    // held as long: once the run is gone its claim lock is free, and a look must not free the claim and start a retry meanwhile
    // (/code-review of baa197583). A closer that dies lets both go, and the claim reads as a crash again.
    // ponytail: a pid read in the microseconds between a new claim's lock and its pid's write is the run before it, long gone: a
    // pid the system handed out again since would be hit. A pid file per run, written before its lock, if that ever matters.
    fn take_over(&mut self, id: &str) -> Result<(), String> {
        let claimed = |db: &Connection| {
            let sql = "SELECT 1 FROM tasks WHERE id = ?1 AND status = 'claimed'";
            db.query_row(sql, [id], |_| Ok(()))
                .optional()
                .map(|r| r.is_some())
        };
        let still = || -> Result<bool, String> {
            let read: rusqlite::Result<bool> = self.log.read(claimed).map_err(said)?;
            read.map_err(|e| e.to_string())
        };
        if !still()? {
            return Err(format!("{id}: not claimed"));
        }
        let closing =
            (self.take_named(id, "close")?).ok_or(format!("{id}: another close of it runs"))?;
        let path = self.lock_path(id, "lock");
        let mut lock = self.take_lock(id)?;
        if lock.is_none() {
            let pid =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let pid: i32 = (pid.trim().parse().ok())
                .filter(|&p: &i32| p > 1)
                .ok_or(format!("{id}: its run wrote no pid yet: still starting"))?;
            if pid as u32 == std::process::id() {
                return Err(format!(
                    "{id}: its lock names this very process, which a close never stops"
                ));
            }
            for signal in [libc::SIGTERM, libc::SIGKILL] {
                // SAFETY: kill takes plain integers and touches no memory of ours.
                if unsafe { libc::kill(-pid, signal) } != 0 {
                    unsafe { libc::kill(pid, signal) };
                }
                for _ in 0..50 {
                    lock = self.take_lock(id)?;
                    if lock.is_some() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                if lock.is_some() {
                    break;
                }
            }
        }
        let lock = lock.ok_or(format!("{id}: its run (its lock's pid) would not stop"))?;
        // Ended by itself as it stopped: how it said it ended stands.
        if !still()? {
            return Err(format!("{id}: its run ended meanwhile, as its row says"));
        }
        self.held.insert(id.to_string(), lock);
        self.held.insert(format!("{id} close"), closing);
        Ok(())
    }

    fn reopen_held(&mut self, id: &str, at_ms: i64) -> Result<bool, String> {
        // A stamp holds its write's time above its low 16 bits (`Clock::tick`).
        let sql = "SELECT 1 FROM tasks WHERE id = ?1 AND status = 'finished' AND
            (SELECT max(stamp) >> 16 FROM ops WHERE tbl = 'tasks' AND row_id = ?1 AND field = 'status') < ?2";
        let mut capped = false;
        let older = |db: &Connection| {
            let row = db
                .query_row(sql, rusqlite::params![id, at_ms], |_| Ok(()))
                .optional()?;
            capped = row.is_some() && over_cap(&standing_in(db)?, id).is_some();
            Ok(row.is_some() && !capped)
        };
        let writes = [
            field(id, "status", Some("open")),
            write(id, "attempts", Value::Int(0)),
            field(id, "pane", None),
            field(id, "session", None),
            field(id, "outcome", None),
            field(id, "said", None),
            field(id, "told", None),
            field(id, "deployed", None),
        ];
        self.log.write_if(older, &writes).map_err(said)?;
        Ok(capped)
    }

    fn untold(&self, id: &str) -> Result<Option<Outcome>, String> {
        let sql = "SELECT outcome, said FROM tasks WHERE id = ?1 AND status = 'finished' AND told IS NULL";
        let row = |db: &Connection| {
            db.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
        };
        let row: Option<(String, String)> = self
            .log
            .read(row)
            .map_err(said)?
            .map_err(|e| e.to_string())?;
        Ok(row.and_then(|(kind, words)| match kind.as_str() {
            "merged" => Some(Outcome::Merged(words)),
            "ended" => Some(Outcome::Ended(words)),
            "failed" => Some(Outcome::Failed(words)),
            _ => None,
        }))
    }

    fn claims(&self) -> Result<Vec<Claim>, String> {
        self.claimed()
    }

    fn told(&mut self, id: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "told", Some("yes"))])
            .map(drop)
            .map_err(said)
    }

    fn handed(&mut self, id: &str, head: &str) -> Result<(), String> {
        let writes = [
            field(id, "handed", Some(head)),
            field(id, "back", Some("1")),
        ];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn rejudged(&mut self, id: &str, words: &str) -> Result<(), String> {
        // Judged again because its PR merged: a reopen it brings is no hand-back (`handed_back`).
        let writes = [field(id, "rejudged", Some(words)), field(id, "back", None)];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn deployed(&mut self, id: &str, words: &str) -> Result<(), String> {
        let writes = [field(id, "deployed", Some(words))];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn note(&mut self, id: &str, words: &str) -> Result<(), String> {
        let writes = [field(id, "note", Some(words))];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn standing(&self) -> Result<Vec<Standing>, String> {
        self.log
            .read(standing_in)
            .map_err(said)?
            .map_err(|e| e.to_string())
    }

    fn hold_pr(&mut self, pr: u64, holder: &str, why: &str) -> Result<Option<Hold>, String> {
        self.hold_at(self.log.now_ms(), pr, holder, why)
    }

    fn unhold(&mut self, pr: u64) -> Result<(), String> {
        let writes = [hold_write(pr, "holder", Value::Null)];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn holds(&self) -> Result<Vec<Hold>, String> {
        self.holds_at(self.log.now_ms())
    }

    fn run_end(&mut self, id: &str, end: &End) -> Result<(), String> {
        self.end_at(self.log.now_ms(), id, end)
    }

    fn status(&self, id: &str) -> Result<Option<String>, String> {
        let row = |db: &Connection| {
            let sql = "SELECT status FROM tasks WHERE id = ?1";
            db.query_row(sql, [id], |r| r.get(0)).optional()
        };
        self.log.read(row).map_err(said)?.map_err(|e| e.to_string())
    }

    /// No sync yet is no queue: an empty one would read as nothing to do.
    fn queue(&self) -> Result<Queue, String> {
        let (listed, seats, seats_why) = self.listed()?;
        let seats = seats.ok_or("no sync of the list yet: its seats are not known")?;
        let routes = self.routes()?;
        let route = |demand: Demand| {
            let set = routes.iter().find(|(d, _)| d == demand.name());
            set.map(|(_, pools)| pools.clone())
        };
        let tasks = listed
            .into_iter()
            .filter(|(p, _)| p.is_some())
            .map(|(_, t)| Queued {
                route: route(t.demand),
                ..t
            })
            .collect();
        Ok(Queue {
            seats: seats as usize,
            seats_why,
            tasks,
        })
    }
}

impl SqliteTasks {
    fn attempts(&self, id: &str) -> Result<u32, String> {
        let row = |db: &Connection| {
            let sql = "SELECT attempts FROM tasks WHERE id = ?1";
            db.query_row(sql, [id], |r| r.get::<_, Option<u32>>(0))
                .optional()
        };
        let read = self
            .log
            .read(row)
            .map_err(said)?
            .map_err(|e| e.to_string())?;
        Ok(read.flatten().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::{End, OPEN_CAP, ORCHESTRATOR, Outcome};
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    /// Rows and locks in one folder: the tests of a store alone.
    fn open(dir: &Path) -> Result<SqliteTasks, String> {
        super::open(dir, dir)
    }

    fn run(outcome: Outcome) -> Run {
        let (pane, session) = (Some("w1:p2".to_string()), None);
        Run {
            pane,
            session,
            outcome,
        }
    }

    #[dash_test]
    fn an_agent_stands_as_its_last_write_says_and_survives_a_reopen() {
        let dir = TestDir::new("store-agents");
        let mut tasks = open(&dir.0).expect("open");
        tasks.agent("p1", "/w", "running").expect("started");
        tasks.agent("p2", "/v", "running").expect("started");
        tasks.agent("p1", "/w", "ended 0").expect("ended");
        let agents = open(&dir.0).expect("reopen").agents().expect("agents");
        let stood: Vec<_> = agents
            .iter()
            .map(|a| (a.pane.as_str(), a.folder.as_str(), a.state.as_str()))
            .collect();
        assert_eq!(stood, [("p1", "/w", "ended 0"), ("p2", "/v", "running")]);
        assert!(agents[0].last_ms > 0);
        assert!(
            tasks.rows().expect("rows").is_empty(),
            "an agent is no task"
        );
    }

    /// Done-when 2 of rust-front-door-known-id: `sweep_once` forgets, once, each row only a kept layer or deadline made that the
    /// list never filed, its fields with it; a listed task is never asked of, and a forgotten id the list later holds comes
    /// back with neither.
    #[dash_test]
    fn forget_removes_a_row_the_list_never_held() {
        let dir = TestDir::new("tasks-forget");
        let mut tasks = open(&dir.0).expect("open");
        tasks
            .deadline("no-such-task-xyz", "2026-11-15")
            .expect("a probe's deadline");
        tasks.layer("a", "design:board").expect("a's layer");
        let listed = |id: &str| Queued {
            id: id.into(),
            title: id.into(),
            ..Queued::default()
        };
        let queue = Queue {
            seats: 1,
            seats_why: String::new(),
            tasks: vec![listed("a")],
        };
        tasks.sync(&queue).expect("sync");
        // a failed sweep writes nothing and runs again; then once only
        assert!(tasks.sweep_once(|_| Err("no list".into())).is_err());
        let swept = tasks.sweep_once(|ids| {
            assert_eq!(ids, ["no-such-task-xyz"], "a listed task is never asked of");
            Ok(ids.to_vec())
        });
        assert_eq!(
            swept.expect("swept"),
            Some(vec!["no-such-task-xyz".to_string()])
        );
        assert!(tasks.never_listed().expect("read").is_empty());
        let again = tasks.sweep_once(|_| panic!("swept once already"));
        assert_eq!(again.expect("read"), None);
        let rows = |tasks: &SqliteTasks| -> Vec<(String, Option<String>)> {
            let (rows, _, _) = tasks.listed().expect("listed");
            (rows.into_iter())
                .map(|(_, t)| (t.id, t.rank.deadline))
                .collect()
        };
        assert_eq!(rows(&tasks), [("a".to_string(), None)]);
        assert_eq!(
            tasks.queue().expect("queue").tasks[0].layer.as_deref(),
            Some("design:board")
        );
        // the list holds it later: back, with no deadline of the probe's
        let both = Queue {
            seats: 1,
            seats_why: String::new(),
            tasks: vec![listed("a"), listed("no-such-task-xyz")],
        };
        tasks.sync(&both).expect("sync");
        let ids: Vec<_> = rows(&tasks)
            .into_iter()
            .map(|(id, d)| (id, d.is_none()))
            .collect();
        assert_eq!(
            ids,
            [
                ("a".to_string(), true),
                ("no-such-task-xyz".to_string(), true)
            ]
        );
        tasks.forget(&[]).expect("nothing to forget");
    }

    /// rust-task-names-layer: a layer filed before its task is listed is in the queue once a sync lists it, and stays across
    /// the syncs that follow, which never write it.
    #[dash_test]
    fn a_task_s_layer_outlives_every_sync_and_reaches_the_queue() {
        let dir = TestDir::new("tasks-layer");
        let mut tasks = open(&dir.0).expect("open");
        tasks.layer("a", "design:board").expect("layer");
        assert!(tasks.layer("not an id", "design:board").is_err());
        let listed = |id: &str| Queued {
            id: id.into(),
            title: id.into(),
            ..Queued::default()
        };
        let queue = Queue {
            seats: 2,
            seats_why: String::new(),
            tasks: vec![listed("a"), listed("b")],
        };
        tasks.sync(&queue).expect("sync");
        tasks.sync(&queue).expect("again");
        let mut tasks = open(&dir.0).expect("reopen");
        let read = tasks.queue().expect("queue");
        let layers: Vec<_> = read.tasks.iter().map(|t| t.layer.as_deref()).collect();
        assert_eq!(layers, [Some("design:board"), None]);
        tasks.layer("a", "model:task").expect("changed");
        assert_eq!(
            tasks.queue().expect("queue").tasks[0].layer.as_deref(),
            Some("model:task")
        );
        // task rust-task-impact: the impact is kept as the layer is, read into its rank, and no sync writes over it
        let impact = Impact::of("1200 Dashboard.Documents /documents 10").expect("an impact");
        tasks.impact("a", &impact).expect("kept");
        tasks.sync(&tasks.queue().expect("queue")).expect("synced");
        assert_eq!(
            tasks.queue().expect("queue").tasks[0].rank.impact,
            Some(impact.clone())
        );
        assert_eq!(tasks.impacts().expect("read").get("a"), Some(&impact));
        // task rust-orch-pick-dynamic: a label that builds nothing reaches the queue, and goes once the list drops it
        let light = |on: bool| Queue {
            seats: 2,
            seats_why: String::new(),
            tasks: vec![Queued {
                builds_nothing: on,
                ..listed("a")
            }],
        };
        assert_eq!(tasks.sync(&light(true)).expect("labelled"), 2, "b unlisted");
        assert!(tasks.queue().expect("queue").tasks[0].builds_nothing);
        assert_eq!(tasks.sync(&light(true)).expect("again"), 0);
        tasks.sync(&light(false)).expect("unlabelled");
        assert!(!tasks.queue().expect("queue").tasks[0].builds_nothing);
        tasks.deadline("a", "2026-11-12").expect("a deadline");
        assert!(tasks.deadline("not an id", "2026-11-12").is_err());
        // done-when 4 and 5: what the rank reads and the free kind are kept from the list, the deadline from the front door,
        // and a sync of the same writes nothing
        let ranked = Queue {
            seats: 2,
            seats_why: String::new(),
            tasks: vec![Queued {
                rank: Rank {
                    horizon: Some("now".into()),
                    item: Some("rust-poc".into()),
                    priority: Some(1),
                    estimate: Some(5),
                    deadline: None,
                    impact: None,
                },
                free: Some(FreeKind::Planning),
                ..listed("a")
            }],
        };
        assert_eq!(tasks.sync(&ranked).expect("ranked"), 5);
        assert_eq!(tasks.sync(&ranked).expect("again"), 0);
        let back = &tasks.queue().expect("queue").tasks[0];
        let mut kept = ranked.tasks[0].rank.clone();
        kept.deadline = Some("2026-11-12".into());
        // the impact kept above, which no sync writes over
        kept.impact = Some(impact);
        assert_eq!((&back.rank, back.free), (&kept, Some(FreeKind::Planning)));
        tasks.sync(&light(false)).expect("its fields dropped");
        let back = &tasks.queue().expect("queue").tasks[0];
        assert_eq!(
            (back.rank.priority, back.rank.horizon.as_deref(), back.free),
            (None, None, None)
        );
        // task rust-graph-why-these: its epic and labels are kept for the graph's selectors, and dropped with the list's
        let filed = Queue {
            seats: 2,
            seats_why: String::new(),
            tasks: vec![Queued {
                epic: Some("rust-poc".into()),
                labels: vec!["Docs".into(), "Bug".into()],
                // task task-dedupe: the files it names, what a start's file hold reads
                files: vec!["a.rs".into(), "bin/x".into()],
                ..listed("a")
            }],
        };
        assert_eq!(tasks.sync(&filed).expect("filed"), 3);
        assert_eq!(tasks.sync(&filed).expect("again"), 0);
        let back = &tasks.queue().expect("queue").tasks[0];
        assert_eq!(
            (back.epic.as_deref(), &back.labels[..], &back.files[..]),
            (
                Some("rust-poc"),
                &filed.tasks[0].labels[..],
                &filed.tasks[0].files[..]
            )
        );
        tasks.sync(&light(false)).expect("its fields dropped");
        let back = &tasks.queue().expect("queue").tasks[0];
        assert_eq!(
            (back.epic.as_deref(), back.labels.len(), back.files.len()),
            (None, 0, 0)
        );
    }

    /// Task rust-pools-fable, done-when 2: a task's demand is the list's, kept by each sync; its route is data, a row of this
    /// store per demand, none while none is set (the policy's own then), and the queue hands each task its own.
    #[dash_test]
    fn a_task_s_demand_and_its_route_are_data_the_queue_resolves() {
        let dir = TestDir::new("tasks-routes");
        let mut tasks = open(&dir.0).expect("open");
        let task = |id: &str, demand| Queued {
            id: id.into(),
            title: id.into(),
            demand,
            ..Queued::default()
        };
        let queue = Queue {
            seats: 2,
            seats_why: String::new(),
            tasks: vec![task("h", Demand::Hard), task("n", Demand::Normal)],
        };
        tasks.sync(&queue).expect("sync");
        assert_eq!(
            tasks.sync(&queue).expect("again"),
            0,
            "the same demand writes nothing"
        );
        let routes = |tasks: &SqliteTasks| {
            let q = tasks.queue().expect("queue");
            q.tasks
                .iter()
                .map(|t| (t.demand, t.route.as_ref().map(|r| r.join(","))))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            routes(&tasks),
            [(Demand::Hard, None), (Demand::Normal, None)],
            "no row: the policy's own"
        );
        // a new constraint is a row (RFC 20h rule 4): a route set here, read back after a reopen
        tasks
            .route(Demand::Hard, &["fable".into(), "sonnet".into()])
            .expect("set");
        tasks.route(Demand::Normal, &[]).expect("none");
        // task linear-projects-view, done-when 1: a roadmap item holds start, target, depends-on and milestones, Rust's own
        tasks
            .plan("rust", &[("start", "2026-11-01"), ("target", "2026-12-15")])
            .expect("dates");
        tasks
            .plan("rust", &[("after", "jobs  klar")])
            .expect("after");
        tasks
            .plan(
                "rust",
                &[(
                    "milestones",
                    "2026-11-20 Design system\n2026-12-01 Pick order",
                )],
            )
            .expect("milestones");
        for (field, value) in [
            ("start", "2026-11-31"),
            ("target", "soon"),
            ("after", "rust"),
            ("after", "a/b"),
            ("milestones", "2026-11-20"),
            ("owner", "x"),
            // after the plan's target, 2026-12-15
            ("start", "2026-12-20"),
        ] {
            assert!(
                tasks.plan("rust", &[(field, value)]).is_err(),
                "{field} {value}"
            );
        }
        // one field that does not read refuses them all: the start given with it is not written
        assert!(
            tasks
                .plan("rust", &[("start", "2026-11-05"), ("target", "soon")])
                .is_err()
        );
        // a start and a target given together are read together
        tasks
            .plan("rust", &[("start", "2026-12-20"), ("target", "2026-12-31")])
            .expect("moved");
        tasks
            .plan("rust", &[("start", "2026-11-01"), ("target", "2026-12-15")])
            .expect("back");
        let plans = tasks.plans().expect("plans");
        assert_eq!(
            plans["rust"],
            Plan {
                start: Some("2026-11-01".into()),
                target: Some("2026-12-15".into()),
                after: vec!["jobs".into(), "klar".into()],
                milestones: vec![
                    ("2026-11-20".into(), "Design system".into()),
                    ("2026-12-01".into(), "Pick order".into())
                ],
            }
        );
        assert!(
            tasks.route(Demand::Hard, &["fa ble".into()]).is_err(),
            "no pool name"
        );
        let tasks = open(&dir.0).expect("reopen");
        assert_eq!(
            routes(&tasks),
            [
                (Demand::Hard, Some("fable,sonnet".into())),
                (Demand::Normal, Some(String::new()))
            ]
        );
        assert_eq!(
            tasks.routes().expect("rows"),
            [
                ("hard".into(), vec!["fable".into(), "sonnet".into()]),
                ("normal".into(), vec![])
            ]
        );
    }

    #[dash_test]
    fn a_task_goes_from_no_row_to_recorded_and_survives_a_reopen() {
        let dir = TestDir::new("tasks-life");
        let mut tasks = open(&dir.0).expect("open");
        assert_eq!(tasks.show("none").expect("show"), None);
        assert_eq!(tasks.status("t1").expect("status"), None);
        assert_eq!(tasks.claim("t1").expect("claim"), 1);
        assert_eq!(
            tasks.status("t1").expect("status").as_deref(),
            Some("claimed")
        );
        assert!(tasks.claim("t1").is_err(), "a run holds it");
        tasks.opened("t1", "w1:p2").expect("opened");
        tasks
            .record("t1", &run(Outcome::Merged("https://pr/7".into())))
            .expect("record");
        drop(tasks);
        let mut tasks = open(&dir.0).expect("reopen");
        let row = tasks.show("t1").expect("show").expect("row");
        assert_eq!(
            row,
            "status: finished\nattempts: 1\npane: w1:p2\nsession: \naccount: \noutcome: merged\nsaid: https://pr/7\nwaits: \nrejudged: \ndeployed: \nnote: "
        );
        // its Activity: the claim with its try, then the end with its outcome, oldest first
        let history = tasks.history("t1").expect("history");
        let told: Vec<_> = (history.iter())
            .map(|e| (e.status.as_str(), e.attempts, e.outcome.as_deref()))
            .collect();
        assert_eq!(
            told,
            [
                ("claimed", Some(1), None),
                ("finished", None, Some("merged"))
            ]
        );
        assert!(history[0].at_ms > 0 && history[0].at_ms <= history[1].at_ms);
        assert_eq!(tasks.history("none").expect("history"), []);
        assert!(
            tasks
                .claim("t1")
                .is_err_and(|e| e.contains("not an open task"))
        );
    }

    /// Task rust-orch-flags-only-stuck: a note is its own line in its page's Activity, after the end it follows, and no run: the
    /// row's last write stays its end's.
    #[dash_test]
    fn a_note_is_its_own_line_in_the_activity_and_no_run() {
        let dir = TestDir::new("tasks-note");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        let why = "met, its session ended while CI ran, then: a conflict";
        tasks
            .record("t1", &run(Outcome::Ended(why.into())))
            .expect("record");
        let last = tasks.rows().expect("rows")[0].last_ms;
        tasks
            .note("t1", "handed back to a new run: a conflict")
            .expect("note");
        let history = tasks.history("t1").expect("history");
        let told: Vec<_> = (history.iter())
            .map(|e| (e.status.as_str(), e.outcome.as_deref()))
            .collect();
        assert_eq!(
            told,
            [
                ("claimed", None),
                ("finished", Some("ended")),
                ("note", Some("handed back to a new run: a conflict"))
            ]
        );
        assert_eq!(tasks.rows().expect("rows")[0].last_ms, last);
    }

    /// A status write's event holds its own commit's fields alone: a later write in the same millisecond, an outcome with no
    /// status, never changes it (/code-review of rust-task-page-linear).
    #[dash_test]
    fn a_later_write_in_the_same_millisecond_keeps_out_of_an_event() {
        let dir = TestDir::new("tasks-history");
        let mut tasks = open(&dir.0).expect("open");
        let ended = run(Outcome::Merged("https://pr/7".into()));
        (tasks
            .log
            .write_at(1_000, &run_writes("t1", "finished", &ended)))
        .expect("finished");
        (tasks
            .log
            .write_at(1_000, &[field("t1", "outcome", Some("waiting"))]))
        .expect("a later outcome");
        let history = tasks.history("t1").expect("history");
        let told: Vec<_> = (history.iter())
            .map(|e| (e.at_ms, e.status.as_str(), e.outcome.as_deref()))
            .collect();
        assert_eq!(told, [(1_000, "finished", Some("merged"))]);
    }

    /// A crashed run's session still at work is watched on by a new run: the same pane, session and login, one try more, its
    /// lock held, and never while a run lives.
    #[dash_test]
    fn a_crashed_claim_is_held_by_a_new_run_with_its_pane_session_and_login() {
        let dir = TestDir::new("tasks-hold");
        let mut crashed = open(&dir.0).expect("open");
        crashed.claim("gone").expect("claim");
        crashed.opened("gone", "w1:p1").expect("opened");
        crashed.adopted("gone", "s1").expect("adopted");
        crashed.login("gone", "second").expect("login");
        crashed.model("gone", "fable").expect("its model");
        let mut living = open(&dir.0).expect("open");
        assert!(
            living
                .hold("gone")
                .is_err_and(|e| e.contains("a run holds it"))
        );
        drop(crashed);

        let held = living.hold("gone").expect("held");
        let (pane, session, account) = (
            Some("w1:p1".into()),
            Some("s1".into()),
            Some("second".into()),
        );
        let (id, attempts) = ("gone".to_string(), 2);
        assert_eq!(
            held,
            Claim {
                id,
                pane,
                session,
                attempts,
                account,
                model: Some("fable".into()),
            },
            "its model kept, so a limit's resume names it (RFC 20h)"
        );
        assert_eq!(
            living.status("gone").expect("status").as_deref(),
            Some("claimed")
        );
        assert!(
            living.crashed().expect("crashed").is_empty(),
            "its new run holds it"
        );
        assert!(open(&dir.0).expect("open").hold("gone").is_err());
        living.add("open", &[]).expect("add");
        assert!(
            living
                .hold("open")
                .is_err_and(|e| e.contains("not claimed"))
        );
    }

    #[dash_test]
    fn of_two_stores_on_one_file_only_one_claim_takes_the_task() {
        let dir = TestDir::new("tasks-claim");
        let mut first = open(&dir.0).expect("open");
        let mut second = open(&dir.0).expect("open again");
        assert!(first.claim("t1").is_ok());
        assert!(second.claim("t1").is_err());
    }

    #[dash_test]
    fn an_id_is_no_path() {
        let dir = TestDir::new("tasks-id");
        let mut tasks = open(&dir.0).expect("open");
        for id in ["../x", "a/b", "", "a b"] {
            assert!(
                tasks.claim(id).is_err_and(|e| e.contains("not a task id")),
                "{id}"
            );
        }
    }

    #[dash_test]
    fn a_living_run_is_stopped_by_its_lock_s_pid_and_its_claim_taken_over() {
        let dir = TestDir::new("tasks-take-over");
        let mut first = open(&dir.0).expect("open");
        first.claim("t").expect("claim");
        drop(first);
        // A run in another process: its lock held, its pid written, as `claim` does.
        let lock = dir.0.join("rust-t.lock");
        let hold = "import fcntl,os,sys,time\nf=open(sys.argv[1],'r+')\nfcntl.flock(f,fcntl.LOCK_EX)\n\
                    f.truncate(0)\nf.write(str(os.getpid()))\nf.flush()\nprint('held',flush=True)\ntime.sleep(60)";
        let mut child = std::process::Command::new("python3")
            .args(["-I", "-c", hold])
            .arg(&lock)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("python3");
        let mut held = String::new();
        let out = child.stdout.take().expect("stdout");
        std::io::BufRead::read_line(&mut std::io::BufReader::new(out), &mut held).expect("held");
        assert_eq!(held.trim(), "held");

        let mut closer = open(&dir.0).expect("open");
        assert_eq!(closer.crashed().expect("claims"), [], "its run lives");
        closer.take_over("t").expect("taken over");
        assert!(
            child.wait().expect("ended").code().is_none(),
            "ended by a signal"
        );
        assert_eq!(
            closer.status("t").expect("status").as_deref(),
            Some("claimed")
        );
        assert_eq!(
            closer.crashed().expect("claims"),
            [],
            "held here now: no look frees it"
        );
        let ended = run(Outcome::Ended("closed".into()));
        closer.record("t", &ended).expect("recorded");
        assert_eq!(
            closer.status("t").expect("status").as_deref(),
            Some("finished")
        );
        assert!(
            closer
                .take_over("t")
                .is_err_and(|e| e.contains("not claimed"))
        );

        // Between its run's end and the close's take of its lock, a look neither reads a crash nor frees the claim; a close that
        // died lets it read as one again (/code-review of baa197583).
        let mut gone = open(&dir.0).expect("open");
        gone.claim("g").expect("claim");
        drop(gone);
        let watcher = &mut open(&dir.0).expect("open");
        let closing = closer
            .take_named("g", "close")
            .expect("look")
            .expect("free");
        assert_eq!(watcher.crashed().expect("claims"), []);
        assert!(!watcher.free("g").expect("a look"));
        drop(closing);
        assert_eq!(watcher.crashed().expect("claims").len(), 1);

        // A pid that is the closer's own is never stopped.
        let mut here = open(&dir.0).expect("open");
        here.claim("u").expect("claim");
        let said = closer.take_over("u").expect_err("its own pid");
        assert!(said.contains("names this very process"), "{said}");
    }

    #[dash_test]
    fn a_crashed_run_s_claim_is_freed_a_living_one_s_is_not_and_it_is_claimed_again() {
        let dir = TestDir::new("tasks-crash");
        let mut crashed = open(&dir.0).expect("open");
        let mut living = open(&dir.0).expect("open");
        crashed.claim("gone").expect("claim");
        crashed.opened("gone", "w1:p1").expect("opened");
        living.claim("here").expect("claim");
        // What a crash does to a claim: the process's handle on its lock ends. Nothing else ran.
        drop(crashed);

        let mut watcher = open(&dir.0).expect("open");
        let pane = Some("w1:p1".to_string());
        let (id, session, attempts) = ("gone".to_string(), None, 1);
        assert_eq!(
            watcher.crashed().expect("crashed"),
            [Claim {
                id,
                pane,
                session,
                attempts,
                account: None,
                model: None,
            }]
        );
        assert_eq!(
            watcher.status("gone").expect("status").as_deref(),
            Some("claimed"),
            "still its run's until what it left is dealt with"
        );
        assert!(watcher.claim("gone").is_err());
        assert!(watcher.free("gone").expect("freed"));
        assert!(!watcher.free("here").expect("a run holds it"));
        assert_eq!(
            watcher.status("gone").expect("status").as_deref(),
            Some("open")
        );
        assert_eq!(
            watcher.status("here").expect("status").as_deref(),
            Some("claimed")
        );
        assert_eq!(watcher.crashed().expect("none left"), []);
        assert_eq!(
            watcher.claim("gone").expect("again"),
            2,
            "its attempts add up"
        );
    }

    #[dash_test]
    fn a_run_that_ended_while_its_claim_was_being_looked_at_stays_as_it_ended() {
        let dir = TestDir::new("tasks-race");
        let mut run_side = open(&dir.0).expect("open");
        let mut watcher = open(&dir.0).expect("open");
        run_side.claim("t1").expect("claim");
        // The watcher read the row as claimed; the run then ends, and its lock goes with it.
        let read = watcher.claimed().expect("read");
        assert_eq!(read.len(), 1);
        let merged = run(Outcome::Merged("https://pr/7".into()));
        run_side.record("t1", &merged).expect("record");
        assert_eq!(watcher.crashed().expect("none"), []);
        assert!(!watcher.free("t1").expect("no longer claimed"));
        let row = watcher.show("t1").expect("show").expect("row");
        assert!(
            row.contains("status: finished") && row.contains("outcome: merged"),
            "{row}"
        );
    }

    #[dash_test]
    fn a_merged_pr_is_untold_until_the_list_hears_of_it() {
        let dir = TestDir::new("tasks-told");
        let mut tasks = open(&dir.0).expect("open");
        assert_eq!(tasks.untold("t1").expect("read"), None);
        tasks.claim("t1").expect("claim");
        tasks
            .record("t1", &run(Outcome::Merged("https://pr/7".into())))
            .expect("record");
        assert_eq!(
            tasks.untold("t1").expect("read"),
            Some(Outcome::Merged("https://pr/7".into()))
        );
        tasks.told("t1").expect("told");
        assert_eq!(tasks.untold("t1").expect("read"), None);

        // Ended is untold the same way; a failure released for a retry is not finished, so it is not.
        tasks.claim("t2").expect("claim");
        let panes = |tasks: &SqliteTasks| -> Vec<Option<String>> {
            let claims = tasks.claims().expect("read");
            claims.into_iter().map(|c| c.pane).collect()
        };
        assert_eq!(panes(&tasks), [None]);
        tasks.opened("t2", "p2").expect("opened");
        assert_eq!(panes(&tasks), [Some("p2".to_string())]);
        tasks
            .release("t2", &run(Outcome::Failed("no pane".into())))
            .expect("release");
        assert_eq!(tasks.claims().expect("read"), []);
        assert_eq!(tasks.untold("t2").expect("read"), None);
        tasks.claim("t2").expect("claim");
        tasks
            .record("t2", &run(Outcome::Ended("given up".into())))
            .expect("record");
        assert_eq!(
            tasks.untold("t2").expect("read"),
            Some(Outcome::Ended("given up".into()))
        );
    }

    /// Task rust-pr-one-owner, done-when 1 and 2 on the real store: one holder a PR; a hold ends on release or 30 minutes
    /// untouched, a run's excepted while its task is claimed, which no orchestrator takes from; a take survives a reopen.
    #[dash_test]
    fn a_pr_has_one_holder_until_released_or_thirty_minutes_untouched() {
        let dir = TestDir::new("tasks-holds");
        let mut tasks = open(&dir.0).expect("open");
        let at = 1_794_186_900_000;
        let minute = 60_000;
        assert_eq!(
            tasks
                .hold_at(at, 7, ORCHESTRATOR, "dash land")
                .expect("taken"),
            None
        );
        let refused = tasks.hold_at(at + minute, 7, "run t1", "handed back");
        assert_eq!(refused, Err("#7 is held by orchestrator: dash land".into()));
        // touched by its holder at +29, so live at +58, gone at +60
        tasks
            .hold_at(at + 29 * minute, 7, ORCHESTRATOR, "dash land")
            .expect("touched");
        let holds = open(&dir.0)
            .expect("reopen")
            .holds_at(at + 58 * minute)
            .expect("read");
        assert_eq!(holds.len(), 1, "{holds:?}");
        assert_eq!((holds[0].pr, holds[0].holder.as_str()), (7, ORCHESTRATOR));
        assert!(tasks.holds_at(at + 60 * minute).expect("read").is_empty());
        let after = at + 60 * minute;
        assert_eq!(
            tasks
                .hold_at(after, 7, "run t1", "handed back")
                .expect("taken"),
            None
        );
        // its task claimed: live past 30 minutes, and kept from the orchestrator
        tasks.claim("t1").expect("claim");
        let later = after + 90 * minute;
        assert_eq!(tasks.holds_at(later).expect("read").len(), 1);
        let refused = tasks.hold_at(later, 7, ORCHESTRATOR, "urgent");
        assert_eq!(refused, Err("#7 is held by run t1: handed back".into()));
        tasks.unhold(7).expect("released");
        assert!(tasks.holds_at(later).expect("read").is_empty());
        assert_eq!(
            tasks
                .hold_at(later, 7, ORCHESTRATOR, "urgent")
                .expect("taken"),
            None
        );
    }

    /// Task rust-pr-one-owner, done-when 3 and 4: each run's end kept apart from its row, the day's read back newest first; its
    /// nudges too (rust-wait-background-shell, done-when 3: the count per task, from the run rows).
    #[dash_test]
    fn each_run_s_end_is_kept_and_read_back_from_a_day() {
        let dir = TestDir::new("tasks-ends");
        let mut tasks = open(&dir.0).expect("open");
        let day = 1_794_182_400_000;
        let dup = End {
            pushed: Some(false),
            duplicate: true,
            tokens: Some(40_000),
            nudges: 2,
        };
        tasks.end_at(day - 1, "t0", &dup).expect("yesterday");
        tasks.end_at(day + 5, "t1", &End::default()).expect("kept");
        tasks.end_at(day + 9, "t1", &dup).expect("kept");
        let ends = open(&dir.0).expect("reopen").ends_since(day).expect("read");
        let read: Vec<_> = ends
            .iter()
            .map(|e| (e.task.as_str(), e.at_ms, &e.end))
            .collect();
        assert_eq!(
            read,
            [("t1", day + 9, &dup), ("t1", day + 5, &End::default())]
        );
    }

    #[dash_test]
    fn a_released_run_can_be_claimed_again_and_says_how_it_ended() {
        let dir = TestDir::new("tasks-release");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        tasks
            .release("t1", &run(Outcome::Failed("no prompt".into())))
            .expect("release");
        let row = tasks.show("t1").expect("show").expect("row");
        assert!(
            row.contains("status: open") && row.contains("said: no prompt"),
            "{row}"
        );
        assert_eq!(tasks.claim("t1").expect("again"), 2);
    }

    #[dash_test]
    fn a_claim_saves_its_session_and_a_later_claim_forgets_it() {
        let dir = TestDir::new("tasks-session");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        tasks.opened("t1", "p1").expect("opened");
        tasks.adopted("t1", "s1").expect("adopted");
        let claim = &tasks.claims().expect("read")[0];
        assert_eq!(claim.session.as_deref(), Some("s1"));
        // A met run's session gone while CI runs: the claim keeps its session and drops its pane, which no seat counts.
        tasks.left("t1").expect("left");
        let claim = &tasks.claims().expect("read")[0];
        assert_eq!((&claim.pane, claim.session.as_deref()), (&None, Some("s1")));
        drop(tasks);
        let mut tasks = open(&dir.0).expect("reopen");
        assert_eq!(
            tasks.crashed().expect("read")[0].session.as_deref(),
            Some("s1")
        );
        assert!(tasks.free("t1").expect("freed"));
        tasks.claim("t1").expect("again");
        let claim = &tasks.claims().expect("read")[0];
        assert_eq!((&claim.pane, &claim.session), (&None, &None));
    }

    /// The lock a run holds is beside the worktree: a second data folder has its own rows and still cannot start one there.
    #[dash_test]
    fn another_data_folder_cannot_claim_a_task_a_run_holds() {
        let dir = TestDir::new("tasks-beside");
        let (one, other, root) = (dir.0.join("one"), dir.0.join("other"), dir.0.join("root"));
        for folder in [&one, &other] {
            std::fs::create_dir_all(folder).expect("dir");
        }
        let mut running = super::open(&one, &root).expect("open");
        let mut second = super::open(&other, &root).expect("open");
        running.claim("t1").expect("claim");
        assert!(root.join("rust-t1.lock").exists());
        assert!(
            second
                .claim("t1")
                .is_err_and(|e| e.contains("a run holds it"))
        );
        running
            .release("t1", &run(Outcome::Failed("no prompt".into())))
            .expect("release");
        assert_eq!(second.claim("t1").expect("free now"), 1);
    }

    #[dash_test]
    fn a_wait_shows_on_its_claimed_row_until_the_session_is_told_to_go_on() {
        let dir = TestDir::new("tasks-waiting");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        let error = Outcome::Offline("API Error: Can't reach the API server".into());
        tasks.waiting("t1", Some(&error)).expect("waiting");
        let row = &tasks.rows().expect("rows")[0];
        assert_eq!(row.status, "claimed");
        assert_eq!(row.outcome.as_deref(), Some("offline"));
        tasks.waiting("t1", None).expect("told");
        let row = &tasks.rows().expect("rows")[0];
        assert_eq!((row.outcome.as_deref(), row.said.as_deref()), (None, None));
    }

    #[dash_test]
    fn a_task_reopened_after_it_finished_starts_over_and_one_older_stays_finished() {
        let dir = TestDir::new("tasks-reopen");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        tasks
            .record("t1", &run(Outcome::Merged("https://pr/7".into())))
            .expect("record");
        tasks.told("t1").expect("told");
        let finished_ms = tasks.rows().expect("rows")[0].last_ms;

        tasks.reopen_held("t1", finished_ms - 1).expect("older");
        assert_eq!(
            tasks.status("t1").expect("status").as_deref(),
            Some("finished"),
            "its finish is the newer: nothing reopened it since"
        );
        tasks.reopen_held("none", finished_ms + 1).expect("no row");
        assert_eq!(tasks.status("none").expect("status"), None);

        tasks.reopen_held("t1", finished_ms + 1).expect("newer");
        let row = tasks.show("t1").expect("show").expect("row");
        assert_eq!(
            row,
            "status: open\nattempts: 0\npane: \nsession: \naccount: \noutcome: \nsaid: \nwaits: \nrejudged: \ndeployed: \nnote: "
        );
        assert_eq!(tasks.claim("t1").expect("again"), 1, "tries start over");
        tasks
            .record("t1", &run(Outcome::Merged("https://pr/8".into())))
            .expect("record");
        assert_eq!(
            tasks.untold("t1").expect("read"),
            Some(Outcome::Merged("https://pr/8".into())),
            "told again: its old told is gone"
        );
        // Claimed or open: a reopen touches only what finished.
        tasks.claim("t2").expect("claim");
        tasks.reopen_held("t2", i64::MAX).expect("claimed");
        assert_eq!(
            tasks.status("t2").expect("status").as_deref(),
            Some("claimed")
        );
    }

    #[dash_test]
    fn the_head_a_task_was_handed_back_at_outlives_its_reopen() {
        let dir = TestDir::new("tasks-handed");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        tasks
            .record("t1", &run(Outcome::Ended("not met".into())))
            .expect("record");
        tasks.handed("t1", "abc").expect("handed");
        let finished_ms = tasks.rows().expect("rows")[0].last_ms;
        tasks.reopen_held("t1", finished_ms + 1).expect("reopened");
        let standing = tasks.standing().expect("rows");
        let row = (standing[0].status.as_str(), standing[0].handed.as_deref());
        assert_eq!(
            row,
            ("open", Some("abc")),
            "a session that moves nothing is not handed it twice"
        );
        // Live through its reopen and a retry; a run on it that finishes, however, or a judge of its merge ends it, the head kept.
        let back = |tasks: &SqliteTasks| {
            let row = &tasks.standing().expect("rows")[0];
            (row.back, row.handed.clone())
        };
        let abc = Some("abc".to_string());
        assert_eq!(back(&tasks), (true, abc.clone()), "live after its reopen");
        tasks.claim("t1").expect("claim");
        tasks
            .release("t1", &run(Outcome::Failed("crashed".into())))
            .expect("release");
        assert_eq!(
            back(&tasks),
            (true, abc.clone()),
            "a retry is the same hand-back"
        );
        tasks.claim("t1").expect("claim");
        tasks
            .record("t1", &run(Outcome::Ended("not met".into())))
            .expect("record");
        assert_eq!(
            back(&tasks),
            (false, abc),
            "ended: a later reopen is no hand-back"
        );
        tasks.handed("t1", "def 1").expect("handed");
        tasks.rejudged("t1", "def met").expect("rejudged");
        let def = Some("def 1".to_string());
        assert_eq!(
            back(&tasks),
            (false, def),
            "a judge of its merge ends it too"
        );
    }

    #[dash_test]
    fn show_names_the_merged_pr_an_ended_run_was_judged_again_at_and_its_verdict() {
        let dir = TestDir::new("tasks-rejudged");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        tasks
            .record("t1", &run(Outcome::Ended("not met".into())))
            .expect("record");
        let words =
            "abc not met, reopened: https://x/pull/7 merged after its run ended, but part 2";
        tasks.rejudged("t1", words).expect("rejudged");
        let finished_ms = tasks.rows().expect("rows")[0].last_ms;
        tasks.reopen_held("t1", finished_ms + 1).expect("reopened");
        let shown = tasks.show("t1").expect("show").expect("a row");
        assert!(shown.contains(&format!("rejudged: {words}")), "{shown}");
        let standing = tasks.standing().expect("rows");
        assert_eq!(
            standing[0].rejudged.as_deref(),
            Some(words),
            "a reopen keeps it, so the same head is not judged again"
        );
        tasks.deployed("t1", "failed 1: x").expect("deployed");
        let standing = tasks.standing().expect("rows");
        let deployed = standing[0].deployed.as_deref();
        assert_eq!(deployed, Some("failed 1: x"), "what the look retries");
    }

    #[dash_test]
    fn the_page_s_read_says_a_claim_no_run_holds_is_gone() {
        let dir = TestDir::new("tasks-gone");
        let mut crashed = open(&dir.0).expect("open");
        let mut living = open(&dir.0).expect("open");
        crashed.claim("gone").expect("claim");
        living.claim("here").expect("claim");
        let page = open(&dir.0).expect("open");
        let gone = |page: &SqliteTasks, id: &str| {
            let rows = page.rows().expect("rows");
            rows.iter().find(|r| r.id == id).expect("row").run_gone
        };
        assert!(!gone(&page, "gone"), "its run lives");
        drop(crashed);
        assert!(gone(&page, "gone"));
        assert!(!gone(&page, "here"));
        // The look lets go at once: it starts nobody's run by mistake and stops none.
        assert!(page.crashed().expect("crashed").len() == 1);
        // A lock nothing can look at is said, never read as a run alive.
        std::fs::remove_file(dir.0.join("rust-gone.lock")).expect("its file");
        std::fs::create_dir(dir.0.join("rust-gone.lock")).expect("a folder in its place");
        assert!(page.crashed().is_err());
        assert!(!gone(&page, "gone"), "the page says nothing it did not see");
    }
    fn names(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[dash_test]
    fn a_task_names_what_it_waits_on_and_a_cycle_is_refused_at_write() {
        let dir = TestDir::new("tasks-waits");
        let mut tasks = open(&dir.0).expect("open");
        tasks.add("a", &[]).expect("a");
        tasks.add("b", &names(&["a", "a"])).expect("b");
        tasks.add("c", &names(&["b"])).expect("c");
        let row = tasks.show("c").expect("show").expect("row");
        assert!(
            row.contains("status: open") && row.contains("waits: b"),
            "{row}"
        );
        let b = tasks.standing().expect("rows").remove(1);
        assert_eq!(b.waits, ["a"], "named once");

        let refusal = tasks.waits("a", &names(&["c"])).expect_err("a cycle");
        assert_eq!(refusal, "a: refused, a cycle: a -> c -> b -> a");
        let own = tasks.waits("a", &names(&["a"])).expect_err("its own wait");
        assert_eq!(own, "a: refused, a cycle: a -> a");
        let unknown = tasks.waits("a", &names(&["nope"])).expect_err("no row");
        assert_eq!(unknown, "a: refused, it names nope, no such task");
        let new_loop = tasks.add("d", &names(&["d"])).expect_err("its own wait");
        assert!(new_loop.contains("a cycle"), "{new_loop}");
        assert_eq!(tasks.status("d").expect("status"), None, "nothing written");
        assert_eq!(
            tasks.standing().expect("rows")[0].waits,
            Vec::<String>::new()
        );

        // A replaced list frees the edge: no cycle is left to refuse, and a run's row keeps its status.
        tasks.claim("a").expect("claim");
        tasks.waits("c", &[]).expect("c again");
        tasks.waits("a", &names(&["c"])).expect("no cycle now");
        assert_eq!(
            tasks.status("a").expect("status").as_deref(),
            Some("claimed")
        );
        assert!(
            tasks
                .add("../x", &[])
                .is_err_and(|e| e.contains("not a task id"))
        );
        // A second add wipes nothing, and a list for no row writes none.
        let again = tasks.add("b", &[]).expect_err("it exists");
        assert!(again.contains("it exists"), "{again}");
        assert_eq!(tasks.standing().expect("rows")[1].waits, ["a"]);
        let none = tasks.waits("nope", &[]).expect_err("no row");
        assert_eq!(none, "nope: refused, no such task");
        assert_eq!(tasks.status("nope").expect("status"), None);
    }

    #[dash_test]
    fn a_claim_refuses_a_task_whose_wait_is_not_merged() {
        let dir = TestDir::new("tasks-claim-waits");
        let mut tasks = open(&dir.0).expect("open");
        tasks.add("first", &[]).expect("add");
        tasks.add("after", &names(&["first"])).expect("add");
        let line = "after: refused, it waits on first (open)";
        assert_eq!(tasks.claim("after").expect_err("it waits"), line);
        assert_eq!(
            tasks.status("after").expect("status").as_deref(),
            Some("open")
        );
        tasks.claim("first").expect("claim");
        let ended = run(Outcome::Ended("given up".into()));
        tasks.record("first", &ended).expect("record");
        let line = "after: refused, it waits on first (ended)";
        assert_eq!(tasks.claim("after").expect_err("ended is not merged"), line);
        tasks.waits("after", &[]).expect("freed by hand");
        assert_eq!(tasks.claim("after").expect("no wait left"), 1);
    }

    #[dash_test]
    fn closing_an_open_row_frees_its_place_under_the_cap() {
        let dir = TestDir::new("tasks-close");
        let mut tasks = open(&dir.0).expect("open");
        for n in 0..OPEN_CAP {
            tasks.add(&format!("t{n}"), &[]).expect("within the cap");
        }
        assert!(tasks.add("more", &[]).is_err());
        tasks.close("t0", "a typo").expect("close");
        assert_eq!(
            tasks.untold("t0").expect("read"),
            Some(Outcome::Ended("a typo".into())),
            "the list hears of it, if it still offers the task"
        );
        tasks.add("more", &[]).expect("room for one");
        tasks.claim("t1").expect("claim");
        for id in ["t1", "t0", "nope"] {
            let line = format!("{id}: refused, not an open task");
            assert_eq!(tasks.close(id, "why").expect_err("not open"), line);
        }
    }

    #[dash_test]
    fn a_row_an_unfinished_task_waits_on_is_not_closed() {
        let dir = TestDir::new("tasks-close-waits");
        let mut tasks = open(&dir.0).expect("open");
        tasks.add("first", &[]).expect("add");
        tasks.add("after", &names(&["first"])).expect("add");
        let line = "first: refused, after waits on it: dash task waits changes that first";
        assert_eq!(tasks.close("first", "a typo").expect_err("a waiter"), line);
        assert_eq!(
            tasks.status("first").expect("status").as_deref(),
            Some("open")
        );
        tasks
            .close("after", "not needed")
            .expect("a finished waiter holds nothing");
        tasks.close("first", "a typo").expect("no waiter left");
    }

    #[dash_test]
    fn a_hand_written_wait_is_no_run() {
        let dir = TestDir::new("tasks-last-run");
        let mut tasks = open(&dir.0).expect("open");
        tasks.add("t1", &[]).expect("add");
        tasks.add("t2", &[]).expect("add");
        tasks.claim("t1").expect("claim");
        tasks
            .record("t1", &run(Outcome::Merged("https://pr/7".into())))
            .expect("record");
        let last = |tasks: &SqliteTasks| {
            let rows = tasks.rows().expect("rows");
            rows.iter().find(|r| r.id == "t1").expect("row").last_ms
        };
        let before = last(&tasks);
        std::thread::sleep(std::time::Duration::from_millis(5));
        tasks.waits("t1", &names(&["t2"])).expect("waits");
        assert_eq!(last(&tasks), before);
    }

    #[dash_test]
    fn a_row_with_no_status_yet_stands_as_open() {
        let dir = TestDir::new("tasks-no-status");
        let db = crate::connect(&dir.0.join("tasks.sqlite")).expect("connect");
        db.execute_batch(SCHEMA).expect("the table");
        db.execute_batch("INSERT INTO tasks (id, waits) VALUES ('half', '')")
            .expect("a split batch");
        drop(db);
        let tasks = open(&dir.0).expect("open");
        assert_eq!(tasks.standing().expect("rows")[0].status, "open");
    }

    #[dash_test]
    fn a_51st_open_task_is_refused_with_a_line_saying_so() {
        let dir = TestDir::new("tasks-cap");
        let mut tasks = open(&dir.0).expect("open");
        for n in 0..OPEN_CAP {
            tasks.add(&format!("t{n}"), &[]).expect("within the cap");
        }
        let line = "t50: refused, 50 Rust tasks are open, the cap: one must finish first";
        assert_eq!(tasks.add("t50", &[]).expect_err("the 51st"), line);
        assert_eq!(tasks.status("t50").expect("status"), None);
        assert_eq!(tasks.claim("t50").expect_err("nor by a claim"), line);
        tasks
            .waits("t0", &names(&["t1"]))
            .expect("an open row may still change");

        // Finished is no longer open: one more fits, and a reopen past the cap stays finished.
        tasks.claim("t1").expect("claim");
        let merged = run(Outcome::Merged("https://pr/7".into()));
        tasks.record("t1", &merged).expect("record");
        tasks.claim("t50").expect("one finished, room for one");
        assert!(tasks.reopen_held("t1", i64::MAX).expect("no room"), "said");
        assert_eq!(
            tasks.status("t1").expect("status").as_deref(),
            Some("finished")
        );
        let open: usize = tasks
            .standing()
            .expect("rows")
            .iter()
            .filter(|r| r.status != "finished")
            .count();
        assert_eq!(open, OPEN_CAP);
    }

    #[dash_test]
    fn a_folder_from_before_waits_keeps_its_rows() {
        let dir = TestDir::new("tasks-old");
        let db = crate::connect(&dir.0.join("tasks.sqlite")).expect("connect");
        db.execute_batch(
            "CREATE TABLE tasks (id TEXT PRIMARY KEY, status TEXT, attempts INTEGER,
             pane TEXT, session TEXT, outcome TEXT, said TEXT, told TEXT) WITHOUT ROWID;
             INSERT INTO tasks (id, status) VALUES ('old', 'open');",
        )
        .expect("the old table");
        drop(db);
        let mut tasks = open(&dir.0).expect("open");
        assert_eq!(
            tasks.status("old").expect("status").as_deref(),
            Some("open")
        );
        tasks.add("new", &names(&["old"])).expect("add");
        drop(tasks);
        let tasks = open(&dir.0).expect("again");
        assert_eq!(tasks.standing().expect("rows")[0].id, "new");
    }
}
