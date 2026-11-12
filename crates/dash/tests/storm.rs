//! Section 20c's worked example, the first lesson moved: the storm of 2026-11-05 (recap-choke-2026-11-05 sections 1 and 2;
//! PR 1110). The app started 82 one-shot judges within a minute; 31 minutes later, 44 more. A seed is a whole day of it, on
//! rust-sim-spike's stream, queue and hash, and every start goes through the real `Defaults` policy.
//!
//! The pass and the wake are models of `Orch.Unblock.judge_due` and `Watch.LoadWake`, which no Rust runs yet (section 20c:
//! a lesson moves as a test, written first): only `Defaults.pick` here is real code. The path a Rust run's judge really takes
//! has its own storm, on `PM.judge_report` and its cap: rust_owner_test.exs. What this cannot see is in that section's « what
//! stays outside ».
//!
//! A failing seed prints the line that reruns it alone: SIM_SEED=<n> cargo nextest run -p dash --test storm storm_2026_11_05.
//! SIM_PLANT brings one of the planted bugs back; SIM_SEEDS widens the sweep, under nextest's `long` profile.
use dash_core::tasks::{Pick, Policy, Queue as TaskQueue, Queued, Snapshot};
use dash_core::{Fnv, Rng, Scheduler};
use dash_meter::memory::TestBudget;
use dash_policy::Defaults;
use dash_sync::sim::{Queue, SplitMix};
use std::cmp::Reverse;
use std::process::Command;

const SWEEP_TEST: &str = "storm_2026_11_05";
const PR_SEEDS: u64 = 64;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

const BLOCKERS: usize = 82;
const NEW_PER_HOUR: u64 = 24;
/// `@judges`, and each judge's measured size: memory is a number fed in, so a real one-shot growing past it still passes.
const SEATS: usize = 2;
const ONE_SHOT_MB: usize = 180;
const JUDGE_MAX_MS: u64 = 80_000;
const FAILING_PCT: u64 = 20;
/// `runner.ex`'s tick. A --start runs one pass at once, on top.
const PASS_MS: i64 = 2 * MINUTE;
const EARLY_PER_HOUR: u64 = 2;
/// `@retry`: a judge that failed or was cut is asked again after this. A not met after a day.
const RETRY_MS: i64 = HOUR;
const MAX_WAIT_MS: i64 = 3 * HOUR;
const RESTARTS: u32 = 2;
/// Each restart 10 to 70 minutes after the last (the recap's was 31), so both land while the 82 are still due.
const RESTART_GAP_MINUTES: u64 = 60;
const MAX_DOWN_MS: u64 = 90_000;
/// `LoadWake`: past a bound this long, over this many samples, one wake; an episode ends this long after its last sample
/// past a bound (`@told_calm_s` once it woke).
const WAKE_AFTER_MS: i64 = 10 * MINUTE;
const WAKE_SAMPLES: u32 = 6;
const CALM_MS: i64 = 5 * MINUTE;
const TOLD_CALM_MS: i64 = 30 * MINUTE;
/// Long enough that two restarts cannot hide the tenth minute: one can delay the first sample by 3, the other take 2 more
/// (seed 13262 of a 20,000-seed sweep found 13 too short).
const LOAD_MIN_MINUTES: i64 = 15;
const LOAD_EXTRA_MINUTES: u64 = 10;
/// A day is about 4,000 events: past this a seed is looping, and it panics, so it prints its rerun line. Nextest's 2 s ends
/// the process with a signal, which runs no `Drop`.
const MAX_EVENTS: u64 = 100_000;

/// Section 20c's planted bugs: its SIM_PLANT, the bug, and the words of the invariant it must fail.
const PLANTS: [(&str, Plant, &str); 8] = [
    ("cap-removed", Plant::CapRemoved, "live one-shots"),
    ("cap-per-pass", Plant::CapPerPass, "live one-shots"),
    ("verdicts-in-memory", Plant::VerdictsInMemory, "asked again"),
    ("newest-first", Plant::NewestFirst, "longest wait"),
    ("wake-removed", Plant::WakeRemoved, "wakes past the bound"),
    ("met-lost", Plant::MetLost, "is gone"),
    ("wake-late", Plant::WakeLate, "after the first overload"),
    ("runaway", Plant::Runaway, "events, a loop"),
];

