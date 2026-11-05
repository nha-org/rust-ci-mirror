//! Section 14's op log, the one write path of section 3: a write stamps one op per field and applies it the way a replay
//! or a merge applies it, so all three end the same. A field's last write wins by `(stamp, writer)`, read from the log
//! itself. A delete is a field write too, on the table's own `gone` column. Replicas converge on any table; their vacuumed
//! files match byte for byte only on WITHOUT ROWID tables, since a rowid records arrival order.
//!
//! ponytail: every op stays in the log, 2.8 times the tables on dash's data (section 14); compaction once sync says which
//! ops every replica holds.
//! ponytail: a far-future stamp wins its field for good and drags every clock with it, and at MAX_STAMP this replica's own
//! writes fail. A replica cannot refuse it alone without parting from the others, so the sync phase refuses it where ops
//! enter, by the server's clock.
//! ponytail: `apply` is all or nothing and refuses a table or field its own schema lacks, so a replica on an older build
//! refuses every batch holding one op of a newer column until it upgrades; the sync phase sends the schema version with its ops.
//! ponytail: a `COLLATE NOCASE` key would make two row strings one row; no table has one, none is checked for.
//! ponytail: the schema is judged at open only, so a migration reopens the replica; `read` stops this connection changing
//! it, not another one.
use dash_core::{Clock, FieldWrite, Fnv, KEY_SEPARATOR, MAX_STAMP, Op, OpLog, Value, WallClock};
use rusqlite::types::{ToSqlOutput, Value as Sql, ValueRef};
use rusqlite::{Connection, OptionalExtension, ToSql, TransactionBehavior, params};
use std::collections::HashMap;
use std::path::Path;

/// WITHOUT ROWID so the log sorts by stamp in every replica, whatever order its ops came in.
const LOG_TABLE: &str = "ops (
    stamp INTEGER NOT NULL, writer TEXT NOT NULL, tbl TEXT NOT NULL, row_id TEXT NOT NULL, field TEXT NOT NULL, value,
    PRIMARY KEY (stamp, writer)) WITHOUT ROWID";
const LOG_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS ops_by_field ON ops (tbl, row_id, field, stamp, writer);";

#[derive(Debug)]
pub enum OpError {
    Sql(rusqlite::Error),
    /// A table the log cannot govern, or an op that names what no replica could apply the same way.
    Refused(String),
}

impl From<rusqlite::Error> for OpError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Sql(e) => write!(f, "{e}"),
            Self::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for OpError {}

fn refused<T>(why: String) -> Result<T, OpError> {
    Err(OpError::Refused(why))
}

struct KeyColumn {
    quoted: String,
    integer: bool,
}

struct Table {
    key: Vec<KeyColumn>,
    /// Made once for each field: a replay runs its upsert millions of times.
    fields: HashMap<String, Field>,
}

struct Field {
    upsert: String,
    read: String,
    not_null: bool,
}

pub fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The words of a table's DDL, upper-cased, outside its quoted names, strings and comments: a column called "checks" is no
/// CHECK, and sqlite_schema keeps comments as typed.
fn ddl_words(ddl: &str) -> Vec<String> {
    let (mut words, mut word, mut quoted) = (vec![], String::new(), None);
    let mut chars = ddl.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (Some(end), _) if c == end => quoted = None,
            (Some(_), _) => {}
            (None, '-') if chars.peek() == Some(&'-') => {
                chars.by_ref().find(|c| *c == '\n');
            }
            (None, '/') if chars.peek() == Some(&'*') => {
                chars.next();
                let mut star = false;
                chars.by_ref().find(|c| {
                    let closes = star && *c == '/';
                    star = *c == '*';
                    closes
                });
            }
            (None, '"' | '\'' | '`') => quoted = Some(c),
            (None, '[') => quoted = Some(']'),
            (None, _) if c.is_alphanumeric() || c == '_' => {
                word.push(c.to_ascii_uppercase());
                continue;
            }
            _ => {}
        }
        if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    words.push(word);
    words
}

/// A default as pragma_table_info gives it, its outer parentheses gone: a number, one string, NULL, TRUE or FALSE. Anything
/// else, CURRENT_TIMESTAMP or a call, may differ from one replica to the next.
fn is_constant(default: &str) -> bool {
    let string = default.len() >= 2 && default.starts_with('\'') && default.ends_with('\'');
    default.parse::<f64>().is_ok()
        || (string
            && !default[1..default.len() - 1]
                .replace("\'\'", "")
                .contains('\''))
        || ["NULL", "TRUE", "FALSE"].contains(&default.to_ascii_uppercase().as_str())
}

