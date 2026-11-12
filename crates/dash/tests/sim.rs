//! Section 17's simulation of two replicas: the sync crate's loop on the store's op log. A failing seed prints the line that
//! reruns it alone: SIM_SEED=<n> cargo nextest run -p dash --test sim. SIM_SEEDS widens the sweep, under nextest's `long` profile.
use dash_core::Rng;
use dash_meter::dash_test;
use dash_store::Connection;
use dash_store::oplog::Replica;
use dash_sync::sim::{Faults, Outcome, Queue, SimClock, SplitMix, run};
use std::process::Command;

const SWEEP_TEST: &str = "two_replicas_under_delay_loss_and_reorder_end_with_one_state_hash";
/// Section 17: a low count on a PR, as for proptest. The sweep takes under 4 s at this count, unoptimized, each seed run twice.
const PR_SEEDS: u64 = 64;

fn notes_db() -> Result<Connection, String> {
    let db = Connection::open_in_memory().map_err(|e| e.to_string())?;
    db.execute_batch("CREATE TABLE notes (id TEXT PRIMARY KEY, title, body) WITHOUT ROWID")
        .map_err(|e| e.to_string())?;
    Ok(db)
}

fn make_log(writer: &str, clock: SimClock) -> Result<Box<dyn dash_core::OpLog>, String> {
    let replica = Replica::with_clock(notes_db()?, writer, Box::new(clock));
    Ok(Box::new(replica.map_err(|e| e.to_string())?))
}

fn env_number(name: &str) -> Option<u64> {
    let text = std::env::var(name).ok()?;
    Some(
        text.parse()
            .unwrap_or_else(|_| panic!("{name}={text} is no number")),
    )
}

/// Says how to rerun the seed whatever ends it, an assert or a panic deep in an adapter.
struct RerunLine {
    seed: u64,
    faults: Faults,
}

impl Drop for RerunLine {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let plant = if self.faults.lying_ack {
                "SIM_PLANT=lying-ack "
            } else {
                ""
            };
            eprintln!(
                "seed {0} failed. Rerun it alone: SIM_SEED={0} {plant}cargo nextest run -p dash --test sim {SWEEP_TEST}",
                self.seed
            );
        }
    }
}

fn simulate(seed: u64, faults: Faults) -> Outcome {
    run(
        &mut SplitMix::new(seed),
        &mut Queue::default(),
        faults,
        &make_log,
    )
    .unwrap_or_else(|e| panic!("seed {seed}: {e}"))
}

fn first_difference(a: &[u64], b: &[u64]) -> Option<usize> {
    (0..a.len().max(b.len())).find(|i| a.get(*i) != b.get(*i))
}

/// The sweep's checks can fail: another seed is another run and another state.
#[dash_test]
fn another_seed_gives_another_run() {
    let first = simulate(7, Faults::ROUGH);
    let other = simulate(8, Faults::ROUGH);
    assert_ne!(first.fingerprint(), other.fingerprint());
    assert_ne!(first.hashes[0], other.hashes[0]);
}

/// The run itself is the contract, on every machine: this number moves only when the loop, the protocol, the stream or the
/// hash changes, and a move on one machine alone is a leak of something a seed does not decide.
#[dash_test]
fn a_seed_gives_the_same_run_on_every_machine() {
    let mut rng = SplitMix::new(0);
    assert_eq!(
        rng.next_u64(),
        0xe220_a839_7b1d_cdaf,
        "SplitMix64's published first value"
    );
    assert_eq!(
        format!("{:016x}", simulate(1, Faults::ROUGH).fingerprint()),
        "9a9ef55bfb1cc9dd"
    );
}

