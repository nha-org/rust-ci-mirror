//! The whole path through a stand-in for each port, Forge and Policy included: section 20b's phase 2 with no tool in it.
use dash_core::tasks::{
    Agent, Checks, Claim, Forge, Launcher, Limit, Outcome, Pick, Policy, Pr, Queue, Queued, Run,
    Screen, Seen, Snapshot, Standing, Step, Task, TaskList, TaskStore, Terminal, Verdict,
};
use dash_meter::memory::TestBudget;
use dash_orch::{Pace, Ports, next, run_task};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

#[derive(Clone, Copy)]
enum Look {
    Gone,
    Idle,
    Working,
    Asking,
    /// The pane id reused: another session idle in it.
    Taken,
    /// An agent listed with no session id.
    NoId,
}
use Look::*;

const DONE: &str = "it is done";

/// The store's stand-in: a task claimable once a run lets it go, with a lock as the real one's: taken by a claim, kept until its
/// run lets go or its process ends (`process_ended`), and a crash is a claimed row with its lock free.
#[derive(Default)]
struct Store {
    rows: HashMap<String, (String, u32)>,
    waits: HashMap<String, Vec<String>>,
    reopen_capped: bool,
    run: Option<Run>,
    panes: Vec<String>,
    /// Each claim's pane and session, as saved.
    saved: HashMap<String, (Option<String>, Option<String>)>,
    locks: HashSet<String>,
    untold: Option<Outcome>,
    note: Option<Notes>,
}

impl Store {
    fn noted(&self, what: &str) {
        if let Some(note) = &self.note {
            note.borrow_mut().push(what.into());
        }
    }
    /// A run that lives, its pane and session saved.
    fn running(&mut self, id: &str, pane: Option<&str>, session: Option<&str>) {
        self.left_by_a_crash(id, pane, session, 1);
        self.locks.insert(id.into());
    }
    /// A claim its process left: the system freed its lock, and nothing else ran.
    fn left_by_a_crash(
        &mut self,
        id: &str,
        pane: Option<&str>,
        session: Option<&str>,
        attempts: u32,
    ) {
        self.rows.insert(id.into(), ("claimed".into(), attempts));
        let saved = (pane.map(String::from), session.map(String::from));
        self.saved.insert(id.into(), saved);
    }
    fn process_ended(&mut self) {
        self.locks.clear();
    }
    fn claim_of(&self, id: &str) -> Claim {
        let (pane, session) = self.saved.get(id).cloned().unwrap_or_default();
        let attempts = self.rows.get(id).map_or(0, |r| r.1);
        Claim {
            id: id.into(),
            pane,
            session,
            attempts,
        }
    }
}

type Notes = std::rc::Rc<RefCell<Vec<String>>>;