/// The tables a log can govern, and what is wrong with the others, read from the database. One field arrives alone on a
/// replica that has never seen its row, the same ops must apply on every replica, and nothing but an op may change a row.
/// So a table that cannot take that is refused here, not at its first merge.
fn read_schema(db: &Connection) -> Result<(HashMap<String, Table>, Vec<String>), OpError> {
    let names: Vec<(String, String, bool)> = db
        .prepare(
            "SELECT l.name, m.sql, l.strict FROM pragma_table_list l JOIN sqlite_schema m ON m.name = l.name
             WHERE l.schema = 'main' AND l.type = 'table' AND l.name NOT GLOB 'sqlite_*'",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let (mut tables, mut wrong) = (HashMap::new(), vec![]);
    for (name, ddl, strict) in names {
        // Table names ignore case, so « OPS » would stand in for the log.
        if name.eq_ignore_ascii_case("ops") {
            if ddl != format!("CREATE TABLE {LOG_TABLE}") {
                wrong.push(format!("{name}: this name is the log's"));
            }
            continue;
        }
        let before = wrong.len();
        if strict {
            wrong.push(format!(
                "{name}: STRICT refuses a value only when its upsert runs, so arrival order would decide"
            ));
        }
        let (mut key, mut fields) = (vec![], vec![]);
        let mut columns = db
            .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)")?;
        let mut rows = columns.query([&name])?;
        while let Some(r) = rows.next()? {
            let (column, kind, not_null, default, pk): (String, String, bool, Option<String>, i64) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
            if pk > 0 {
                let integer = kind.eq_ignore_ascii_case("INTEGER");
                if !integer && !kind.eq_ignore_ascii_case("TEXT") {
                    wrong.push(format!("{name}.{column}: a key is INTEGER or TEXT"));
                }
                let quoted = quote(&column);
                key.push((pk, KeyColumn { quoted, integer }));
                continue;
            }
            match default {
                None if not_null => wrong.push(format!(
                    "{name}.{column}: NOT NULL with no default fails when another field arrives first"
                )),
                // Each replica would fill it at its own arrival, with no op to reconcile them.
                Some(d) if !is_constant(&d) => {
                    wrong.push(format!("{name}.{column}: a default that varies"));
                }
                _ => {}
            }
            fields.push((column, not_null));
        }
        if key.is_empty() || fields.is_empty() {
            wrong.push(format!("{name}: an op needs a primary key and a field"));
        }
        let unique: i64 = db.query_row(
            "SELECT count(*) FROM pragma_index_list(?1) WHERE \"unique\" = 1 AND origin != 'pk'",
            [&name],
            |r| r.get(0),
        )?;
        if unique > 0 {
            wrong.push(format!(
                "{name}: a unique index fails a merge that its own writes passed"
            ));
        }
        let triggers: i64 = db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1 COLLATE NOCASE",
            [&name],
            |r| r.get(0),
        )?;
        if triggers > 0 {
            wrong.push(format!("{name}: a trigger writes what no op records"));
        }
        let words = ddl_words(&ddl);
        if words.iter().any(|w| w == "CHECK") {
            wrong.push(format!(
                "{name}: a CHECK across fields fails a merge that its own writes passed"
            ));
        }
        if words.iter().any(|w| w == "AUTOINCREMENT") {
            wrong.push(format!(
                "{name}: AUTOINCREMENT mints the same id on two replicas"
            ));
        }
        if wrong.len() > before {
            continue;
        }
        key.sort_by_key(|(pk, _)| *pk);
        let key: Vec<KeyColumn> = key.into_iter().map(|(_, column)| column).collect();
        let key_list = key
            .iter()
            .map(|c| c.quoted.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let holes = vec!["?"; key.len() + 1].join(", ");
        let key_match = key
            .iter()
            .map(|c| format!("{} = ?", c.quoted))
            .collect::<Vec<_>>()
            .join(" AND ");
        let table = quote(&name);
        let fields = fields
            .into_iter()
            .map(|(field, not_null)| {
                let quoted = quote(&field);
                let upsert = format!(
                    "INSERT INTO {table} ({key_list}, {quoted}) VALUES ({holes}) ON CONFLICT ({key_list}) DO UPDATE SET {quoted} = excluded.{quoted}"
                );
                let read = format!("SELECT {quoted} FROM {table} WHERE {key_match}");
                (field, Field { upsert, read, not_null })
            })
            .collect();
        tables.insert(name, Table { key, fields });
    }
    Ok((tables, wrong))
}

/// Every table of `db` the log would refuse, and why. Empty: `Replica::new` takes it.
pub fn refusals(db: &Connection) -> Result<Vec<String>, OpError> {
    Ok(read_schema(db)?.1)
}

struct Bound<'a>(&'a Value);

impl ToSql for Bound<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self.0 {
            Value::Null => ValueRef::Null,
            Value::Int(i) => ValueRef::Integer(*i),
            Value::Real(f) => ValueRef::Real(*f),
            Value::Text(t) => ValueRef::Text(t.as_bytes()),
            Value::Blob(b) => ValueRef::Blob(b),
        }))
    }
}

pub fn from_sql(value: Sql) -> Value {
    match value {
        Sql::Null => Value::Null,
        Sql::Integer(i) => Value::Int(i),
        Sql::Real(f) => Value::Real(f),
        Sql::Text(t) => Value::Text(t),
        Sql::Blob(b) => Value::Blob(b),
    }
}

