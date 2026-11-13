//! The stores under load (task rust-store-load-measured, done-when 3): whether SQLite holds 10 sessions at once, and that a
//! lock hold counts no wait. Here, not in src: the store reads no clock of its own (dash-gate's simulation rule), and these
//! time real runs.
use dash_meter::{TestDir, dash_test};
use dash_store::Connection;
use dash_store::timed::{on_statement, trace};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Barrier, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A BEGIN IMMEDIATE that waits 300 ms behind another connection's lock holds it only from when it gets it: the wait is the
/// BEGIN's own time, never a hold (the load test below first read every wait as one). It measures that wait, never a wall
/// bound: the clock starts before the lock's owner sleeps, so a hold that excludes the wait leaves at least 250 ms of the
/// whole call outside it on any runner, and one that counts the wait leaves none (task rust-store-rebuild-write-flake).
#[dash_test]
fn a_hold_counts_from_the_lock_taken_never_the_wait_for_it() {
    static HOLDS: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now);
    on_statement(
        Box::new(|ran| {
            if ran.lock {
                HOLDS.lock().unwrap().push(ran.took);
            }
        }),
        // 10 s on, so a hold counted from the BEGIN's start shows the wait rather than stopping at zero
        || START.get().map_or(Duration::ZERO, Instant::elapsed) + Duration::from_secs(10),
    );
    let dir = TestDir::new("timed-wait");
    let path = dir.0.join("tasks.sqlite");
    let other = Connection::open(&path).unwrap();
    other
        .execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE t (x); BEGIN IMMEDIATE")
        .unwrap();
    let mut db = Connection::open(&path).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    trace(&mut db, &path).unwrap();
    let began = Instant::now();
    let waited = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        other.execute_batch("COMMIT").unwrap();
    });
    db.execute_batch("BEGIN IMMEDIATE; INSERT INTO t VALUES (1); COMMIT;")
        .unwrap();
    let whole = began.elapsed();
    waited.join().unwrap();
    let holds = HOLDS.lock().unwrap().clone();
    assert_eq!(holds.len(), 1, "{holds:?}");
    assert!(
        whole.saturating_sub(holds[0]) >= Duration::from_millis(250),
        "a hold of {holds:?} counts the wait for the lock: the whole call took {whole:?}"
    );
}

/// Done-when 3: 10 sessions, each on its own connection as each dash process is, write a field and read the board at once,
/// 50 rounds each, on a store holding the live one's 250 listed rows and 40 open tasks. Every write lands under 200 ms; past
/// it SQLite would be the wrong store. The line printed names the slowest write beside the longest lock hold: a slow write
/// then says whether it waited or held.
#[dash_test]
fn ten_sessions_writing_and_reading_at_once_keep_each_write_under_200_ms() {
    const SESSIONS: usize = 10;
    const ROUNDS: usize = 50;
    static HOLD: AtomicU64 = AtomicU64::new(0);
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now);
    on_statement(
        Box::new(|ran| {
            if ran.lock {
                HOLD.fetch_max(ran.took.as_micros() as u64, Ordering::Relaxed);
            }
        }),
        || START.get().map_or(Duration::ZERO, Instant::elapsed),
    );
    let dir = TestDir::new("tasks-load");
    let open = || dash_store::tasks::open(&dir.0, &dir.0).expect("open");
    let mut seed = open();
    for n in 0..40 {
        seed.add(&format!("open{n}"), &[]).expect("within the cap");
    }
    for n in 0..250 {
        seed.layer(&format!("listed{n}"), "rfc:storage")
            .expect("listed");
    }
    drop(seed);
    let start = Barrier::new(SESSIONS);
    let mut took: Vec<Duration> = std::thread::scope(|s| {
        let sessions: Vec<_> = (0..SESSIONS)
            .map(|session| {
                let (open, start) = (&open, &start);
                s.spawn(move || {
                    let mut tasks = open();
                    start.wait();
                    (0..ROUNDS)
                        .map(|round| {
                            let id = format!("listed{}", (session * ROUNDS + round) % 250);
                            let began = Instant::now();
                            tasks
                                .layer(&id, &format!("rfc:s{session}r{round}"))
                                .expect("write");
                            let write = began.elapsed();
                            assert_eq!(tasks.rows().expect("read").len(), 40);
                            write
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        sessions
            .into_iter()
            .flat_map(|h| h.join().expect("session"))
            .collect()
    });
    took.sort();
    let at = |q: usize| took[(took.len() - 1) * q / 100].as_secs_f64() * 1000.0;
    let line = format!(
        "{} writes by {SESSIONS} sessions: p50 {:.1} ms, p99 {:.1} ms, max {:.1} ms, longest lock hold {:.1} ms",
        took.len(),
        at(50),
        at(99),
        at(100),
        HOLD.load(Ordering::Relaxed) as f64 / 1000.0
    );
    eprintln!("{line}");
    assert_eq!(took.len(), SESSIONS * ROUNDS);
    assert!(at(100) < 200.0, "{line}");
}