impl TaskStore for Store {
    fn claim(&mut self, id: &str) -> Result<u32, String> {
        if self.locks.contains(id) {
            return Err(format!("{id}: a run holds it"));
        }
        let row = self.rows.entry(id.into()).or_insert(("open".into(), 0));
        if row.0 != "open" {
            return Err(format!("{id}: not an open task"));
        }
        *row = ("claimed".into(), row.1 + 1);
        self.locks.insert(id.into());
        self.saved.remove(id);
        Ok(self.rows[id].1)
    }
    fn opened(&mut self, id: &str, pane: &str) -> Result<(), String> {
        self.panes.push(pane.into());
        self.saved.entry(id.into()).or_default().0 = Some(pane.into());
        Ok(())
    }
    fn adopted(&mut self, id: &str, session: &str) -> Result<(), String> {
        self.saved.entry(id.into()).or_default().1 = Some(session.into());
        Ok(())
    }
    fn waiting(&mut self, _id: &str, on: Option<&Outcome>) -> Result<(), String> {
        let kind = on.map_or("none", |o| o.parts().0);
        self.noted(&format!("waiting {kind}"));
        Ok(())
    }
    fn record(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.rows.entry(id.into()).or_default().0 = "finished".into();
        self.run = Some(run.clone());
        self.locks.remove(id);
        self.noted("record");
        Ok(())
    }
    fn release(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.rows.entry(id.into()).or_default().0 = "open".into();
        self.run = Some(run.clone());
        self.locks.remove(id);
        self.noted("release");
        Ok(())
    }
    fn crashed(&self) -> Result<Vec<Claim>, String> {
        let gone = |(id, row): (&String, &(String, u32))| {
            (row.0 == "claimed" && !self.locks.contains(id)).then(|| self.claim_of(id))
        };
        Ok(self.rows.iter().filter_map(gone).collect())
    }
    fn free(&mut self, id: &str) -> Result<bool, String> {
        let row = self.rows.entry(id.into()).or_default();
        let free = row.0 == "claimed" && !self.locks.contains(id);
        if free {
            row.0 = "open".into();
        }
        Ok(free)
    }
    fn reopen_held(&mut self, id: &str, at_ms: i64) -> Result<bool, String> {
        if self.reopen_capped {
            return Ok(true);
        }
        if at_ms > 0 && self.rows.get(id).is_some_and(|r| r.0 == "finished") {
            self.rows.insert(id.into(), ("open".into(), 0));
        }
        Ok(false)
    }
    fn status(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self.rows.get(id).map(|r| r.0.clone()))
    }
    fn untold(&self, _id: &str) -> Result<Option<Outcome>, String> {
        Ok(self.untold.clone())
    }
    fn claims(&self) -> Result<Vec<Claim>, String> {
        let live = self.rows.iter().filter(|(_, r)| r.0 == "claimed");
        Ok(live.map(|(id, _)| self.claim_of(id)).collect())
    }
    fn told(&mut self, _id: &str) -> Result<(), String> {
        self.untold = None;
        Ok(())
    }
    fn standing(&self) -> Result<Vec<Standing>, String> {
        let row = |(id, row): (&String, &(String, u32))| Standing {
            id: id.clone(),
            status: row.0.clone(),
            waits: self.waits.get(id).cloned().unwrap_or_default(),
            ..Standing::default()
        };
        Ok(self.rows.iter().map(row).collect())
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
    outcomes: RefCell<VecDeque<Outcome>>,
    verdicts: RefCell<VecDeque<Result<Verdict, String>>>,
    checks: RefCell<VecDeque<Result<Checks, String>>>,
    pr: RefCell<Option<Pr>>,
    earlier: Cell<bool>,
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
    /// How many closes fail before one works.
    close_fails: Cell<u32>,
    limits: RefCell<VecDeque<Vec<Limit>>>,
    limit_reads: Cell<u32>,
    limits_fail: Cell<u32>,
    /// Probes that fail before one connects.
    down: Cell<u32>,
    queue: Vec<&'static str>,
    reopened_ms: Option<i64>,
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

/// A session that reaches its prompt has one look taken by its first words, as `tell` looks before it types: an idle agent.
fn script(screens: &[&'static str], looks: &[Look]) -> Script {
    let told = (!screens.is_empty()).then_some(Idle);
    let looks = told.into_iter().chain(looks.iter().copied());
    Script {
        did: Notes::default(),
        screens: RefCell::new(screens.iter().copied().collect()),
        looks: RefCell::new(looks.collect()),
        no_outcome: Cell::new(0),
        outcomes: RefCell::new([Outcome::Done(DONE.into())].into_iter().collect()),
        verdicts: RefCell::new([Ok(Verdict::Met)].into_iter().collect()),
        checks: RefCell::new([Ok(Checks::Green)].into_iter().collect()),
        pr: RefCell::new(Some(pr(false))),
        earlier: Cell::new(false),
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
        close_fails: Cell::new(0),
        limits: RefCell::new([vec![]].into_iter().collect()),
        limit_reads: Cell::new(0),
        limits_fail: Cell::new(0),
        down: Cell::new(0),
        queue: vec!["t1"],
        reopened_ms: None,
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
            Taken => Some(Seen {
                idle: true,
                asking: false,
                session: Some("his-orchestrator".to_string()),
            }),
            NoId => Some(Seen {
                idle: true,
                asking: false,
                session: None,
            }),
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
        if self.close_fails.get() > 0 {
            self.close_fails.set(self.close_fails.get() - 1);
            return Err("herdr: no answer".into());
        }
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
        Ok(take(&self.outcomes))
    }
    fn stop_words(&self) -> String {
        "leave".into()
    }
    fn limits(&self) -> Result<Vec<Limit>, String> {
        self.limit_reads.set(self.limit_reads.get() + 1);
        if self.limits_fail.get() > 0 {
            self.limits_fail.set(self.limits_fail.get() - 1);
            return Err("no such folder".into());
        }
        Ok(take(&self.limits).unwrap_or_default())
    }
    fn reachable(&self) -> bool {
        let down = self.down.get();
        self.down.set(down.saturating_sub(1));
        down == 0
    }
}

impl TaskList for Script {
    fn waiting(&self, id: &str, on: Option<&Outcome>) -> Result<(), String> {
        let kind = on.map_or("over", |o| o.parts().0);
        self.note(format!("page {id} {kind}"))
    }
    fn queue(&self) -> Result<Queue, String> {
        let task = |id: &&str| Queued {
            id: id.to_string(),
            title: String::new(),
            reopened_ms: self.reopened_ms,
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
    /// An open PR comes from the session's work, so none before its first words, unless an earlier run left it.
    fn pr(&self, _id: &str) -> Result<Option<Pr>, String> {
        let told = self.earlier.get() || self.did().iter().any(|d| d.starts_with("send "));
        Ok(self.pr.borrow().clone().filter(|pr| pr.merged || told))
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
    limit_retry: Duration::ZERO,
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
        step: &|_, _| {},
    }
}

fn run_with(tasks: &mut Store, s: &Script, pace: Pace) -> Run {
    run_task("t1", None, &mut ports(tasks, s), pace).expect("recorded")
}

fn run(s: &Script, pace: Pace) -> Run {
    run_with(&mut Store::default(), s, pace)
}

fn limit(account: &str, week: f64) -> Limit {
    Limit {
        account: account.into(),
        five_hours: 0.0,
        five_hours_resets: 0,
        week,
        week_resets: 1,
        key: false,
    }
}

fn merged() -> Outcome {
    Outcome::Merged("https://pr/7".into())
}

#[test]
fn a_task_goes_from_the_queue_to_a_merged_pr_through_a_stand_in_for_each_port() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    // Not listed yet, idle with no reply yet, working, idle with it, still here when told to leave and after, gone.
    let s = script(
        &["boot", "dialog", "up"],
        &[Gone, Idle, Working, Idle, Idle, Idle, Gone],
    );
    // The look it is not listed in reads none, as the idle one with no reply yet.
    s.no_outcome.set(2);
    *s.limits.borrow_mut() = [vec![limit("spent-first", 0.0)]].into_iter().collect();
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
fn a_look_and_a_run_name_each_step_in_order() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    let s = script(&["up"], &[Idle, Gone]);
    let seen = RefCell::new(vec![]);
    let step = |name: &str, _: Duration| seen.borrow_mut().push(name.to_string());
    let mut with_steps = ports(&mut tasks, &s);
    with_steps.step = &step;
    next(&mut with_steps).expect("a look");
    run_task("t1", None, &mut with_steps, PACE).expect("recorded");
    assert_eq!(
        *seen.borrow(),
        [
            "free_crashed",
            "queue",
            "pickable",
            "sessions",
            "limits",
            "pick",
            "claim",
            "worktree",
            "brief",
            "open",
            "start",
            "supervise",
            "record",
            "close",
            "leave"
        ]
    );
}

#[test]
fn next_picks_only_what_no_run_holds_and_within_the_seats() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.running("t1", None, None);
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
fn next_skips_a_task_whose_wait_is_not_merged() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.rows.insert("t1".into(), ("open".into(), 0));
    tasks.rows.insert("t2".into(), ("open".into(), 0));
    tasks.waits.insert("t1".into(), vec!["t2".into()]);
    let mut s = script(&[], &[]);
    s.queue = vec!["t1", "t2"];
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(
        matches!(pick, Pick::Start { id, .. } if id == "t2"),
        "t1 waits"
    );
}

#[test]
fn a_new_task_the_cap_holds_is_flagged_and_is_the_answer() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    for n in 0..dash_core::tasks::OPEN_CAP {
        tasks.rows.insert(format!("open-{n}"), ("open".into(), 0));
    }
    let mut s = script(&[], &[]);
    s.queue = vec!["t1"];
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    let why = "50 Rust tasks are open, the cap: one must finish first";
    assert_eq!(pick, Pick::Wait(format!("t1: {why}")));
    assert_eq!(s.count(&format!("flag t1: {why}")), 1, "{:?}", s.did());
}

#[test]
fn a_reopen_the_cap_holds_is_flagged_and_its_old_outcome_never_closes_it() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.rows.insert("t1".into(), ("finished".into(), 1));
    tasks.untold = Some(merged());
    tasks.reopen_capped = true;
    let mut s = script(&[], &[]);
    s.queue = vec!["t1"];
    s.reopened_ms = Some(1);
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(s.count("merged t1 https://pr/7"), 0, "{:?}", s.did());
    assert!(
        tasks.untold.is_some(),
        "still its own to tell, once reopened or not"
    );
    assert!(
        s.did().iter().any(|d| d.contains("its reopen waits")),
        "{:?}",
        s.did()
    );
}