/// What every replica must refuse alike, whatever order its ops came in; of an op it accepts, the field and the row's key parts.
fn checked<'a>(
    tables: &'a HashMap<String, Table>,
    op: &'a Op,
) -> Result<(&'a Field, Vec<&'a str>), OpError> {
    let write = &op.write;
    if !(0..=MAX_STAMP).contains(&op.stamp) {
        return refused(format!("stamp {} is out of range", op.stamp));
    }
    // SQLite stores NaN as NULL, so the log would no longer hold the op it was sent.
    if matches!(write.value, Value::Real(f) if f.is_nan()) {
        return refused("NaN is no value".to_string());
    }
    let Some(table) = tables.get(&write.table) else {
        return refused(format!("no table {}", write.table));
    };
    let Some(field) = table.fields.get(&write.field) else {
        return refused(format!("{} has no field {}", write.table, write.field));
    };
    // An upsert would refuse it only if no later write of the field had come first.
    if field.not_null && write.value == Value::Null {
        return refused(format!("{}.{} takes no NULL", write.table, write.field));
    }
    let parts: Vec<&str> = write.row.split(KEY_SEPARATOR).collect();
    if parts.len() != table.key.len() {
        return refused(format!(
            "{}: row {:?} has {} key parts, not {}",
            write.table,
            write.row,
            parts.len(),
            table.key.len()
        ));
    }
    for (part, column) in parts.iter().zip(&table.key) {
        // SQLite would turn « 05 » into 5: two row strings, one row, and the log's field order no longer holds.
        if column.integer && part.parse::<i64>().map(|n| n.to_string()).as_deref() != Ok(*part) {
            return refused(format!("{}: row part {part:?} is no integer", write.table));
        }
    }
    Ok((field, parts))
}

/// Logs one op and, unless the log already holds a later write of its field, applies it. False: it was already logged.
fn apply_one(db: &Connection, tables: &HashMap<String, Table>, op: &Op) -> Result<bool, OpError> {
    let (field, parts) = checked(tables, op)?;
    let write = &op.write;
    let (stamp, writer) = (op.stamp, &op.writer);
    let logged = db
        .prepare_cached(
            "INSERT INTO ops (stamp, writer, tbl, row_id, field, value) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT DO NOTHING",
        )?
        .execute(params![stamp, writer, write.table, write.row, write.field, Bound(&write.value)])?;
    if logged == 0 {
        let held = db
            .prepare_cached("SELECT * FROM ops WHERE stamp = ?1 AND writer = ?2")?
            .query_row(params![stamp, writer], read_op)?;
        if held != *op {
            return refused(format!("two ops share stamp {stamp} of {writer}"));
        }
        return Ok(false);
    }
    let later = db
        .prepare_cached(
            "SELECT 1 FROM ops WHERE tbl = ?1 AND row_id = ?2 AND field = ?3 AND (stamp, writer) > (?4, ?5) LIMIT 1",
        )?
        .query_row(params![write.table, write.row, write.field, stamp, writer], |_| Ok(()))
        .optional()?
        .is_some();
    if !later {
        let mut values: Vec<&dyn ToSql> = parts.iter().map(|p| p as &dyn ToSql).collect();
        let value = Bound(&write.value);
        values.push(&value);
        db.prepare_cached(&field.upsert)?.execute(&values[..])?;
    }
    Ok(true)
}

/// The field as its column kept it: affinity makes `Int(5)` into '5' in a TEXT column.
fn stored(db: &Connection, tables: &HashMap<String, Table>, op: &Op) -> Result<Value, OpError> {
    let (field, parts) = checked(tables, op)?;
    let keys: Vec<&dyn ToSql> = parts.iter().map(|p| p as &dyn ToSql).collect();
    let value: Sql = db
        .prepare_cached(&field.read)?
        .query_row(&keys[..], |r| r.get(0))?;
    Ok(from_sql(value))
}

fn read_op(r: &rusqlite::Row) -> rusqlite::Result<Op> {
    Ok(Op {
        stamp: r.get("stamp")?,
        writer: r.get("writer")?,
        write: FieldWrite {
            table: r.get("tbl")?,
            row: r.get("row_id")?,
            field: r.get("field")?,
            value: from_sql(r.get("value")?),
        },
    })
}

/// The clock port's real adapter: the one place the store reads the time.
pub struct SystemClock;

impl WallClock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    }
}

pub struct Replica {
    db: Connection,
    writer: String,
    clock: Clock,
    wall: Box<dyn WallClock + Send>,
    tables: HashMap<String, Table>,
}

impl Replica {
    /// Takes `db` with its tables made.
    pub fn new(db: Connection, writer: &str) -> Result<Self, OpError> {
        Self::with_clock(db, writer, Box::new(SystemClock))
    }

