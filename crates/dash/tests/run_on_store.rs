//! The orchestrator on the real store and the real policy, a stand-in for each tool: what only the store's locks, stamps and
//! rows can show. The binary is the one crate that may reach both orch and store (section 3).
use dash_core::tasks::{
    Agent, Checks, Forge, Launcher, Limit, Outcome, Pick, Pr, Queue, Queued, Screen, Seen, Task,
    TaskList, TaskStore, Terminal, Verdict,
};
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use dash_orch::{Pace, Ports, next, run_task};
use dash_policy::Defaults;
use dash_store::tasks::{SqliteTasks, open};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

const PACE: Pace = Pace {
    poll: Duration::ZERO,
    start_limit: Duration::from_secs(5),
    limit: Duration::from_secs(5),
    stuck_limit: Duration::from_secs(5),
    stop_polls: 3,
    ci_poll: Duration::ZERO,
    ci_limit: Duration::from_secs(5),
    judge_poll: Duration::ZERO,
    patient_poll: Duration::ZERO,
    limit_retry: Duration::ZERO,
};

const MINE: &str = "s1";
const HIS: &str = "his-orchestrator";

/// Each read plays the next entry, the last one stays.
fn take<T: Clone>(script: &RefCell<VecDeque<T>>) -> Option<T> {
    let mut script = script.borrow_mut();
    if script.len() > 1 {
        return script.pop_front();
    }
    script.front().cloned()
}

/// A session listed in the pane, by id and whether idle.
type Look = Option<(&'static str, bool)>;

struct Tools<'a> {
    did: RefCell<Vec<String>>,
    looks: RefCell<VecDeque<Look>>,
    outcomes: RefCell<VecDeque<Outcome>>,
    verdicts: RefCell<VecDeque<Verdict>>,
    limits: RefCell<VecDeque<Vec<Limit>>>,
    queue: RefCell<Queue>,
    on_close: Option<&'a dyn Fn()>,
    judged: Cell<u32>,
    /// A run that never reaches a prompt: what makes a failed one to retry.
    screen: &'static str,
}

fn tools<'a>(looks: &[Look]) -> Tools<'a> {
    Tools {
        did: RefCell::default(),
        looks: RefCell::new(looks.iter().copied().collect()),
        outcomes: RefCell::new([Outcome::Done("done".into())].into_iter().collect()),
        verdicts: RefCell::new([Verdict::Met].into_iter().collect()),
        limits: RefCell::new([vec![]].into_iter().collect()),
        queue: RefCell::new(queue(None)),
        on_close: None,
        judged: Cell::new(0),
        screen: "up",
    }
}

fn queue(reopened_ms: Option<i64>) -> Queue {
    Queue {
        seats: 5,
        tasks: vec![Queued {
            id: "t1".into(),
            title: String::new(),
            reopened_ms,
        }],
    }
}

impl Tools<'_> {
    fn note(&self, what: String) -> Result<(), String> {
        self.did.borrow_mut().push(what);
        Ok(())
    }
    fn count(&self, what: &str) -> usize {
        self.did.borrow().iter().filter(|d| *d == what).count()
    }
    fn typed(&self) -> Vec<String> {
        let did = self.did.borrow();
        let typed = did.iter().filter(|d| d.starts_with("send"));
        typed.cloned().collect()
    }
}

impl Terminal for Tools<'_> {
    fn open(&self, _folder: &str, _label: &str) -> Result<String, String> {
        Ok("p1".into())
    }
    fn run(&self, _pane: &str, _line: &str) -> Result<(), String> {
        Ok(())
    }
    fn agent(&self, _pane: &str) -> Result<Option<Seen>, String> {
        Ok(take(&self.looks).flatten().map(|(session, idle)| Seen {
            idle,
            asking: false,
            session: Some(session.to_string()),
        }))
    }
    fn sessions(&self) -> Result<usize, String> {
        Ok(0)
    }
    fn read(&self, _pane: &str) -> Result<String, String> {
        Ok(self.screen.to_string())
    }
    fn send(&self, _pane: &str, text: &str) -> Result<(), String> {
        self.note(format!("send {text}"))
    }
    fn key(&self, _pane: &str, _key: &str) -> Result<(), String> {
        Ok(())
    }
    fn close(&self, pane: &str) -> Result<(), String> {
        if let Some(on_close) = self.on_close {
            on_close();
        }
        self.note(format!("close {pane}"))
    }
}

