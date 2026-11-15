//! The core of section 3: the data model, the registry and the ports. It depends on nothing of ours (crates/gate holds that).
pub mod forecast;
pub mod form;
mod host;
pub mod oplog;
pub mod plugin;
pub mod projects;
pub mod registry;
pub mod switches;
pub mod tasks;

// the root names a plugin takes (ARCHITECTURE.md section 5 rule 18): dash-forge's `dash projects follow`
pub use form::form_decoded;
#[cfg(feature = "memory-log")]
pub use oplog::MemoryLog;
pub use oplog::{FieldWrite, Fnv, KEY_SEPARATOR, Op, OpLog, Value, WallClock};
pub use projects::{command as projects_command, folder as project_folder};
// what a plugin registers and what the host hands it (task rust-move-core-ports)
pub use plugin::{
    Answer, Asked, BOARD, Channel, Check, Command, ContextProvider, Drawing, Element, HTML, Host,
    JSON, Later, Message, Page, Post, Reads, Recurring, Row, SignIn, TEXT, TaskPorts, Tick,
    Transport, Tree, View, Warmer, channel, context, element, page, param, placed, post,
    segment_decoded,
};
pub use registry::Hit;
// a tree's props, built without a crate of their own
pub use serde_json::json;
// main was red by one reach once #1610 and #1615 had merged past each other: the comments plugin takes it here
pub use tasks::Triaged;

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

/// A day written whole, « 2026-11-15 » and nothing after it, that exists.
pub fn is_day(s: &str) -> bool {
    s.len() == 10 && days(s).is_some()
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

/// A time in Unix seconds in UTC, as GitHub writes them: « 2026-11-11T16:44:10Z ».
pub fn iso(secs: i64) -> String {
    let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        date(days),
        s / 3600,
        s / 60 % 60,
        s % 60
    )
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
    #[dash_meter::dash_test]
    fn ping_answers_pong() {
        assert_eq!(super::ping(), "pong");
    }

    #[dash_meter::dash_test]
    fn days_and_dates_agree_and_a_day_that_does_not_exist_is_none() {
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

    #[dash_meter::dash_test]
    fn the_clock_never_goes_back_and_passes_what_it_saw() {
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
