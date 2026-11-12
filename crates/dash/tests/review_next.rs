//! `dash review next` (task rust-review-next-port): the PR lane's pick and claim, on Rust's store and a stand-in forge.
use dash_core::tasks::{HOLD_MS, LANE, ORCHESTRATOR, TaskStore};
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use dash_orch::review::{Lane, Next, Open, next, park};
use dash_store::tasks::SqliteTasks;

/// A forge: its open PRs, the green ones by number, the tasks a reviewer works for.
struct StandIn {
    open: Vec<Open>,
    green: Vec<u64>,
    reviewed: Vec<String>,
}

impl Lane for StandIn {
    fn open(&self) -> Result<Vec<Open>, String> {
        Ok(self.open.clone())
    }
    fn green(&self, pr: &Open) -> Result<bool, String> {
        Ok(self.green.contains(&pr.number))
    }
    fn reviewed(&self) -> Result<Vec<String>, String> {
        Ok(self.reviewed.clone())
    }
}

fn pr(number: u64, branch: &str, day: u32) -> Open {
    Open {
        number,
        url: format!("https://github.com/nha-org/site_perso/pull/{number}"),
        branch: branch.into(),
        created: format!("2026-11-{day:02}T10:00:00Z"),
    }
}

fn store(dir: &TestDir) -> SqliteTasks {
    let (data, locks) = (dir.0.join("data"), dir.0.join("locks"));
    std::fs::create_dir_all(&data).expect("folder");
    dash_store::tasks::open(&data, &locks).expect("the store")
}

fn holders(tasks: &SqliteTasks) -> Vec<(u64, String)> {
    let holds = tasks.holds().expect("the holds");
    holds.into_iter().map(|h| (h.pr, h.holder)).collect()
}

/// Done-when 1: the oldest green PR nobody claims, its claim recorded; asked again, the lane's own is handed back as held.
#[test]
fn review_next_claims_the_oldest_green_pr_and_records_it() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("review-next-oldest");
    let mut tasks = store(&dir);
    tasks.hold_pr(1, ORCHESTRATOR, "his").expect("a hold");
    let lane = StandIn {
        // listed newest first, as gh does
        open: vec![
            pr(4, "b4", 4),
            pr(3, "b3", 3),
            pr(2, "b2", 2),
            pr(1, "b1", 1),
            pr(5, "b5", 1),
        ],
        green: vec![1, 3, 4],
        reviewed: vec![],
    };
    // 1 held, 5 and 2 not green: 3, the oldest left
    assert_eq!(next(&lane, &mut tasks), Ok(Next::Took(pr(3, "b3", 3))));
    let lane_holds = |t: &SqliteTasks| holders(t).into_iter().filter(|(_, h)| h == LANE).count();
    assert_eq!(
        holders(&tasks),
        [(1, ORCHESTRATOR.into()), (3, LANE.into())]
    );
    assert_eq!(next(&lane, &mut tasks), Ok(Next::Held(pr(3, "b3", 3))));
    assert_eq!(lane_holds(&tasks), 1);
    // parked, it is passed over: 4 next
    assert_eq!(park(&mut tasks, 3, "handed over"), Ok(true));
    assert_eq!(next(&lane, &mut tasks), Ok(Next::Took(pr(4, "b4", 4))));
    assert_eq!(park(&mut tasks, 4, "waits on him"), Ok(true));
    assert_eq!(next(&lane, &mut tasks), Ok(Next::None));
    // a PR the lane does not hold is not parked
    assert_eq!(park(&mut tasks, 1, "no"), Ok(false));
}

/// Done-when 1: a PR whose task a reviewer works for is passed over.
#[test]
fn a_pr_its_reviewer_works_on_is_never_handed_out() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("review-next-reviewer");
    let mut tasks = store(&dir);
    let lane = StandIn {
        open: vec![pr(7, "task/y", 1)],
        green: vec![7],
        reviewed: vec!["y".into()],
    };
    assert_eq!(next(&lane, &mut tasks), Ok(Next::None));
}

