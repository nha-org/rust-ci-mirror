//! A read of some seconds, shown from its last result: the server routes on one thread, so a view shows the last read and,
//! past `FRESH`, starts the next on its own thread. /agents and /tools read this way (/code-review of rust-access-map: one
//! copy, so a fix reaches both).

use std::sync::Mutex;

/// How old a read may be before a view starts the next.
const FRESH: u64 = 300;

/// The last read and when it started (Unix seconds); `reading` while a thread reads.
pub struct Read<T> {
    last: Option<(T, u64)>,
    /// Read by /agents' test, waiting for its read to end.
    pub reading: bool,
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
