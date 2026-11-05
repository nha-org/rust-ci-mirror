//! Section 20's numbers for rust-op-log, on a copy of dash's database (`sqlite3 -readonly <db> ".backup <copy>"`). It prints
//! counts and times only, never a row.
//!   cargo run --release -p dash-store --example oplog_measure -- census <copy>
//!   ... -- write <copy> <a.db>      every row through the write path, then 1,000 single commits beside a plain UPDATE
//!   ... -- replay <a.db> <b.db>     its own process, so its peak is the replay's alone
//!   ... -- check <copy> <a.db> <b.db> <dir>
use dash_core::{FieldWrite, KEY_SEPARATOR, Op, Value};
use dash_meter::memory::memory;
use dash_store::connect;
use dash_store::oplog::{Replica, from_sql, quote, refusals};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OpenFlags, params};
use std::path::Path;
use std::time::Instant;

const PAGE: i64 = 5_000;

fn read_only(path: &str) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open read-only")
}

/// `(type, name, table, sql)` of what `db` holds beside the log, in its schema order. VACUUM keeps that order, so a replay
/// that makes its tables in another is not byte-identical.
fn schema(db: &Connection) -> Vec<(String, String, String, String)> {
    db.prepare(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
         AND name NOT IN ('ops', 'ops_by_field') ORDER BY rowid",
    )
    .expect("schema")
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
    .expect("schema")
    .collect::<Result<_, _>>()
    .expect("schema")
}

fn table_names(db: &Connection) -> Vec<String> {
    schema(db)
        .into_iter()
        .filter(|(kind, ..)| kind == "table")
        .map(|(_, name, ..)| name)
        .collect()
}

/// A table as the port would make it, from its columns: no NOT NULL a lone field breaks, no CHECK, no AUTOINCREMENT.
fn ported_ddl(db: &Connection, table: &str) -> String {
    let mut stmt = db
        .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid")
        .expect("columns");
    let rows: Vec<(String, String, bool, Option<String>, i64)> = stmt
        .query_map([table], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("columns")
        .collect::<Result<_, _>>()
        .expect("columns");
    let mut parts: Vec<String> = rows
        .iter()
        .map(|(name, kind, not_null, default, _)| {
            let kept = default.as_ref().map_or(String::new(), |d| {
                format!(" DEFAULT {d}{}", if *not_null { " NOT NULL" } else { "" })
            });
            format!("{} {kind}{kept}", quote(name))
        })
        .collect();
    let mut key: Vec<&(String, String, bool, Option<String>, i64)> =
        rows.iter().filter(|r| r.4 > 0).collect();
    key.sort_by_key(|r| r.4);
    if !key.is_empty() {
        let names: Vec<String> = key.iter().map(|r| quote(&r.0)).collect();
        parts.push(format!("PRIMARY KEY ({})", names.join(", ")));
    }
    format!("CREATE TABLE {} ({})", quote(table), parts.join(", "))
}

fn takes(ddl: &str) -> bool {
    let db = Connection::open_in_memory().expect("memory");
    db.execute_batch(ddl).expect("ddl");
    refusals(&db).expect("refusals").is_empty()
}

fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT count(*) FROM {}", quote(table)), [], |r| {
        r.get(0)
    })
    .expect("count")
}