#[derive(Clone, Copy, PartialEq)]
enum Plant {
    CapRemoved,
    CapPerPass,
    VerdictsInMemory,
    NewestFirst,
    WakeRemoved,
    /// A met saved as a not met: nothing asks it again within the day, so only the end of the run sees it.
    MetLost,
    /// The watch misses the first overload and wakes on the second, still one wake.
    WakeLate,
    /// The minute's sample comes back at once, the simulated clock stuck.
    Runaway,
}

enum Event {
    Arrive,
    /// The pass timer of one life of the app: a restart ends its chain.
    Pass(u32),
    Early,
    /// A judge of one life of the app ends: a restart cut it if that life is over.
    Done {
        life: u32,
        id: usize,
        read: Read,
    },
    Restart,
    Back,
    Sample,
}

#[derive(Clone, Copy, PartialEq)]
enum Read {
    /// A judge's marker, a row a restart cut leaves behind.
    Judging,
    /// No verdict: asked again after `RETRY_MS`.
    Failed,
    Met,
    NotMet,
}

/// A row of `unblock_verdicts`.
#[derive(Clone, Copy)]
struct Row {
    read: Read,
    at: i64,
}

#[derive(Default, Clone, Copy)]
struct LoadWatch {
    since: Option<i64>,
    last: i64,
    samples: u32,
    told: bool,
}

impl LoadWatch {
    fn going(&self, now: i64) -> bool {
        let calm = if self.told { TOLD_CALM_MS } else { CALM_MS };
        self.since.is_some() && now - self.last <= calm
    }

    /// One minute's reading. Whether it wakes.
    fn sample(&mut self, now: i64, past: bool) -> bool {
        if !past {
            if !self.going(now) {
                *self = Self::default();
            }
            return false;
        }
        if !self.going(now) {
            *self = Self {
                since: Some(now),
                ..Self::default()
            };
        }
        self.last = now;
        self.samples += 1;
        let wake = self.samples >= WAKE_SAMPLES
            && self.since.is_some_and(|s| now - s >= WAKE_AFTER_MS)
            && !self.told;
        self.told |= wake;
        wake
    }
}

/// What a whole run came to.
#[derive(Debug, Default, PartialEq)]
struct Report {
    fingerprint: u64,
    peak_live: usize,
    arrived: usize,
    longest_wait_ms: i64,
    starts: u64,
    failed: u64,
    /// Restarts inside an overload: without them its episode surviving in the settings goes unexercised.
    restarts_under_load: u64,
    /// Early passes that landed while a judge ran: without them a cap per pass would hold the limit.
    early_inside_a_judge: u64,
    wakes: u32,
}

struct Storm {
    plant: Option<Plant>,
    now: i64,
    up: bool,
    life: u32,
    live: usize,
    rows: Vec<Option<Row>>,
    arrived_at: Vec<i64>,
    /// Where each blocker sits in the task list, which says nothing of its age.
    position: Vec<u64>,
    /// What the store cannot lose, so a bug that loses it is seen: when each blocker last had a verdict, and each met.
    answered: Vec<Option<i64>>,
    met: Vec<bool>,
    asked_again: u64,
    trace: Fnv,
    report: Report,
}

impl Storm {
    /// `due?` of judge_due: when this blocker became due, or `None` if it is not going to be.
    fn due_since(&self, id: usize) -> Option<i64> {
        match self.rows[id] {
            None => Some(self.arrived_at[id]),
            Some(Row {
                read: Read::Met, ..
            }) => None,
            Some(Row {
                read: Read::NotMet,
                at,
            }) => Some(at + DAY),
            Some(Row {
                read: Read::Judging | Read::Failed,
                at,
            }) => Some(at + RETRY_MS),
        }
    }

    fn due(&self) -> Vec<usize> {
        let due = |id: &usize| self.due_since(*id).is_some_and(|since| since <= self.now);
        (0..self.rows.len()).filter(due).collect()
    }

