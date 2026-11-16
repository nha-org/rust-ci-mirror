//! A plugin's own SQLite file in plain SQL (ARCHITECTURE.md section 3, decision 8): `open` applies its schema and the
//! `changesets` table; `transact` runs its SQL in one transaction and, in that same commit, keeps the transaction's
//! changeset (SQLite's session extension) there: the file's one event stream, which the views read with `changes_after`,
//! from any process that wrote it.
use crate::connect;
use dash_core::{FieldWrite, Host, Later, Op, OpLog, Transport, Value, WallClock};
use rusqlite::session::{ChangesetItem, ChangesetIter, Session};
use rusqlite::types::ValueRef;
use rusqlite::{
    Connection, OptionalExtension, fallible_streaming_iterator::FallibleStreamingIterator,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Each commit's changeset, in commit order.
/// ponytail: never trimmed, so a start replays every one (drafts: a few hundred records, kilobytes each); folding the oldest
/// into one baseline changeset past a count is the upgrade once a replay shows in a start's time. A table of the host's, never a plugin's: the session records none of its rows.
pub const CHANGESETS: &str = "changesets";
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS changesets (seq INTEGER PRIMARY KEY, at INTEGER NOT NULL, changeset BLOB NOT NULL)";

/// The plugin file `path`, each of `schema` applied. A file opened for the first time with rows already in it (one the op log
/// wrote) records them as its first changeset, so the views read every row from the stream alone.
pub fn open(path: &Path, schema: &[&str], now_ms: i64) -> Result<Connection, String> {
    let db = connect(path).map_err(|e| e.to_string())?;
    for s in schema {
        db.execute_batch(s).map_err(|e| e.to_string())?;
    }
    if !kept_changesets(&db)? {
        baseline(&db, &tables(schema), now_ms)?;
    }
    Ok(db)
}

/// Whether the file keeps its changesets already.
fn kept_changesets(db: &Connection) -> Result<bool, String> {
    let exists = "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'changesets'";
    let found = db.query_row(exists, [], |_| Ok(())).optional();
    found.map(|f| f.is_some()).map_err(|e| e.to_string())
}

/// The tables `schema` creates, by name.
fn tables(schema: &[&str]) -> Vec<String> {
    let statements = schema.iter().flat_map(|s| s.split(';'));
    let names = statements.filter_map(|s| {
        let w: Vec<&str> = s.split_whitespace().take(6).collect();
        let created = w.len() > 2
            && w[0].eq_ignore_ascii_case("create")
            && w[1].eq_ignore_ascii_case("table");
        let name = if w.get(2)?.eq_ignore_ascii_case("if") {
            w.get(5)?
        } else {
            w[2]
        };
        created.then(|| name.split('(').next().unwrap_or(name).to_string())
    });
    names.collect()
}

/// Every row of `tables` as one insert each, written by a session's diff against an empty copy of each table (attached
/// outside the transaction, as SQLite asks): the rows the file held before it kept changesets.
fn baseline(db: &Connection, tables: &[String], now_ms: i64) -> Result<(), String> {
    let e = |e: rusqlite::Error| e.to_string();
    db.execute_batch("ATTACH ':memory:' AS empty").map_err(e)?;
    let made = tables.iter().try_for_each(|t| {
        let schema = "SELECT sql FROM main.sqlite_schema WHERE type = 'table' AND name = ?1";
        let sql: String = db.query_row(schema, [t], |r| r.get(0)).map_err(e)?;
        let body = sql.split_once('(').map_or("", |(_, b)| b);
        db.execute_batch(&format!("CREATE TABLE empty.\"{t}\" ({body}"))
            .map_err(e)
    });
    let kept = made.and_then(|()| {
        transact(db, now_ms, &mut |db| {
            // checked again under the write lock: a second process opening the file at once records no second baseline
            if kept_changesets(db)? {
                return Ok(());
            }
            db.execute_batch(SCHEMA).map_err(e)?;
            let mut session = Session::new(db).map_err(e)?;
            for t in tables {
                session.attach(Some(t.as_str())).map_err(e)?;
                session.diff::<&str, &str>("empty", t.as_str()).map_err(e)?;
            }
            keep(db, &mut session, now_ms)
        })
    });
    db.execute_batch("DETACH empty").map_err(e)?;
    kept
}

/// What `session` recorded, kept as the next changeset; nothing when it recorded nothing.
fn keep(db: &Connection, session: &mut Session, at: i64) -> Result<(), String> {
    if session.is_empty() {
        return Ok(());
    }
    let mut changeset = Vec::new();
    session
        .changeset_strm(&mut changeset)
        .map_err(|e| e.to_string())?;
    let insert = "INSERT INTO changesets (at, changeset) VALUES (?1, ?2)";
    db.execute(insert, (at, changeset))
        .map(drop)
        .map_err(|e| e.to_string())
}

/// `run`'s SQL on `db` in one write transaction, its changeset kept in the same commit at `now_ms`, the host's clock; rolled
/// back whole on an `Err`.
pub fn transact(
    db: &Connection,
    now_ms: i64,
    run: &mut dyn FnMut(&Connection) -> Result<(), String>,
) -> Result<(), String> {
    db.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let done = (|| {
        let mut session = Session::new(db).map_err(|e| e.to_string())?;
        session.table_filter(Some(|t: &str| t != CHANGESETS));
        session.attach(None::<&str>).map_err(|e| e.to_string())?;
        run(db)?;
        keep(db, &mut session, now_ms)
    })();
    let ended = match done {
        Ok(()) => db.execute_batch("COMMIT").map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    // a COMMIT that failed leaves the transaction open: rolled back too, so the connection never keeps the write lock
    if ended.is_err() && !db.is_autocommit() {
        db.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
    }
    ended
}

/// The changesets after `after` as ops, `limit` at most, for the views' cursor: each op at `(seq, its place in the
/// changeset)`, a field's new value, a delete its row's `gone`, an insert its `gone` 0. An insert or update writes each
/// column but the key.
/// ponytail: decodes the changeset at the cursor again on each read (the tail and `Cursor::replaced`, about twice a second
/// with a page following); a whole-table changeset there costs a few ms a read. A memo of the last changeset's ops, keyed
/// by its seq, in `changes` if that shows in the views' time.
pub fn changes_after(
    db: &Connection,
    after: Option<(i64, &str)>,
    limit: i64,
) -> Result<Vec<Op>, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let (from, past) = after.map_or((0, None), |(s, w)| (s, Some(w.to_string())));
    let mut read = db
        .prepare("SELECT seq, changeset FROM changesets WHERE seq >= ?1 ORDER BY seq")
        .map_err(e)?;
    let mut rows = read.query([from]).map_err(e)?;
    let (mut ops, mut columns) = (Vec::new(), HashMap::new());
    while let Some(row) = rows.next().map_err(e)? {
        let (seq, blob): (i64, Vec<u8>) = (row.get(0).map_err(e)?, row.get(1).map_err(e)?);
        let mut input: &[u8] = &blob;
        let input: &mut dyn std::io::Read = &mut input;
        let mut items = ChangesetIter::start_strm(&input).map_err(e)?;
        let mut place = 0usize;
        while let Some(item) = items.next().map_err(e)? {
            for write in writes(db, &mut columns, item)? {
                let writer = format!("{place:08}");
                place += 1;
                let passed = seq == from && past.as_deref().is_some_and(|p| writer.as_str() <= p);
                if !passed {
                    ops.push(Op {
                        stamp: seq,
                        writer,
                        write,
                    });
                }
            }
            if ops.len() as i64 >= limit {
                ops.truncate(limit as usize);
                return Ok(ops);
            }
        }
    }
    Ok(ops)
}

/// The changesets of the plugin file `path` as the views read a log, the file opened at the first read and again after a
/// read that failed: each commit of the plugin, from this process or a command's, reaches its views at their next read.
/// ponytail: a baseline its first open records is stamped at 0, the feed holding no clock.
pub fn changes(path: PathBuf, schema: &'static [&'static str]) -> Box<Changes> {
    let mut db: Option<Connection> = None;
    Box::new(move |after, n| {
        let d = match db.take() {
            Some(d) => d,
            None => open(&path, schema, 0)?,
        };
        let ops = changes_after(&d, after, n);
        if ops.is_ok() {
            db = Some(d);
        }
        ops
    })
}