#[test]
fn a_crashed_run_s_claim_is_freed_its_pane_closed_and_the_task_is_picked_again() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", Some("p9"), Some("s1"), 1);
    let s = script(&[], &[Idle]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(pick, Pick::Start { id, .. } if id == "t1"));
    assert_eq!(
        s.did(),
        ["close p9", "page t1 over"],
        "a wait it crashed in ends"
    );
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
    tasks.left_by_a_crash("t1", None, None, 3);
    let s = script(&[], &[]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(pick, Pick::Wait("nothing".into()));
    assert!(s.did()[1].starts_with("flag t1: its run crashed"));
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
    tasks.running("t0", None, None);
    let mut s = script(&[], &[Gone]);
    s.sessions = 4;
    let look = |tasks: &mut Store, s: &Script| next(&mut ports(tasks, s)).expect("a look");
    assert_eq!(
        look(&mut tasks, &s),
        Pick::Wait("no seat".into()),
        "no pane yet"
    );
    tasks.running("t0", Some("p0"), Some("s1"));
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
    *s.outcomes.borrow_mut() = [Outcome::Blocked("TASK_BLOCKED".into())]
        .into_iter()
        .collect();
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

    // A retry's merged PR may be its own, landed just before a crash.
    let already = script(&["up"], &[Idle, Gone]);
    *already.pr.borrow_mut() = Some(pr(true));
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", None, None, 1);
    next(&mut ports(&mut tasks, &already)).expect("a look");
    assert_eq!(run_with(&mut tasks, &already, PACE).outcome, merged());
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
fn each_limit_fails_the_run_saying_why_and_a_pane_closes_only_with_its_agent_there() {
    let _budget = TestBudget::new();
    let none = Duration::ZERO;
    let pace = Pace {
        start_limit: none,
        limit: none,
        stuck_limit: none,
        stop_polls: 1,
        ..PACE
    };
    for (screen, look, says, closes) in [
        (
            "make: no such file",
            Gone,
            "the pane shows: make: no such file",
            1,
        ),
        ("up", Gone, "no outcome", 0),
    ] {
        let s = script(&[screen], &[look]);
        s.no_outcome.set(u32::MAX);
        let finished = run(&s, pace);
        assert!(
            matches!(finished.outcome, Outcome::Failed(e) if e.contains(says)),
            "{says}"
        );
        assert_eq!(s.count("close p1"), closes, "{says}");
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

fn typed(s: &Script) -> Vec<String> {
    let did = s.did().into_iter();
    did.filter(|d| d.starts_with("send") || d.starts_with("key"))
        .collect()
}

/// The audit's finding 10: an id reused while a run watched its pane. Nothing of the session now there is touched.
#[test]
fn a_pane_another_session_took_is_never_typed_into_or_closed() {
    let _budget = TestBudget::new();
    let stop_polls = 2;
    let pace = Pace { stop_polls, ..PACE };
    let s = script(&["up"], &[Working, Taken]);
    s.no_outcome.set(u32::MAX);
    let mut tasks = Store::default();
    let finished = run_with(&mut tasks, &s, pace);
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("left with no outcome")),
        "{finished:?}"
    );
    assert_eq!(finished.session.as_deref(), Some("s1"), "never the other");
    assert_eq!(s.count("close p1"), 0);
    assert_eq!(
        typed(&s),
        ["send the brief of t1", "key Enter"],
        "its brief alone"
    );
    // It may be this work under a new id (a /clear): no retry beside it, and whoever coordinates hears.
    assert_eq!(tasks.rows["t1"].0, "finished");
    assert!(
        s.did()
            .iter()
            .any(|d| d.starts_with("flag t1:") && d.contains("no retry starts beside it"))
    );
}

#[test]
fn a_nudge_never_goes_to_a_session_that_took_the_pane() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Taken]);
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("part 2".into()))].into_iter().collect();
    let finished = run(&s, PACE);
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("left before it could read"))
    );
    assert!(!s.did().iter().any(|d| d.starts_with("send not yet")));
    assert_eq!(s.count("close p1"), 0);
}

