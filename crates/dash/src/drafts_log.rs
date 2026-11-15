//! The drafts table's op log, drafts.sqlite: below both its writer (drafts.rs) and the views tasks.rs starts, which read it
//! for pending drafts, so neither reaches the other (dash-gate's module-cycle rule), as docs_log.rs. Opened as the plugin's
//! own commands open it, through the store's one opener.
use dash_store::oplog::Replica;
use std::path::Path;

pub fn log(dir: &Path) -> Result<Replica, String> {
    let file = dir.join(dash_drafts::command::FILE);
    dash_store::replica(&file, &dash_drafts::command::SCHEMA)
}
