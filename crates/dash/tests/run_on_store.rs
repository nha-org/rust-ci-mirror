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
            waits: vec![],
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
    // The script's list synced into the real store, as each pass does first: a pick reads only the store.
    tasks.sync(&s.queue.borrow()).expect("the sync");
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
            key: false,
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
        waits: vec![],
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

/// The older app, asked nothing: a pick that called it would panic here.
struct NoOlderApp;

impl TaskList for NoOlderApp {
    fn brief(&self, id: &str) -> Result<String, String> {
        panic!("the older app was asked the brief of {id}")
    }
    fn judge(&self, id: &str, _said: &str) -> Result<Verdict, String> {
        panic!("the older app was asked to judge {id}")
    }
    fn merged(&self, id: &str, _pr: &str) -> Result<(), String> {
        panic!("the older app was told {id} merged")
    }
    fn ended(&self, id: &str, _why: &str) -> Result<(), String> {
        panic!("the older app was told {id} ended")
    }
    fn flag(&self, id: &str, _what: &str) -> Result<(), String> {
        panic!("the older app was asked to flag {id}")
    }
    fn waiting(&self, id: &str, _on: Option<&Outcome>) -> Result<(), String> {
        panic!("the older app was told what {id} waits on")
    }
}

/// The older app's database as it files tasks.edn rows, as much of it as the sync reads.
fn older_db(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("dashboard_dev.db");
    let db = dash_store::Connection::open(&path).expect("the older app's db");
    db.execute_batch(
        "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT,
           blocked_by TEXT, owner TEXT);
         CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
         CREATE TABLE project_settings (project TEXT, field TEXT, value TEXT NOT NULL);
         INSERT INTO tasks VALUES ('tasks.edn', 1, 'after', 'open', '{}', 'After', char(57344) || '@waits-on/0' || char(57345), 'rust'),
           ('tasks.edn', 2, 'first', 'open', '{}', 'First', NULL, 'rust'),
           ('tasks.edn', 3, 'his', 'open', '{}', 'His', NULL, 'claude'),
           ('tasks.edn', 4, 'put-off', 'open', '{\"later\":\"next week\"}', 'Later', NULL, 'rust');
         INSERT INTO links VALUES ('tasks:after', 'waits-on', 'tasks:first', 0);",
    )
    .expect("its tables");
    path
}

/// One look of the picker on `tasks`, the older app a stand-in that panics if anything asks it.
fn look_alone(tasks: &mut SqliteTasks, s: &Tools) -> Pick {
    let mut ports = Ports {
        tasks,
        list: &NoOlderApp,
        terminal: s,
        launcher: s,
        agent: s,
        forge: s,
        policy: &Defaults,
        step: &|_, _| {},
    };
    next(&mut ports).expect("a look")
}

fn listed(id: &str, waits: &[&str]) -> Queued {
    Queued {
        id: id.into(),
        waits: waits.iter().map(|w| w.to_string()).collect(),
        ..Queued::default()
    }
}

/// rust-own-queue's done-when 1: `dash orch`'s pick reads the open owner-rust tasks, their waits and his seats from Rust's store,
/// and the older app is asked nothing (2026-11-09: its queue offered one ended task while six were ready).
#[test]
fn the_pick_reads_tasks_waits_and_seats_from_rust_s_store_and_never_asks_the_older_app() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("own-queue-pick");
    let mut tasks = store(&dir.0);
    let s = tools(&[]);
    let err = TaskStore::queue(&tasks).expect_err("no sync yet");
    assert!(err.contains("no sync"), "{err}");
    let list = Queue {
        seats: 5,
        tasks: vec![listed("after", &["first"]), listed("first", &[])],
    };
    tasks.sync(&list).expect("the sync");
    let pick = look_alone(&mut tasks, &s);
    assert!(
        matches!(pick, Pick::Start { ref id, .. } if id == "first"),
        "after waits on first, as the store says: {pick:?}"
    );
    // Its wait met once first is merged here: the store's rows decide.
    tasks.add("first", &[]).expect("a row");
    tasks.claim("first").expect("claim");
    let merged = dash_core::tasks::Run {
        pane: None,
        session: None,
        outcome: Outcome::Merged("https://pr/1".into()),
    };
    tasks.record("first", &merged).expect("merged");
    // Its run told the list itself, as run_task does: what is left to tell is a run's, never a pick's.
    TaskStore::told(&mut tasks, "first").expect("told");
    let pick = look_alone(&mut tasks, &s);
    assert!(
        matches!(pick, Pick::Start { ref id, .. } if id == "after"),
        "{pick:?}"
    );
    // His seats are the store's: none left, nothing starts.
    tasks.sync(&Queue { seats: 0, ..list }).expect("seats");
    let pick = look_alone(&mut tasks, &s);
    assert!(matches!(pick, Pick::Wait(_)), "{pick:?}");
}

/// rust-own-queue's done-when 2: what the older app files in tasks.edn is in Rust's store after one sync, the first step of a
/// pass, and what it closes leaves the list at the next.
#[test]
fn a_task_the_older_app_files_reaches_rust_s_store_within_one_sync() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("own-queue-sync");
    let mut tasks = store(&dir.0);
    let older = older_db(&dir.0);
    let sync = |tasks: &mut SqliteTasks| {
        let rows = dash_store::older::queue(&older).expect("the older app's rows");
        tasks.sync(&rows).expect("the sync")
    };
    let ids = |tasks: &SqliteTasks| -> Vec<String> {
        let list = TaskStore::queue(tasks).expect("the store's list");
        list.tasks.into_iter().map(|t| t.id).collect()
    };
    assert!(sync(&mut tasks) > 0);
    assert_eq!(sync(&mut tasks), 0, "an unchanged list writes nothing");
    assert_eq!(
        ids(&tasks),
        ["after", "first"],
        "Rust's open tasks, put-off and his left out"
    );
    let waits = &TaskStore::queue(&tasks).expect("list").tasks[0].waits;
    assert_eq!(waits, &["first"], "its link, its ref's own text left out");
    let db = dash_store::Connection::open(&older).expect("db");
    let row = "INSERT INTO tasks VALUES ('tasks.edn', 0, 'new', 'open', '{}', 'New', NULL, 'rust')";
    db.execute(row, []).expect("a new task");
    sync(&mut tasks);
    assert_eq!(
        ids(&tasks),
        ["new", "after", "first"],
        "filed since, first in its order"
    );
    let pick = look_alone(&mut tasks, &tools(&[]));
    assert!(
        matches!(pick, Pick::Start { ref id, .. } if id == "new"),
        "{pick:?}"
    );
    db.execute("UPDATE tasks SET stage = 'done' WHERE id = 'new'", [])
        .expect("done");
    sync(&mut tasks);
    assert_eq!(ids(&tasks), ["after", "first"], "done there, unlisted here");
}
