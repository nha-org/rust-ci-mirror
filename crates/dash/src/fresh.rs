//! A read of some seconds, shown from its last result: the server routes on one thread, so a view shows the last read and,
//! past `FRESH`, starts the next on its own thread. /agents and /tools read this way (/code-review of rust-access-map: one
//! copy, so a fix reaches both).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How old a read may be before a view starts the next.
const FRESH: u64 = 300;

/// The last read and when it started (Unix seconds); `reading` while a thread reads.
pub struct Read<T> {
    last: Option<(T, u64)>,
    reading: bool,
}

impl<T> Read<T> {
    pub const fn new() -> Self {
        Self {
            last: None,
            reading: false,
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
    let mut read = cell.lock().map_err(|e| e.to_string())?;
    let stale = read.last.as_ref().is_none_or(|(_, at)| now >= at + FRESH);
    if stale && !read.reading {
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
    Ok(read.last.clone())
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
}