    /// judge_due: what is due, the longest waiting first (never asked before asked), one start at a time through the policy.
    /// `waited(nil)` is 0 and `Enum.sort_by` is stable, so the task list's order decides among the never asked.
    fn pass(&mut self, rng: &mut dyn Rng, queue: &mut dyn Scheduler<Event>) {
        let mut due = self.due();
        due.sort_by_key(|id| self.position[*id]);
        if self.plant == Some(Plant::NewestFirst) {
            due.sort_by_key(|id| Reverse(self.due_since(*id)));
        } else {
            due.sort_by_key(|id| self.rows[*id].map(|row| row.at));
        }
        let seats = match self.plant {
            Some(Plant::CapRemoved) => usize::MAX,
            _ => SEATS,
        };
        for (started, id) in due.into_iter().enumerate() {
            // The head of what is due alone: `Defaults` reads only the first, and a list of hundreds at each pass costs time.
            let tasks = vec![Queued {
                held: None,
                id: id.to_string(),
                title: String::new(),
                reopened_ms: None,
                waits: vec![],
                layer: None,
                builds_nothing: false,
                rank: Default::default(),
                free: None,
                demand: Default::default(),
                route: None,
                ..Default::default()
            }];
            // What the seats count: processes alive, the lesson of the second row of section 20c's table.
            let sessions = match self.plant {
                Some(Plant::CapPerPass) => started,
                _ => self.live,
            };
            let snapshot = Snapshot {
                queue: TaskQueue {
                    seats,
                    seats_why: String::new(),
                    tasks,
                },
                sessions,
                limits: vec![],
                free_percent: None,
                now: 0,
            };
            let Pick::Start { id, .. } = Defaults.pick(&snapshot) else {
                return;
            };
            self.start(id.parse().expect("an id of ours"), rng, queue);
        }
    }

    fn start(&mut self, id: usize, rng: &mut dyn Rng, queue: &mut dyn Scheduler<Event>) {
        let since = self.due_since(id).expect("only what is due starts");
        let waited = self.now - since;
        self.report.longest_wait_ms = self.report.longest_wait_ms.max(waited);
        if self.answered[id].is_some_and(|at| self.now - at < DAY) {
            self.asked_again += 1;
        }
        self.trace.bytes(&(id as u64).to_le_bytes());
        let at = self.now;
        // ponytail: saved at once, where judge_due saves it in the judge's own task: one cut before its first line is asked
        // again at once, at most 2 a restart and never a verdict. Model that cut if a due-after-restart rule ever hangs on it.
        self.rows[id] = Some(Row {
            read: Read::Judging,
            at,
        });
        // All three draws always, so an outcome never shifts the stream under what follows (nanolathe's I4).
        let (lasts, fails, met) = (
            1 + rng.below(JUDGE_MAX_MS) as i64,
            rng.below(100) < FAILING_PCT,
            rng.below(2) == 0,
        );
        let read = match (fails, met) {
            (true, _) => Read::Failed,
            (_, true) => Read::Met,
            _ => Read::NotMet,
        };
        let life = self.life;
        queue.at(self.now + lasts, Event::Done { life, id, read });
        self.live += 1;
        self.report.starts += 1;
        self.report.peak_live = self.report.peak_live.max(self.live);
    }

    fn done(&mut self, id: usize, read: Read) {
        self.live -= 1;
        self.trace.bytes(&[read as u8]);
        self.trace.bytes(&(id as u64).to_le_bytes());
        let at = self.rows[id].expect("its judge's marker").at;
        let saved = match (self.plant, read) {
            (Some(Plant::MetLost), Read::Met) => Read::NotMet,
            _ => read,
        };
        self.rows[id] = Some(Row { read: saved, at });
        match read {
            Read::Failed => self.report.failed += 1,
            Read::Met => {
                self.answered[id] = Some(at);
                self.met[id] = true;
            }
            _ => self.answered[id] = Some(at),
        }
    }
}

