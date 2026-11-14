//! A read of some seconds, shown from its last result: the server routes on one thread, so a view shows the last read and,
//! past `FRESH`, starts the next on its own thread. /agents and /tools read this way (/code-review of rust-access-map: one
//! copy, so a fix reaches both). A read kept on disk (`Read::saved`) is shown after a restart until the next ends, and each page
//! registers a `Warmer` that `dash serve` runs after start (task rust-warmup-queue; @nha, 2026-11-11: « /insights takes a long
//! time to load. reloading is fine. … perhaps an async queue of warming-ups at startup, each plugin can register something. »).

use serde_json::{Value, json};
use std::path::PathBuf;
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
    saved: Option<Saved<T>>,
}

/// Where a read is kept between restarts, and its JSON both ways.
struct Saved<T> {
    name: &'static str,
    to: fn(&T) -> Value,
    from: fn(&Value) -> Option<T>,
}

impl<T> Read<T> {
    pub const fn new() -> Self {
        Self {
            last: None,
            reading: false,
            since: 0,
            saved: None,
        }
    }

    /// A read kept on disk under `name`: the first view after a restart shows the last one, never a blank.
    pub const fn saved(
        name: &'static str,
        to: fn(&T) -> Value,
        from: fn(&Value) -> Option<T>,
    ) -> Self {
        Self {
            last: None,
            reading: false,
            since: 0,
            saved: Some(Saved { name, to, from }),
        }
    }
}

/// Whether a read runs now: a warmer waits on it.
pub fn reading<T>(cell: &Mutex<Read<T>>) -> bool {
    cell.lock().is_ok_and(|r| r.reading)
}

/// A kept read's file, in the served data folder; none outside `dash serve` (a test sets its own).
fn file(name: &str) -> Option<PathBuf> {
    let dir = crate::pages::serve_from_dir().ok()?;
    Some(dir.join("warm").join(format!(
        "{}.json",
        name.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    )))
}

/// Keeps `value`, read at `at` (Unix seconds, for every caller: one unit on disk), for the next process. A failed write costs the next restart its last read,
/// said, never the read.
pub fn save(name: &str, at: u64, value: Value) {
    let Some(path) = file(name) else { return };
    let tmp = path.with_extension("tmp");
    let wrote = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&tmp, json!({"at": at, "value": value}).to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = wrote {
        eprintln!("{name}: its read not kept: {e}");
    }
}

