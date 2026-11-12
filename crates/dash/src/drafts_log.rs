//! The drafts table's op log, drafts.sqlite: below both its writer (drafts.rs) and the views tasks.rs starts, which read it
//! for pending drafts, so neither reaches the other (dash-gate's module-cycle rule), as docs_log.rs.
use dash_store::oplog::Replica;
use std::path::Path;

pub fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("drafts.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(dash_drafts::SCHEMA)
        .map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}
