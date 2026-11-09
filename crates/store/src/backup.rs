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
        let tables = "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name";
        let names: Vec<String> = db
            .prepare(tables)
            .and_then(|mut q| q.query_map([], |r| r.get(0)).and_then(Iterator::collect))
            .map_err(|e| format!("{store}: {e}"))?;
        for table in names {
            let n: i64 = db
                .query_row(
                    &format!("SELECT count(*) FROM \"{}\"", table.replace('"', "\"\"")),
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| format!("{store}: {e}"))?;
            found.push((store.clone(), table, n as u64));
        }
    }
    Ok(found)
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
        let sql = dump(&dir.0.join("tasks.sqlite")).expect("its SQL");
        assert!(
            sql.contains("VALUES('it''s',0.1,X'00ff')") && sql.contains("CREATE INDEX by_n"),
            "{sql}"
        );
    }
}
