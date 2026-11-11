//! The core of section 3: the data model, the registry and the ports. It depends on nothing of ours (crates/gate holds that).
pub mod registry;
pub mod tasks;

pub use registry::Hit;

/// The ping answer the Elixir dash gives (`dash ping`), the first command the Rust app will answer too.
pub fn ping() -> &'static str {
    "pong"
}

/// The months' short names, January first: a date's month as mail, tasks and the Gantt write it.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since 1970-01-01 of a date written 2026-11-15 (its first ten characters), the civil calendar's arithmetic. A day
/// that does not exist (2026-02-31, 2026-13-01) or a field that is not four or two digits is none: only a date that reads
/// back the same is one.
pub fn days(date: &str) -> Option<i64> {
    let date = date.get(..10)?;
    let mut p = date.split('-').map(|n| n.parse::<i64>().ok());
    let (y, m, d) = (p.next()??, p.next()??, p.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let n = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (self::date(n) == date).then_some(n)
}

/// The date of a day number since 1970-01-01, written 2026-11-15.
pub fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A time in Unix milliseconds as the old app's `Status.stamp` writes it: « 2026-11-09 00:15 UTC ». /watchers' beats and a task's
/// Activity both.
pub fn stamp(ms: i64) -> String {
    let s = ms.div_euclid(1000);
    let (day, min) = (s.div_euclid(86_400), s.rem_euclid(86_400) / 60);
    format!("{} {:02}:{:02} UTC", date(day), min / 60, min % 60)
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
    fn days_and_dates_agree_and_a_day_that_does_not_exist_is_none() {
        let _budget = dash_meter::memory::TestBudget::new();
        assert_eq!(super::days("1970-01-01"), Some(0));
        assert_eq!(super::date(20_772), "2026-11-15");
        assert_eq!(super::days("2026-11-15T04:59:59Z"), Some(20_772));
        assert_eq!(super::days("2024-02-29"), Some(19_782), "a leap day");
        for bad in [
            "2026-13-01",
            "2026-02-31",
            "2026-02-29",
            "2026x11x15",
            "+026-11-15",
            "2026-1-150",
        ] {
            assert_eq!(super::days(bad), None, "{bad}");
        }
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
