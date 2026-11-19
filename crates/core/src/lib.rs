//! The core of section 3, the host's side of dash-sdk: the data model, the registry and the ports. It depends on dash-sdk
//! alone of ours (crates/gate holds that).
pub mod forecast;
pub mod form;
pub mod oplog;
pub mod plugin;
pub mod projects;
pub mod registry;
pub mod switches;
pub mod tasks;

/// A contribution with its origin stamped, `module_path!()` where it is built (rfc-superapp-plugins section 3): its tools'
/// and slots' plugin is that crate's, never a name typed beside it. `contribution! { tools: vec![..] }`, the kinds left out
/// empty.
#[macro_export]
macro_rules! contribution {
    ($($kind:ident: $entries:expr),* $(,)?) => {
        $crate::registry::Contribution {
            origin: ::core::module_path!().into(),
            $($kind: $entries,)*
            ..::core::default::Default::default()
        }
    };
}

// the sdk's names, kept at this root while a plugin still takes dash-core (task untangle-sdk-crate: each moves to dash-sdk)
pub use dash_sdk::{
    ANCHORED, Answer, Asked, COMMENT_ON_ANCHOR, Channel, Check, Command, Connection,
    ContextProvider, Draw, Drawing, Element, FieldWrite, Follows, HTML, Host, JSON, KEY_SEPARATOR,
    Later, MONTHS, Message, Op, OpLog, Post, Program, Ran, Recurring, Row, SignIn, TEXT, Tick,
    Transport, Tree, Value, View, WallClock, channel, context, date, days, element, form_decoded,
    is_day, iso, json, lines, on_anchor, param, placed, post, segment_decoded, stamp, submit,
};
#[cfg(feature = "memory-log")]
pub use dash_sdk::{MemoryLog, StandIn};
pub use form::form_parts;
pub use oplog::{Cursor, Fnv, Pages};
// the host's own pages (task rust-move-core-ports)
pub use plugin::{BOARD, Page, Reads, Warmer, held, page};
pub use projects::{command as projects_command, folder as project_folder};
pub use registry::Hit;
// main was red by one reach once #1610 and #1615 had merged past each other: the comments plugin takes it here
pub use tasks::{Holder, PUSH_TIMED_OUT, Triaged, holders};
// the hot files' check and its reviewers' model (task hot-files-fable): the comments, forge and orch plugins take them here
pub use registry::{Automation, Does, Every};
pub use tasks::{
    HOT_DAYS, HOT_FIXES, HOT_MODEL, fix_counts, fix_log, hot, hot_why, pr_files, title_task,
};

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