/// (key columns in key order, the others).
fn columns(db: &Connection, table: &str) -> (Vec<String>, Vec<String>) {
    let mut stmt = db
        .prepare("SELECT name, pk FROM pragma_table_info(?1) ORDER BY pk, cid")
        .expect("columns");
    let rows: Vec<(String, i64)> = stmt
        .query_map([table], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("columns")
        .collect::<Result<_, _>>()
        .expect("columns");
    let key = rows.iter().filter(|(_, pk)| *pk > 0);
    let rest = rows.iter().filter(|(_, pk)| *pk == 0);
    (
        key.map(|(n, _)| n.clone()).collect(),
        rest.map(|(n, _)| n.clone()).collect(),
    )
}

fn census(copy: &str) {
    let source = read_only(copy);
    let tables = table_names(&source);
    // The real schema, with its indexes and triggers, judged by the rule Replica::new applies.
    let probe = Connection::open_in_memory().expect("memory");
    for (.., sql) in schema(&source) {
        probe.execute_batch(&sql).expect("schema");
    }
    let wrong = refusals(&probe).expect("refusals");
    let refused_table = |line: &String| line.split(['.', ':']).next().unwrap_or("").to_string();
    let mut refused: Vec<String> = wrong.iter().map(refused_table).collect();
    refused.sort();
    refused.dedup();
    let mut by_rule = String::new();
    for rule in [
        "NOT NULL",
        "unique",
        "trigger",
        "CHECK",
        "AUTOINCREMENT",
        "varies",
        "primary key",
        "INTEGER or TEXT",
    ] {
        let mut names: Vec<String> = wrong
            .iter()
            .filter(|l| l.contains(rule))
            .map(refused_table)
            .collect();
        names.sort();
        names.dedup();
        let rows: i64 = names.iter().map(|n| count(&source, n)).sum();
        by_rule += &format!("{rule}: {} tables, {rows} rows; ", names.len());
    }
    let (mut composite, mut positional, mut rows_all) = (0, 0, 0);
    let mut left_out = vec![];
    for name in &tables {
        rows_all += count(&source, name);
        let (key, _) = columns(&source, name);
        composite += usize::from(key.len() > 1);
        positional += usize::from(key.iter().any(|k| k == "pos"));
        if !takes(&ported_ddl(&source, name)) {
            left_out.push(format!("{name} ({} rows)", count(&source, name)));
        }
    }
    let objects = schema(&source);
    let of_kind = |kind: &str| objects.iter().filter(|o| o.0 == kind).count();
    println!(
        "tables {} rows {rows_all}; fit as they stand {}; ported {}; left out even ported {left_out:?}; {by_rule}\
         composite keys {composite}; keyed by position {positional}; indexes {}; triggers {}",
        tables.len(),
        tables.len() - refused.len(),
        tables.len() - left_out.len(),
        of_kind("index"),
        of_kind("trigger"),
    );
}

/// The tables the log can take once ported and their indexes, none unique, made in `to` in `from`'s order. Triggers stay
/// behind: the log and the views do their work.
fn make_ported(from: &Connection, to: &Connection) -> Vec<String> {
    let mut taken = vec![];
    for (kind, name, table, sql) in schema(from) {
        if kind == "table" {
            let ddl = ported_ddl(from, &name);
            if takes(&ddl) {
                to.execute_batch(&ddl).expect("make table");
                taken.push(name);
            }
        } else if kind == "index" && taken.contains(&table) {
            let plain = sql.replacen("CREATE UNIQUE INDEX", "CREATE INDEX", 1);
            to.execute_batch(&plain).expect("make index");
        }
    }
    taken
}

/// `from`'s schema as it stands: a replay of a database that was already ported.
fn copy_schema(from: &Connection, to: &Connection) {
    for (.., sql) in schema(from) {
        to.execute_batch(&sql).expect("copy schema");
    }
}

/// The row's key as an op names it; none when a part is NULL, or of a type its column would not read back.
fn row_key(parts: &[Sql]) -> Option<String> {
    let mut key = vec![];
    for part in parts {
        key.push(match part {
            Sql::Integer(i) => i.to_string(),
            Sql::Text(t) if !t.contains(KEY_SEPARATOR) => t.clone(),
            _ => return None,
        });
    }
    Some(key.join(&KEY_SEPARATOR.to_string()))
}

fn latency(mut commit: impl FnMut(i64)) -> (u128, u128) {
    let mut us: Vec<u128> = (0..1_000)
        .map(|i| {
            let start = Instant::now();
            commit(i);
            start.elapsed().as_micros()
        })
        .collect();
    us.sort();
    (us[500], us[950])
}

fn write(copy: &str, out: &str) {
    let source = read_only(copy);
    let target = connect(Path::new(out)).expect("target");
    let taken = make_ported(&source, &target);
    target
        .execute_batch("CREATE TABLE probe (id TEXT PRIMARY KEY, v)")
        .expect("probe");
    let mut replica = Replica::new(target, "mac").expect("replica");
    let (mut rows, mut skipped, mut ops, mut batch) = (0u64, 0u64, 0u64, vec![]);
    let start = Instant::now();
    for table in &taken {
        let (key, fields) = columns(&source, table);
        let names: Vec<String> = key.iter().chain(&fields).map(|c| quote(c)).collect();
        let sql = format!("SELECT {} FROM {}", names.join(", "), quote(table));
        let mut stmt = source.prepare(&sql).expect("read");
        let mut cursor = stmt.query([]).expect("read");
        while let Some(r) = cursor.next().expect("row") {
            let parts: Vec<Sql> = (0..key.len()).map(|i| r.get(i).expect("key")).collect();
            let Some(row) = row_key(&parts) else {
                skipped += 1;
                continue;
            };
            rows += 1;
            for (i, field) in fields.iter().enumerate() {
                batch.push(FieldWrite {
                    table: table.clone(),
                    row: row.clone(),
                    field: field.clone(),
                    value: from_sql(r.get(key.len() + i).expect("field")),
                });
            }
            if batch.len() >= PAGE as usize {
                ops += replica.write(&batch).expect("write").len() as u64;
                batch.clear();
            }
        }
    }
    ops += replica.write(&batch).expect("write").len() as u64;
    let write_s = start.elapsed().as_secs_f64();

    let (log_p50, log_p95) = latency(|i| {
        replica
            .write(&[FieldWrite {
                table: "probe".into(),
                row: "row".into(),
                field: "v".into(),
                value: Value::Int(i),
            }])
            .expect("a single write");
    });
    let db = replica.connection();
    db.execute("INSERT INTO probe (id, v) VALUES ('plain', 0)", [])
        .expect("plain row");
    let (plain_p50, plain_p95) = latency(|i| {
        db.execute("UPDATE probe SET v = ?1 WHERE id = 'plain'", params![i])
            .expect("a plain update");
    });
    // The plain row went round the log, so a replay could not bring it back.
    db.execute("DELETE FROM probe WHERE id = 'plain'", [])
        .expect("plain row");
    println!(
        "tables {} rows {rows} skipped {skipped} ops {ops}; write {write_s:.1} s, {:.0} ops/s; \
         one field, one commit: log p50 {log_p50} µs, p95 {log_p95}; plain UPDATE p50 {plain_p50} µs, p95 {plain_p95}; peak {:.1} MB",
        taken.len(),
        ops as f64 / write_s,
        memory().peak_mb,
    );
}

fn replay(from: &str, out: &str) {
    let source = Replica::new(capped(from), "reader").expect("source");
    let target = connect(Path::new(out)).expect("target");
    copy_schema(source.connection(), &target);
    let mut replica = Replica::new(target, "replay").expect("replica");
    let (mut ops, mut after): (u64, Option<(i64, String)>) = (0, None);
    let start = Instant::now();
    loop {
        let cursor = after.as_ref().map(|(s, w)| (*s, w.as_str()));
        let page: Vec<Op> = source.ops_after(cursor, PAGE).expect("page");
        let Some(last) = page.last() else { break };
        after = Some((last.stamp, last.writer.clone()));
        ops += page.len() as u64;
        replica.apply(&page).expect("apply");
    }
    let replay_s = start.elapsed().as_secs_f64();
    println!(
        "replay {ops} ops in {replay_s:.1} s, {:.0} ops/s; peak {:.1} MB",
        ops as f64 / replay_s,
        memory().peak_mb,
    );
}

/// Section 4's cache, without its WAL: a measure's reads must leave no side files.
fn capped(path: &str) -> Connection {
    let db = Connection::open(path).expect("open");
    db.execute_batch("PRAGMA cache_size = -8000;")
        .expect("cache");
    db
}

fn size_mb(path: &Path) -> f64 {
    std::fs::metadata(path).expect("size").len() as f64 / 1_048_576.0
}

fn check(copy: &str, a: &str, b: &str, dir: &str) {
    let dir = Path::new(dir);
    let (a_copy, b_copy) = (dir.join("a.vacuumed"), dir.join("b.vacuumed"));
    let vacuum = |path: &str, into: &Path| {
        capped(path)
            .execute("VACUUM INTO ?1", [into.to_str().expect("path")])
            .expect("vacuum");
    };
    vacuum(a, &a_copy);
    vacuum(b, &b_copy);
    let identical = std::fs::read(&a_copy).expect("a") == std::fs::read(&b_copy).expect("b");

    let db = capped(a_copy.to_str().expect("path"));
    db.execute_batch("DROP INDEX ops_by_field; VACUUM;")
        .expect("drop index");
    let no_index = size_mb(&a_copy);
    db.execute_batch("DROP TABLE ops; VACUUM;")
        .expect("drop log");
    let no_log = size_mb(&a_copy);
    drop(db);
    let whole = size_mb(&b_copy);

    let source = read_only(copy);
    let replayed = read_only(b);
    let (mut compared, mut differ, mut extra) = (0u64, 0u64, 0i64);
    for table in table_names(&replayed) {
        if table == "probe" {
            continue;
        }
        let (key, fields) = columns(&source, &table);
        let names: Vec<String> = key.iter().chain(&fields).map(|c| quote(c)).collect();
        let order: Vec<String> = key.iter().map(|c| quote(c)).collect();
        let sql = format!(
            "SELECT {} FROM {} WHERE {} ORDER BY {}",
            names.join(", "),
            quote(&table),
            order
                .iter()
                .map(|c| format!("{c} IS NOT NULL"))
                .collect::<Vec<_>>()
                .join(" AND "),
            order.join(", "),
        );
        let (mut left, mut right) = (
            source.prepare(&sql).expect("left"),
            replayed.prepare(&sql).expect("right"),
        );
        let n = names.len();
        let (mut left, mut right) = (
            left.query([]).expect("left"),
            right.query([]).expect("right"),
        );
        loop {
            match (left.next().expect("left"), right.next().expect("right")) {
                (Some(l), Some(r)) => {
                    compared += 1;
                    let same = (0..n).all(|i| {
                        l.get::<_, Sql>(i).expect("left") == r.get::<_, Sql>(i).expect("right")
                    });
                    differ += u64::from(!same);
                }
                (None, None) => break,
                _ => {
                    extra += 1;
                    break;
                }
            }
        }
    }
    println!(
        "vacuumed copies byte-identical: {identical}; whole {whole:.1} MB, tables and their indexes {no_log:.1}, log {:.1}, its index {:.1}; \
         rows compared to the live copy {compared}, differing {differ}, tables of another length {extra}",
        no_index - no_log,
        whole - no_index,
    );
    let _ = std::fs::remove_file(a_copy);
    let _ = std::fs::remove_file(b_copy);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args[..] {
        ["census", copy] => census(copy),
        ["write", copy, out] => write(copy, out),
        ["replay", from, out] => replay(from, out),
        ["check", copy, a, b, dir] => check(copy, a, b, dir),
        _ => panic!("census|write|replay|check: see the file's first lines"),
    }
}