/// One day of it: live one-shots and verdicts asked again checked at each step, the wait, the mets and the wake at its end.
/// A broken one panics naming the seed.
fn storm(seed: u64, plant: Option<Plant>) -> Report {
    let mut rng = SplitMix::new(seed);
    let mut queue = Queue::default();
    let position = (0..BLOCKERS).map(|_| rng.below(u64::MAX)).collect();
    let mut s = Storm {
        plant,
        now: 0,
        up: true,
        life: 0,
        live: 0,
        rows: vec![None; BLOCKERS],
        arrived_at: vec![0; BLOCKERS],
        position,
        answered: vec![None; BLOCKERS],
        met: vec![false; BLOCKERS],
        asked_again: 0,
        trace: Fnv::new(),
        report: Report::default(),
    };
    for hour in 0..24 {
        for _ in 0..NEW_PER_HOUR {
            queue.at(hour * HOUR + rng.below(HOUR as u64) as i64, Event::Arrive);
        }
        for _ in 0..EARLY_PER_HOUR {
            queue.at(hour * HOUR + rng.below(HOUR as u64) as i64, Event::Early);
        }
    }
    let mut restart_at = 0;
    for _ in 0..RESTARTS {
        restart_at += 10 * MINUTE + rng.below(RESTART_GAP_MINUTES) as i64 * MINUTE;
        queue.at(restart_at, Event::Restart);
    }
    let load_from = rng.below(4 * 60) as i64 * MINUTE;
    let load_minutes = LOAD_MIN_MINUTES + rng.below(LOAD_EXTRA_MINUTES) as i64;
    let load_to = load_from + load_minutes * MINUTE;
    // The same overload again, past the calm of an episode not told and inside the one of an episode told: a native build in
    // three parts wakes once (`LoadWake`, task overload-wake-quiet).
    let again_from = load_to + (CALM_MS / MINUTE + 1 + rng.below(20) as i64) * MINUTE;
    let again_to = again_from + LOAD_MIN_MINUTES * MINUTE;
    let first = |t: i64| (load_from..=load_to).contains(&t);
    let second = |t: i64| (again_from..=again_to).contains(&t);
    queue.at(0, Event::Pass(0));
    queue.at(0, Event::Sample);
    let mut watch = LoadWatch::default();
    let mut woke_at = None;
    let mut events = 0;
    while let Some((due, event)) = queue.pop() {
        if due > DAY {
            break;
        }
        events += 1;
        assert!(
            events <= MAX_EVENTS,
            "seed {seed}: over {MAX_EVENTS} events, a loop, at {} s",
            due / 1000
        );
        s.now = due;
        let kind = match event {
            Event::Arrive => {
                s.rows.push(None);
                s.arrived_at.push(due);
                s.position.push(rng.below(u64::MAX));
                s.answered.push(None);
                s.met.push(false);
                s.report.arrived += 1;
                0u8
            }
            Event::Pass(life) => {
                if life == s.life {
                    s.pass(&mut rng, &mut queue);
                    queue.at(due + PASS_MS, Event::Pass(life));
                }
                1
            }
            Event::Early => {
                if s.up {
                    s.report.early_inside_a_judge += u64::from(s.live > 0);
                    s.pass(&mut rng, &mut queue);
                }
                2
            }
            Event::Done { life, id, read } => {
                if life == s.life {
                    s.done(id, read);
                }
                3
            }
            Event::Restart => {
                assert!(
                    !s.due().is_empty(),
                    "seed {seed}: a restart with nothing due, so it proved nothing"
                );
                // Ends every judge: a one-shot that outlives its app is a real-process fact, section 20c's « outside ».
                (s.life, s.live, s.up) = (s.life + 1, 0, false);
                s.report.restarts_under_load += u64::from(first(due) || second(due));
                if plant == Some(Plant::VerdictsInMemory) {
                    s.rows.fill(None);
                }
                queue.at(due + 1 + rng.below(MAX_DOWN_MS) as i64, Event::Back);
                4
            }
            Event::Back => {
                s.up = true;
                queue.at(due, Event::Pass(s.life));
                5
            }
            Event::Sample => {
                // The watch runs inside the app: down, it reads nothing, and its episode survives in the settings.
                let past = (first(due) && plant != Some(Plant::WakeLate)) || second(due);
                if s.up && watch.sample(due, past) && plant != Some(Plant::WakeRemoved) {
                    s.report.wakes += 1;
                    woke_at = Some(due);
                }
                let next = if plant == Some(Plant::Runaway) {
                    0
                } else {
                    MINUTE
                };
                queue.at(due + next, Event::Sample);
                6
            }
        };
        s.trace.bytes(&due.to_le_bytes());
        s.trace.bytes(&[kind, s.live as u8]);
        assert!(
            s.live <= SEATS,
            "seed {seed}: {} live one-shots at {} s, {} MB, over the {SEATS} allowed",
            s.live,
            due / 1000,
            s.live * ONE_SHOT_MB
        );
        assert!(
            s.asked_again == 0,
            "seed {seed}: a verdict asked again within a day, at {} s",
            due / 1000
        );
    }
    s.now = DAY;
    // A blocker still due at the end waited too: one never started must not slip past the bound.
    for id in s.due() {
        let since = s.due_since(id).expect("due");
        s.report.longest_wait_ms = s.report.longest_wait_ms.max(DAY - since);
    }
    let wait = s.report.longest_wait_ms;
    assert!(
        wait <= MAX_WAIT_MS,
        "seed {seed}: longest wait {:.2} h, over {} h",
        hours(wait),
        MAX_WAIT_MS / HOUR
    );
    for (id, met) in s.met.iter().enumerate() {
        let stands = s.rows[id].is_some_and(|row| row.read == Read::Met);
        assert!(
            !met || stands,
            "seed {seed}: the met of blocker {id} is gone"
        );
    }
    assert!(
        s.report.wakes == 1,
        "seed {seed}: {} wakes past the bound, load past it from {} to {} min",
        s.report.wakes,
        load_from / MINUTE,
        load_to / MINUTE
    );
    // Its load lasts long enough that two restarts cannot hide the tenth minute, so the first overload is the one that wakes.
    assert!(
        woke_at.is_some_and(first),
        "seed {seed}: the wake at {} min came after the first overload, {} to {} min",
        woke_at.unwrap_or_default() / MINUTE,
        load_from / MINUTE,
        load_to / MINUTE
    );
    s.report.fingerprint = s.trace.finish();
    s.report
}

