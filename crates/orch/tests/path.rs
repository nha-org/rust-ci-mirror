//! The whole path through a stand-in for each port, Forge and Policy included: section 20b's phase 2 with no tool in it.
use dash_core::tasks::{
    Agent, Checks, Forge, Freed, Launcher, Limit, Outcome, Pick, Policy, Pr, Queue, Queued, Run,
    Screen, Seen, Snapshot, Step, Task, TaskList, TaskStore, Terminal, Verdict,
};
use dash_meter::memory::TestBudget;
use dash_orch::{Pace, Ports, next, run_task};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

#[derive(Clone, Copy)]
enum Look {
    Gone,
    Idle,
    Working,
    Asking,
}
use Look::*;

const DONE: &str = "it is done";

/// The store's stand-in: a task claimable once a run lets it go. `crashed`: what `free_crashed` finds, once.
#[derive(Default)]
struct Store {
    rows: HashMap<String, (String, u32)>,
    run: Option<Run>,
    panes: Vec<String>,
    crashed: Vec<Freed>,
    untold: Option<Outcome>,
    note: Option<Notes>,
}

type Notes = std::rc::Rc<RefCell<Vec<String>>>;

impl TaskStore for Store {
    fn claim(&mut self, id: &str) -> Result<u32, String> {
        let row = self.rows.entry(id.into()).or_insert(("open".into(), 0));
        if row.0 != "open" {
            return Err(format!("{id}: not an open task"));
        }
        *row = ("claimed".into(), row.1 + 1);
        Ok(row.1)
    }
    fn opened(&mut self, _id: &str, pane: &str) -> Result<(), String> {
        self.panes.push(pane.into());
        Ok(())
    }
    fn record(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.rows.entry(id.into()).or_default().0 = "finished".into();
        self.run = Some(run.clone());
        if let Some(note) = &self.note {
            note.borrow_mut().push("record".into());
        }
        Ok(())
    }
    fn release(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.rows.entry(id.into()).or_default().0 = "open".into();
        self.run = Some(run.clone());
        Ok(())
    }
    fn free_crashed(&mut self) -> Result<Vec<Freed>, String> {
        let freed = std::mem::take(&mut self.crashed);
        for f in &freed {
            self.rows.insert(f.id.clone(), ("open".into(), f.attempts));
        }
        Ok(freed)
    }
    fn status(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self.rows.get(id).map(|r| r.0.clone()))
    }
    fn untold(&self, _id: &str) -> Result<Option<Outcome>, String> {
        Ok(self.untold.clone())
    }
    fn claims(&self) -> Result<Vec<Option<String>>, String> {
        let live = self.rows.values().filter(|r| r.0 == "claimed");
        Ok(live.map(|_| self.panes.first().cloned()).collect())
    }
    fn told(&mut self, _id: &str) -> Result<(), String> {
        self.untold = None;
        Ok(())
    }
}

/// Every other port's stand-in, playing a script: each read takes the next entry, and the last one stays. Each call's words
/// go on `did`, so a test reads the whole path in order.
struct Script {
    did: Notes,
    screens: RefCell<VecDeque<&'static str>>,
    looks: RefCell<VecDeque<Look>>,
    /// How many times `outcome` answers none.
    no_outcome: Cell<u32>,
    outcome: RefCell<Outcome>,
    verdicts: RefCell<VecDeque<Result<Verdict, String>>>,
    checks: RefCell<VecDeque<Result<Checks, String>>>,
    pr: RefCell<Option<Pr>>,
    merge_refusal: RefCell<Option<String>>,
    /// How many merges fail before one lands.
    merge_hiccups: Cell<u32>,
    stale: Vec<String>,
    flag_fails: Cell<u32>,
    said: RefCell<String>,
    merged_fails: Cell<bool>,
    open_fails: bool,
    worktree_fails: bool,
    looks_fail: Cell<u32>,
    reads_fail: Cell<u32>,
    limits: Vec<Limit>,
    queue: Vec<&'static str>,
    sessions: usize,
}

fn pr(merged: bool) -> Pr {
    Pr {
        number: 7,
        url: "https://pr/7".into(),
        head: "abc".into(),
        merged,
    }
}

