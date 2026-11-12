//! Ten sessions writing and reading the task store at once (task rust-store-load-measured; @nha, 2026-11-12: « my guess is
//! sqlite is plenty for our current situation if we use it right but measures might beat instinct. »). Each session is its own
//! connection, as each `dash task` process is: it adds a task, reads every row, and closes the task, round after round.
//! A write's time is from its call to its commit, its wait on the others' write locks included.
//!
//! On a fixture by default; `DASH_LOAD_DIR=<a copy of the data folder>` runs it on that copy's tasks.sqlite, copied again
//! first, so the live store is never written (rfc-rust-unified#storage holds the numbers on the live copy).
use dash_meter::{TestDir, dash_test};
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const SESSIONS: usize = 10;
const ROUNDS: usize = 20;
const WRITE_BUDGET: Duration = Duration::from_millis(200);

/// Each write's time and each read's, in ms, over every session.
fn load(dir: &Path) -> (Vec<f64>, Vec<f64>) {
    let locks = dir.join("locks");
    std::fs::create_dir_all(&locks).expect("locks");
    let start = Arc::new(Barrier::new(SESSIONS));
    let sessions: Vec<_> = (0..SESSIONS)
        .map(|s| {
            let (dir, locks, start) = (dir.to_path_buf(), locks.clone(), Arc::clone(&start));
            std::thread::spawn(move || {
                let mut store = dash_store::tasks::open(&dir, &locks).expect("open");
                let (mut writes, mut reads) = (vec![], vec![]);
                start.wait();
                for r in 0..ROUNDS {
                    let id = format!("load-{s}-{r}");
                    let at = Instant::now();
                    store.add(&id, &[]).expect("add");
                    writes.push(at.elapsed().as_secs_f64() * 1000.0);
                    let at = Instant::now();
                    assert!(store.rows().expect("rows").iter().any(|row| row.id == id));
                    reads.push(at.elapsed().as_secs_f64() * 1000.0);
                    let at = Instant::now();
                    store.close(&id, "load test").expect("close");
                    writes.push(at.elapsed().as_secs_f64() * 1000.0);
                }
                (writes, reads)
            })
        })
        .collect();
    let (mut writes, mut reads) = (vec![], vec![]);
    for s in sessions {
        let (w, r) = s.join().expect("a session");
        writes.extend(w);
        reads.extend(r);
    }
    (writes, reads)
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

/// As many rows as the live store held on 2026-11-12, 252; its ops log is smaller (1,250 here, 29,701 there).
fn fixture(dir: &Path) {
    let locks = dir.join("locks");
    std::fs::create_dir_all(&locks).expect("locks");
    let mut store = dash_store::tasks::open(dir, &locks).expect("open");
    for i in 0..250 {
        let id = format!("fixture-{i}");
        store.add(&id, &[]).expect("add");
        store.close(&id, "fixture").expect("close");
    }
}

#[dash_test]
fn ten_sessions_writing_and_reading_at_once_keep_each_write_under_200_ms() {
    let dir = TestDir::new("load");
    match std::env::var_os("DASH_LOAD_DIR") {
        Some(copy) => {
            std::fs::copy(
                Path::new(&copy).join("tasks.sqlite"),
                dir.0.join("tasks.sqlite"),
            )
            .expect("the copy");
        }
        None => fixture(&dir.0),
    }
    let began = Instant::now();
    let (mut writes, mut reads) = load(&dir.0);
    let took = began.elapsed();
    writes.sort_by(f64::total_cmp);
    reads.sort_by(f64::total_cmp);
    let line = format!(
        "{SESSIONS} sessions, {} writes in {:.2} s: write p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms (budget {WRITE_BUDGET:?}); read p50 {:.1} ms, max {:.1} ms",
        writes.len(),
        took.as_secs_f64(),
        quantile(&writes, 0.5),
        quantile(&writes, 0.95),
        writes[writes.len() - 1],
        quantile(&reads, 0.5),
        reads[reads.len() - 1],
    );
    println!("{line}");
    assert_eq!(writes.len(), SESSIONS * ROUNDS * 2, "{line}");
    // The budget is this Mac's, where the tool runs: Linux CI syncs each commit to its disk, and its tail is not ours.
    #[cfg(target_os = "macos")]
    assert!(
        writes[writes.len() - 1] < WRITE_BUDGET.as_secs_f64() * 1000.0,
        "{line}"
    );
}