/// After a reboot the saved id may be his orchestrator's: only the saved session closes.
#[test]
fn after_a_crash_only_the_saved_session_s_pane_closes() {
    let _budget = TestBudget::new();
    for (saved, look, closes) in [
        (Some("s1"), Taken, 0),
        (Some("s1"), Gone, 0),
        (Some("s1"), Idle, 1),
        // Nothing saved: nothing here can name what that pane holds.
        (None, Idle, 0),
        (None, Gone, 0),
    ] {
        let mut tasks = Store::default();
        tasks.left_by_a_crash("t1", Some("p9"), saved, 1);
        let s = script(&[], &[look]);
        let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
        assert!(matches!(pick, Pick::Start { id, .. } if id == "t1"));
        assert_eq!(s.count("close p9"), closes, "{saved:?}");
        assert!(typed(&s).is_empty());
    }
}

/// Finding 4 of the review: a first try finds a merged PR only from before its run, so a met verdict there is no landing.
#[test]
fn a_first_try_never_lands_on_a_pr_merged_before_it() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Working, Idle, Gone]);
    *s.pr.borrow_mut() = Some(pr(true));
    let pace = Pace {
        stop_polls: 2,
        ..PACE
    };
    let finished = run(&s, pace);
    assert_ne!(finished.outcome, merged());
    assert!(
        s.did()
            .iter()
            .any(|d| d.starts_with("send not yet: https://pr/7 merged before this run")),
        "{:?}",
        s.did()
    );
}

/// Finding 2 of the review: a first brief has `start_limit` for the listing, more looks than a known session gets.
#[test]
fn a_first_brief_waits_its_start_limit_for_a_slow_listing() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Gone, Gone, Gone, Gone, Gone, Idle, Gone]);
    s.looks.borrow_mut().pop_front();
    let poll = Duration::from_millis(1);
    let finished = run(&s, Pace { poll, ..PACE });
    assert_eq!(finished.outcome, merged());
}

/// Finding 5 of the review: an agent about to show its id closes as this run's, and the retry stays.
#[test]
fn a_failed_run_waits_for_an_agent_about_to_show_its_id_and_keeps_its_retry() {
    let _budget = TestBudget::new();
    let start_limit = Duration::ZERO;
    let pace = Pace {
        start_limit,
        ..PACE
    };
    let s = script(&["boot"], &[NoId, NoId, Idle, Gone]);
    s.looks.borrow_mut().pop_front();
    let mut tasks = Store::default();
    let finished = run_with(&mut tasks, &s, pace);
    assert!(matches!(finished.outcome, Outcome::Failed(_)));
    assert_eq!(s.count("close p1"), 1);
    assert_eq!(tasks.rows["t1"].0, "open", "its retry");
}

/// Finding 6 of the review: another session in a crashed run's pane may be its own after a /clear, so its retry is flagged.
#[test]
fn a_retry_beside_a_session_the_crash_did_not_save_is_flagged() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", Some("p9"), Some("s1"), 1);
    let s = script(&[], &[Taken]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(pick, Pick::Start { .. }));
    assert_eq!(s.count("close p9"), 0);
    assert!(s.did().iter().any(|d| d.contains("a retry starts")));
}