fn script(screens: &[&'static str], looks: &[Look]) -> Script {
    Script {
        did: Notes::default(),
        screens: RefCell::new(screens.iter().copied().collect()),
        looks: RefCell::new(looks.iter().copied().collect()),
        no_outcome: Cell::new(0),
        outcome: RefCell::new(Outcome::Done(DONE.into())),
        verdicts: RefCell::new([Ok(Verdict::Met)].into_iter().collect()),
        checks: RefCell::new([Ok(Checks::Green)].into_iter().collect()),
        pr: RefCell::new(Some(pr(false))),
        merge_refusal: RefCell::new(None),
        merge_hiccups: Cell::new(0),
        stale: vec![],
        flag_fails: Cell::new(0),
        said: RefCell::new(String::new()),
        merged_fails: Cell::new(false),
        open_fails: false,
        worktree_fails: false,
        looks_fail: Cell::new(0),
        reads_fail: Cell::new(0),
        limits: vec![],
        queue: vec!["t1"],
        sessions: 0,
    }
}

fn take<T: Clone>(script: &RefCell<VecDeque<T>>) -> Option<T> {
    let mut script = script.borrow_mut();
    if script.len() > 1 {
        return script.pop_front();
    }
    script.front().cloned()
}

impl Script {
    fn note(&self, what: impl Into<String>) -> Result<(), String> {
        self.did.borrow_mut().push(what.into());
        Ok(())
    }
    fn did(&self) -> Vec<String> {
        self.did.borrow().clone()
    }
    fn count(&self, what: &str) -> usize {
        self.did().iter().filter(|d| *d == what).count()
    }
}

impl Terminal for Script {
    fn open(&self, folder: &str, label: &str) -> Result<String, String> {
        if self.open_fails {
            return Err("no terminal".into());
        }
        self.note(format!("open {folder} {label}"))?;
        Ok("p1".into())
    }
    fn run(&self, _pane: &str, line: &str) -> Result<(), String> {
        self.note(format!("run {line}"))
    }
    fn agent(&self, _pane: &str) -> Result<Option<Seen>, String> {
        if self.looks_fail.get() > 0 {
            self.looks_fail.set(self.looks_fail.get() - 1);
            return Err("slow".into());
        }
        let session = Some("s1".to_string());
        Ok(match take(&self.looks).unwrap_or(Gone) {
            Gone => None,
            Idle => Some(Seen {
                idle: true,
                asking: false,
                session,
            }),
            Working => Some(Seen {
                idle: false,
                asking: false,
                session,
            }),
            Asking => Some(Seen {
                idle: false,
                asking: true,
                session,
            }),
        })
    }
    fn sessions(&self) -> Result<usize, String> {
        Ok(self.sessions)
    }
    fn read(&self, _pane: &str) -> Result<String, String> {
        Ok(take(&self.screens).unwrap_or_default().to_string())
    }
    fn send(&self, _pane: &str, text: &str) -> Result<(), String> {
        self.note(format!("send {text}"))
    }
    fn key(&self, _pane: &str, key: &str) -> Result<(), String> {
        self.note(format!("key {key}"))
    }
    fn close(&self, pane: &str) -> Result<(), String> {
        self.note(format!("close {pane}"))
    }
}

impl Launcher for Script {
    fn command(&self, task: &Task, account: Option<&str>) -> Result<String, String> {
        Ok(format!(
            "start {} on {}",
            task.id,
            account.unwrap_or("none")
        ))
    }
}

impl Agent for Script {
    fn screen(&self, shown: &str) -> Screen {
        match shown {
            "up" => Screen::Ready,
            "dialog" => Screen::Keys(vec!["Down".into(), "Enter".into()]),
            _ => Screen::Starting,
        }
    }
    fn outcome(&self, _folder: &str, _session: &str) -> Result<Option<Outcome>, String> {
        if self.reads_fail.get() > 0 {
            self.reads_fail.set(self.reads_fail.get() - 1);
            return Err("EIO".into());
        }
        if self.no_outcome.get() > 0 {
            self.no_outcome.set(self.no_outcome.get() - 1);
            return Ok(None);
        }
        Ok(Some(self.outcome.borrow().clone()))
    }
    fn stop_words(&self) -> String {
        "leave".into()
    }
    fn limits(&self) -> Result<Vec<Limit>, String> {
        Ok(self.limits.clone())
    }
}

impl TaskList for Script {
    fn queue(&self) -> Result<Queue, String> {
        let task = |id: &&str| Queued {
            id: id.to_string(),
            title: String::new(),
        };
        Ok(Queue {
            seats: 5,
            tasks: self.queue.iter().map(task).collect(),
        })
    }
    fn brief(&self, id: &str) -> Result<String, String> {
        Ok(format!("the brief of {id}"))
    }
    fn judge(&self, _id: &str, said: &str) -> Result<Verdict, String> {
        self.note("judge")?;
        *self.said.borrow_mut() = said.to_string();
        take(&self.verdicts).expect("a verdict")
    }
    fn merged(&self, id: &str, pr: &str) -> Result<(), String> {
        self.note(format!("merged {id} {pr}"))?;
        if self.merged_fails.get() {
            return Err("the app restarts".into());
        }
        Ok(())
    }
    // The older app flags it as it takes the PR: the same row to read here.
    fn ended(&self, id: &str, why: &str) -> Result<(), String> {
        self.note(format!("flag {id}: {why}"))
    }
    fn flag(&self, id: &str, what: &str) -> Result<(), String> {
        if self.flag_fails.get() > 0 {
            self.flag_fails.set(self.flag_fails.get() - 1);
            return Err("the app restarts".into());
        }
        self.note(format!("flag {id}: {what}"))
    }
}

impl Forge for Script {
    fn worktree(&self, id: &str) -> Result<String, String> {
        if self.worktree_fails {
            return Err("no space".into());
        }
        self.note(format!("worktree {id}"))?;
        Ok(format!("/work/rust-{id}"))
    }
    fn pr(&self, _id: &str) -> Result<Option<Pr>, String> {
        Ok(self.pr.borrow().clone())
    }
    fn checks(&self, _pr: &Pr) -> Result<Checks, String> {
        self.note("checks")?;
        take(&self.checks).expect("a check")
    }
    fn stale(&self, _pr: &Pr) -> Result<Vec<String>, String> {
        Ok(self.stale.clone())
    }
    fn merge(&self, pr: &Pr) -> Result<(), String> {
        self.note(format!("merge {} at {}", pr.number, pr.head))?;
        if self.merge_hiccups.get() > 0 {
            self.merge_hiccups.set(self.merge_hiccups.get() - 1);
            return Err("gh: no answer in 30s".into());
        }
        self.merge_refusal.borrow().clone().map_or(Ok(()), Err)
    }
    fn leave(&self, id: &str) -> Result<(), String> {
        self.note(format!("leave {id}"))
    }
}

/// The policy's stand-in: the first task on the first account, three tries, and each verdict's plain step.
impl Policy for Script {
    fn pick(&self, snapshot: &Snapshot) -> Pick {
        let Some(task) = snapshot.queue.tasks.first() else {
            return Pick::Wait("nothing".into());
        };
        if snapshot.sessions >= snapshot.queue.seats {
            return Pick::Wait("no seat".into());
        }
        let id = task.id.clone();
        let account = snapshot.limits.first().map(|l| l.account.clone());
        Pick::Start { id, account }
    }
    fn after(&self, verdict: &Verdict, nudges: u32, past_limit: bool) -> Step {
        match verdict {
            Verdict::Met => Step::Land,
            Verdict::NotMet(_) if past_limit => Step::Keep,
            Verdict::NotMet(why) if nudges >= 3 => Step::GiveUp(why.clone()),
            Verdict::NotMet(why) => Step::Nudge(format!("not yet: {why}")),
            Verdict::Blocked(_) => Step::Keep,
            Verdict::NotOurs(why) => Step::GiveUp(why.clone()),
        }
    }
    fn retry(&self, attempts: u32) -> bool {
        attempts < 3
    }
}

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
};

