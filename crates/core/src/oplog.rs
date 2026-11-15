//! The op log's types, the write path every replica shares (section 14): below the root, so a module the root re-exports
//! can take them with no module loop (task rust-move-core-ports), and the clock that stamps its ops.

/// The clock port: what the hybrid `Clock` reads. The store's adapter is the system's, the simulator's is its own time
/// (rfc-rust-unified section 17; nanolathe's I6: a simulation never reads the wall clock).
pub trait WallClock {
    fn now_ms(&self) -> i64;
}

/// The sync port of section 3: what the exchange of ops asks of a store.
pub trait OpLog {
    fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String>;
    fn apply(&mut self, ops: &[Op]) -> Result<usize, String>;
    fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String>;
    /// One number for the tables and the log, equal on two replicas only if they hold the same rows and ops.
    fn state_hash(&self) -> Result<u64, String>;
}

/// The op log in memory, for tests: its stamps count up, as one replica's do. `.1`: every write refused, as a full disk would.
#[cfg(feature = "memory-log")]
#[derive(Default)]
pub struct MemoryLog(pub Vec<Op>, pub bool);

#[cfg(feature = "memory-log")]
impl OpLog for MemoryLog {
    fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String> {
        if self.1 {
            return Err("disk full".into());
        }
        let ops: Vec<Op> = writes
            .iter()
            .map(|w| Op {
                stamp: self.0.len() as i64 + 1,
                writer: "t".into(),
                write: w.clone(),
            })
            .collect();
        self.0.extend(ops.iter().cloned());
        Ok(ops)
    }
    fn apply(&mut self, _ops: &[Op]) -> Result<usize, String> {
        unreachable!("a test log takes no remote ops")
    }
    fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String> {
        let from = after.map_or(0, |(s, _)| s);
        let ops = self.0.iter().filter(|o| o.stamp > from);
        Ok(ops.take(limit as usize).cloned().collect())
    }
    fn state_hash(&self) -> Result<u64, String> {
        unreachable!("a test log has no tables")
    }
}

/// FNV-1a: stable across releases and machines, which std's hashers do not promise. A check against accidents, not an adversary.
pub struct Fnv(u64);

impl Fnv {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
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

impl Value {
    /// Its kind and bytes, the length first so that two values never read as one.
    pub fn digest(&self, hash: &mut Fnv) {
        let (kind, bytes): (u8, &[u8]) = match self {
            Self::Null => (0, &[]),
            Self::Int(i) => {
                hash.bytes(&[1]);
                hash.bytes(&i.to_le_bytes());
                return;
            }
            Self::Real(f) => {
                hash.bytes(&[2]);
                hash.bytes(&f.to_bits().to_le_bytes());
                return;
            }
            Self::Text(t) => (3, t.as_bytes()),
            Self::Blob(b) => (4, b),
        };
        hash.bytes(&[kind]);
        hash.bytes(&(bytes.len() as u64).to_le_bytes());
        hash.bytes(bytes);
    }
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

/// How a reader reads a log: the ops after a `(stamp, writer)`, at most `limit`, empty once caught up. `OpLog::ops_after`, or
/// a source that opens its file late (the views').
pub type Pages<'a> = dyn FnMut(Option<(i64, &str)>, i64) -> Result<Vec<Op>, String> + 'a;

/// Where a reader of a log stands: past the last op it read, kept whole, and its predecessor's place, so a log replaced in
/// place (a backup restored under a running reader, a later write taking the same stamp) is told from a log that grew (task
/// untangle-serve-no-queue's /code-review). The comments' follower and the views read through it.
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    before: Option<(i64, String)>,
    last: Option<Op>,
}

impl Cursor {
    /// Whether the log no longer holds the last op read, the same op, right after its predecessor: then the cursor starts
    /// over, and what its reader built from the old log is to be dropped.
    pub fn replaced(&mut self, read: &mut Pages) -> Result<bool, String> {
        let Some(last) = &self.last else {
            return Ok(false);
        };
        let moved = read(at(&self.before), 1)?.first() != Some(last);
        if moved {
            *self = Self::default();
        }
        Ok(moved)
    }

    /// Where to read after: the last op read's place.
    pub fn after(&self) -> Option<(i64, &str)> {
        self.last.as_ref().map(|op| (op.stamp, op.writer.as_str()))
    }

    /// Past `ops`, a page just read after `after()`.
    pub fn passed(&mut self, ops: &[Op]) {
        let Some((last, rest)) = ops.split_last() else {
            return;
        };
        let before = rest.last().or(self.last.as_ref());
        self.before = before.map(|op| (op.stamp, op.writer.clone()));
        self.last = Some(last.clone());
    }

    /// Each page after the cursor, `page` ops a read, the cursor moved past each: none once caught up.
    pub fn tail(
        &mut self,
        read: &mut Pages,
        page: i64,
        mut each: impl FnMut(&[Op]),
    ) -> Result<(), String> {
        loop {
            let ops = read(self.after(), page)?;
            if ops.is_empty() {
                return Ok(());
            }
            each(&ops);
            self.passed(&ops);
        }
    }
}

fn at(key: &Option<(i64, String)>) -> Option<(i64, &str)> {
    key.as_ref().map(|(s, w)| (*s, w.as_str()))
}
