//! /settings' backup (task dash-settings-backup; @nha, 2026-11-09: « needs a settings tab, and a backup sql in there. sql ->
//! zip, and reimport with overwrite confirmation with count of entries per type. »): every SQLite store of the data folder as
//! SQL text, one file a store, in one zip; and what a zip holds, the count of rows of each table of each store.
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The data folder's stores, by name.
pub fn stores(data: &Path) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(data).into_iter().flatten().flatten();
    let mut found: Vec<PathBuf> = entries
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sqlite"))
        .collect();
    found.sort();
    found
}

/// A store as SQL: each table's CREATE then its rows, then its indexes, in one transaction. Read in one transaction too, so a
/// store the server writes meanwhile is dumped as one moment.
pub fn dump(path: &Path) -> Result<String, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", path.display());
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(named)?;
    db.execute_batch("BEGIN").map_err(named)?;
    let sql = "SELECT type, name, sql FROM sqlite_master WHERE sql NOT NULL AND name NOT LIKE 'sqlite_%' \
               ORDER BY type <> 'table', name";
    let mut schema = db.prepare(sql).map_err(named)?;
    let schema: Vec<(String, String, String)> = schema
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .and_then(Iterator::collect)
        .map_err(named)?;
    let mut out = String::from("BEGIN;\n");
    for (kind, name, create) in &schema {
        out += &format!("{create};\n");
        if kind != "table" {
            continue;
        }
        let quoted = format!("\"{}\"", name.replace('"', "\"\""));
        let mut rows = db
            .prepare(&format!("SELECT * FROM {quoted}"))
            .map_err(named)?;
        let columns = rows.column_count();
        let mut rows = rows.query([]).map_err(named)?;
        while let Some(row) = rows.next().map_err(named)? {
            let values: Vec<String> = (0..columns)
                .map(|i| row.get_ref(i).map(literal))
                .collect::<Result<_, _>>()
                .map_err(named)?;
            out += &format!("INSERT INTO {quoted} VALUES({});\n", values.join(","));
        }
    }
    db.execute_batch("COMMIT").map_err(named)?;
    Ok(out + "COMMIT;\n")
}