    /// `new`, its time read from `wall`: a simulation's own, so a rerun stamps the same.
    pub fn with_clock(
        db: Connection,
        writer: &str,
        wall: Box<dyn WallClock + Send>,
    ) -> Result<Self, OpError> {
        // rusqlite's bundled SQLite turns these on, and a child's op may come before its parent's.
        db.execute_batch("PRAGMA foreign_keys = OFF;")?;
        // A statement per field of each table, or a replay misses this cache at every op.
        db.set_prepared_statement_cache_capacity(512);
        // Judged first: a refused open leaves the file as it was.
        let (tables, wrong) = read_schema(&db)?;
        if !wrong.is_empty() {
            return refused(wrong.join("; "));
        }
        db.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {LOG_TABLE};{LOG_INDEX}"
        ))?;
        Ok(Self {
            db,
            writer: writer.to_string(),
            clock: Clock::starting_at(0),
            wall,
            tables,
        })
    }

    /// One commit: the ops it produced, which is also what the views get (section 6).
    pub fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, OpError> {
        self.write_at(self.wall.now_ms(), writes)
    }

    pub fn write_at(&mut self, now_ms: i64, writes: &[FieldWrite]) -> Result<Vec<Op>, OpError> {
        // The clock starts past the log inside the write lock: a restart under a slow wall clock still sorts last, and a second
        // process on this file under this writer never takes a stamp this one holds.
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let last: i64 = tx
            .prepare_cached("SELECT coalesce(max(stamp), 0) FROM ops")?
            .query_row([], |r| r.get(0))?;
        self.clock.observe(last);
        let mut ops = vec![];
        for write in writes {
            let mut write = write.clone();
            // A NaN is written as what SQLite would make of it; `apply` refuses one from another replica.
            if matches!(write.value, Value::Real(f) if f.is_nan()) {
                write.value = Value::Null;
            }
            let mut op = Op {
                stamp: self.clock.tick(now_ms),
                writer: self.writer.clone(),
                write,
            };
            apply_one(&tx, &self.tables, &op)?;
            // The log and the views get what the table holds, so a recompute from the table agrees with them.
            let kept = stored(&tx, &self.tables, &op)?;
            if kept != op.write.value {
                tx.prepare_cached("UPDATE ops SET value = ?1 WHERE stamp = ?2 AND writer = ?3")?
                    .execute(params![Bound(&kept), op.stamp, op.writer])?;
                op.write.value = kept;
            }
            ops.push(op);
        }
        tx.commit()?;
        Ok(ops)
    }

    /// Takes ops from a replay or another replica, in any order, any number of times: all of them or none. Returns how many
    /// were new.
    pub fn apply(&mut self, ops: &[Op]) -> Result<usize, OpError> {
        let tx = self.db.transaction()?;
        let mut new = 0;
        for op in ops {
            new += usize::from(apply_one(&tx, &self.tables, op)?);
        }
        tx.commit()?;
        for op in ops {
            self.clock.observe(op.stamp);
        }
        Ok(new)
    }

    /// The log in stamp order, after `after`, a page at a time: a rebuild must not hold the whole log in memory.
    pub fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, OpError> {
        let (stamp, writer) = after.unwrap_or((-1, ""));
        let ops = self
            .db
            .prepare_cached(
                "SELECT * FROM ops WHERE (stamp, writer) > (?1, ?2) ORDER BY stamp, writer LIMIT ?3",
            )?
            .query_map(params![stamp, writer, limit], read_op)?
            .collect::<Result<_, _>>()?;
        Ok(ops)
    }

    /// The database with its free pages and insert history gone: a replay's copy equals the original's byte for byte.
    pub fn vacuum_into(&self, path: &Path) -> Result<(), OpError> {
        let path = path.to_str().ok_or(OpError::Refused("path".into()))?;
        self.db.execute("VACUUM INTO ?1", [path])?;
        Ok(())
    }

    /// Every governed table and the log, by name and key: the same rows and ops give the same number on any replica, in
    /// any arrival order. Names sorted, since a HashMap's order differs from one process to the next.
    pub fn state_hash(&self) -> Result<u64, OpError> {
        let mut names: Vec<&String> = self.tables.keys().collect();
        names.sort();
        let mut orders: Vec<(String, String)> = names
            .into_iter()
            .map(|name| {
                let key: Vec<&str> = self.tables[name]
                    .key
                    .iter()
                    .map(|c| c.quoted.as_str())
                    .collect();
                (quote(name), key.join(", "))
            })
            .collect();
        orders.push(("ops".to_string(), "stamp, writer".to_string()));
        let mut hash = Fnv::new();
        for (table, order) in orders {
            hash.bytes(table.as_bytes());
            let mut rows = self
                .db
                .prepare_cached(&format!("SELECT * FROM {table} ORDER BY {order}"))?;
            let columns = rows.column_count();
            let mut rows = rows.query([])?;
            while let Some(row) = rows.next()? {
                for column in 0..columns {
                    from_sql(row.get(column)?).digest(&mut hash);
                }
            }
        }
        Ok(hash.finish())
    }

    /// Reads with `f`, on this connection set to query only: a write there would go round the log, and no replay brings it back.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> T) -> Result<T, OpError> {
        self.db.execute_batch("PRAGMA query_only = ON")?;
        let out = f(&self.db);
        self.db.execute_batch("PRAGMA query_only = OFF")?;
        Ok(out)
    }
}

impl OpLog for Replica {
    fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String> {
        Replica::write(self, writes).map_err(|e| e.to_string())
    }

    fn apply(&mut self, ops: &[Op]) -> Result<usize, String> {
        Replica::apply(self, ops).map_err(|e| e.to_string())
    }

    fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String> {
        Replica::ops_after(self, after, limit).map_err(|e| e.to_string())
    }

