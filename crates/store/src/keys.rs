//! Section 20b's key rows: a key names its secret, a name bin/secret holds, never its value; its spend is our count of its
//! runs, against the cap its row sets. In the run store's file, on the op log: a removal is a field write on `gone`.
use crate::oplog::{OpError, Replica};
use dash_core::tasks::Limit;
use dash_core::{FieldWrite, Value};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS keys (id TEXT PRIMARY KEY, secret TEXT, cap REAL, gone INTEGER) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS key_runs (id TEXT PRIMARY KEY, key TEXT, usd REAL) WITHOUT ROWID;";

/// A key's row as `dash agents` prints it. `cap` and `spent` in US dollars.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub id: String,
    pub secret: String,
    pub cap: f64,
    pub spent: f64,
}

impl Key {
    /// Its spend against its cap, in percent: a cap of 0, his default, leaves no room, so nothing is paid until he sets one.
    pub fn limit(&self) -> Limit {
        let week = if self.cap > 0.0 {
            100.0 * self.spent / self.cap
        } else {
            100.0
        };
        let (account, five_hours, five_hours_resets, week_resets, key) =
            (self.id.clone(), 0.0, 0, 0, true);
        Limit {
            account,
            five_hours,
            five_hours_resets,
            week,
            week_resets,
            key,
        }
    }
}

pub struct Keys {
    log: Replica,
}

pub fn open(dir: &Path) -> Result<Keys, String> {
    let db = crate::connect(&dir.join("tasks.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    let log = Replica::new(db, "dash").map_err(said)?;
    Ok(Keys { log })
}

fn said(e: OpError) -> String {
    format!("{e:?}")
}

fn write(table: &str, row: &str, field: &str, value: Value) -> FieldWrite {
    let (table, row, field) = (table.into(), row.into(), field.into());
    FieldWrite {
        table,
        row,
        field,
        value,
    }
}

impl Keys {
    /// The row of `id`, made or replaced. `secret` is a name the caller checked bin/secret holds: this only stores it.
    pub fn add(&mut self, id: &str, secret: &str, cap: f64) -> Result<(), String> {
        let writes = [
            write("keys", id, "secret", Value::Text(secret.into())),
            write("keys", id, "cap", Value::Real(cap)),
            write("keys", id, "gone", Value::Null),
        ];
        self.log.write(&writes).map(drop).map_err(said)
    }

    /// False when no row of `id` stands. Its runs' spend stays counted: a row added again starts where it left off.
    pub fn remove(&mut self, id: &str) -> Result<bool, String> {
        let sql = "SELECT 1 FROM keys WHERE id = ?1 AND gone IS NULL";
        let stands =
            |db: &Connection| Ok(db.query_row(sql, [id], |_| Ok(())).optional()?.is_some());
        let gone = [write("keys", id, "gone", Value::Int(1))];
        let written = self.log.write_if(stands, &gone).map_err(said)?;
        Ok(written.is_some())
    }

    /// One run's spend on `key`, under the run's own id: a second write of one run replaces its first.
    pub fn spend(&mut self, run: &str, key: &str, usd: f64) -> Result<(), String> {
        let writes = [
            write("key_runs", run, "key", Value::Text(key.into())),
            write("key_runs", run, "usd", Value::Real(usd)),
        ];
        self.log.write(&writes).map(drop).map_err(said)
    }

    /// Every standing row, by id.
    pub fn list(&self) -> Result<Vec<Key>, String> {
        let sql = "SELECT id, secret, coalesce(cap, 0),
            (SELECT coalesce(sum(usd), 0) FROM key_runs WHERE key_runs.key = keys.id)
            FROM keys WHERE gone IS NULL ORDER BY id";
        let rows = |db: &Connection| {
            let mut rows = db.prepare(sql)?;
            let rows = rows.query_map([], |r| {
                Ok(Key {
                    id: r.get(0)?,
                    secret: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    cap: r.get(2)?,
                    spent: r.get(3)?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_row_is_added_spent_on_removed_and_its_limit_is_spend_against_cap() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("store-keys");
        let mut keys = open(&dir.0).expect("open");
        keys.add("anthropic", "anthropic", 2.0).expect("add");
        keys.spend("r1", "anthropic", 0.25).expect("spend");
        keys.spend("r2", "anthropic", 0.25).expect("spend");
        keys.spend("r2", "anthropic", 0.5)
            .expect("a run's spend replaced");
        let listed = open(&dir.0).expect("reopen").list().expect("list");
        let key = Key {
            id: "anthropic".into(),
            secret: "anthropic".into(),
            cap: 2.0,
            spent: 0.75,
        };
        assert_eq!(listed, std::slice::from_ref(&key));
        let limit = key.limit();
        assert!((limit.week, limit.key) == (37.5, true) && limit.room());
        let uncapped = Key { cap: 0.0, ..key }.limit();
        assert!(!uncapped.room(), "no cap, no room");
        assert!(keys.remove("anthropic").expect("rm"));
        assert!(keys.list().expect("list").is_empty());
        assert!(!keys.remove("anthropic").expect("rm"), "gone already");
        keys.add("anthropic", "anthropic", 0.0).expect("again");
        assert_eq!(keys.list().expect("list")[0].spent, 0.75);
    }
}
