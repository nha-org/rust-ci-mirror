//! The docs table's op log, below both its readers: docs.rs (/v/ and the docs pass) writes it, the views tasks.rs starts
//! read it, and neither reaches the other (dash-gate's module-cycle rule).
use dash_store::oplog::Replica;
use std::path::Path;

pub const TABLE: &str = "docs";
const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS docs (id TEXT PRIMARY KEY, record TEXT, body TEXT) WITHOUT ROWID;";

/// The docs table's op log, `body` added to a table made before it (the schema is judged at open, so before the replica).
pub fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("docs.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    let has_body = (db.prepare("SELECT 1 FROM pragma_table_info('docs') WHERE name = 'body'"))
        .and_then(|mut q| q.exists([]))
        .map_err(|e| e.to_string())?;
    // two processes opening an older file at once both add it: the second one's duplicate is the first one's column (/code-review)
    if !has_body
        && let Err(e) = db.execute_batch("ALTER TABLE docs ADD COLUMN body TEXT")
        && !e.to_string().contains("duplicate column name")
    {
        return Err(e.to_string());
    }
    Replica::new(db, "dash").map_err(|e| e.to_string())
}
