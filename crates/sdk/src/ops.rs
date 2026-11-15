//! The op log's types, the write path a plugin's rows take (rfc-rust-unified section 14), and the clock that stamps them.
//! The store's adapter implements `OpLog`; its hash of a row (`dash_core::oplog::Fnv`) is the host's.

/// The clock port. The store's adapter is the system's, the simulator's is its own time (rfc-rust-unified section 17;
/// nanolathe's I6: a simulation never reads the wall clock).
pub trait WallClock {
    fn now_ms(&self) -> i64;
}

/// The sync port: what the exchange of ops asks of a store.
pub trait OpLog {
    fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String>;
    fn apply(&mut self, ops: &[Op]) -> Result<usize, String>;
    fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String>;
    /// One number for the tables and the log, equal on two replicas only if they hold the same rows and ops.
    fn state_hash(&self) -> Result<u64, String>;
}

/// A column's value, as SQLite types it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// One field's new value: what a writer asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldWrite {
    pub table: String,
    /// The row's primary key; a composite key's parts joined by `KEY_SEPARATOR`.
    pub row: String,
    pub field: String,
    pub value: Value,
}

pub const KEY_SEPARATOR: char = '\u{1f}';

/// Section 14: one field write, with its clock stamp and its writer. A field's last write wins, by `(stamp, writer)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub stamp: i64,
    pub writer: String,
    pub write: FieldWrite,
}
