//! Every table of the drafts' file is `(id, record)`, a record one JSON text: its read and its write, below every module that
//! keeps one, so none reaches another for them.
use dash_sdk::Connection;
use std::collections::BTreeMap;

/// Each row of `table` as its id and its record.
pub fn records(db: &Connection, table: &str) -> Result<BTreeMap<String, String>, String> {
    let e = |e: dash_sdk::rusqlite::Error| e.to_string();
    let mut read = db
        .prepare(&format!("SELECT id, record FROM {table}"))
        .map_err(e)?;
    let rows = read.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
        ))
    });
    rows.map_err(e)?.collect::<Result<_, _>>().map_err(e)
}

/// Row `id` of `table` holds `record` now. Inside the caller's transaction.
pub fn put(db: &Connection, table: &str, id: &str, record: &str) -> Result<(), String> {
    let put = format!(
        "INSERT INTO {table} (id, record) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET record = excluded.record"
    );
    db.execute(&put, [id, record])
        .map(drop)
        .map_err(|e| e.to_string())
}

/// A file in memory with `schema`'s tables, for a test.
#[cfg(test)]
pub fn memory(schema: &[&str]) -> Connection {
    let db = Connection::open_in_memory().expect("in memory");
    schema
        .iter()
        .for_each(|s| db.execute_batch(s).expect("schema"));
    db
}
