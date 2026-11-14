//! The cache and budget ports' adapters (task rust-jev-tool): a tool's answers by question, and each key's spend. Text rows,
//! as the audit log's: task rust-cache-crate's on-disk store takes the cache's place behind the same port.
//! ponytail: an answer is kept forever, right for Jev, whose answer to a text never changes; a tool reading what changes
//! (a listing, a status) waits for rust-cache-crate's expiry (its done-when 4) or says `effects`.
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) const TABLES: &str =
    "CREATE TABLE IF NOT EXISTS tool_cache (key TEXT PRIMARY KEY, answer TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS key_spend (key TEXT PRIMARY KEY, spent REAL NOT NULL);";

/// A cache read that fails is a miss: the tool answers it, and its put says why.
pub(crate) fn get(db: &Connection, key: &str) -> Option<String> {
    let q = "SELECT answer FROM tool_cache WHERE key = ?1";
    db.query_row(q, [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub(crate) fn put(db: &Connection, key: &str, answer: &str) -> Result<(), String> {
    let q = "INSERT OR REPLACE INTO tool_cache (key, answer) VALUES (?1, ?2)";
    db.execute(q, params![key, answer])
        .map(drop)
        .map_err(|e| e.to_string())
}

pub(crate) fn charge(db: &Connection, key: &str, cost: f64) -> Result<(), String> {
    let q = "INSERT INTO key_spend (key, spent) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET spent = spent + ?2";
    db.execute(q, params![key, cost])
        .map(drop)
        .map_err(|e| e.to_string())
}

/// A store `open_read` found without the table, one older than the port, has spent nothing yet (the live store read from a
/// sandbox, before any process that writes it made the table).
pub(crate) fn spent(db: &Connection, key: &str) -> Result<f64, String> {
    let table = "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'key_spend'";
    let made = db
        .query_row(table, [], |_| Ok(()))
        .optional()
        .map_err(|e| e.to_string())?;
    if made.is_none() {
        return Ok(0.0);
    }
    let q = "SELECT spent FROM key_spend WHERE key = ?1";
    let spent = db.query_row(q, [key], |r| r.get(0)).optional();
    spent
        .map(Option::unwrap_or_default)
        .map_err(|e| e.to_string())
}