impl Launcher for Tools<'_> {
    fn command(&self, _task: &Task, _account: Option<&str>) -> Result<String, String> {
        Ok("start".into())
    }
}

impl Agent for Tools<'_> {
    fn screen(&self, shown: &str) -> Screen {
        match shown {
            "up" => Screen::Ready,
            _ => Screen::Starting,
        }
    }
    fn outcome(&self, _folder: &str, _session: &str) -> Result<Option<Outcome>, String> {
        Ok(take(&self.outcomes))
    }
    fn stop_words(&self) -> String {
        "leave".into()
    }
    fn limits(&self) -> Result<Vec<Limit>, String> {
        Ok(take(&self.limits).unwrap_or_default())
    }
    fn reachable(&self) -> bool {
        true
    }
}

impl TaskList for Tools<'_> {
    fn waiting(&self, _id: &str, _on: Option<&Outcome>) -> Result<(), String> {
        Ok(())
    }
    fn queue(&self) -> Result<Queue, String> {
        Ok(self.queue.borrow().clone())
    }
    fn brief(&self, id: &str) -> Result<String, String> {
        Ok(format!("the brief of {id}"))
    }
    fn judge(&self, _id: &str, _said: &str) -> Result<Verdict, String> {
        self.judged.set(self.judged.get() + 1);
        take(&self.verdicts).ok_or("no verdict".to_string())
    }
    fn merged(&self, _id: &str, _pr: &str) -> Result<(), String> {
        Ok(())
    }
    fn ended(&self, _id: &str, _why: &str) -> Result<(), String> {
        Ok(())
    }
    fn flag(&self, _id: &str, _what: &str) -> Result<(), String> {
        Ok(())
    }
}

impl Forge for Tools<'_> {
    fn worktree(&self, id: &str) -> Result<String, String> {
        Ok(format!("/work/rust-{id}"))
    }
    fn pr(&self, _id: &str) -> Result<Option<Pr>, String> {
        // An open PR comes from the session's work: none before its first words.
        if self.typed().is_empty() {
            return Ok(None);
        }
        Ok(Some(Pr {
            number: 7,
            url: "https://pr/7".into(),
            head: "abc".into(),
            merged: false,
        }))
    }
    fn checks(&self, _pr: &Pr) -> Result<Checks, String> {
        Ok(Checks::Green)
    }
    fn stale(&self, _pr: &Pr) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    fn merge(&self, _pr: &Pr) -> Result<(), String> {
        Ok(())
    }
    fn leave(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }
}

fn ports<'a>(tasks: &'a mut SqliteTasks, s: &'a Tools) -> Ports<'a> {
    Ports {
        tasks,
        list: s,
        terminal: s,
        launcher: s,
        agent: s,
        forge: s,
        policy: &Defaults,
        step: &|_, _| {},
    }
}

fn store(dir: &Path) -> SqliteTasks {
    open(dir, dir).expect("the store")
}

fn field(tasks: &SqliteTasks, name: &str) -> String {
    let row = tasks.show("t1").expect("show").expect("row");
    let line = row
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name}: ")));
    line.expect(name).to_string()
}

/// A whole run: listed idle at its brief and its report, still there when told to leave, gone.
const RUN: [Look; 4] = [
    Some((MINE, true)),
    Some((MINE, true)),
    Some((MINE, true)),
    None,
];