/// The views' read of a log (`dash_views::Source`).
pub type Changes = dyn FnMut(Option<(i64, &str)>, i64) -> Result<Vec<Op>, String> + Send;

/// One changed row's field writes.
fn writes(
    db: &Connection,
    columns: &mut HashMap<String, Vec<String>>,
    item: &ChangesetItem,
) -> Result<Vec<FieldWrite>, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let op = item.op().map_err(e)?;
    let table = op.table_name().to_string();
    let n = op.number_of_columns() as usize;
    let deleted = matches!(op.code(), rusqlite::hooks::Action::SQLITE_DELETE);
    let inserted = matches!(op.code(), rusqlite::hooks::Action::SQLITE_INSERT);
    if !columns.contains_key(&table) {
        let mut info = db
            .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
            .map_err(e)?;
        let names = info.query_map([&table], |r| r.get(0)).map_err(e)?;
        columns.insert(table.clone(), names.collect::<Result<_, _>>().map_err(e)?);
    }
    let names = &columns[&table];
    let pk = item.pk().map_err(e)?;
    // an update's key is among its old values only: a key never changes, so its new value is left out (/code-review)
    let side = |c: usize| {
        if inserted {
            item.new_value(c)
        } else {
            item.old_value(c)
        }
    };
    let key: Vec<String> = (0..n)
        .filter(|&c| pk[c] != 0)
        .map(|c| side(c).map(text))
        .collect::<Result<_, _>>()
        .map_err(e)?;
    let row = key.join(&dash_core::KEY_SEPARATOR.to_string());
    let field = |field: &str, value| FieldWrite {
        table: table.clone(),
        row: row.clone(),
        field: field.into(),
        value,
    };
    if deleted {
        return Ok(vec![field("gone", Value::Int(1))]);
    }
    // an insert brings back a row a delete left `gone` (/code-review)
    let mut out: Vec<FieldWrite> = inserted
        .then(|| field("gone", Value::Int(0)))
        .into_iter()
        .collect();
    for c in (0..n).filter(|&c| pk[c] == 0) {
        // an update's untouched column has no new value
        if let Ok(v) = item.new_value(c) {
            let name = names
                .get(c)
                .ok_or(format!("{table}: no column {c} in its schema now"))?;
            out.push(field(name, value(v)));
        }
    }
    Ok(out)
}