fn ports<'a>(tasks: &'a mut Store, s: &'a Script) -> Ports<'a> {
    Ports {
        tasks,
        list: s,
        terminal: s,
        launcher: s,
        agent: s,
        forge: s,
        policy: s,
    }
}

fn run_with(tasks: &mut Store, s: &Script, pace: Pace) -> Run {
    run_task("t1", None, &mut ports(tasks, s), pace).expect("recorded")
}

fn run(s: &Script, pace: Pace) -> Run {
    run_with(&mut Store::default(), s, pace)
}

fn merged() -> Outcome {
    Outcome::Merged("https://pr/7".into())
}

#[test]
fn a_task_goes_from_the_queue_to_a_merged_pr_through_a_stand_in_for_each_port() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    // Not listed yet, idle with no reply yet, working, idle with it, told to leave and still here, gone.
    let mut s = script(
        &["boot", "dialog", "up"],
        &[Gone, Idle, Working, Idle, Idle, Gone],
    );
    s.no_outcome.set(1);
    s.limits = vec![Limit {
        account: "spent-first".into(),
        five_hours: 0.0,
        week: 0.0,
        week_resets: 1,
    }];
    *s.checks.borrow_mut() = [
        Ok(Checks::Pending),
        Err("gh: slow".into()),
        Ok(Checks::Green),
    ]
    .into_iter()
    .collect();

    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    let Pick::Start { id, account } = pick else {
        panic!("{pick:?}");
    };
    assert_eq!(
        (id.as_str(), account.as_deref()),
        ("t1", Some("spent-first"))
    );
    let finished =
        run_task(&id, account.as_deref(), &mut ports(&mut tasks, &s), PACE).expect("recorded");

    let (pane, session) = (Some("p1".into()), Some("s1".into()));
    let outcome = merged();
    let expected = Run {
        pane,
        session,
        outcome,
    };
    assert_eq!(finished, expected);
    assert_eq!(tasks.run, Some(expected));
    assert_eq!(tasks.panes, ["p1"], "known before anything could crash");
    assert_eq!(tasks.rows["t1"], ("finished".to_string(), 1));
    // The idle look with no reply is the third Enter's.
    assert_eq!(
        s.did().join(", "),
        "worktree t1, open /work/rust-t1 rust t1, run start t1 on spent-first, key Down, key Enter, \
         send the brief of t1, key Enter, key Enter, judge, checks, checks, checks, merge 7 at abc, \
         merged t1 https://pr/7, send leave, key Enter, close p1, leave t1"
    );
    let mut none_left = Script {
        queue: vec![],
        ..script(&[], &[])
    };
    none_left.sessions = 0;
    let pick = next(&mut ports(&mut tasks, &none_left)).expect("a look");
    assert_eq!(pick, Pick::Wait("nothing".into()));
}

