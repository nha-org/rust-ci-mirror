//! The core of section 3: the data model and the ports. It depends on nothing of ours (crates/gate holds that).

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