    fn state_hash(&self) -> Result<u64, String> {
        Replica::state_hash(self).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use proptest::prelude::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    const SCHEMA: &str = "
      CREATE TABLE notes (id TEXT PRIMARY KEY, title, body) WITHOUT ROWID;
      CREATE TABLE links (source TEXT, pos INTEGER, target, PRIMARY KEY (source, pos)) WITHOUT ROWID;";

    fn replica(writer: &str) -> Replica {
        let db = Connection::open_in_memory().expect("open");
        db.execute_batch(SCHEMA).expect("schema");
        Replica::new(db, writer).expect("replica")
    }

    fn set(table: &str, row: &str, field: &str, value: Value) -> FieldWrite {
        FieldWrite {
            table: table.into(),
            row: row.into(),
            field: field.into(),
            value,
        }
    }

    fn note(row: &str, field: &str, value: &str) -> FieldWrite {
        set("notes", row, field, Value::Text(value.into()))
    }

    fn log(replica: &Replica) -> Vec<Op> {
        replica.ops_after(None, i64::MAX).expect("the log")
    }

    fn read(replica: &Replica, id: &str, field: &str) -> Value {
        let sql = format!("SELECT {} FROM notes WHERE id = ?1", quote(field));
        replica
            .read(|db| db.query_row(&sql, [id], |r| r.get::<_, Sql>(0)))
            .expect("read")
            .map_or(Value::Null, from_sql)
    }

    fn bytes(replica: &Replica, dir: &TestDir, name: &str) -> Vec<u8> {
        let path = dir.0.join(name);
        replica.vacuum_into(&path).expect("vacuum");
        let bytes = std::fs::read(&path).expect("read");
        std::fs::remove_file(&path).expect("remove");
        bytes
    }

    fn replayed(ops: &[Op]) -> Replica {
        let mut copy = replica("replay");
        copy.apply(ops).expect("replay");
        copy
    }

    #[test]
    fn a_write_produces_ops_and_replaying_them_gives_an_identical_database() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("oplog-replay");
        let mut a = replica("mac");
        let ops = a
            .write(&[note("n1", "title", "first"), note("n1", "body", "words")])
            .expect("a write");
        assert_eq!(ops.len(), 2, "one op per field");
        assert_eq!(ops[0].write, note("n1", "title", "first"));
        assert_eq!(ops[0].writer, "mac");
        assert!(ops[0].stamp < ops[1].stamp);
        assert_eq!(log(&a), ops, "the log holds what the write returned");

        a.write(&[note("n1", "title", "second")]).expect("an edit");
        a.write(&[set(
            "links",
            &format!("n1{KEY_SEPARATOR}3"),
            "target",
            Value::Int(7),
        )])
        .expect("a composite key");
        assert_eq!(read(&a, "n1", "title"), Value::Text("second".into()));
        let target: i64 = a
            .read(|db| {
                db.query_row(
                    "SELECT target FROM links WHERE source = 'n1' AND pos = 3",
                    [],
                    |r| r.get(0),
                )
            })
            .expect("read")
            .expect("the composite row");
        assert_eq!(target, 7);

        let original = bytes(&a, &dir, "original");
        assert!(original.len() > 4096, "a real file: {}", original.len());
        assert_eq!(original, bytes(&replayed(&log(&a)), &dir, "copy"));

        let all = log(&a);
        let less = bytes(&replayed(&all[..all.len() - 1]), &dir, "less");
        assert_ne!(original, less, "the check can fail: one op missing");
    }

    #[test]
    fn a_page_of_the_log_starts_after_its_cursor() {
        let _budget = TestBudget::new();
        let mut a = replica("mac");
        let ops = a
            .write(&[
                note("n", "title", "1"),
                note("n", "body", "2"),
                note("m", "body", "3"),
            ])
            .expect("write");
        let first = a.ops_after(None, 2).expect("page");
        assert_eq!(first, ops[..2]);
        let last = &first[1];
        let next = a
            .ops_after(Some((last.stamp, &last.writer)), 2)
            .expect("page");
        assert_eq!(next, ops[2..]);
    }

