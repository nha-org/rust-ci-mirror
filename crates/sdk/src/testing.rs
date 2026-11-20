//! What a plugin's tests take, behind `features = ["testing"]` (`memory-log` without the meter) so no shipped build links it (ARCHITECTURE.md section 7
//! gotcha 8): a stand-in Host, the op log in memory, and `dash_test` with the meter it expands to.
use crate::host::{Connection, Host};
use crate::ops::{FieldWrite, Op, OpLog, WallClock};
use std::path::{Path, PathBuf};

/// The test attribute and its crate: `use dash_sdk::{dash_meter, dash_test};` in a test module, and its Cargo.toml lists
/// dash-sdk alone.
#[cfg(any(test, feature = "testing"))]
pub use dash_meter::{self, dash_test};

/// A test's host: its clock, repository and data folder as set, no home, each database its file in that folder (a test
/// sets a folder of its own before it opens one), and each op log in memory. What it
/// does not stand in for (a sign-in, a tool, a later run) says so, the `Host` defaults.
pub struct StandIn {
    pub now_ms: i64,
    pub repo: PathBuf,
    pub data: PathBuf,
}

impl Default for StandIn {
    fn default() -> Self {
        Self {
            now_ms: 0,
            repo: "/repo".into(),
            data: "/data".into(),
        }
    }
}

impl WallClock for StandIn {
    fn now_ms(&self) -> i64 {
        self.now_ms
    }
}

impl Host for StandIn {
    fn repo(&self) -> PathBuf {
        self.repo.clone()
    }
    fn data(&self) -> PathBuf {
        self.data.clone()
    }
    /// A test keeps no switches: every plugin on.
    fn switched(&self) -> Option<PathBuf> {
        None
    }
    fn home(&self) -> Result<PathBuf, String> {
        Err("HOME: unset".into())
    }
    fn arm(&self, _: &Path, _: &str) {}
    /// `file` under its data folder, a test's own: each `open` sees the last one's commits.
    fn open(&self, file: &str, schema: &[&str]) -> Result<Connection, String> {
        let db = Connection::open(self.data.join(file)).map_err(|e| e.to_string())?;
        for s in schema {
            db.execute_batch(s).map_err(|e| e.to_string())?;
        }
        Ok(db)
    }
    /// A plain transaction: it records no changeset (the store's host does, `dash_store::plugin`).
    fn transact(
        &self,
        db: &Connection,
        run: &mut dyn FnMut(&Connection) -> Result<(), String>,
    ) -> Result<(), String> {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        run(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    fn log(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Ok(Box::new(MemoryLog::default()))
    }
}

/// The op log in memory, for tests: its stamps count up, as one replica's do. `.1`: every write refused, as a full disk would.
#[derive(Default)]
pub struct MemoryLog(pub Vec<Op>, pub bool);

impl OpLog for MemoryLog {
    fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String> {
        if self.1 {
            return Err("disk full".into());
        }
        let ops: Vec<Op> = writes
            .iter()
            .map(|w| Op {
                stamp: self.0.len() as i64 + 1,
                writer: "t".into(),
                write: w.clone(),
            })
            .collect();
        self.0.extend(ops.iter().cloned());
        Ok(ops)
    }
    fn apply(&mut self, _ops: &[Op]) -> Result<usize, String> {
        unreachable!("a test log takes no remote ops")
    }
    fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String> {
        let from = after.map_or(0, |(s, _)| s);
        let ops = self.0.iter().filter(|o| o.stamp > from);
        Ok(ops.take(limit as usize).cloned().collect())
    }
    fn state_hash(&self) -> Result<u64, String> {
        unreachable!("a test log has no tables")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::Value;
    use dash_meter::dash_test;

    /// Done-when 1 of untangle-sdk-crate: the stand-in Host hands its clock and folders, refuses what it does not stand in
    /// for, and opens a database that keeps each write as an op, its stamps counting up.
    #[dash_test]
    fn the_stand_in_host_keeps_writes_in_memory() {
        let host = StandIn {
            now_ms: 7,
            ..StandIn::default()
        };
        assert_eq!(
            (host.now_ms(), host.repo(), host.data()),
            (7, "/repo".into(), "/data".into())
        );
        assert_eq!(host.home(), Err("HOME: unset".into()));
        assert_eq!(
            host.call("x/y", "{}"),
            Err("this host calls no tool: x/y".into())
        );
        let mut db = host
            .log(Path::new("/d"), "t.sqlite", &[])
            .expect("in memory");
        let w = FieldWrite {
            table: "t".into(),
            row: "1".into(),
            field: "f".into(),
            value: Value::Int(3),
        };
        let first = db.write(std::slice::from_ref(&w)).expect("written");
        let second = db.write(std::slice::from_ref(&w)).expect("written");
        assert_eq!((first[0].stamp, second[0].stamp), (1, 2));
        assert_eq!(db.ops_after(Some((1, "t")), 10).expect("read"), second);
        let mut full = MemoryLog(vec![], true);
        assert_eq!(full.write(&[w]), Err("disk full".into()));
    }

    /// Done-when 1 of drafts-genuinely-good: a plugin's file is plain SQL, each open seeing the last commit, and a transaction
    /// that answers an error leaves nothing.
    #[dash_test]
    fn a_plugin_s_file_is_plain_sql_and_a_failed_transaction_leaves_nothing() {
        let dir = dash_meter::TestDir::new("stand-in-open");
        let host = StandIn {
            data: dir.0.clone(),
            ..StandIn::default()
        };
        let schema = ["CREATE TABLE IF NOT EXISTS t (id TEXT PRIMARY KEY, n INTEGER)"];
        let db = host.open("p.sqlite", &schema).expect("opened");
        let put = |db: &Connection, n: i64| {
            db.execute("INSERT OR REPLACE INTO t VALUES ('a', ?1)", [n])
                .map(drop)
        };
        host.transact(&db, &mut |db| put(db, 1).map_err(|e| e.to_string()))
            .expect("kept");
        let failed = host.transact(&db, &mut |db| {
            put(db, 2).map_err(|e| e.to_string())?;
            Err("refused".into())
        });
        assert_eq!(failed, Err("refused".into()));
        let again = host.open("p.sqlite", &schema).expect("again");
        let n: i64 = again
            .query_row("SELECT n FROM t WHERE id = 'a'", [], |r| r.get(0))
            .expect("row");
        assert_eq!(n, 1);
    }
}
