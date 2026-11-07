//! The core of section 3: the data model, the registry and the ports. It depends on nothing of ours (crates/gate holds that).
pub mod registry;
pub mod tasks;

pub use registry::Hit;

/// The ping answer the Elixir dash gives (`dash ping`), the first command the Rust app will answer too.
pub fn ping() -> &'static str {
    "pong"
}

pub struct Scanned {
    pub rows: i64,
    pub bytes: i64,
}

/// The store port: another store is another adapter, and nothing above it changes.
pub trait Store {
    /// One full read, so the whole store passes through its cache.
    fn scan(&self) -> Result<Scanned, String>;
    /// The facts holding `query`, at most `limit`.
    fn search(&self, query: &str, limit: u32) -> Result<Vec<Hit>, String>;
}

/// The clock port: what the hybrid `Clock` reads. The store's adapter is the system's, the simulator's is its own time
/// (rfc-rust-unified section 17; nanolathe's I6: a simulation never reads the wall clock).
pub trait WallClock {
    fn now_ms(&self) -> i64;
}

/// The randomness port: one stream, whose call order is behavior (nanolathe's I4), so one seed gives one run.
pub trait Rng {
    fn next_u64(&mut self) -> u64;

    /// Uniform enough for a fault: the modulo's bias is nothing at the small `n` a simulation draws.
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// The scheduler port: when each event runs. Events due at one time run in the order they were scheduled.
pub trait Scheduler<E> {
    fn at(&mut self, due_ms: i64, event: E);
    /// The next event, with its due time.
    fn pop(&mut self) -> Option<(i64, E)>;
}

/// The sync port of section 3: what the exchange of ops asks of a store.
pub trait OpLog {
    fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String>;
    fn apply(&mut self, ops: &[Op]) -> Result<usize, String>;
    fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String>;
    /// One number for the tables and the log, equal on two replicas only if they hold the same rows and ops.
    fn state_hash(&self) -> Result<u64, String>;
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

/// The stamp no op may pass. A constant, never « now plus a drift »: every replica must accept the same ops, or they part.
pub const MAX_STAMP: i64 = 1 << 62;

/// Section 14's hybrid clock (Kulkarni et al., 2014), one integer so it sorts as it stores: milliseconds in the high bits,
/// a counter in the low 16. It follows wall time and never goes back, so a replica's next stamp passes all it has seen.
pub struct Clock {
    last: i64,
}

impl Clock {
    pub fn starting_at(last: i64) -> Self {
        Self { last }
    }

    pub fn tick(&mut self, now_ms: i64) -> i64 {
        self.last = (self.last + 1).max(now_ms << 16);
        self.last
    }

    pub fn observe(&mut self, stamp: i64) {
        self.last = self.last.max(stamp);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ping_answers_pong() {
        let _budget = dash_meter::memory::TestBudget::new();
        assert_eq!(super::ping(), "pong");
    }

    #[test]
    fn the_clock_never_goes_back_and_passes_what_it_saw() {
        let _budget = dash_meter::memory::TestBudget::new();
        let mut clock = super::Clock::starting_at(0);
        let first = clock.tick(1_000);
        assert_eq!(first, 1_000 << 16, "it follows wall time");
        assert!(clock.tick(1_000) > first, "same millisecond");
        let second = clock.tick(1);
        assert!(second > first, "the wall clock went back");
        clock.observe(5_000 << 16);
        assert!(clock.tick(2_000) > 5_000 << 16, "a remote clock ahead");
        clock.observe(0);
        assert!(clock.tick(0) > 5_000 << 16, "a remote clock behind");
    }
}