#[dash_test]
fn two_replicas_under_delay_loss_and_reorder_end_with_one_state_hash() {
    let plant = std::env::var("SIM_PLANT").ok();
    assert!(
        matches!(plant.as_deref(), None | Some("lying-ack")),
        "SIM_PLANT={plant:?} is no planted bug: lying-ack"
    );
    let faults = Faults {
        lying_ack: plant.is_some(),
        ..Faults::ROUGH
    };
    let alone = env_number("SIM_SEED");
    let seeds: Vec<u64> = match alone {
        Some(seed) => vec![seed],
        None => (0..env_number("SIM_SEEDS").unwrap_or(PR_SEEDS)).collect(),
    };
    let count = seeds.len();
    let (mut sent, mut lost, mut reordered) = (0, 0, 0);
    for seed in seeds {
        let _rerun = RerunLine { seed, faults };
        let run = simulate(seed, faults);
        assert!(
            run.one_state(),
            "seed {seed}: trace {:016x}, settled {}, hashes {:016x} {:016x}, {:?}",
            run.fingerprint(),
            run.settled,
            run.hashes[0],
            run.hashes[1],
            run.stats
        );
        // Each seed twice: a leak of real time or order on a path few seeds reach still converges.
        let again = simulate(seed, faults);
        assert_eq!(
            first_difference(&run.trace, &again.trace),
            None,
            "seed {seed}: the first event that differs on a rerun"
        );
        assert_eq!(run, again, "seed {seed}: its hashes and op logs on a rerun");
        // The op log alone rebuilds the state, in another replica under another clock, on every seed: a rebuild bug few
        // seeds reach (/code-review). The two logs are one: state_hash covers them.
        let mut replay = Replica::new(notes_db().expect("db"), "replay").expect("replica");
        replay.apply(&run.logs[0]).expect("replay");
        assert_eq!(
            replay.state_hash().expect("hash"),
            run.hashes[0],
            "seed {seed}: a replay"
        );
        sent += run.stats.sent;
        lost += run.stats.lost;
        reordered += run.stats.reordered;
    }
    println!("{count} seeds: {sent} messages sent, {lost} lost, {reordered} reordered");
    // One seed proves nothing either way about the network as a whole, so a rerun does not fail on it.
    if alone.is_some() {
        return;
    }
    assert!(lost > 0, "the network lost nothing, so this proved nothing");
    assert!(
        reordered > 0,
        "the network reordered nothing, so this proved nothing"
    );
}

/// The planted bug fails, its seed is named, and that one seed fails the same way in a process of its own.
#[dash_test]
fn a_failing_seed_prints_and_reruns_alone() {
    let faults = Faults {
        lying_ack: true,
        ..Faults::ROUGH
    };
    let (seed, failed) = (0..PR_SEEDS)
        .map(|seed| (seed, simulate(seed, faults)))
        .find(|(_, run)| !run.one_state())
        .expect("a lost push acked anyway parts some seed");
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", SWEEP_TEST, "--nocapture"])
        .env("SIM_SEED", seed.to_string())
        .env("SIM_PLANT", "lying-ack")
        .output()
        .expect("the rerun");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{printed}");
    for says in [
        format!("SIM_SEED={seed} SIM_PLANT=lying-ack cargo nextest run"),
        format!("seed {seed}: trace {:016x}", failed.fingerprint()),
    ] {
        assert!(printed.contains(&says), "{says} in: {printed}");
    }
    assert_eq!(
        printed.matches("failed. Rerun it alone").count(),
        1,
        "one seed only: {printed}"
    );
}

/// The ports are the only way in: an Rng that always draws 0 loses every message, and the run ends at its deadline, no hang.
#[dash_test]
fn a_run_that_cannot_settle_ends_and_says_so() {
    struct Zeros;
    impl Rng for Zeros {
        fn next_u64(&mut self) -> u64 {
            0
        }
    }
    let run = run(&mut Zeros, &mut Queue::default(), Faults::ROUGH, &make_log).expect("run");
    assert!(!run.settled);
    assert!(!run.one_state());
    assert_eq!(run.stats.lost, run.stats.sent, "every message");
    assert!(run.stats.sent > 0);
}