/// A value as SQLite reads it back: a real keeps every digit, a text its quotes doubled, a blob its hex.
fn literal(value: ValueRef) -> String {
    match value {
        ValueRef::Null => "NULL".into(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => format!("{f:?}"),
        ValueRef::Text(t) => format!("'{}'", String::from_utf8_lossy(t).replace('\'', "''")),
        ValueRef::Blob(b) => format!(
            "X'{}'",
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        ),
    }
}

/// Every store of `data` as `<store>.sql` in one zip.
pub fn zip(data: &Path) -> Result<Vec<u8>, String> {
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for store in stores(data) {
        let name = store.file_stem().unwrap_or_default().to_string_lossy();
        out.start_file(format!("{name}.sql"), options)
            .map_err(|e| e.to_string())?;
        out.write_all(dump(&store)?.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    Ok(out.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Each table of one store and its rows, by name.
fn counted(store: &str, db: &Connection) -> Result<Vec<(String, String, u64)>, String> {
    let named = |e: rusqlite::Error| format!("{store}: {e}");
    let tables = "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name";
    let names: Vec<String> = db
        .prepare(tables)
        .and_then(|mut q| q.query_map([], |r| r.get(0)).and_then(Iterator::collect))
        .map_err(named)?;
    let mut found = vec![];
    for table in names {
        let sql = format!("SELECT count(*) FROM \"{}\"", table.replace('"', "\"\""));
        let n: i64 = db.query_row(&sql, [], |r| r.get(0)).map_err(named)?;
        found.push((store.to_string(), table, n as u64));
    }
    Ok(found)
}

/// What a backup made now would hold, counted in the stores themselves: /settings shows it on each view, so it never builds
/// the zip only to count it (67 ms a view on 2026-11-10, the zip and its read back; task settings-counts-direct).
pub fn held(data: &Path) -> Result<Vec<(String, String, u64)>, String> {
    let mut found = vec![];
    for path in stores(data) {
        let store = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| format!("{store}: {e}"))?;
        // one moment for the whole store, as `dump` reads it
        db.execute_batch("BEGIN")
            .map_err(|e| format!("{store}: {e}"))?;
        found.extend(counted(&store, &db)?);
    }
    Ok(found)
}

/// What a backup holds: each store, each of its tables and the rows it would bring back, read by running its SQL in memory.
pub fn counts(zip: &[u8]) -> Result<Vec<(String, String, u64)>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    let mut found = vec![];
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let store = file.name().trim_end_matches(".sql").to_string();
        let mut sql = String::new();
        file.read_to_string(&mut sql)
            .map_err(|e| format!("{store}: {e}"))?;
        let db = Connection::open_in_memory().map_err(|e| e.to_string())?;
        db.execute_batch(&sql)
            .map_err(|e| format!("{store}: {e}"))?;
        found.extend(counted(&store, &db)?);
    }
    Ok(found)
}

/// Each store of a backup put back, one transaction a store: its tables' rows replaced by the backup's. A store the folder
/// lacks is made from it. A backup table the live store lacks, or whose columns differ, refuses that store whole, nothing of it
/// written, the others still put back. What was done, a line a store.
pub fn restore(data: &Path, zip: &[u8]) -> Result<Vec<String>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    let mut said = vec![];
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let store = file.name().trim_end_matches(".sql").to_string();
        if store.contains(['/', '\\']) || store.starts_with('.') {
            said.push(format!("{store}: refused, not a store's name"));
            continue;
        }
        let mut sql = String::new();
        file.read_to_string(&mut sql)
            .map_err(|e| format!("{store}: {e}"))?;
        let staged = data.join(format!("{store}.restoring"));
        let _ = std::fs::remove_file(&staged);
        let built = Connection::open(&staged).and_then(|db| db.execute_batch(&sql));
        let done = built
            .map_err(|e| e.to_string())
            .and_then(|()| put_back(&data.join(format!("{store}.sqlite")), &staged));
        let _ = std::fs::remove_file(&staged);
        said.push(match done {
            Ok(n) => format!("{store}: {n} tables replaced"),
            Err(e) => format!("{store}: refused, nothing written: {e}"),
        });
    }
    Ok(said)
}

