//! The comment queue's file, which the watcher's rows share (dash_comments::watch, task rust-watch-port): a leaf, so a block,
//! a lesson or a Datadog push lands from any module (tasks, the front door, alerts) without a module loop through comments.
use dash_comments::watch::Item;
use dash_store::oplog::Replica;
use std::path::Path;

/// `dir`'s comments.sqlite: the comment queue and the watcher's rows.
pub(crate) fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("comments.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(dash_comments::SCHEMA)
        .map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// A row landed in `dir`'s queue: true when new. It wakes on the comment pass that finds it due.
pub(crate) fn land(dir: &Path, item: Item) -> Result<bool, String> {
    dash_comments::watch::land(&mut log(dir)?, item)
}

/// The watcher's rows in `dir`, by key.
#[cfg(test)]
pub(crate) fn watched(dir: &Path) -> Result<std::collections::BTreeMap<String, Item>, String> {
    dash_comments::watch::load(&log(dir)?)
}
