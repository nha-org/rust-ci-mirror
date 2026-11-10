//! A read of some seconds, shown from its last result: the server routes on one thread, so a view shows the last read and,
//! past `FRESH`, starts the next on its own thread. /agents and /tools read this way (/code-review of rust-access-map: one
//! copy, so a fix reaches both).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How old a read may be before a view starts the next.
const FRESH: u64 = 300;

/// How long a first read may run before a line says it has no answer: inside the 20 s a line may read after a cold start
/// (task rust-today-doc; @nha, 2026-11-09: « mail: reading. » and nothing else).
pub const SLOW: u64 = 15;

/// The last read and when it started (Unix seconds); `reading` while a thread reads. `since`: when the first read was first
/// tried, 0 before; kept through a panic or a thread that did not start, so `bounded` ends those too (/code-review).
pub struct Read<T> {
    last: Option<(T, u64)>,
    reading: bool,
    since: u64,
}

impl<T> Read<T> {
    pub const fn new() -> Self {
        Self {
            last: None,
            reading: false,
            since: 0,
        }
    }
}

/// A page's note on its last read.
pub fn ago(now: u64, at: u64) -> String {
    format!("Read {} min ago.", now.saturating_sub(at) / 60)
}

/// The last read, after starting the next when it is past `FRESH` and none runs. A panic keeps the last read and frees the
/// next view to try again, logged under `what`.
pub fn last<T, F>(
    cell: &'static Mutex<Read<T>>,
    now: u64,
    what: &'static str,
    read_all: F,
) -> Result<Option<(T, u64)>, String>
where
    T: Clone + Send + 'static,
    F: FnOnce() -> T + Send + std::panic::UnwindSafe + 'static,
{
    kept(cell, now, what, read_all).map(|(last, _)| last)
}

/// `last`, and when the first read was first tried, both under one lock.
fn kept<T, F>(
    cell: &'static Mutex<Read<T>>,
    now: u64,
    what: &'static str,
    read_all: F,
) -> Result<(Option<(T, u64)>, u64), String>
where
    T: Clone + Send + 'static,
    F: FnOnce() -> T + Send + std::panic::UnwindSafe + 'static,
{
    let mut read = cell.lock().map_err(|e| e.to_string())?;
    let stale = read.last.as_ref().is_none_or(|(_, at)| now >= at + FRESH);
    if stale && !read.reading {
        if read.last.is_none() && read.since == 0 {
            read.since = now;
        }
        let started = std::thread::Builder::new().spawn(move || {
            let got = std::panic::catch_unwind(read_all);
            if got.is_err() {
                eprintln!("{what}: the read panicked; the last one stays");
            }
            if let Ok(mut read) = cell.lock() {
                if let Ok(got) = got {
                    read.last = Some((got, now));
                }
                read.reading = false;
            }
        });
        read.reading = started.is_ok();
    }
    Ok((read.last.clone(), read.since))
}

/// `last` for a line that must end: no first read ended `SLOW` seconds after the first try (one still running, one that
/// panicked, a thread that did not start) is its error, never a line reading for good. A read still running goes on, and a
/// view after it ends shows its count.
pub fn bounded<T, F>(
    cell: &'static Mutex<Read<T>>,
    now: u64,
    what: &'static str,
    read_all: F,
) -> Result<Option<(T, u64)>, String>
where
    T: Clone + Send + 'static,
    F: FnOnce() -> T + Send + std::panic::UnwindSafe + 'static,
{
    let (got, since) = kept(cell, now, what, read_all)?;
    if got.is_none() && now >= since + SLOW {
        return Err(format!("no count {SLOW} s after the first read"));
    }
    Ok(got)
}

/// The last read, waited for up to `limit` while the first one runs: an agent's text is the rows, never « ask again » (the
/// live check of rust-access-map got 202 from /tools after a restart, and a GET reads a 202 as no page).
/// ponytail: polls every 100 ms on the caller's thread, never the routing one; a Condvar if many wait at once.
pub fn waited<T: Clone>(
    cell: &Mutex<Read<T>>,
    limit: Duration,
) -> Result<Option<(T, u64)>, String> {
    let end = Instant::now() + limit;
    loop {
        let read = cell.lock().map_err(|e| e.to_string())?;
        if read.last.is_some() || !read.reading || Instant::now() >= end {
            return Ok(read.last.clone());
        }
        drop(read);
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// A first read still running is waited for and its result returned; one past the limit is still none.
    #[test]
    fn an_agent_waits_for_the_first_read() {
        let _budget = TestBudget::new();
        static CELL: Mutex<Read<u8>> = Mutex::new(Read::new());
        static SLOW: Mutex<Read<u8>> = Mutex::new(Read::new());
        let slow = |ms, n| {
            move || {
                std::thread::sleep(Duration::from_millis(ms));
                n
            }
        };
        assert_eq!(last(&CELL, 1000, "t", slow(300, 7)), Ok(None));
        assert_eq!(waited(&CELL, Duration::from_secs(5)), Ok(Some((7, 1000))));
        assert_eq!(last(&SLOW, 1000, "t", slow(2000, 9)), Ok(None));
        assert_eq!(waited(&SLOW, Duration::from_millis(200)), Ok(None));
    }

    /// A first read that never ends is an error `SLOW` seconds after it started, never « reading » for good; one that ended is
    /// its value, however late it is asked.
    #[test]
    fn a_bounded_first_read_ends_in_its_error() {
        let _budget = TestBudget::new();
        static HUNG: Mutex<Read<u8>> = Mutex::new(Read::new());
        static QUICK: Mutex<Read<u8>> = Mutex::new(Read::new());
        let hung = || {
            std::thread::sleep(Duration::from_secs(600));
            1
        };
        assert_eq!(bounded(&HUNG, 1000, "t", hung), Ok(None));
        assert_eq!(bounded(&HUNG, 1000 + SLOW - 1, "t", hung), Ok(None));
        let late = bounded(&HUNG, 1000 + SLOW, "t", hung);
        assert_eq!(late, Err(format!("no count {SLOW} s after the first read")));
        assert_eq!(bounded(&QUICK, 1000, "t", || 7), Ok(None));
        assert_eq!(waited(&QUICK, Duration::from_secs(5)), Ok(Some((7, 1000))));
        assert_eq!(bounded(&QUICK, 1000 + SLOW, "t", || 8), Ok(Some((7, 1000))));
    }

    /// A first read that panics, each try again, still ends in its error `SLOW` seconds after the first try: the retries do not
    /// move its start (/code-review: `reading` false after a panic left the line reading for good).
    #[test]
    fn a_first_read_that_panics_ends_in_its_error_too() {
        let _budget = TestBudget::new();
        static PANICS: Mutex<Read<u8>> = Mutex::new(Read::new());
        let boom = || -> u8 { panic!("a bad export") };
        for now in [1000, 1005, 1010] {
            assert_eq!(bounded(&PANICS, now, "t", boom), Ok(None));
            let _ = waited(&PANICS, Duration::from_secs(5));
        }
        assert_eq!(
            bounded(&PANICS, 1000 + SLOW, "t", boom),
            Err(format!("no count {SLOW} s after the first read"))
        );
    }
}
