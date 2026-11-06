//! Core's TaskStore on the op log, the one write path: a run's whole history replays and merges as any table does.
//! ponytail: its own file, since `facts` has a NOT NULL column the log refuses; one file once dash's model moves here.
//!
//! A claim is a row and a lock: the row says who is claimed, the lock that its run still lives. The system frees a lock when
//! its process ends, however it ends, so no pid is ever read back or mistaken for a later process's.
use crate::oplog::{OpError, Replica};
use dash_core::tasks::{Freed, Outcome, Run, TaskStore};
use dash_core::{FieldWrite, Value};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, status TEXT, attempts INTEGER,
  pane TEXT, session TEXT, outcome TEXT, said TEXT, told TEXT) WITHOUT ROWID;";

pub struct SqliteTasks {
    log: Replica,
    locks: PathBuf,
    held: HashMap<String, File>,
}

pub fn open(dir: &Path) -> Result<SqliteTasks, String> {
    let db = crate::connect(&dir.join("tasks.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    let locks = dir.join("claims");
    std::fs::create_dir_all(&locks).map_err(|e| e.to_string())?;
    let held = HashMap::new();
    let log = Replica::new(db, "dash").map_err(said)?;
    Ok(SqliteTasks { log, locks, held })
}

fn said(e: OpError) -> String {
    format!("{e:?}")
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

fn run_writes(id: &str, status: &str, run: &Run) -> [FieldWrite; 5] {
    let (kind, words) = run.outcome.parts();
    [
        field(id, "status", Some(status)),
        field(id, "pane", run.pane.as_deref()),
        field(id, "session", run.session.as_deref()),
        field(id, "outcome", Some(kind)),
        field(id, "said", Some(words)),
    ]
}

impl SqliteTasks {
    /// The lock of `id`'s claim, when no run holds it. An id is a file's name here, and comes from outside.
    fn take_lock(&self, id: &str) -> Result<Option<File>, String> {
        let safe = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c));
        if !safe {
            return Err(format!("{id}: not a task id"));
        }
        let path = self.locks.join(id);
        let file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The row, one `field: value` a line, as a command prints it.
    pub fn show(&self, id: &str) -> Result<Option<String>, String> {
        let fields = ["status", "attempts", "pane", "session", "outcome", "said"];
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

    /// `status` set, this run's claim let go.
    fn finish(&mut self, id: &str, status: &str, run: &Run) -> Result<(), String> {
        let written = self.log.write(&run_writes(id, status, run));
        self.held.remove(id);
        written.map(drop).map_err(said)
    }

    fn claimed(&self) -> Result<Vec<Freed>, String> {
        let sql = "SELECT id, pane, attempts FROM tasks WHERE status = 'claimed'";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([], |r| {
                Ok(Freed {
                    id: r.get(0)?,
                    pane: r.get(1)?,
                    attempts: r.get(2)?,
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
        let free = |db: &Connection| {
            let row = db.query_row(sql, [id], |_| Ok(())).optional()?;
            Ok(row.is_none())
        };
        let attempts = self.attempts(id)? + 1;
        let writes = [
            field(id, "status", Some("claimed")),
            write(id, "attempts", Value::Int(attempts.into())),
            field(id, "pane", None),
        ];
        let written = self.log.write_if(free, &writes);
        written
            .map_err(said)?
            .ok_or(format!("{id}: not an open task"))?;
        self.held.insert(id.to_string(), lock);
        Ok(attempts)
    }

    fn opened(&mut self, id: &str, pane: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "pane", Some(pane))])
            .map(drop)
            .map_err(said)
    }

    fn record(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.finish(id, "finished", run)
    }

    fn release(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.finish(id, "open", run)
    }

    fn free_crashed(&mut self) -> Result<Vec<Freed>, String> {
        let mut freed = vec![];
        for claim in self.claimed()? {
            // Taken here, then dropped: only a run that lives holds it.
            let Some(_lock) = self.take_lock(&claim.id)? else {
                continue;
            };
            let writes = [
                field(&claim.id, "status", Some("open")),
                field(&claim.id, "outcome", Some("failed")),
                field(&claim.id, "said", Some("its run crashed")),
            ];
            // Still claimed, now its lock is held: a run that ended since the read above has said how, and that stands.
            let sql = "SELECT 1 FROM tasks WHERE id = ?1 AND status = 'claimed'";
            let id = claim.id.as_str();
            let still = |db: &Connection| {
                let row = db.query_row(sql, [id], |_| Ok(())).optional()?;
                Ok(row.is_some())
            };
            let written = self.log.write_if(still, &writes);
            if written.map_err(said)?.is_none() {
                continue;
            }
            freed.push(claim);
        }
        Ok(freed)
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

    fn claims(&self) -> Result<Vec<Option<String>>, String> {
        Ok(self.claimed()?.into_iter().map(|c| c.pane).collect())
    }

    fn told(&mut self, id: &str) -> Result<(), String> {
        self.log
            .write(&[field(id, "told", Some("yes"))])
            .map(drop)
            .map_err(said)
    }

    fn status(&self, id: &str) -> Result<Option<String>, String> {
        let row = |db: &Connection| {
            let sql = "SELECT status FROM tasks WHERE id = ?1";
            db.query_row(sql, [id], |r| r.get(0)).optional()
        };
        self.log.read(row).map_err(said)?.map_err(|e| e.to_string())
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
    use dash_core::tasks::Outcome;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    fn run(outcome: Outcome) -> Run {
        let (pane, session) = (Some("w1:p2".to_string()), None);
        Run {
            pane,
            session,
            outcome,
        }
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
            "status: finished\nattempts: 1\npane: w1:p2\nsession: \noutcome: merged\nsaid: https://pr/7"
        );
        assert!(
            tasks
                .claim("t1")
                .is_err_and(|e| e.contains("not an open task"))
        );
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
        let (id, attempts) = ("gone".to_string(), 1);
        assert_eq!(
            watcher.free_crashed().expect("freed"),
            [Freed { id, pane, attempts }]
        );
        assert_eq!(
            watcher.status("gone").expect("status").as_deref(),
            Some("open")
        );
        assert_eq!(
            watcher.status("here").expect("status").as_deref(),
            Some("claimed")
        );
        assert_eq!(watcher.free_crashed().expect("none left"), []);
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
        assert_eq!(watcher.free_crashed().expect("none"), []);
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
        assert_eq!(tasks.claims().expect("read"), [None]);
        tasks.opened("t2", "p2").expect("opened");
        assert_eq!(tasks.claims().expect("read"), [Some("p2".to_string())]);
        tasks
            .release("t2", &run(Outcome::Failed("no pane".into())))
            .expect("release");
        assert_eq!(tasks.claims().expect("read"), Vec::<Option<String>>::new());
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
}