fn hours(ms: i64) -> f64 {
    ms as f64 / HOUR as f64
}

fn env_number(name: &str) -> Option<u64> {
    let text = std::env::var(name).ok()?;
    let number = text.parse();
    Some(number.unwrap_or_else(|_| panic!("{name}={text} is no number")))
}

/// Says how to rerun the seed a panic ended. The 2 s kill runs no `Drop`: `MAX_EVENTS` makes a loop a panic first.
struct RerunLine {
    seed: u64,
    plant: Option<&'static str>,
}

impl Drop for RerunLine {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let plant = self
                .plant
                .map_or(String::new(), |p| format!("SIM_PLANT={p} "));
            eprintln!(
                "seed {0} failed. Rerun it alone: SIM_SEED={0} {plant}cargo nextest run -p dash --test storm {SWEEP_TEST}",
                self.seed
            );
        }
    }
}

#[test]
fn storm_2026_11_05() {
    let _budget = TestBudget::new();
    let named = std::env::var("SIM_PLANT").ok();
    let plant = named.as_deref().map(|name| {
        let found = PLANTS.iter().find(|(n, ..)| *n == name);
        *found.unwrap_or_else(|| panic!("SIM_PLANT={name} is no planted bug of this storm"))
    });
    let alone = env_number("SIM_SEED");
    let seeds: Vec<u64> = match alone {
        Some(seed) => vec![seed],
        None => (0..env_number("SIM_SEEDS").unwrap_or(PR_SEEDS)).collect(),
    };
    let count = seeds.len();
    let mut all = Report::default();
    for seed in seeds {
        let _rerun = RerunLine {
            seed,
            plant: plant.map(|(name, ..)| name),
        };
        let run = storm(seed, plant.map(|(_, bug, _)| bug));
        all.peak_live = all.peak_live.max(run.peak_live);
        all.longest_wait_ms = all.longest_wait_ms.max(run.longest_wait_ms);
        all.arrived += run.arrived;
        all.starts += run.starts;
        all.failed += run.failed;
        all.early_inside_a_judge += run.early_inside_a_judge;
        all.restarts_under_load += run.restarts_under_load;
        all.wakes += run.wakes;
    }
    println!(
        "{count} seeds, {BLOCKERS} blockers each, {RESTARTS} restarts each: {} more arrived, {} judges started, {} failed, \
         at most {} live ({} MB), longest wait {:.2} h, {} early passes inside a judge, {} restarts under load, {} wakes",
        all.arrived,
        all.starts,
        all.failed,
        all.peak_live,
        all.peak_live * ONE_SHOT_MB,
        hours(all.longest_wait_ms),
        all.early_inside_a_judge,
        all.restarts_under_load,
        all.wakes
    );
    // One seed proves nothing either way about these, so a rerun does not fail on them.
    if alone.is_some() {
        return;
    }
    assert!(all.failed > 0, "no judge failed, so this proved nothing");
    assert!(
        all.early_inside_a_judge > 0,
        "no early pass landed inside a judge, so a cap per pass would pass"
    );
    assert!(
        all.restarts_under_load > 0,
        "no restart landed in an overload, so its episode surviving went unexercised"
    );
}