#[test]
fn next_picks_only_what_no_run_holds_and_within_the_seats() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.rows.insert("t1".into(), ("claimed".into(), 1));
    tasks.rows.insert("t2".into(), ("finished".into(), 1));
    tasks.rows.insert("t3".into(), ("open".into(), 1));
    let mut s = script(&[], &[]);
    s.queue = vec!["t1", "t2", "t3", "t4"];
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(
        matches!(pick, Pick::Start { id, .. } if id == "t3"),
        "open again after a failure"
    );
    s.sessions = 5;
    assert_eq!(
        next(&mut ports(&mut tasks, &s)).expect("a look"),
        Pick::Wait("no seat".into())
    );
}

#[test]
fn a_crashed_run_s_claim_is_freed_its_pane_closed_and_the_task_is_picked_again() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    let (id, pane, attempts) = ("t1".to_string(), Some("p9".to_string()), 1);
    tasks.crashed = vec![Freed { id, pane, attempts }];
    let s = script(&[], &[]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(pick, Pick::Start { id, .. } if id == "t1"));
    assert_eq!(s.did(), ["close p9"]);
    let s = script(&["up"], &[Idle, Gone]);
    let finished = run_with(&mut tasks, &s, PACE);
    assert_eq!(finished.outcome, merged());
    assert_eq!(
        tasks.rows["t1"],
        ("finished".to_string(), 2),
        "its second try"
    );
    assert!(
        s.did()
            .iter()
            .any(|d| d.starts_with("send Try 2") && d.contains("the brief of t1")),
        "told what the last run left"
    );
}

#[test]
fn a_crash_on_its_last_try_is_recorded_and_flagged_not_picked_again() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    let (id, pane, attempts) = ("t1".to_string(), None, 3);
    tasks.crashed = vec![Freed { id, pane, attempts }];
    let s = script(&[], &[]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(pick, Pick::Wait("nothing".into()));
    assert!(s.did()[0].starts_with("flag t1: its run crashed"));
    assert_eq!(tasks.rows["t1"].0, "finished");
}