/// Finding 9 of the review: a limit wait is no time at work, so the turn after it is not past its limit.
#[test]
fn a_long_limit_wait_does_not_read_as_past_its_time_limit() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Idle, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    *s.limits.borrow_mut() = [vec![limit("a", 100.0)], vec![limit("a", 10.0)]]
        .into_iter()
        .collect();
    let pace = Pace {
        limit: Duration::from_millis(200),
        patient_poll: Duration::from_millis(250),
        ..PACE
    };
    let mut tasks = Store::default();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), pace).expect("recorded");
    assert_eq!(finished.outcome, merged());
    assert_eq!(
        *s.said.borrow(),
        DONE,
        "its report, not a judge past its limit"
    );
}

/// Listed with no id yet: it may be its session, so nothing retries beside it, and nothing closes it until the id shows.
#[test]
fn after_a_crash_an_agent_with_no_id_yet_holds_the_claim() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", Some("p9"), Some("s1"), 1);
    let s = script(&[], &[NoId, Idle]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(pick, Pick::Wait("nothing".into()));
    assert_eq!(tasks.rows["t1"].0, "claimed");
    assert!(
        s.did().iter().any(|d| d.contains("the claim holds")),
        "no silent wait"
    );
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(pick, Pick::Start { .. }));
    assert_eq!(s.count("close p9"), 1);
}

/// Listed in that pane, yet not this claim's session: it holds its seat as one not listed, the side within his seats.
#[test]
fn a_claim_whose_pane_another_took_still_holds_its_seat() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.running("t0", Some("p0"), Some("s1"));
    let mut s = script(&[], &[Taken]);
    s.sessions = 4;
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(pick, Pick::Wait("no seat".into()));
}

/// The audit's finding 13: the row opens, and the lock goes, only once the stop has returned.
#[test]
fn a_retry_is_let_go_only_after_its_stop_returned() {
    let _budget = TestBudget::new();
    let start_limit = Duration::ZERO;
    let pace = Pace {
        start_limit,
        ..PACE
    };
    let s = script(&["boot"], &[Idle, Gone]);
    let mut tasks = Store {
        note: Some(s.did.clone()),
        ..Store::default()
    };
    let finished = run_with(&mut tasks, &s, pace);
    assert!(matches!(finished.outcome, Outcome::Failed(_)));
    let did = s.did();
    let at = |what: &str| did.iter().position(|d| d == what).expect(what);
    assert!(at("send leave") < at("release"), "{did:?}");
    assert!(at("close p1") < at("release"), "{did:?}");
    assert_eq!(tasks.rows["t1"], ("open".to_string(), 1));
    assert!(tasks.locks.is_empty());
    assert_eq!(
        tasks.run.expect("released").session.as_deref(),
        Some("s1"),
        "what its stop saw"
    );
}

/// A close that failed may have left its session at work: nothing retries beside it, until a close works.
#[test]
fn a_failed_close_holds_the_retry_until_a_later_look_closes_it() {
    let _budget = TestBudget::new();
    let start_limit = Duration::ZERO;
    let pace = Pace {
        start_limit,
        ..PACE
    };
    let s = script(&["boot"], &[Idle]);
    s.close_fails.set(2);
    let mut tasks = Store::default();
    let failed = run_task("t1", None, &mut ports(&mut tasks, &s), pace);
    assert!(failed.is_err_and(|e| e.contains("stayed open")));
    assert_eq!(tasks.rows["t1"].0, "claimed", "not released");
    assert_eq!(s.count("flag t1: no prompt"), 0);
    let held = run_task("t1", None, &mut ports(&mut tasks, &s), pace);
    assert!(held.is_err_and(|e| e.contains("a run holds it")));

    tasks.process_ended();
    let look = |tasks: &mut Store| next(&mut ports(tasks, &s)).expect("a look");
    assert_eq!(
        look(&mut tasks),
        Pick::Wait("nothing".into()),
        "close fails"
    );
    assert_eq!(tasks.rows["t1"].0, "claimed");
    assert!(
        s.did().iter().any(|d| d.contains("will not close")),
        "no silent wait"
    );
    s.looks_fail.set(1);
    assert_eq!(look(&mut tasks), Pick::Wait("nothing".into()), "no look");
    assert!(matches!(look(&mut tasks), Pick::Start { id, .. } if id == "t1"));
    assert_eq!(s.count("close p1"), 1);
}

#[test]
fn a_task_the_list_opened_again_is_picked_again_one_never_reopened_is_not() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.rows.insert("t1".into(), ("finished".into(), 1));
    let mut s = script(&[], &[]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(pick, Pick::Wait("nothing".into()));
    s.reopened_ms = Some(1);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(pick, Pick::Start { id, .. } if id == "t1"));
}

fn limited() -> Outcome {
    Outcome::Limited("You've hit your weekly limit".into())
}