fn text(v: ValueRef) -> String {
    match value(v) {
        Value::Text(t) => t,
        Value::Int(i) => i.to_string(),
        Value::Real(r) => r.to_string(),
        Value::Blob(b) => String::from_utf8_lossy(&b).into_owned(),
        Value::Null => String::new(),
    }
}

fn value(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(r) => Value::Real(r),
        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
    }
}

/// The process's host with what one request adds: a page's views, which the server holds. Every other answer is the
/// process's host's, its data folder included (the one `--data` names, read once in `main`: task untangle-one-host); no
/// plugin reads a path or a view of its own.
pub struct Handed<'a> {
    pub host: &'a dyn Host,
    pub view: Option<&'a (dyn dash_core::View + Sync)>,
}

impl WallClock for Handed<'_> {
    fn now_ms(&self) -> i64 {
        self.host.now_ms()
    }
}

impl Host for Handed<'_> {
    fn repo(&self) -> PathBuf {
        self.host.repo()
    }
    fn data(&self) -> PathBuf {
        self.host.data()
    }
    // a default left unforwarded answers None: every page's switch read then saw each plugin on (/code-review of PR 1667)
    fn switched(&self) -> Option<PathBuf> {
        self.host.switched()
    }
    fn home(&self) -> Result<PathBuf, String> {
        self.host.home()
    }
    fn arm(&self, dir: &Path, command: &str) {
        self.host.arm(dir, command)
    }
    fn open(&self, file: &str, schema: &[&str]) -> Result<Connection, String> {
        self.host.open(file, schema)
    }
    fn transact(
        &self,
        db: &Connection,
        run: &mut dyn FnMut(&Connection) -> Result<(), String>,
    ) -> Result<(), String> {
        self.host.transact(db, run)
    }
    fn view(&self) -> Result<&dyn dash_core::View, String> {
        match self.view {
            Some(view) => Ok(view),
            None => self.host.view(),
        }
    }
    fn log(&self, dir: &Path, file: &str, schema: &[&str]) -> Result<Box<dyn OpLog>, String> {
        self.host.log(dir, file, schema)
    }
    fn signed_in(&self, who: &str) -> Result<Transport, String> {
        self.host.signed_in(who)
    }
    fn after(&self, ms: i64, run: Later) -> Result<(), String> {
        self.host.after(ms, run)
    }
    fn call(&self, tool: &str, arguments: &str) -> Result<String, String> {
        self.host.call(tool, arguments)
    }
    fn forge(&self, args: &[&str]) -> Result<String, String> {
        self.host.forge(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    const T: &str =
        "CREATE TABLE IF NOT EXISTS t (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";

    struct Switched;
    impl WallClock for Switched {
        fn now_ms(&self) -> i64 {
            0
        }
    }
    impl Host for Switched {
        fn repo(&self) -> PathBuf {
            PathBuf::new()
        }
        fn data(&self) -> PathBuf {
            PathBuf::new()
        }
        fn switched(&self) -> Option<PathBuf> {
            Some("kept".into())
        }
        fn home(&self) -> Result<PathBuf, String> {
            Err("none".into())
        }
        fn arm(&self, _: &Path, _: &str) {}
        fn log(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
            Err("none".into())
        }
    }

    #[dash_test]
    fn a_handed_host_keeps_the_switches_of_the_host_it_wraps() {
        let handed = Handed {
            host: &Switched,
            view: None,
        };
        assert_eq!(handed.switched(), Some("kept".into()));
    }

    fn fields(ops: &[Op]) -> Vec<(i64, &str, &str, &Value)> {
        ops.iter()
            .map(|o| {
                (
                    o.stamp,
                    o.write.row.as_str(),
                    o.write.field.as_str(),
                    &o.write.value,
                )
            })
            .collect()
    }

    /// Done-when 2 of drafts-genuinely-good: each commit's changeset is kept with it and read back as ops, a rolled back
    /// transaction keeps none, a delete is its row's `gone`, and a file that held rows before records them first.
    #[dash_test]
    fn each_commit_keeps_its_changeset_and_the_rows_before_it_come_first() {
        let dir = TestDir::new("plugin-changesets");
        let file = dir.0.join("p.sqlite");
        let old = Connection::open(&file).expect("old file");
        old.execute_batch(T).expect("table");
        old.execute("INSERT INTO t VALUES ('a', 'before')", [])
            .expect("row");
        drop(old);
        let db = open(&file, &[T], 1).expect("opened");
        let put = |id: &'static str, r: &'static str| {
            move |db: &Connection| {
                db.execute("INSERT OR REPLACE INTO t VALUES (?1, ?2)", [id, r])
                    .map(drop)
                    .map_err(|e| e.to_string())
            }
        };
        transact(&db, 2, &mut put("b", "one")).expect("b");
        let failed = transact(&db, 3, &mut |db| {
            put("c", "never")(db)?;
            Err("refused".into())
        });
        assert_eq!(failed, Err("refused".into()));
        transact(&db, 4, &mut |db| {
            db.execute("DELETE FROM t WHERE id = 'a'", [])
                .map(drop)
                .map_err(|e| e.to_string())
        })
        .expect("delete");
        // a second write of a row is an update: its key from the old values, its new record the op (/code-review)
        let upsert = "INSERT INTO t VALUES ('b', 'two') ON CONFLICT (id) DO UPDATE SET record = excluded.record";
        transact(&db, 5, &mut |db| {
            db.execute(upsert, []).map(drop).map_err(|e| e.to_string())
        })
        .expect("update");
        // a row deleted then written again is back: its insert says it is no longer gone (/code-review)
        transact(&db, 6, &mut put("a", "again")).expect("a again");
        let ops = changes_after(&db, None, 100).expect("read");
        let (text, back) = (|t: &str| Value::Text(t.into()), Value::Int(0));
        assert_eq!(
            fields(&ops),
            vec![
                (1, "a", "gone", &back),
                (1, "a", "record", &text("before")),
                (2, "b", "gone", &back),
                (2, "b", "record", &text("one")),
                (3, "a", "gone", &Value::Int(1)),
                (4, "b", "record", &text("two")),
                (5, "a", "gone", &back),
                (5, "a", "record", &text("again"))
            ]
        );
        // a cursor's read after an op, one at a time, never repeats nor skips one
        let (mut after, mut one_by_one) = (None::<(i64, String)>, vec![]);
        loop {
            let page =
                changes_after(&db, after.as_ref().map(|(s, w)| (*s, w.as_str())), 1).expect("page");
            let Some(op) = page.last() else { break };
            after = Some((op.stamp, op.writer.clone()));
            one_by_one.extend(page);
        }
        assert_eq!(one_by_one, ops);
        // opened again, the baseline is not recorded twice
        drop(db);
        let db = open(&file, &[T], 5).expect("again");
        assert_eq!(changes_after(&db, None, 100).expect("read"), ops);
        assert_eq!(
            tables(&[T, "CREATE TABLE u(id PRIMARY KEY)"]),
            vec!["t", "u"]
        );
    }
}