/// The tables of `staged` into `live`, in one transaction: each one's rows deleted, then the backup's inserted.
fn put_back(live: &Path, staged: &Path) -> Result<usize, String> {
    if !live.exists() {
        std::fs::copy(staged, live).map_err(|e| e.to_string())?;
        return Ok(0);
    }
    let db = Connection::open(live).map_err(|e| e.to_string())?;
    db.execute("ATTACH ?1 AS staged", [staged.to_string_lossy()])
        .map_err(|e| e.to_string())?;
    let columns = |schema: &str, table: &str| -> rusqlite::Result<Vec<String>> {
        let sql = format!("SELECT name FROM pragma_table_info(?1, '{schema}') ORDER BY cid");
        db.prepare(&sql)?
            .query_map([table], |r| r.get(0))?
            .collect()
    };
    let tables: Vec<String> = db
        .prepare("SELECT name FROM staged.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .and_then(|mut q| q.query_map([], |r| r.get(0)).and_then(Iterator::collect))
        .map_err(|e| e.to_string())?;
    let replaced = (|| -> Result<usize, String> {
        db.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        for table in &tables {
            let (here, there) = (columns("main", table), columns("staged", table));
            match (here, there) {
                (Ok(here), Ok(there)) if !here.is_empty() && here == there => {}
                _ => return Err(format!("{table}: not the same columns here")),
            }
            let quoted = format!("\"{}\"", table.replace('"', "\"\""));
            db.execute_batch(&format!("DELETE FROM main.{quoted}; INSERT INTO main.{quoted} SELECT * FROM staged.{quoted};"))
                .map_err(|e| format!("{table}: {e}"))?;
        }
        db.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        Ok(tables.len())
    })();
    if replaced.is_err() {
        let _ = db.execute_batch("ROLLBACK");
    }
    let _ = db.execute_batch("DETACH staged");
    replaced
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, memory::TestBudget};

    /// Done-when 3 of dash-settings-backup, its download half: two stores dump, zip and read back, each table its rows, a
    /// text with a quote, a real and a blob kept as they were.
    #[test]
    fn every_store_dumps_zips_and_counts_back_as_it_was() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("backup");
        let a = Connection::open(dir.0.join("tasks.sqlite")).expect("a store");
        a.execute_batch(
            "CREATE TABLE tasks (id TEXT PRIMARY KEY, n REAL, b BLOB); CREATE INDEX by_n ON tasks (n);
             INSERT INTO tasks VALUES ('it''s', 0.1, X'00ff'), ('b', NULL, NULL);",
        )
        .expect("rows");
        let b = Connection::open(dir.0.join("comments.sqlite")).expect("a store");
        b.execute_batch("CREATE TABLE queue (id TEXT); INSERT INTO queue VALUES ('c');")
            .expect("rows");
        std::fs::write(dir.0.join("codemap.json"), "{}").expect("not a store");
        let zipped = zip(&dir.0).expect("a zip");
        let counts = counts(&zipped).expect("its counts");
        let row = |s: &str, t: &str, n: u64| (s.to_string(), t.to_string(), n);
        assert_eq!(
            counts,
            [row("comments", "queue", 1), row("tasks", "tasks", 2)]
        );
        // counted in the stores, the same as in their backup
        assert_eq!(held(&dir.0).expect("held"), counts);
        let sql = dump(&dir.0.join("tasks.sqlite")).expect("its SQL");
        assert!(
            sql.contains("VALUES('it''s',0.1,X'00ff')") && sql.contains("CREATE INDEX by_n"),
            "{sql}"
        );
    }

    /// Done-when 2 of dash-settings-backup, the store's half: a backup put back replaces each table's rows; a store whose
    /// table has other columns is refused whole and keeps its rows.
    #[test]
    fn a_backup_put_back_replaces_rows_and_a_changed_store_is_refused_whole() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("restore");
        let live = Connection::open(dir.0.join("tasks.sqlite")).expect("a store");
        live.execute_batch("CREATE TABLE tasks (id TEXT); INSERT INTO tasks VALUES ('a'), ('b');")
            .expect("rows");
        let other = Connection::open(dir.0.join("comments.sqlite")).expect("a store");
        other
            .execute_batch("CREATE TABLE queue (id TEXT); INSERT INTO queue VALUES ('q');")
            .expect("rows");
        let zipped = zip(&dir.0).expect("a zip");
        live.execute_batch("INSERT INTO tasks VALUES ('c');")
            .expect("a row since");
        other
            .execute_batch("DROP TABLE queue; CREATE TABLE queue (id TEXT, more TEXT); INSERT INTO queue VALUES ('x', 'y');")
            .expect("changed");
        let said = restore(&dir.0, &zipped).expect("put back");
        assert_eq!(said[1], "tasks: 1 tables replaced", "{said:?}");
        assert!(
            said[0].starts_with("comments: refused, nothing written: queue"),
            "{said:?}"
        );
        let n: i64 = live
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 2, "the backup's two rows, the later one gone");
        let kept: i64 = other
            .query_row("SELECT count(*) FROM queue", [], |r| r.get(0))
            .expect("count");
        assert_eq!(kept, 1, "the refused store keeps its row");
        assert!(
            !dir.0.join("tasks.restoring").exists(),
            "nothing staged is left"
        );
    }
}