/// The audit's finding 12: its limit is no word of the session's. No judge, no nudge, its tries untouched.
#[test]
fn a_limit_stop_waits_for_room_then_says_continue_with_no_judge_or_nudge() {
    let _budget = TestBudget::new();
    // Its stop, two looks during the wait, listed for the words, at work, its report, gone.
    let s = script(&["up"], &[Idle, Idle, Idle, Idle, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    // Its own account full twice; another's being full holds nothing.
    let full = vec![limit("a", 100.0), limit("b", 0.0)];
    let room = vec![limit("a", 10.0), limit("b", 100.0)];
    *s.limits.borrow_mut() = [full.clone(), full, room].into_iter().collect();
    let mut tasks = Store::default();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(finished.outcome, merged());
    assert_eq!(s.limit_reads.get(), 3, "until it had room");
    assert_eq!(s.count("send continue"), 1);
    assert_eq!(s.count("judge"), 1, "its report alone");
    assert_eq!(*s.said.borrow(), DONE);
    assert!(!s.did().iter().any(|d| d.starts_with("send not yet")));
    assert_eq!(tasks.rows["t1"], ("finished".to_string(), 1));
}

#[test]
fn a_run_on_no_named_account_waits_until_every_one_has_room() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Idle, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    let one_full = vec![limit("a", 100.0), limit("b", 0.0)];
    // A key's row spent to its cap is a headless run's, never this session's: it holds no wait.
    let key = Limit {
        key: true,
        ..limit("a-key", 100.0)
    };
    *s.limits.borrow_mut() = [one_full, vec![limit("a", 0.0), key]].into_iter().collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.limit_reads.get(), 2);
}

/// « continue » lost, or the limit back too fast for a look to see it work: told again, never read as a session stuck.
#[test]
fn a_limit_stop_still_the_last_word_is_told_again_never_failed() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        ..PACE
    };
    // Its stop; listed for the words; the same stop still; listed for them again; at work; its report; gone.
    let s = script(&["up"], &[Idle, Idle, Idle, Idle, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited(), limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.count("send continue"), 2, "{:?}", s.did());
    assert_eq!(s.count("judge"), 1);
}

fn offline() -> Outcome {
    Outcome::Offline(
        "API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)".into(),
    )
}

const CARRY_ON: &str = "send carry on: the network dropped and is back, resume where you stopped";

/// Case: rust-linear-view, session 543b81aa, 2026-11-07 17:54: its 3 nudges each hit this error and the run ended unfinished.
#[test]
fn a_network_stop_waits_for_the_network_then_says_carry_on_once_with_no_judge_or_nudge() {
    let _budget = TestBudget::new();
    // Its stop, two looks during the wait, one that it did not move, listed for the words, at work, its report, gone.
    let s = script(
        &["up"],
        &[Idle, Idle, Idle, Idle, Idle, Working, Idle, Gone],
    );
    *s.outcomes.borrow_mut() = [offline(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    s.down.set(3);
    let notes = Notes::default();
    let mut tasks = Store {
        note: Some(notes.clone()),
        ..Store::default()
    };
    let finished = run_with(&mut tasks, &s, PACE);
    assert_eq!(finished.outcome, merged());
    assert_eq!(s.down.get(), 0, "until the probe connected");
    assert_eq!(s.count(CARRY_ON), 1, "{:?}", s.did());
    assert_eq!(s.count("judge"), 1, "its report alone");
    assert_eq!(*s.said.borrow(), DONE);
    assert!(!s.did().iter().any(|d| d.starts_with("send not yet")));
    assert!(s.limit_reads.get() <= 1, "a network stop reads no limit");
    assert_eq!(tasks.rows["t1"], ("finished".to_string(), 1));
    let notes = notes.borrow();
    let waits: Vec<&str> = (notes.iter().map(String::as_str))
        .filter(|n| n.starts_with("waiting"))
        .collect();
    assert_eq!(waits, ["waiting offline", "waiting none"]);
    assert_eq!(s.count("page t1 offline"), 1, "{:?}", s.did());
    assert_eq!(s.count("page t1 over"), 1, "{:?}", s.did());
}

/// The nudges the judge's path counts stay whole however many drops a run lives through: it never ends for the network.
#[test]
fn many_drops_in_one_run_end_nothing_and_spend_no_judge() {
    let _budget = TestBudget::new();
    let turn = [Idle, Idle, Idle, Working];
    let s = script(
        &["up"],
        &[turn.as_slice(), &turn, &turn, &turn, &turn, &[Idle, Gone]].concat(),
    );
    let drops = std::iter::repeat_n(offline(), 5);
    *s.outcomes.borrow_mut() = drops.chain([Outcome::Done(DONE.into())]).collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count(CARRY_ON), 5, "{:?}", s.did());
    assert_eq!(s.count("judge"), 1);
}

/// An API error while the probe connects: told slowly and flagged once, as a limit its readings do not show.
#[test]
fn an_api_error_while_the_network_answers_is_flagged_once_and_told_slowly() {
    let _budget = TestBudget::new();
    let limit_retry = Duration::from_millis(600);
    let pace = Pace {
        limit_retry,
        ..PACE
    };
    let turn = [Idle, Idle, Idle, Working];
    let s = script(
        &["up"],
        &[turn.as_slice(), &turn, &turn, &[Idle, Gone]].concat(),
    );
    let stops = [offline(), offline(), offline(), Outcome::Done(DONE.into())];
    *s.outcomes.borrow_mut() = stops.into_iter().collect();
    let started = std::time::Instant::now();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.count(CARRY_ON), 3, "{:?}", s.did());
    assert!(started.elapsed() >= limit_retry * 2);
    let flags = s
        .did()
        .into_iter()
        .filter(|d| d.contains("network answers"));
    assert_eq!(flags.count(), 1);
}