#[test]
fn a_session_past_its_limit_is_judged_not_closed_and_goes_on_working() {
    let _budget = TestBudget::new();
    let limit = Duration::from_millis(30);
    let pace = Pace { limit, ..PACE };
    // Working past the limit each time, then idle with its reply.
    let looks: Vec<Look> = std::iter::repeat_n(Working, 3000)
        .chain([Idle, Gone])
        .collect();
    let s = script(&["up"], &looks);
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("no report".into())), Ok(Verdict::Met)]
        .into_iter()
        .collect();
    let mut tasks = Store::default();
    // Slow enough that the 3000 looks outlast the limit at least once.
    let poll = Duration::from_micros(100);
    let finished = run_with(&mut tasks, &s, Pace { poll, ..pace });

    assert_eq!(finished.outcome, merged());
    let did = s.did();
    let at = |what: &str| did.iter().position(|d| d == what).expect(what);
    assert!(at("judge") < at("close p1"));
    assert!(
        s.count("judge") >= 2,
        "past the limit, then at its stop: {did:?}"
    );
    assert!(
        !did.iter().any(|d| d.starts_with("send not yet")),
        "working: never nudged"
    );
    assert_eq!(s.count("close p1"), 1, "closed once, after it landed");
}

#[test]
fn a_session_past_its_limit_that_is_met_lands_while_it_works() {
    let _budget = TestBudget::new();
    let limit = Duration::ZERO;
    let s = script(&["up"], &[Working, Working, Gone]);
    let finished = run(&s, Pace { limit, ..PACE });
    assert_eq!(finished.outcome, merged());
}

#[test]
fn a_not_met_stop_is_nudged_and_its_next_turn_judged_never_the_old_one_again() {
    let _budget = TestBudget::new();
    // Idle at its report; idle still, the words not yet landed; working; idle with its second report.
    let s = script(&["up"], &[Idle, Idle, Working, Idle, Gone]);
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("part 2".into())), Ok(Verdict::Met)]
        .into_iter()
        .collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count("judge"), 2, "{:?}", s.did());
    assert_eq!(s.count("send not yet: part 2"), 1);
}

#[test]
fn nudges_run_out_and_the_run_ends_flagged_with_its_worktree_kept() {
    let _budget = TestBudget::new();
    let turns = [Idle, Idle, Working].repeat(4);
    let s = script(&["up"], &[turns.as_slice(), &[Idle, Gone]].concat());
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("part 2".into()))].into_iter().collect();
    let finished = run(&s, PACE);
    assert_eq!(finished.outcome, Outcome::Ended("part 2".into()));
    assert_eq!(s.count("send not yet: part 2"), 3);
    assert_eq!(s.count("flag t1: part 2"), 1);
    assert_eq!(s.count("leave t1"), 0);
}

#[test]
fn a_judge_with_no_answer_is_asked_again_flagged_past_its_tries_and_no_run_ends_on_it() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    let none = std::iter::repeat_n(Err("the app restarts".to_string()), 12);
    *s.verdicts.borrow_mut() = none.chain([Ok(Verdict::Met)]).collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count("judge"), 13);
    let flags = s
        .did()
        .into_iter()
        .filter(|d| d.starts_with("flag"))
        .count();
    assert_eq!(flags, 1);
}

#[test]
fn what_the_session_said_goes_to_the_judge_past_its_limit_nothing_does() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(*s.said.borrow(), DONE);
    let limit = Duration::ZERO;
    let s = script(&["up"], &[Working, Gone]);
    assert_eq!(run(&s, Pace { limit, ..PACE }).outcome, merged());
    assert_eq!(*s.said.borrow(), "");
}

#[test]
fn words_never_go_to_a_pane_the_agent_left() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("part 2".into()))].into_iter().collect();
    let finished = run(&s, PACE);
    assert!(
        matches!(finished.outcome, Outcome::Failed(e) if e.contains("left before it could read"))
    );
    assert!(!s.did().iter().any(|d| d.starts_with("send not yet")));
}

#[test]
fn a_merge_the_list_did_not_hear_of_is_told_at_the_next_look_once() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    s.merged_fails.set(true);
    let mut tasks = Store::default();
    let told = run_task("t1", None, &mut ports(&mut tasks, &s), PACE);
    assert!(told.is_err_and(|e| e.contains("not told yet")));
    assert_eq!(s.count("leave t1"), 1, "its worktree leaves all the same");

    tasks.untold = Some(merged());
    s.merged_fails.set(false);
    let before = s.count("merged t1 https://pr/7");
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(pick, Pick::Wait("nothing".into()));
    assert_eq!(s.count("merged t1 https://pr/7"), before + 1);
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(s.count("merged t1 https://pr/7"), before + 1, "told once");
}

#[test]
fn a_run_ended_and_not_told_is_told_at_the_next_look_once() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.rows.insert("t1".into(), ("finished".into(), 1));
    tasks.untold = Some(Outcome::Ended("no CI verdict".into()));
    let s = script(&[], &[]);
    next(&mut ports(&mut tasks, &s)).expect("a look");
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(s.count("flag t1: no CI verdict"), 1);
}