fn run_alone(plant: &str, seed: Option<u64>) -> String {
    let mut command = Command::new(std::env::current_exe().expect("this test binary"));
    command
        .args(["--exact", SWEEP_TEST, "--nocapture"])
        .env("SIM_PLANT", plant)
        .env_remove("SIM_SEEDS")
        .env_remove("SIM_SEED");
    if let Some(seed) = seed {
        command.env("SIM_SEED", seed.to_string());
    }
    let output = command.output().expect("the run");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{plant} passed: {printed}");
    printed
}

/// Each bug fails the sweep on the invariant it breaks, its seed is named, and that seed fails the same way in a process of
/// its own. A scenario no bug can fail proves nothing (section 20c's rule, part five).
#[test]
fn each_planted_bug_fails_it_and_its_seed_reruns_alone() {
    let _budget = TestBudget::new();
    for (plant, _, invariant) in PLANTS {
        let printed = run_alone(plant, None);
        let seed: u64 = printed
            .split("Rerun it alone: SIM_SEED=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|seed| seed.parse().ok())
            .unwrap_or_else(|| panic!("{plant} printed no seed: {printed}"));
        let line = format!(
            "SIM_SEED={seed} SIM_PLANT={plant} cargo nextest run -p dash --test storm {SWEEP_TEST}"
        );
        let says = |text: &str| {
            let said = text
                .lines()
                .find(|l| l.starts_with(&format!("seed {seed}: ")));
            said.unwrap_or_else(|| panic!("{plant}: no failure of seed {seed}: {text}"))
                .to_string()
        };
        for text in [line.as_str(), invariant] {
            assert!(printed.contains(text), "{plant}: {text} in: {printed}");
        }
        assert_eq!(
            printed.matches("failed. Rerun it alone").count(),
            1,
            "{printed}"
        );
        assert_eq!(
            says(&run_alone(plant, Some(seed))),
            says(&printed),
            "{plant}"
        );
    }
}

/// The run is the contract, on every machine: this number moves only when the scenario, the policy, the stream or the hash
/// changes, and a move on one machine alone is a leak of something a seed does not decide.
#[test]
fn a_seed_gives_the_same_storm_on_every_machine() {
    let _budget = TestBudget::new();
    let (one, other) = (storm(1, None), storm(2, None));
    assert_ne!(one.fingerprint, other.fingerprint);
    // Each seed twice, as sim.rs, here since this test has no retries: nextest's on the sweep would pass a leak of real time
    // or order that breaks an invariant on few seeds, and its 2 s has no room for a second run.
    for seed in 0..PR_SEEDS {
        assert_eq!(
            storm(seed, None),
            storm(seed, None),
            "seed {seed}: its rerun"
        );
    }
    assert_eq!(format!("{:016x}", one.fingerprint), "59bfba272901c5aa");
}