/// « carry on » lost: the same stop again, never read as a session stuck.
#[test]
fn a_network_stop_still_the_last_word_is_told_again_never_failed() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        ..PACE
    };
    let s = script(
        &["up"],
        &[Idle, Idle, Idle, Idle, Idle, Working, Idle, Gone],
    );
    *s.outcomes.borrow_mut() = [offline(), offline(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.count(CARRY_ON), 2, "{:?}", s.did());
}

/// A 500 on every send, the probe up: past `SOON` it is judged as any stop, so its nudges bound a run that waiting cannot mend.
#[test]
fn an_api_error_that_keeps_coming_back_with_the_network_up_is_judged_in_the_end() {
    let _budget = TestBudget::new();
    let limit_retry = Duration::from_millis(300);
    let pace = Pace {
        limit_retry,
        ..PACE
    };
    let turn = [Idle, Idle, Idle, Working];
    let turns = [turn.as_slice(), &turn, &turn, &turn, &[Idle, Gone]].concat();
    let s = script(&["up"], &turns);
    *s.outcomes.borrow_mut() = std::iter::repeat_n(offline(), 5).collect();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.count(CARRY_ON), 4, "{:?}", s.did());
    assert_eq!(s.count("judge"), 1, "{:?}", s.did());
    assert!(s.said.borrow().contains("Can't reach the API server"));
}

/// 3 quick API errors, hours of work, then a real drop: the bound is over, so it waits and is told, with no judge.
#[test]
fn a_drop_long_after_quick_api_errors_waits_again() {
    let _budget = TestBudget::new();
    let pace = Pace {
        limit_retry: Duration::from_millis(600),
        poll: Duration::from_millis(10),
        ..PACE
    };
    let turn = [Idle, Idle, Idle, Working];
    let long = [Working; 30];
    let turns = [
        turn.as_slice(),
        &turn,
        &turn,
        &turn,
        &long,
        &turn,
        &[Idle, Gone],
    ]
    .concat();
    let s = script(&["up"], &turns);
    let stops = std::iter::repeat_n(offline(), 5);
    *s.outcomes.borrow_mut() = stops.chain([Outcome::Done(DONE.into())]).collect();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.count(CARRY_ON), 5, "{:?}", s.did());
    assert_eq!(s.count("judge"), 1, "{:?}", s.did());
    assert_eq!(*s.said.borrow(), DONE);
}

/// An API error back slower than the bound counts, the network up: no wait took place, so its time runs out and it is judged.
#[test]
fn an_api_error_with_the_network_up_keeps_its_clock_running() {
    let _budget = TestBudget::new();
    let pace = Pace {
        limit: Duration::from_millis(25),
        poll: Duration::from_millis(10),
        ..PACE
    };
    let turn = [Idle, Idle, Idle, Working];
    let turns = [turn.as_slice(), &turn, &turn, &turn, &turn, &[Idle, Gone]].concat();
    let s = script(&["up"], &turns);
    *s.outcomes.borrow_mut() = std::iter::repeat_n(offline(), 5).collect();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(*s.said.borrow(), "", "judged at its limit: {:?}", s.did());
}

/// A rest with the network up says nothing of the network on the page; an outage's wait is flagged once, and only a long one.
#[test]
fn the_page_says_network_only_when_it_is_down_and_a_long_wait_is_flagged() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Idle, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [offline(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count("page t1 offline"), 0, "{:?}", s.did());

    let looks = [[Idle; 14].as_slice(), &[Working, Idle, Gone]].concat();
    let s = script(&["up"], &looks);
    *s.outcomes.borrow_mut() = [offline(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    s.down.set(12);
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count("page t1 offline"), 1, "{:?}", s.did());
    let flags = s
        .did()
        .into_iter()
        .filter(|d| d.contains("probe of the API"));
    assert_eq!(flags.count(), 1);
}

/// A reopened task's first try: its earlier run's PR and worktree are named, or it would redo the work.
#[test]
fn a_first_try_with_an_open_pr_is_told_of_the_earlier_run() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Working, Idle, Gone]);
    s.earlier.set(true);
    assert_eq!(run(&s, PACE).outcome, merged());
    let told = |d: &String| d.starts_with("send This task was opened again: https://pr/7");
    assert!(s.did().iter().any(told), "{:?}", s.did());
}

/// As `Dashboard.Network`: typed into during the outage, it is at work when the network is back.
#[test]
fn a_session_at_work_when_the_network_is_back_gets_no_carry_on() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Working, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [offline(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    s.down.set(2);
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count(CARRY_ON), 0, "{:?}", s.did());
    assert_eq!(s.count("judge"), 1);
}