/// The value kept under `name` and when it was read, if any.
pub fn load(name: &str) -> Option<(Value, u64)> {
    let text = std::fs::read_to_string(file(name)?).ok()?;
    let mut kept: Value = serde_json::from_str(&text).ok()?;
    Some((kept.get_mut("value")?.take(), kept.get("at")?.as_u64()?))
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
    if read.last.is_none()
        && read.since == 0
        && let Some(saved) = &read.saved
    {
        read.last = load(saved.name).and_then(|(v, at)| Some(((saved.from)(&v)?, at)));
    }
    let stale = read.last.as_ref().is_none_or(|(_, at)| now >= at + FRESH);
    if stale && !read.reading {
        if read.last.is_none() && read.since == 0 {
            read.since = now;
        }
        let saved = read.saved.as_ref().map(|s| (s.name, s.to));
        let started = std::thread::Builder::new().spawn(move || {
            let got = crate::reporting::read(what, || std::panic::catch_unwind(read_all));
            if got.is_err() {
                eprintln!("{what}: the read panicked; the last one stays");
            }
            if let (Ok(got), Some((name, to))) = (&got, saved) {
                save(name, now, to(got));
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

/// A page's or plugin's warm-up after a restart: `start` begins its reads, `reading` says one still runs (task
/// rust-warmup-queue). Registered beside the page, as its `Page` is.
pub struct Warmer {
    pub page: &'static str,
    pub start: fn(),
    pub reading: fn() -> bool,
}
inventory::collect!(Warmer);

/// How long a warmer may read before the next starts: its read goes on, the queue does not wait on it for good.
const WARM_MAX: Duration = Duration::from_secs(120);

/// Runs each warmer, one at a time, each started once `calm` says the load allows, checked every `pause`; `warmed` gets each
/// page whose reads ended, never one given up on at `WARM_MAX`, which is said and whose read goes on beside the next warmer.
/// `dash serve` runs it on a thread of its own after it is bound.
pub fn warm(warmers: &[&Warmer], calm: &dyn Fn() -> bool, pause: Duration, warmed: &dyn Fn(&str)) {
    for warmer in warmers {
        // a warm-up held by the load says so, never waits unseen (rust/CLAUDE.md rule 5)
        let held = Instant::now();
        if !calm() {
            eprintln!(
                "{}: its warm-up waits for the load under its cap",
                warmer.page
            );
            while !calm() {
                std::thread::sleep(pause);
            }
        }
        let at = Instant::now();
        (warmer.start)();
        loop {
            if !(warmer.reading)() {
                eprintln!(
                    "{}: warmed in {} ms, {} ms held by the load",
                    warmer.page,
                    at.elapsed().as_millis(),
                    at.duration_since(held).as_millis()
                );
                warmed(warmer.page);
                break;
            }
            if at.elapsed() >= WARM_MAX {
                eprintln!(
                    "{}: its warm-up still reads after {} s; the next starts",
                    warmer.page,
                    WARM_MAX.as_secs()
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The load cap a warmer starts under: the 1-minute load under 3 times the cores, where the older app's load watch lifts its
/// hold on starts (`Dashboard.Watch.Load`'s lift). A load not read is calm: a warm-up is cheaper than a page read cold.
pub fn calm() -> bool {
    load_per_core(0).is_none_or(|load| load < 3.0)
}

/// The machine's load per core: `at` 0 the 1-minute load, 1 the 5-minute one. Over the host's online cores, never
/// `available_parallelism`, which a cgroup quota or taskset lowers while the load stays host-wide (PR 1463's /code-review).
/// `None` unread.
pub(crate) fn load_per_core(at: usize) -> Option<f64> {
    let mut load = [0f64; 2];
    // SAFETY: at most two f64 written into a buffer of two
    let read = unsafe { libc::getloadavg(load.as_mut_ptr(), 2) };
    // SAFETY: sysconf reads a constant
    let cores = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    (read > at as i32 && cores > 0).then(|| load[at] / cores as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task rust-warmup-queue, done-when 2: /insights and /tools each register a warmer, and `warm` runs every warmer one at a
    /// time, the next only once the last one's reads ended, and none while the load is over its cap.
    #[dash_test]
    fn warmers_run_one_at_a_time_under_the_load_cap() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let pages: Vec<&str> = inventory::iter::<Warmer>().map(|w| w.page).collect();
        for page in ["/insights", "/tools"] {
            assert!(
                pages.contains(&page),
                "{page} registers a warmer: {pages:?}"
            );
        }
        // each warmer reads for three polls; `RUNNING` counts the warmers reading at once
        static RUNNING: AtomicUsize = AtomicUsize::new(0);
        static MOST: AtomicUsize = AtomicUsize::new(0);
        static POLLS: AtomicUsize = AtomicUsize::new(0);
        static LOAD_ASKED: AtomicUsize = AtomicUsize::new(0);
        fn start() {
            let running = RUNNING.fetch_add(1, SeqCst) + 1;
            MOST.fetch_max(running, SeqCst);
            POLLS.store(3, SeqCst);
        }
        fn reading() -> bool {
            let left = POLLS.load(SeqCst);
            if left == 0 {
                RUNNING.fetch_sub(1, SeqCst);
                return false;
            }
            POLLS.store(left - 1, SeqCst);
            true
        }
        let a = Warmer {
            page: "/a",
            start,
            reading,
        };
        let b = Warmer {
            page: "/b",
            start,
            reading,
        };
        // over the cap for the first two asks: no warmer starts until it is calm
        let calm = || {
            let asked = LOAD_ASKED.fetch_add(1, SeqCst);
            asked >= 2 && RUNNING.load(SeqCst) == 0
        };
        let order = Mutex::new(vec![]);
        warm(&[&a, &b], &calm, Duration::from_millis(1), &|page| {
            order
                .lock()
                .expect("order")
                .push((page.to_string(), RUNNING.load(SeqCst)));
        });
        assert_eq!(MOST.load(SeqCst), 1, "one warmer at a time");
        assert_eq!(
            LOAD_ASKED.load(SeqCst),
            4,
            "two asks over the cap, then one before each warmer"
        );
        let order = order.into_inner().expect("order");
        assert_eq!(
            order,
            [("/a".to_string(), 0), ("/b".to_string(), 0)],
            "each ended before the next"
        );
    }

    /// A first read still running is waited for and its result returned; one past the limit is still none.
    #[dash_test]
    fn an_agent_waits_for_the_first_read() {
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
    #[dash_test]
    fn a_bounded_first_read_ends_in_its_error() {
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
    #[dash_test]
    fn a_first_read_that_panics_ends_in_its_error_too() {
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