#[test]
fn a_run_claimed_and_not_listed_yet_holds_its_seat() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.rows.insert("t0".into(), ("claimed".into(), 1));
    let mut s = script(&[], &[Gone]);
    s.sessions = 4;
    let look = |tasks: &mut Store, s: &Script| next(&mut ports(tasks, s)).expect("a look");
    assert_eq!(
        look(&mut tasks, &s),
        Pick::Wait("no seat".into()),
        "no pane yet"
    );
    tasks.panes = vec!["p0".into()];
    assert_eq!(
        look(&mut tasks, &s),
        Pick::Wait("no seat".into()),
        "not listed"
    );
    // Listed, the terminal's count holds it: not counted twice.
    let mut s = script(&[], &[Working]);
    s.sessions = 4;
    assert!(matches!(look(&mut tasks, &s), Pick::Start { .. }));
}

#[test]
fn a_task_handed_back_ends_its_run_at_the_judge_s_refusal_never_asked_again() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    let back = Verdict::NotOurs("its owner is not rust".into());
    *s.verdicts.borrow_mut() = [Ok(back)].into_iter().collect();
    assert_eq!(
        run(&s, PACE).outcome,
        Outcome::Ended("its owner is not rust".into())
    );
    assert_eq!(s.count("judge"), 1);
    assert_eq!(s.count("leave t1"), 0);
    assert_eq!(s.count("merge 7 at abc"), 0);
}

#[test]
fn a_merge_with_no_answer_is_looked_at_again_costing_no_nudge() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    s.merge_hiccups.set(2);
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count("merge 7 at abc"), 3);
    assert_eq!(
        s.did()
            .iter()
            .filter(|d| d.starts_with("send not yet"))
            .count(),
        0
    );
}

#[test]
fn a_base_that_moved_goes_to_the_session_with_no_merge_tried() {
    let _budget = TestBudget::new();
    let mut s = script(&["up"], &[Idle, Gone]);
    s.stale = vec!["a.rs".into()];
    let pace = Pace {
        stop_polls: 2,
        ..PACE
    };
    let _ = run(&s, pace);
    assert_eq!(s.count("merge 7 at abc"), 0);
    assert!(
        s.did()
            .iter()
            .all(|d| !d.starts_with("send not yet") || d.contains("the base branch changed a.rs"))
    );
}

#[test]
fn a_flag_that_did_not_arrive_is_sent_again() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        ..PACE
    };
    let s = script(&["up"], &[Asking, Asking, Asking, Working, Idle, Gone]);
    s.flag_fails.set(1);
    assert_eq!(run(&s, pace).outcome, merged());
    let flags = s.did().into_iter().filter(|d| d.starts_with("flag"));
    assert_eq!(flags.count(), 1);
}

#[test]
fn a_session_waiting_on_someone_is_looked_at_slowly() {
    let _budget = TestBudget::new();
    let patient_poll = Duration::from_millis(40);
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        patient_poll,
        ..PACE
    };
    let s = script(&["up"], &[Asking, Asking, Asking, Working, Idle, Gone]);
    let started = std::time::Instant::now();
    assert_eq!(run(&s, pace).outcome, merged());
    assert!(started.elapsed() >= patient_poll, "{:?}", started.elapsed());
}

#[test]
fn a_blocked_session_stays_until_it_moves_flagged_by_the_judge_never_closed() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        ..PACE
    };
    let s = script(&["up"], &[Idle, Idle, Idle, Working, Idle, Gone]);
    *s.outcome.borrow_mut() = Outcome::Blocked("TASK_BLOCKED".into());
    *s.verdicts.borrow_mut() = [Ok(Verdict::Blocked("a tap".into())), Ok(Verdict::Met)]
        .into_iter()
        .collect();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.count("judge"), 2);
}

#[test]
fn a_question_is_flagged_once_never_answered_and_the_session_stays() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        ..PACE
    };
    let s = script(&["up"], &[Asking, Asking, Asking, Working, Idle, Gone]);
    assert_eq!(run(&s, pace).outcome, merged());
    let flags: Vec<String> = s
        .did()
        .into_iter()
        .filter(|d| d.starts_with("flag"))
        .collect();
    assert_eq!(flags.len(), 1, "{flags:?}");
    assert!(flags[0].contains("the pane shows: up"));
    let did = s.did();
    let before = did.iter().take_while(|d| !d.contains("leave"));
    assert_eq!(
        before.filter(|d| *d == "key Enter").count(),
        1,
        "the prompt's alone"
    );
}

