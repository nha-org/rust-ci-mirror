//! Core's TaskStore on the op log, the one write path: a run's whole history replays and merges as any table does.
//! ponytail: its own file, since `facts` has a NOT NULL column the log refuses; one file once dash's model moves here.
//!
//! A claim is a row and a lock: the row says who is claimed, the lock that its run still lives. The system frees a lock when
//! its process ends, however it ends, so no pid is ever read back or mistaken for a later process's. The lock is the worktree's
//! as much as the row's, so its file sits beside that worktree: a second `--data` folder has its own rows, and still no second
//! run starts in that folder.
use crate::oplog::{OpError, Replica};
use dash_core::tasks::{
    Claim, Outcome, Queue, Queued, Run, Standing, TaskStore, over_cap, refused, waits_unmet,
};
use dash_core::{FieldWrite, Op, Value};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, status TEXT, attempts INTEGER,
  pane TEXT, session TEXT, outcome TEXT, said TEXT, told TEXT, waits TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS agents (id TEXT PRIMARY KEY, folder TEXT, state TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS listed (id TEXT PRIMARY KEY, title TEXT, pos INTEGER, since_ms INTEGER, waits TEXT) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS settings (id TEXT PRIMARY KEY, value INTEGER) WITHOUT ROWID;";
/// A folder from before a column (`waits`, `handed`, `rejudged`, `deployed`) keeps its rows: the column is added. Two processes
/// opening an old folder after a deploy can race here: only the column's absence afterwards is a failure.
fn add_column(db: &Connection, name: &str) -> Result<(), String> {
    let sql = "SELECT count(*) FROM pragma_table_info('tasks') WHERE name = ?1";
    let has = || {
        db.query_row(sql, [name], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())
    };
    if has()? == 0 {
        let added = db.execute_batch(&format!("ALTER TABLE tasks ADD COLUMN {name} TEXT"));
        if has()? == 0 {
            added.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
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
    for column in ["waits", "handed", "rejudged", "deployed"] {
        add_column(&db, column)?;
    }
    let locks = locks.to_path_buf();
    std::fs::create_dir_all(&locks).map_err(|e| e.to_string())?;
    let held = HashMap::new();
    let log = Replica::new(db, "dash").map_err(said)?;
    Ok(SqliteTasks { log, locks, held })
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
    let sql = "SELECT id, status, outcome, waits, attempts, pane, session, handed, rejudged FROM tasks ORDER BY id";
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
/// list may name a wait in words.
fn listed_in(db: &Connection) -> rusqlite::Result<Vec<(Option<i64>, Queued)>> {
    let sql = "SELECT id, title, pos, since_ms, waits FROM listed ORDER BY pos";
    let mut rows = db.prepare(sql)?;
    let rows = rows.query_map([], |r| {
        let waits: Option<String> = r.get(4)?;
        let task = Queued {
            id: r.get(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            reopened_ms: r.get(3)?,
            waits: waits
                .unwrap_or_default()
                .lines()
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

type Listed = (Vec<(Option<i64>, Queued)>, Option<i64>);

impl SqliteTasks {
    fn listed(&self) -> Result<Listed, String> {
        let read =
            |db: &Connection| -> rusqlite::Result<Listed> { Ok((listed_in(db)?, seats_in(db)?)) };
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
        let (before, seats) = self.listed()?;
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
            if old.is_none_or(|(p, _)| *p != Some(pos as i64)) {
                writes.push(int(id, "pos", Some(pos as i64)));
            }
            if old.is_none_or(|(_, t)| t.reopened_ms != task.reopened_ms) {
                writes.push(int(id, "since_ms", task.reopened_ms));
            }
            if old.is_none_or(|(_, t)| t.waits != task.waits) {
                writes.push(text(id, "waits", &task.waits.join("\n")));
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
        if !writes.is_empty() {
            self.log.write(&writes).map_err(said)?;
        }
        Ok(writes.len())
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

    /// The lock of `id`'s claim, when no run holds it.
    fn take_lock(&self, id: &str) -> Result<Option<File>, String> {
        if !is_id(id) {
            return Err(format!("{id}: not a task id"));
        }
        let path = self.locks.join(format!("rust-{id}.lock"));
        let file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
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

    /// The row, one `field: value` a line, as a command prints it.
    pub fn show(&self, id: &str) -> Result<Option<String>, String> {
        let fields = [
            "status", "attempts", "pane", "session", "outcome", "said", "waits", "rejudged",
            "deployed",
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

    /// Every row, the newest write of a run first; `told` and `deployed`, written after a merge, and `waits`, written by hand, are
    /// no run. A stamp holds its write's time above its low 16 bits (`Clock::tick`).
    pub fn rows(&self) -> Result<Vec<Row>, String> {
        let sql = "SELECT id, status, attempts, outcome, said,
            (SELECT max(stamp) FROM ops WHERE tbl = 'tasks' AND row_id = id AND field NOT IN ('told', 'deployed', 'waits')) AS last
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
        let written = self.log.write(&run_writes(id, status, run));
        self.held.remove(id);
        written.map(drop).map_err(said)
    }

    fn claimed(&self) -> Result<Vec<Claim>, String> {
        let sql = "SELECT id, pane, session, attempts FROM tasks WHERE status = 'claimed'";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([], |r| {
                Ok(Claim {
                    id: r.get(0)?,
                    pane: r.get(1)?,
                    session: r.get(2)?,
                    attempts: r.get(3)?,
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
        ];
        let written = self.log.write_if(free, &writes);
        let written = written.map_err(said)?;
        if let Some(line) = refusal {
            return Err(line);
        }
        written.ok_or(format!("{id}: not an open task"))?;
        self.held.insert(id.to_string(), lock);
        Ok(attempts)
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
            if self.take_lock(&claim.id)?.is_some() {
                crashed.push(claim);
            }
        }
        Ok(crashed)
    }

    fn free(&mut self, id: &str) -> Result<bool, String> {
        let Some(_lock) = self.take_lock(id)? else {
            return Ok(false);
        };
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
        let writes = [field(id, "handed", Some(head))];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn rejudged(&mut self, id: &str, words: &str) -> Result<(), String> {
        let writes = [field(id, "rejudged", Some(words))];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn deployed(&mut self, id: &str, words: &str) -> Result<(), String> {
        let writes = [field(id, "deployed", Some(words))];
        self.log.write(&writes).map(drop).map_err(said)
    }

    fn standing(&self) -> Result<Vec<Standing>, String> {
        self.log
            .read(standing_in)
            .map_err(said)?
            .map_err(|e| e.to_string())
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
        let (listed, seats) = self.listed()?;
        let seats = seats.ok_or("no sync of the list yet: its seats are not known")?;
        let tasks = listed
            .into_iter()
            .filter(|(p, _)| p.is_some())
            .map(|(_, t)| t)
            .collect();
        Ok(Queue {
            seats: seats as usize,
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
    use dash_core::tasks::{OPEN_CAP, Outcome};
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

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

    #[test]
    fn an_agent_stands_as_its_last_write_says_and_survives_a_reopen() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_task_goes_from_no_row_to_recorded_and_survives_a_reopen() {
        let _budget = TestBudget::new();
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
            "status: finished\nattempts: 1\npane: w1:p2\nsession: \noutcome: merged\nsaid: https://pr/7\nwaits: \nrejudged: \ndeployed: "
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

    /// A status write's event holds its own commit's fields alone: a later write in the same millisecond, an outcome with no
    /// status, never changes it (/code-review of rust-task-page-linear).
    #[test]
    fn a_later_write_in_the_same_millisecond_keeps_out_of_an_event() {
        let _budget = TestBudget::new();
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

    #[test]
    fn of_two_stores_on_one_file_only_one_claim_takes_the_task() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("tasks-claim");
        let mut first = open(&dir.0).expect("open");
        let mut second = open(&dir.0).expect("open again");
        assert!(first.claim("t1").is_ok());
        assert!(second.claim("t1").is_err());
    }

    #[test]
    fn an_id_is_no_path() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("tasks-id");
        let mut tasks = open(&dir.0).expect("open");
        for id in ["../x", "a/b", "", "a b"] {
            assert!(
                tasks.claim(id).is_err_and(|e| e.contains("not a task id")),
                "{id}"
            );
        }
    }

    #[test]
    fn a_crashed_run_s_claim_is_freed_a_living_one_s_is_not_and_it_is_claimed_again() {
        let _budget = TestBudget::new();
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
                attempts
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

    #[test]
    fn a_run_that_ended_while_its_claim_was_being_looked_at_stays_as_it_ended() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_merged_pr_is_untold_until_the_list_hears_of_it() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_released_run_can_be_claimed_again_and_says_how_it_ended() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_claim_saves_its_session_and_a_later_claim_forgets_it() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("tasks-session");
        let mut tasks = open(&dir.0).expect("open");
        tasks.claim("t1").expect("claim");
        tasks.opened("t1", "p1").expect("opened");
        tasks.adopted("t1", "s1").expect("adopted");
        let claim = &tasks.claims().expect("read")[0];
        assert_eq!(claim.session.as_deref(), Some("s1"));
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
    #[test]
    fn another_data_folder_cannot_claim_a_task_a_run_holds() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_wait_shows_on_its_claimed_row_until_the_session_is_told_to_go_on() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_task_reopened_after_it_finished_starts_over_and_one_older_stays_finished() {
        let _budget = TestBudget::new();
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
            "status: open\nattempts: 0\npane: \nsession: \noutcome: \nsaid: \nwaits: \nrejudged: \ndeployed: "
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

    #[test]
    fn the_head_a_task_was_handed_back_at_outlives_its_reopen() {
        let _budget = TestBudget::new();
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
    }

    #[test]
    fn show_names_the_merged_pr_an_ended_run_was_judged_again_at_and_its_verdict() {
        let _budget = TestBudget::new();
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
    }

    #[test]
    fn the_page_s_read_says_a_claim_no_run_holds_is_gone() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_task_names_what_it_waits_on_and_a_cycle_is_refused_at_write() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_claim_refuses_a_task_whose_wait_is_not_merged() {
        let _budget = TestBudget::new();
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

    #[test]
    fn closing_an_open_row_frees_its_place_under_the_cap() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_row_an_unfinished_task_waits_on_is_not_closed() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_hand_written_wait_is_no_run() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_row_with_no_status_yet_stands_as_open() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("tasks-no-status");
        let db = crate::connect(&dir.0.join("tasks.sqlite")).expect("connect");
        db.execute_batch(SCHEMA).expect("the table");
        db.execute_batch("INSERT INTO tasks (id, waits) VALUES ('half', '')")
            .expect("a split batch");
        drop(db);
        let tasks = open(&dir.0).expect("open");
        assert_eq!(tasks.standing().expect("rows")[0].status, "open");
    }

    #[test]
    fn a_51st_open_task_is_refused_with_a_line_saying_so() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_folder_from_before_waits_keeps_its_rows() {
        let _budget = TestBudget::new();
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
