//! The op log's hash and a reader's cursor, the host's side of the write path every replica shares (section 14); its types are the sdk's.
use dash_sdk::{Op, Value};

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

    /// A value's kind and bytes, the length first so that two values never read as one.
    pub fn value(&mut self, value: &Value) {
        let (kind, bytes): (u8, &[u8]) = match value {
            Value::Null => (0, &[]),
            Value::Int(i) => {
                self.bytes(&[1]);
                self.bytes(&i.to_le_bytes());
                return;
            }
            Value::Real(f) => {
                self.bytes(&[2]);
                self.bytes(&f.to_bits().to_le_bytes());
                return;
            }
            Value::Text(t) => (3, t.as_bytes()),
            Value::Blob(b) => (4, b),
        };
        self.bytes(&[kind]);
        self.bytes(&(bytes.len() as u64).to_le_bytes());
        self.bytes(bytes);
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
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