#[test]
fn red_ci_goes_to_the_session_and_its_next_report_lands() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Working, Idle, Gone]);
    *s.checks.borrow_mut() = [Ok(Checks::Red("test".into())), Ok(Checks::Green)]
        .into_iter()
        .collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert!(
        s.did()
            .iter()
            .any(|d| d.starts_with("send not yet: CI is red on https://pr/7: test"))
    );
}

#[test]
fn no_pr_a_merged_one_and_a_refused_merge_each_read_as_they_are() {
    let _budget = TestBudget::new();
    let none = script(&["up"], &[Idle, Idle, Working, Idle, Gone]);
    *none.pr.borrow_mut() = None;
    // Asked for one, this session leaves: the words reached it, which is what the run owes.
    let finished = run(
        &none,
        Pace {
            stop_polls: 2,
            ..PACE
        },
    );
    assert!(
        none.did()
            .iter()
            .any(|d| d.starts_with("send not yet: no pull request holds the head of task/t1"))
    );
    assert!(
        matches!(finished.outcome, Outcome::Failed(e) if e.contains("left before it could read"))
    );

    let already = script(&["up"], &[Idle, Gone]);
    *already.pr.borrow_mut() = Some(pr(true));
    assert_eq!(run(&already, PACE).outcome, merged());
    assert_eq!(already.count("checks"), 0, "nothing to wait on");

    let refused = script(&["up"], &[Idle, Idle, Working, Idle, Gone]);
    *refused.merge_refusal.borrow_mut() = Some("not mergeable".into());
    let finished = run(
        &refused,
        Pace {
            stop_polls: 2,
            ..PACE
        },
    );
    assert!(
        refused
            .did()
            .iter()
            .any(|d| d
                .starts_with("send not yet: the merge of https://pr/7 was refused: not mergeable")),
        "{finished:?}"
    );
}

#[test]
fn no_ci_verdict_within_its_limit_ends_the_run_flagged_saying_why_a_failed_read_no_verdict() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    *s.checks.borrow_mut() = [Err("gh: down".to_string())].into_iter().collect();
    let ci_limit = Duration::ZERO;
    let finished = run(&s, Pace { ci_limit, ..PACE });
    assert!(
        matches!(&finished.outcome, Outcome::Ended(e) if e.contains("no CI verdict") && e.contains("gh: down"))
    );
    assert_eq!(
        s.did()
            .iter()
            .filter(|d| d.starts_with("flag t1: no CI"))
            .count(),
        1
    );
    assert_eq!(s.count("leave t1"), 0);
}

#[test]
fn a_fix_that_never_lands_runs_out_of_nudges() {
    let _budget = TestBudget::new();
    let turns = [Idle, Idle, Working].repeat(4);
    let s = script(&["up"], &[turns.as_slice(), &[Idle, Gone]].concat());
    *s.merge_refusal.borrow_mut() = Some("conflict".into());
    let finished = run(&s, PACE);
    assert!(
        matches!(&finished.outcome, Outcome::Ended(e) if e.contains("conflict")),
        "{finished:?}"
    );
    assert_eq!(
        s.did()
            .iter()
            .filter(|d| d.starts_with("send not yet: the merge"))
            .count(),
        3
    );
}

#[test]
fn a_second_question_is_flagged_too_after_the_first_was_answered() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        ..PACE
    };
    let s = script(
        &["up"],
        &[Asking, Asking, Working, Asking, Asking, Working, Idle, Gone],
    );
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.did().iter().filter(|d| d.starts_with("flag")).count(), 2);
}

#[test]
fn past_its_limit_still_working_a_question_after_is_flagged() {
    let _budget = TestBudget::new();
    let (limit, stuck_limit) = (Duration::ZERO, Duration::ZERO);
    let pace = Pace {
        limit,
        stuck_limit,
        ..PACE
    };
    let s = script(&["up"], &[Working, Asking, Asking, Working, Idle, Gone]);
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("no report".into())), Ok(Verdict::Met)]
        .into_iter()
        .collect();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(
        s.did().iter().filter(|d| d.starts_with("flag")).count(),
        1,
        "{:?}",
        s.did()
    );
}