/// A limit's words and a network stop's say nothing of each other: an API error right after a continue is no error told again.
#[test]
fn a_limit_s_rest_and_flag_are_not_a_network_stop_s() {
    let _budget = TestBudget::new();
    let limit_retry = Duration::from_secs(60);
    let pace = Pace {
        limit_retry,
        ..PACE
    };
    let looks = [Idle, Idle, Working, Idle, Idle, Idle, Working, Idle, Gone];
    let s = script(&["up"], &looks);
    let stops = [limited(), offline(), Outcome::Done(DONE.into())];
    *s.outcomes.borrow_mut() = stops.into_iter().collect();
    let started = std::time::Instant::now();
    assert_eq!(run(&s, pace).outcome, merged());
    assert!(
        started.elapsed() < limit_retry,
        "no rest for the first drop"
    );
    assert!(
        !s.did().iter().any(|d| d.starts_with("flag")),
        "{:?}",
        s.did()
    );
}

#[test]
fn a_session_that_left_during_a_network_wait_ends_it() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    *s.outcomes.borrow_mut() = [offline()].into_iter().collect();
    s.down.set(u32::MAX);
    let finished = run(&s, PACE);
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("left before it could read"))
    );
    assert_eq!(s.count(CARRY_ON), 0);
}

#[test]
fn a_first_brief_never_goes_to_a_pane_no_agent_is_listed_in() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Gone]);
    // The look `script` gives its first words, spent on none: the shell of a launch that died.
    s.looks.borrow_mut().pop_front();
    let finished = run(&s, PACE);
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("left before it could read"))
    );
    assert!(typed(&s).is_empty(), "{:?}", s.did());
    assert_eq!(s.count("close p1"), 0, "no session listed, none closed");
}

/// Part 1 with nothing saved yet: an agent listed with no id names no session, so nothing types into it or closes it, and no
/// retry starts beside it.
#[test]
fn an_agent_with_no_id_is_never_typed_into_or_closed_and_no_retry_starts_beside_it() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[NoId]);
    s.looks.borrow_mut().pop_front();
    let mut tasks = Store::default();
    let finished = run_with(&mut tasks, &s, PACE);
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("left before it could read"))
    );
    assert!(typed(&s).is_empty(), "{:?}", s.did());
    assert_eq!(s.count("close p1"), 0);
    assert_eq!(finished.session, None);
    assert_eq!(tasks.rows["t1"].0, "finished", "no retry beside it");
    assert!(
        s.did()
            .iter()
            .any(|d| d.starts_with("flag t1:") && d.contains("no retry starts beside it"))
    );
}

/// A failed start that never listed a session: its pane is not provably this run's, so it stays; its stop looked, and the row
/// is free for a retry.
#[test]
fn a_failed_start_with_no_session_listed_closes_nothing() {
    let _budget = TestBudget::new();
    let start_limit = Duration::ZERO;
    let pace = Pace {
        start_limit,
        ..PACE
    };
    let s = script(&["boot"], &[Gone]);
    s.looks.borrow_mut().pop_front();
    let mut tasks = Store::default();
    let finished = run_with(&mut tasks, &s, pace);
    assert!(matches!(finished.outcome, Outcome::Failed(_)));
    assert_eq!(s.count("close p1"), 0);
    assert!(typed(&s).is_empty(), "{:?}", s.did());
    assert_eq!(tasks.rows["t1"].0, "open");
}

/// A limit the readings do not show: said slowly and flagged once, never every look until a reset days away.
#[test]
fn a_limit_its_readings_do_not_show_is_flagged_once_and_told_slowly() {
    let _budget = TestBudget::new();
    let limit_retry = Duration::from_millis(30);
    let pace = Pace {
        limit_retry,
        ..PACE
    };
    let turn = [Idle, Idle, Working];
    let s = script(
        &["up"],
        &[turn.as_slice(), &turn, &turn, &[Idle, Gone]].concat(),
    );
    let stops = [limited(), limited(), limited(), Outcome::Done(DONE.into())];
    *s.outcomes.borrow_mut() = stops.into_iter().collect();
    let started = std::time::Instant::now();
    assert_eq!(run(&s, pace).outcome, merged());
    assert_eq!(s.count("send continue"), 3, "{:?}", s.did());
    assert!(
        started.elapsed() >= limit_retry * 2,
        "{:?}",
        started.elapsed()
    );
    let flags = s
        .did()
        .into_iter()
        .filter(|d| d.contains("readings do not show"));
    assert_eq!(flags.count(), 1);
}

#[test]
fn a_session_that_left_during_a_limit_wait_ends_it_and_failed_readings_are_flagged_once() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited()].into_iter().collect();
    *s.limits.borrow_mut() = [vec![limit("a", 100.0)]].into_iter().collect();
    let finished = run(&s, PACE);
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("left before it could read"))
    );
    assert_eq!(s.count("send continue"), 0);

    let looks: Vec<Look> = std::iter::repeat_n(Idle, 14)
        .chain([Working, Idle, Gone])
        .collect();
    let s = script(&["up"], &looks);
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    s.limits_fail.set(12);
    assert_eq!(run(&s, PACE).outcome, merged());
    let flags = s
        .did()
        .into_iter()
        .filter(|d| d.contains("its readings fail"));
    assert_eq!(flags.count(), 1);
}