    #[test]
    fn a_restart_stamps_past_its_log_under_a_slow_wall_clock() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("oplog-restart");
        let path = dir.0.join("db");
        let schema = |db: &Connection| db.execute_batch(SCHEMA).expect("schema");
        let db = Connection::open(&path).expect("open");
        schema(&db);
        let mut a = Replica::new(db, "mac").expect("replica");
        let before = a
            .write_at(5_000, &[note("n", "title", "before")])
            .expect("write");
        drop(a);
        let db = Connection::open(&path).expect("reopen");
        let mut a = Replica::new(db, "mac").expect("replica");
        let after = a
            .write_at(1_000, &[note("n", "title", "after")])
            .expect("write");
        assert!(after[0].stamp > before[0].stamp);
        assert_eq!(read(&a, "n", "title"), Value::Text("after".into()));
    }

    #[test]
    fn the_later_write_wins_whichever_arrives_first_and_the_writer_breaks_a_tie() {
        let _budget = TestBudget::new();
        let mut a = replica("a");
        let mut b = replica("b");
        let older = a.write_at(100, &[note("n", "title", "older")]).expect("a");
        let newer = b.write_at(200, &[note("n", "title", "newer")]).expect("b");
        a.apply(&newer).expect("in order");
        b.apply(&older).expect("late");
        assert_eq!(read(&a, "n", "title"), Value::Text("newer".into()));
        assert_eq!(read(&b, "n", "title"), Value::Text("newer".into()));

        let mut a = replica("a");
        let mut b = replica("b");
        let from_a = a.write_at(100, &[note("n", "title", "a")]).expect("a");
        let from_b = b.write_at(100, &[note("n", "title", "b")]).expect("b");
        assert_eq!(from_a[0].stamp, from_b[0].stamp, "a true tie");
        a.apply(&from_b).expect("b wins");
        b.apply(&from_a).expect("a loses");
        assert_eq!(read(&a, "n", "title"), Value::Text("b".into()));
        assert_eq!(read(&b, "n", "title"), Value::Text("b".into()));
    }

    #[test]
    fn an_op_seen_twice_changes_nothing_and_one_seen_once_moves_the_clock() {
        let _budget = TestBudget::new();
        let mut a = replica("a");
        let mut b = replica("b");
        let ops = a.write_at(9_000, &[note("n", "title", "x")]).expect("a");
        assert_eq!(b.apply(&ops).expect("first"), 1);
        assert_eq!(b.apply(&ops).expect("again"), 0);
        assert_eq!(log(&b).len(), 1);
        let mine = b.write_at(1, &[note("n", "title", "y")]).expect("b");
        assert!(
            mine[0].stamp > ops[0].stamp,
            "a slow writer still wins with a later write"
        );
    }

    #[test]
    fn a_bad_op_is_refused_and_its_whole_batch_with_it() {
        let _budget = TestBudget::new();
        let mut a = replica("a");
        let good = a.write_at(10, &[note("n", "title", "x")]).expect("write");
        let mut b = replica("b");
        let with = |write: FieldWrite, stamp: i64| Op {
            stamp,
            writer: "w".into(),
            write,
        };
        for (bad, says) in [
            (
                with(set("notes", "n", "title", Value::Real(f64::NAN)), 5),
                "NaN",
            ),
            (with(note("n", "title", "x"), -1), "out of range"),
            (with(note("n", "title", "x"), MAX_STAMP + 1), "out of range"),
            (with(set("ops", "n", "value", Value::Null), 5), "no table"),
            (
                with(
                    set("notes\"; DROP TABLE notes; --", "n", "title", Value::Null),
                    5,
                ),
                "no table",
            ),
            (with(note("n", "id", "other"), 5), "no field"),
            (with(note("n", "absent\" = 1 --", "x"), 5), "no field"),
            (
                with(set("links", "only-one-part", "target", Value::Null), 5),
                "key parts",
            ),
            (
                with(
                    set(
                        "links",
                        &format!("s{KEY_SEPARATOR}05"),
                        "target",
                        Value::Null,
                    ),
                    5,
                ),
                "no integer",
            ),
        ] {
            let e = b
                .apply(&[good[0].clone(), bad])
                .expect_err(says)
                .to_string();
            assert!(e.contains(says), "{e}");
            assert!(log(&b).is_empty(), "nothing of the batch stayed");
        }
        let mut twin = good[0].clone();
        twin.write.value = Value::Text("another value".into());
        b.apply(&good).expect("the good one");
        let e = b.apply(&[twin]).expect_err("the twin").to_string();
        assert!(e.contains("share stamp"), "{e}");
        assert_eq!(read(&b, "n", "title"), Value::Text("x".into()));
    }

    #[test]
    fn a_table_that_cannot_take_one_field_alone_is_refused_at_open() {
        let _budget = TestBudget::new();
        for (schema, says) in [
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a TEXT NOT NULL)",
                "NOT NULL",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a TEXT UNIQUE)",
                "unique",
            ),
            ("CREATE TABLE t (a TEXT)", "primary key"),
            ("CREATE TABLE t (id TEXT PRIMARY KEY)", "primary key"),
            ("CREATE TABLE t (id REAL PRIMARY KEY, a)", "INTEGER or TEXT"),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a); CREATE UNIQUE INDEX u ON t (a)",
                "unique",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a); CREATE TRIGGER g AFTER UPDATE ON t BEGIN SELECT 1; END",
                "trigger",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a, b, CHECK (a < b))",
                "CHECK",
            ),
            (
                "CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, a)",
                "AUTOINCREMENT",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, at DEFAULT CURRENT_TIMESTAMP)",
                "varies",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, n DEFAULT ('a' || random() || 'b'))",
                "varies",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, n DEFAULT (random()))",
                "varies",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a); CREATE TRIGGER g AFTER UPDATE ON T BEGIN SELECT 1; END",
                "trigger",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a, -- it's a note\n b, CHECK (a < b))",
                "CHECK",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a /* it's */, b, CHECK (a < b))",
                "CHECK",
            ),
            (
                "CREATE TABLE t (id TEXT PRIMARY KEY, a TEXT) STRICT",
                "STRICT",
            ),
            ("CREATE TABLE OPS (id TEXT PRIMARY KEY, a)", "the log's"),
        ] {
            let db = Connection::open_in_memory().expect("open");
            db.execute_batch(schema).expect(schema);
            let e = Replica::new(db, "a").err().expect(schema).to_string();
            assert!(e.contains(says), "{schema}: {e}");
        }
        let db = Connection::open_in_memory().expect("open");
        db.execute_batch(
            "CREATE TABLE t (id TEXT PRIMARY KEY, a TEXT NOT NULL DEFAULT '', \"checks\" TEXT, [autoincrement] TEXT);
             CREATE TABLE two (id TEXT PRIMARY KEY, a, b TEXT DEFAULT 'CHECK (x)', c INTEGER DEFAULT -1.5, d DEFAULT 'it''s', e DEFAULT false, f DEFAULT NULL);
             CREATE TABLE three (id TEXT PRIMARY KEY, a -- no check here
               , b /* autoincrement */);
             CREATE TABLE sqlitefoo (id TEXT PRIMARY KEY, a);",
        )
        .expect("what stays");
        assert_eq!(refusals(&db).expect("refusals"), Vec::<String>::new());
        let mut a = Replica::new(db, "a").expect("a constant default and these names are fine");
        a.write(&[set("sqlitefoo", "n", "a", Value::Int(1))])
            .expect("a name that only looks like SQLite's is governed");
    }

    #[test]
    fn a_refused_open_leaves_the_file_as_it_was() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("oplog-refused");
        let path = dir.0.join("db");
        let db = Connection::open(&path).expect("open");
        db.execute_batch("CREATE TABLE t (a TEXT)").expect("schema");
        assert!(Replica::new(db, "a").is_err());
        let db = Connection::open(&path).expect("reopen");
        let objects: i64 = db
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
            .expect("count");
        assert_eq!(objects, 1, "the table alone");
    }

    #[test]
    fn a_value_its_column_refuses_is_refused_in_either_order() {
        let _budget = TestBudget::new();
        let schema = "CREATE TABLE t (id TEXT PRIMARY KEY, a TEXT NOT NULL DEFAULT '')";
        let db = Connection::open_in_memory().expect("open");
        db.execute_batch(schema).expect("schema");
        let mut b = Replica::new(db, "b").expect("replica");
        let op = |stamp, value| Op {
            stamp,
            writer: "w".into(),
            write: set("t", "n", "a", value),
        };
        let (null, later) = (op(5, Value::Null), op(6, Value::Text("v".into())));
        for batch in [[null.clone(), later.clone()], [later.clone(), null.clone()]] {
            let e = b.apply(&batch).expect_err("either order").to_string();
            assert!(e.contains("takes no NULL"), "{e}");
        }
        b.apply(&[later]).expect("the good one alone");
        let e = b
            .apply(&[null])
            .expect_err("after the later one")
            .to_string();
        assert!(e.contains("takes no NULL"), "{e}");
    }

    #[test]
    fn two_processes_on_one_file_under_one_writer_never_share_a_stamp() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("oplog-two-processes");
        let path = dir.0.join("db");
        let db = Connection::open(&path).expect("open");
        db.execute_batch(SCHEMA).expect("schema");
        let mut app = Replica::new(db, "mac").expect("the app");
        let cli = Connection::open(&path).expect("open");
        let mut cli = Replica::new(cli, "mac").expect("the cli");
        let first = app
            .write_at(1_000, &[note("n", "title", "app")])
            .expect("app");
        let second = cli
            .write_at(1_000, &[note("n", "body", "cli")])
            .expect("cli");
        assert!(second[0].stamp > first[0].stamp);
        assert_eq!(log(&app).len(), 2);
    }

    #[test]
    fn a_write_returns_and_logs_what_its_column_kept() {
        let _budget = TestBudget::new();
        let db = Connection::open_in_memory().expect("open");
        db.execute_batch("CREATE TABLE t (id TEXT PRIMARY KEY, words TEXT, n INTEGER)")
            .expect("schema");
        let mut a = Replica::new(db, "a").expect("replica");
        let ops = a
            .write(&[
                set("t", "r", "words", Value::Int(5)),
                set("t", "r", "n", Value::Real(1.0)),
            ])
            .expect("write");
        assert_eq!(ops[0].write.value, Value::Text("5".into()));
        assert_eq!(ops[1].write.value, Value::Int(1));
        assert_eq!(log(&a), ops);
    }

    #[test]
    fn a_read_cannot_write_and_the_write_path_still_can() {
        let _budget = TestBudget::new();
        let mut a = replica("a");
        let tried = a
            .read(|db| db.execute("INSERT INTO notes (id) VALUES ('x')", []))
            .expect("read");
        assert!(tried.is_err(), "query only");
        a.write(&[note("n", "title", "x")])
            .expect("write after a read");
    }

    #[test]
    fn a_childs_op_may_come_before_its_parents() {
        let _budget = TestBudget::new();
        let db = Connection::open_in_memory().expect("open");
        db.execute_batch(
            "CREATE TABLE parent (id TEXT PRIMARY KEY, a);
             CREATE TABLE child (id TEXT PRIMARY KEY, parent TEXT REFERENCES parent (id));",
        )
        .expect("schema");
        let mut a = Replica::new(db, "a").expect("replica");
        a.write(&[set("child", "c", "parent", Value::Text("not yet".into()))])
            .expect("a child alone");
    }

    #[test]
    fn a_write_of_nan_logs_what_sqlite_keeps() {
        let _budget = TestBudget::new();
        let mut a = replica("a");
        let ops = a
            .write(&[set("notes", "n", "title", Value::Real(f64::NAN))])
            .expect("write");
        assert_eq!(ops[0].write.value, Value::Null);
        assert_eq!(log(&a), ops);
    }

    #[derive(Clone, Debug)]
    enum Step {
        Write {
            on: usize,
            now_ms: i64,
            writes: Vec<FieldWrite>,
        },
        /// The other replica's log so far, the ops this mask keeps, in the order of this seed.
        Deliver { to: usize, mask: u64, seed: u64 },
    }

    fn value() -> impl Strategy<Value = Value> {
        prop_oneof![
            Just(Value::Null),
            any::<i64>().prop_map(Value::Int),
            any::<f64>().prop_map(Value::Real),
            ".{0,12}".prop_map(Value::Text),
            prop::collection::vec(any::<u8>(), 0..8).prop_map(Value::Blob),
        ]
    }

    fn field_write() -> impl Strategy<Value = FieldWrite> {
        let note = (0..3u8, prop::sample::select(vec!["title", "body"]), value())
            .prop_map(|(row, field, value)| set("notes", &format!("n{row}"), field, value));
        let link = (0..2u8, 0..3i64, value()).prop_map(|(source, pos, value)| {
            set(
                "links",
                &format!("s{source}{KEY_SEPARATOR}{pos}"),
                "target",
                value,
            )
        });
        prop_oneof![3 => note, 1 => link]
    }

    fn step() -> impl Strategy<Value = Step> {
        // Few milliseconds, so stamps tie across the replicas and clocks run ahead of each other.
        let write = (
            0..2usize,
            0..4i64,
            prop::collection::vec(field_write(), 1..4),
        )
            .prop_map(|(on, now_ms, writes)| Step::Write { on, now_ms, writes });
        let deliver = (0..2usize, any::<u64>(), any::<u64>())
            .prop_map(|(to, mask, seed)| Step::Deliver { to, mask, seed });
        prop_oneof![2 => write, 1 => deliver]
    }

    fn shuffled(mut ops: Vec<Op>, seed: u64) -> Vec<Op> {
        ops.sort_by_key(|op| {
            let mut hash = DefaultHasher::new();
            (seed, op.stamp, &op.writer).hash(&mut hash);
            hash.finish()
        });
        ops
    }

    /// What the merge must give, worked out from the ops alone: each field's op with the highest `(stamp, writer)`.
    fn winners(ops: &[Op]) -> HashMap<(String, String, String), &Op> {
        let mut won: HashMap<(String, String, String), &Op> = HashMap::new();
        for op in ops {
            let write = &op.write;
            let key = (write.table.clone(), write.row.clone(), write.field.clone());
            let held = won.entry(key).or_insert(op);
            if (op.stamp, &op.writer) > (held.stamp, &held.writer) {
                *held = op;
            }
        }
        won
    }

    fn read_field(replica: &Replica, write: &FieldWrite) -> Value {
        let (sql, keys): (String, Vec<&str>) = if write.table == "notes" {
            (
                format!("SELECT {} FROM notes WHERE id = ?1", quote(&write.field)),
                vec![&write.row],
            )
        } else {
            (
                "SELECT target FROM links WHERE source = ?1 AND pos = ?2".to_string(),
                write.row.split(KEY_SEPARATOR).collect(),
            )
        };
        let keys: Vec<&dyn ToSql> = keys.iter().map(|k| k as &dyn ToSql).collect();
        let value = replica
            .read(|db| db.query_row(&sql, &keys[..], |r| r.get::<_, Sql>(0)))
            .expect("read")
            .expect("the field's row");
        from_sql(value)
    }

    proptest! {
        // Section 17: a low count on PRs; PROPTEST_CASES raises it for a nightly.
        #![proptest_config(ProptestConfig::with_cases(
            std::env::var("PROPTEST_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(64)
        ))]

        #[test]
        fn two_replicas_merge_to_the_same_state(
            steps in prop::collection::vec(step(), 1..24),
            final_seed in any::<u64>(),
        ) {
            let _budget = TestBudget::new();
            let dir = TestDir::new(&format!("oplog-merge-{final_seed}"));
            let mut replicas = [replica("a"), replica("b")];
            for step in steps {
                match step {
                    Step::Write { on, now_ms, writes } => {
                        replicas[on].write_at(now_ms, &writes).expect("write");
                    }
                    Step::Deliver { to, mask, seed } => {
                        let some: Vec<Op> = log(&replicas[1 - to])
                            .into_iter()
                            .enumerate()
                            .filter(|(i, _)| mask >> (i % 64) & 1 == 1)
                            .map(|(_, op)| op)
                            .collect();
                        replicas[to].apply(&shuffled(some, seed)).expect("deliver");
                    }
                }
            }
            let (for_a, for_b) = (log(&replicas[1]), log(&replicas[0]));
            replicas[0].apply(&shuffled(for_a, final_seed)).expect("to a");
            replicas[1].apply(&shuffled(for_b, final_seed ^ 1)).expect("to b");

            let merged = bytes(&replicas[0], &dir, "a");
            prop_assert!(merged == bytes(&replicas[1], &dir, "b"), "the replicas parted");

            let all = log(&replicas[0]);
            for op in winners(&all).values() {
                prop_assert_eq!(&read_field(&replicas[0], &op.write), &op.write.value, "{:?}", op);
            }
            prop_assert!(merged == bytes(&replayed(&all), &dir, "replay"), "a replay of the merged log differs");
        }
    }
}