/// The audit's finding 11: on the real store, whose finish a reopen must be newer than.
#[test]
fn a_task_reopened_after_its_merge_runs_again_one_older_never() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("run-reopen");
    let mut tasks = store(&dir.0);
    let s = tools(&RUN);
    let first = run_task("t1", None, &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(first.outcome, Outcome::Merged("https://pr/7".into()));
    assert_eq!(field(&tasks, "status"), "finished");
    let finished_ms = tasks.rows().expect("rows")[0].last_ms;

    let look = |tasks: &mut SqliteTasks, s: &Tools| next(&mut ports(tasks, s)).expect("a look");
    let wait = |pick: Pick| assert!(matches!(pick, Pick::Wait(_)), "{pick:?}");
    wait(look(&mut tasks, &s));
    *s.queue.borrow_mut() = queue(Some(finished_ms - 1));
    wait(look(&mut tasks, &s));

    *s.queue.borrow_mut() = queue(Some(finished_ms + 1));
    let pick = look(&mut tasks, &s);
    assert!(
        matches!(pick, Pick::Start { ref id, .. } if id == "t1"),
        "{pick:?}"
    );
    let s2 = tools(&RUN);
    let again = run_task("t1", None, &mut ports(&mut tasks, &s2), PACE).expect("recorded");
    assert_eq!(again.outcome, Outcome::Merged("https://pr/7".into()));
    assert_eq!(field(&tasks, "attempts"), "1", "its tries start over");
    assert_eq!(s2.judged.get(), 1);
    // Reopened once: its second finish is newer than that reopen.
    wait(look(&mut tasks, &s2));
}

/// `dash task reopen`'s other half: a run that ended without a merge, its worktree kept, starts over on the newer `:since`.
#[test]
fn a_task_reopened_after_its_run_ended_runs_again_with_its_tries_reset() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("run-reopen-ended");
    let mut tasks = store(&dir.0);
    let s = tools(&RUN);
    *s.verdicts.borrow_mut() = [Verdict::NotOurs("given up".into())].into_iter().collect();
    let first = run_task("t1", None, &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(first.outcome, Outcome::Ended("given up".into()));
    assert_eq!(field(&tasks, "status"), "finished");
    let finished_ms = tasks.rows().expect("rows")[0].last_ms;

    *s.queue.borrow_mut() = queue(Some(finished_ms + 1));
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(
        matches!(pick, Pick::Start { ref id, .. } if id == "t1"),
        "{pick:?}"
    );
    let s2 = tools(&RUN);
    let again = run_task("t1", None, &mut ports(&mut tasks, &s2), PACE).expect("recorded");
    assert_eq!(again.outcome, Outcome::Merged("https://pr/7".into()));
    assert_eq!(field(&tasks, "attempts"), "1");
}

/// The audit's finding 12: a limit stop spends no nudge, and its row holds the session for the whole wait.
#[test]
fn a_limit_stop_waits_for_room_with_no_nudge_its_claim_held() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("run-limit");
    let mut tasks = store(&dir.0);
    let s = tools(&[
        Some((MINE, true)),
        Some((MINE, true)),
        Some((MINE, true)),
        Some((MINE, true)),
        Some((MINE, true)),
        Some((MINE, false)),
        Some((MINE, true)),
        Some((MINE, true)),
        None,
    ]);
    let limit = "You've hit your weekly limit · resets Nov 8 at 6am (Europe/Rome)";
    *s.outcomes.borrow_mut() = [Outcome::Limited(limit.into()), Outcome::Done("done".into())]
        .into_iter()
        .collect();
    let reading = |week: f64| {
        vec![Limit {
            account: "a".into(),
            five_hours: 0.0,
            five_hours_resets: 0,
            week,
            week_resets: 1,
        }]
    };
    *s.limits.borrow_mut() = [reading(100.0), reading(100.0), reading(5.0)]
        .into_iter()
        .collect();
    let run = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(run.outcome, Outcome::Merged("https://pr/7".into()));
    assert_eq!(
        s.typed(),
        ["send the brief of t1", "send continue", "send leave"],
        "no nudge"
    );
    assert_eq!(s.judged.get(), 1, "its report alone");
    assert_eq!(field(&tasks, "attempts"), "1");
    assert_eq!(field(&tasks, "said"), "https://pr/7");
}