#[test]
fn a_failed_start_is_released_for_a_retry_and_flagged_only_when_out_of_tries() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    let s = Script {
        open_fails: true,
        ..script(&[], &[])
    };
    for attempt in 1..=3u32 {
        let finished = run_with(&mut tasks, &s, PACE);
        assert_eq!(
            finished.outcome,
            Outcome::Failed("open: no terminal".into())
        );
        assert_eq!(tasks.rows["t1"].1, attempt);
    }
    assert_eq!(tasks.rows["t1"].0, "finished");
    assert_eq!(s.count("flag t1: open: no terminal"), 1);
    let again = run_task("t1", None, &mut ports(&mut tasks, &s), PACE);
    assert!(again.is_err_and(|e| e.contains("not an open task")));

    let no_worktree = Script {
        worktree_fails: true,
        ..script(&[], &[])
    };
    let finished = run(&no_worktree, PACE);
    assert_eq!(
        finished.outcome,
        Outcome::Failed("worktree: no space".into())
    );
    assert_eq!(finished.pane, None);
}

#[test]
fn each_limit_fails_the_run_saying_why_and_the_pane_still_closes() {
    let _budget = TestBudget::new();
    let none = Duration::ZERO;
    let pace = Pace {
        start_limit: none,
        limit: none,
        stuck_limit: none,
        stop_polls: 1,
        ..PACE
    };
    for (screen, look, says) in [
        (
            "make: no such file",
            Gone,
            "the pane shows: make: no such file",
        ),
        ("up", Gone, "no outcome"),
    ] {
        let s = script(&[screen], &[look]);
        let finished = run(&s, pace);
        assert!(
            matches!(finished.outcome, Outcome::Failed(e) if e.contains(says)),
            "{says}"
        );
        assert_eq!(s.did().last().map(String::as_str), Some("close p1"));
    }
}

#[test]
fn an_agent_listed_then_gone_is_read_and_told_nothing() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Working, Gone]);
    assert_eq!(run(&s, PACE).outcome, merged());
    assert!(
        !s.did()
            .iter()
            .any(|d| d.contains("leave") && d.starts_with("send"))
    );
}

#[test]
fn a_failed_look_or_a_gap_in_the_listing_is_no_verdict_but_a_lasting_one_is() {
    let _budget = TestBudget::new();
    let failing = script(&["up"], &[Idle, Gone]);
    failing.looks_fail.set(2);
    assert_eq!(run(&failing, PACE).outcome, merged());

    let gap = script(&["up"], &[Working, Gone, Idle, Gone]);
    gap.no_outcome.set(2);
    assert_eq!(run(&gap, PACE).outcome, merged());

    let gone = script(&["up"], &[Working, Gone]);
    gone.no_outcome.set(99);
    let left = Outcome::Failed("the agent left with no outcome".into());
    assert_eq!(run(&gone, PACE).outcome, left);
}

#[test]
fn enter_is_pressed_a_bounded_number_of_times_on_an_idle_agent_with_no_reply() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::from_millis(100);
    let s = script(&["up"], &[Idle]);
    s.no_outcome.set(u32::MAX);
    let finished = run(
        &s,
        Pace {
            stuck_limit,
            ..PACE
        },
    );
    assert!(matches!(finished.outcome, Outcome::Failed(e) if e.contains("the pane shows: up")));
    let did = s.did();
    let before_leave = did.iter().take_while(|d| !d.contains("leave"));
    let enters = before_leave.filter(|d| *d == "key Enter").count();
    assert_eq!(enters, 31, "the prompt's, then the tries");
}

#[test]
fn a_failed_read_of_the_outcome_is_no_verdict_and_a_lasting_one_says_why() {
    let _budget = TestBudget::new();
    let healed = script(&["up"], &[Idle, Gone]);
    healed.reads_fail.set(2);
    assert_eq!(run(&healed, PACE).outcome, merged());

    let lasting = script(&["up"], &[Working, Gone]);
    lasting.reads_fail.set(u32::MAX);
    let finished = run(&lasting, PACE);
    assert!(matches!(finished.outcome, Outcome::Failed(e) if e.contains("EIO")));
}

#[test]
fn the_outcome_is_recorded_before_the_pane_is_told_to_leave() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Gone]);
    let mut tasks = Store {
        note: Some(s.did.clone()),
        ..Store::default()
    };
    run_with(&mut tasks, &s, PACE);
    let did = s.did();
    let at = |what: &str| did.iter().position(|d| d == what).expect(what);
    assert!(at("record") < at("send leave"), "{did:?}");
}