/// Done-when 2: a PR whose task's run is claimed in Rust's store is its run's, never the lane's.
#[test]
fn a_pr_its_own_run_holds_is_never_handed_out() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("review-next-own-run");
    let mut tasks = store(&dir);
    tasks.claim("x").expect("its run");
    let lane = StandIn {
        open: vec![pr(8, "task/x", 1), pr(9, "task/z", 2)],
        green: vec![8, 9],
        reviewed: vec![],
    };
    // z has no row: Rust runs nothing on it
    assert_eq!(next(&lane, &mut tasks), Ok(Next::Took(pr(9, "task/z", 2))));
    assert_eq!(park(&mut tasks, 9, "done"), Ok(true));
    // a run's own hold too, whatever its row
    tasks.hold_pr(10, "run w", "its run").expect("a hold");
    let lane = StandIn {
        open: vec![pr(10, "task/w", 1)],
        green: vec![10],
        reviewed: vec![],
    };
    assert_eq!(next(&lane, &mut tasks), Ok(Next::None));
}

/// Done-when 2: a PR `dash pr hold` holds is never handed out.
#[test]
fn a_pr_dash_pr_hold_holds_is_never_handed_out() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("review-next-pr-hold");
    let mut tasks = store(&dir);
    tasks
        .hold_pr(11, ORCHESTRATOR, "dash pr hold")
        .expect("a hold");
    let lane = StandIn {
        open: vec![pr(11, "b", 1)],
        green: vec![11],
        reviewed: vec![],
    };
    assert_eq!(next(&lane, &mut tasks), Ok(Next::None));
    assert_eq!(holders(&tasks), [(11, ORCHESTRATOR.into())]);
}

/// A parked hold locks nobody out, and the orchestrator takes the lane's live one: a lane that crashed holds no PR 6 hours
/// against `dash land` or a run (/code-review of 870c6830f).
#[test]
fn a_lane_hold_never_locks_out_the_orchestrator_or_a_run() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("review-next-takeover");
    let mut tasks = store(&dir);
    let lane = StandIn {
        open: vec![pr(1, "b1", 1), pr(2, "b2", 2)],
        green: vec![1, 2],
        reviewed: vec![],
    };
    assert_eq!(next(&lane, &mut tasks), Ok(Next::Took(pr(1, "b1", 1))));
    assert_eq!(park(&mut tasks, 1, "handed over"), Ok(true));
    tasks
        .hold_pr(1, "run x", "its run")
        .expect("a run takes a parked hold");
    assert_eq!(next(&lane, &mut tasks), Ok(Next::Took(pr(2, "b2", 2))));
    tasks
        .hold_pr(2, ORCHESTRATOR, "dash land")
        .expect("the orchestrator takes the lane's");
    assert_eq!(
        holders(&tasks),
        [(1, "run x".into()), (2, ORCHESTRATOR.into())]
    );
    // both held by others now
    assert_eq!(next(&lane, &mut tasks), Ok(Next::None));
}

/// The lane's hold outlives `HOLD_MS`, as a review does: the older app's claim held 6 hours. Another's untouched one ends.
#[test]
fn a_lane_hold_outlasts_a_plain_one() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("review-next-long-hold");
    let mut tasks = store(&dir);
    let then = 1_000_000_000_000;
    tasks.hold_at(then, 1, LANE, "review").expect("a hold");
    tasks.hold_at(then, 2, ORCHESTRATOR, "his").expect("a hold");
    let later = tasks.holds_at(then + HOLD_MS + 1).expect("the holds");
    assert_eq!(later.iter().map(|h| h.pr).collect::<Vec<_>>(), [1]);
    assert_eq!(
        tasks
            .holds_at(then + 6 * 3600 * 1000 + 1)
            .expect("the holds"),
        []
    );
}