/// The audit's finding 13: at the moment its pane closes the failed run still holds its row and its lock, against this data
/// folder and any other; once its stop returned the task is free.
#[test]
fn a_retry_starts_only_after_the_stop_returned_the_lock_beside_the_worktree() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("run-retry");
    let (data, other, worktrees) = (dir.0.join("data"), dir.0.join("other"), dir.0.join("trees"));
    for folder in [&data, &other] {
        std::fs::create_dir_all(folder).expect("dir");
    }
    let mut tasks = open(&data, &worktrees).expect("the store");
    let seen = RefCell::new(vec![]);
    let at_close = || {
        // A look of `dash orch` on the same data, then a run on another folder's: the same worktree either way.
        let same = open(&data, &worktrees).expect("the store");
        let status = same.status("t1").expect("status").expect("a row");
        let mut second = open(&other, &worktrees).expect("the store");
        let claimed = second
            .claim("t1")
            .map(drop)
            .map_err(|e| e.contains("a run holds it"));
        seen.borrow_mut().push((status, claimed));
    };
    let mut s = tools(&[Some((MINE, true)), Some((MINE, true)), None]);
    s.screen = "boot";
    s.on_close = Some(&at_close);
    let pace = Pace {
        start_limit: Duration::ZERO,
        ..PACE
    };
    let run = run_task("t1", None, &mut ports(&mut tasks, &s), pace).expect("released");
    assert!(matches!(run.outcome, Outcome::Failed(_)), "{run:?}");
    assert_eq!(s.count("close p1"), 1);
    assert_eq!(
        *seen.borrow(),
        [("claimed".to_string(), Err(true))],
        "held while it stopped"
    );
    assert!(worktrees.join("rust-t1.lock").exists());
    assert_eq!(field(&tasks, "status"), "open");
    assert_eq!(field(&tasks, "session"), MINE, "its stop saw it");
    assert_eq!(tasks.claim("t1").expect("free now"), 2);
}

/// The audit's finding 10: a reboot's crash, its saved session read back from the store. The same pane id, another session.
#[test]
fn after_a_crash_a_reused_pane_is_left_and_the_saved_session_s_is_closed() {
    let _budget = TestBudget::new();
    for (listed, closes) in [(HIS, 0), (MINE, 1)] {
        let dir = TestDir::new(&format!("run-reused-{closes}"));
        let mut run_side = store(&dir.0);
        run_side.claim("t1").expect("claim");
        run_side.opened("t1", "p1").expect("opened");
        run_side.adopted("t1", MINE).expect("adopted");
        // What a crash does to a claim: the process's handle on its lock ends.
        drop(run_side);
        let mut tasks = store(&dir.0);
        let s = tools(&[Some((listed, true))]);
        let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
        assert!(
            matches!(pick, Pick::Start { ref id, .. } if id == "t1"),
            "{pick:?}"
        );
        assert_eq!(s.count("close p1"), closes, "{listed}");
        assert!(s.typed().is_empty());
        assert_eq!(field(&tasks, "status"), "open");
    }
}

/// A live run's session seen once is what the store holds, so a crash at any later moment still names it.
#[test]
fn the_session_is_saved_when_first_seen_not_at_the_end() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("run-saved");
    let mut tasks = store(&dir.0);
    let reader = store(&dir.0);
    let at_close = || assert_eq!(field(&reader, "session"), MINE);
    let mut s = tools(&RUN);
    s.on_close = Some(&at_close);
    run_task("t1", None, &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(s.count("close p1"), 1);
}

/// The page's half of this same case is tasks.rs's the_page_names_what_plan_runs_next_and_why_the_others_wait.
#[test]
fn the_picker_starts_what_the_page_names() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("run-plan");
    let mut tasks = open(&dir.0, &dir.0).expect("open");
    tasks.add("first", &[]).expect("add");
    tasks.add("after", &["first".into()]).expect("add");
    let s = tools(&[]);
    let queued = |id: &str| Queued {
        id: id.into(),
        title: String::new(),
        reopened_ms: None,
    };
    let (seats, tasks_queued) = (5, vec![queued("after"), queued("first"), queued("also")]);
    *s.queue.borrow_mut() = Queue {
        seats,
        tasks: tasks_queued,
    };
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(
        matches!(pick, Pick::Start { ref id, .. } if id == "first"),
        "{pick:?}"
    );
}
