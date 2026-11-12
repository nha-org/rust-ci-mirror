//! The whole path through a stand-in for each port, Forge and Policy included: section 20b's phase 2 with no tool in it.
use dash_core::tasks::{
    Agent, Base, Checks, Claim, End, Forge, Hold, Launcher, Limit, ORCHESTRATOR, Outcome, Pick,
    Policy, Pr, Queue, Queued, RfcScore, Run, Screen, Seen, Snapshot, Standing, Step, Task,
    TaskList, TaskStore, Terminal, Verdict,
};
use dash_meter::memory::TestBudget;
use dash_orch::{
    NOT_HELD, Pace, Ports, adopt_task, at_work, close, deploy_pending, hand_backs, landed, next,
    plan_now, redeploy, run_task, run_task_on, seats_taken, shells_running,
};
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
    /// The list as a sync left it: the script's, copied by `ports`.
    listed: Option<Queue>,
    /// The PR head each was handed back at.
    handed: HashMap<String, String>,
    /// Each live hand-back: handed back, and no run on it finished since.
    back: HashSet<String>,
    /// The merged head each ended run was judged again at, and its verdict.
    rejudged: HashMap<String, String>,
    /// Each word on what became of main after its merge, in order.
    deployed: Vec<(String, String)>,
    /// Each note on a task's page, in order: what wakes no one.
    notes: Vec<(String, String)>,
    /// Each login a run was written on, in order.
    logins: Vec<String>,
    /// Each PR's hold, live: the stand-in keeps no clock, the real store's test reads `HOLD_MS`.
    holds: HashMap<u64, Hold>,
    /// Each run's end, in order.
    ends: Vec<(String, End)>,
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
            account: None,
            model: None,
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
    fn hold(&mut self, id: &str) -> Result<Claim, String> {
        if self.locks.contains(id) {
            return Err(format!("{id}: a run holds it"));
        }
        let row = self.rows.get_mut(id).filter(|r| r.0 == "claimed");
        let row = row.ok_or(format!("{id}: not claimed"))?;
        row.1 += 1;
        self.locks.insert(id.into());
        Ok(self.claim_of(id))
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
    fn left(&mut self, id: &str) -> Result<(), String> {
        self.saved.entry(id.into()).or_default().0 = None;
        self.noted("left");
        Ok(())
    }
    fn login(&mut self, _id: &str, account: &str) -> Result<(), String> {
        self.logins.push(account.into());
        Ok(())
    }
    fn waiting(&mut self, _id: &str, on: Option<&Outcome>) -> Result<(), String> {
        let kind = on.map_or("none", |o| o.parts().0);
        self.noted(&format!("waiting {kind}"));
        Ok(())
    }
    fn record(&mut self, id: &str, run: &Run) -> Result<(), String> {
        self.rows.entry(id.into()).or_default().0 = "finished".into();
        self.back.remove(id);
        self.run = Some(run.clone());
        self.untold = Some(run.outcome.clone());
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
    /// Its run's process ended and its lock taken here: what `record` lets go.
    fn take_over(&mut self, id: &str) -> Result<(), String> {
        if self.rows.get(id).is_none_or(|r| r.0 != "claimed") {
            return Err(format!("{id}: not claimed"));
        }
        self.locks.insert(id.into());
        self.noted("take over");
        Ok(())
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
    fn handed(&mut self, id: &str, head: &str) -> Result<(), String> {
        self.handed.insert(id.into(), head.into());
        self.back.insert(id.into());
        Ok(())
    }
    fn rejudged(&mut self, id: &str, words: &str) -> Result<(), String> {
        self.rejudged.insert(id.into(), words.into());
        self.back.remove(id);
        Ok(())
    }
    fn deployed(&mut self, id: &str, words: &str) -> Result<(), String> {
        self.deployed.push((id.into(), words.into()));
        Ok(())
    }
    fn note(&mut self, id: &str, words: &str) -> Result<(), String> {
        self.noted(&format!("note {id}: {words}"));
        self.notes.push((id.into(), words.into()));
        Ok(())
    }
    fn hold_pr(&mut self, pr: u64, holder: &str, why: &str) -> Result<Option<Hold>, String> {
        let other = self.holds.get(&pr).filter(|h| h.holder != holder).cloned();
        if let Some(other) = &other {
            let run = other.holder.strip_prefix("run ");
            let claimed = |id: &str| self.rows.get(id).is_some_and(|r| r.0 == "claimed");
            if holder != ORCHESTRATOR || run.is_none_or(claimed) {
                return Err(format!("#{pr} is held by {}: {}", other.holder, other.why));
            }
        }
        let (holder, why) = (holder.into(), why.into());
        let hold = Hold {
            pr,
            holder,
            why,
            touched_ms: 0,
        };
        self.holds.insert(pr, hold);
        Ok(other)
    }
    fn unhold(&mut self, pr: u64) -> Result<(), String> {
        self.holds.remove(&pr);
        Ok(())
    }
    fn holds(&self) -> Result<Vec<Hold>, String> {
        Ok(self.holds.values().cloned().collect())
    }
    fn run_end(&mut self, id: &str, end: &End) -> Result<(), String> {
        self.ends.push((id.into(), end.clone()));
        Ok(())
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
            attempts: row.1,
            outcome: (row.0 == "finished")
                .then(|| self.run.as_ref().map(|r| r.outcome.parts().0.to_string()))
                .flatten(),
            pane: self.run.as_ref().and_then(|r| r.pane.clone()),
            session: self.run.as_ref().and_then(|r| r.session.clone()),
            handed: self.handed.get(id).cloned(),
            back: self.back.contains(id),
            rejudged: self.rejudged.get(id).cloned(),
            deployed: (self.deployed.iter().rev())
                .find(|(d, _)| d == id)
                .map(|(_, w)| w.clone()),
            waits: self.waits.get(id).cloned().unwrap_or_default(),
        };
        Ok(self.rows.iter().map(row).collect())
    }
    fn queue(&self) -> Result<Queue, String> {
        self.listed.clone().ok_or("no sync yet".into())
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
    bases: RefCell<VecDeque<Base>>,
    deploy_fails: Cell<bool>,
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
    /// A launcher that resumes nothing, as Codex's.
    resume_fails: Cell<bool>,
    pr_fails: Cell<bool>,
    /// What its worktree holds that no remote has.
    unsaved: RefCell<Option<String>>,
    /// The layer the queue's tasks name, and each read of a branch's diff in turn.
    layer: Option<&'static str>,
    diffs: RefCell<VecDeque<&'static str>>,
    /// Jev's score of each diff it judges against its RFC section, in turn; empty: the list judges none.
    rfc: RefCell<VecDeque<f64>>,
    /// Whether its worktree pushed a commit of its own; `None`: the read fails.
    own: Cell<Option<bool>>,
    /// The worktrees holding a build, and the tasks the list holds done or dropped.
    built: Vec<&'static str>,
    closed: Vec<&'static str>,
    /// The builds whose clean fails.
    clean_fails: Vec<&'static str>,
    /// The bytes each build holds, `own` the repository's; the builds a cargo runs in.
    sizes: Vec<(&'static str, u64)>,
    cargo_in: Vec<&'static str>,
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
        bases: RefCell::new([Base::Holds].into_iter().collect()),
        deploy_fails: Cell::new(false),
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
        resume_fails: Cell::new(false),
        pr_fails: Cell::new(false),
        unsaved: RefCell::new(None),
        layer: None,
        diffs: RefCell::new([""].into_iter().collect()),
        rfc: RefCell::new(VecDeque::new()),
        own: Cell::new(Some(true)),
        built: vec![],
        closed: vec![],
        clean_fails: vec![],
        sizes: vec![],
        cargo_in: vec![],
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
        let model = task
            .model
            .as_ref()
            .map_or(String::new(), |m| format!(" with {m}"));
        Ok(format!(
            "start {} on {}{model}",
            task.id,
            account.unwrap_or("none")
        ))
    }
    fn resume(
        &self,
        task: &Task,
        account: Option<&str>,
        session: &str,
        _from: &str,
    ) -> Result<String, String> {
        if self.resume_fails.get() {
            return Err("one login: none moves".into());
        }
        let on = account.unwrap_or("none");
        let model = task
            .model
            .as_ref()
            .map_or(String::new(), |m| format!(" with {m}"));
        Ok(format!(
            "resume {session} of {} in {} on {on}{model}",
            task.id, task.folder
        ))
    }
}

impl Agent for Script {
    fn screen(&self, shown: &str) -> Screen {
        match shown {
            "up" | "menu" => Screen::Ready,
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
    fn limit_menu(&self, shown: &str) -> bool {
        shown == "menu"
    }
    fn limits(&self) -> Result<Vec<Limit>, String> {
        self.limit_reads.set(self.limit_reads.get() + 1);
        if self.limits_fail.get() > 0 {
            self.limits_fail.set(self.limits_fail.get() - 1);
            return Err("no such folder".into());
        }
        Ok(take(&self.limits).unwrap_or_default())
    }
    fn tokens(&self, _folder: &str, _session: &str) -> Result<u64, String> {
        Ok(1_200)
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
    fn brief(&self, id: &str, _folder: &str) -> Result<String, String> {
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
    fn reopen(&self, id: &str) -> Result<(), String> {
        self.note(format!("reopen {id}"))
    }
    fn closed(&self, ids: &[String]) -> Result<Vec<String>, String> {
        self.note(format!("closed? {}", ids.join(" ")))?;
        Ok(ids
            .iter()
            .filter(|id| self.closed.contains(&id.as_str()))
            .cloned()
            .collect())
    }
    fn rfc_check(&self, id: &str, layer: &str, _diff: &str) -> Result<Option<RfcScore>, String> {
        let Some(score) = take(&self.rfc) else {
            return Ok(None);
        };
        self.note(format!("rfc-check {id} {layer}"))?;
        let rule = format!("rfc-rust-unified#{}", layer.trim_start_matches("rfc:"));
        Ok(Some(RfcScore { score, rule }))
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
        if self.pr_fails.get() {
            return Err("gh: no answer".into());
        }
        let told = self.earlier.get() || self.did().iter().any(|d| d.starts_with("send "));
        Ok(self.pr.borrow().clone().filter(|pr| pr.merged || told))
    }
    fn checks(&self, _pr: &Pr) -> Result<Checks, String> {
        self.note("checks")?;
        take(&self.checks).expect("a check")
    }
    fn base(&self, _pr: &Pr) -> Result<Base, String> {
        self.note("base")?;
        Ok(take(&self.bases).expect("a base"))
    }
    fn merge(&self, pr: &Pr) -> Result<(), String> {
        self.note(format!("merge {} at {}", pr.number, pr.head))?;
        if self.merge_hiccups.get() > 0 {
            self.merge_hiccups.set(self.merge_hiccups.get() - 1);
            return Err("gh: no answer in 30s".into());
        }
        self.merge_refusal.borrow().clone().map_or(Ok(()), Err)
    }
    /// Main merged in and pushed: the PR's head moves to `def`.
    fn update(&self, id: &str, pr: &Pr) -> Result<(), String> {
        self.note(format!("update {id} {} at {}", pr.number, pr.head))?;
        if let Some(pr) = self.pr.borrow_mut().as_mut() {
            pr.head = "def".into();
        }
        Ok(())
    }
    fn leave(&self, id: &str) -> Result<(), String> {
        self.note(format!("leave {id}"))
    }
    fn restack(&self, merged: &Pr, _busy: &[String]) -> Result<Vec<String>, String> {
        self.note(format!("restack {}", merged.number))?;
        Ok(vec![])
    }
    fn clean(&self, id: &str) -> Result<(), String> {
        self.note(format!("clean {id}"))?;
        match self.clean_fails.contains(&id) {
            true => Err("EACCES".into()),
            false => Ok(()),
        }
    }
    fn built(&self) -> Result<Vec<String>, String> {
        Ok(self.built.iter().map(|id| id.to_string()).collect())
    }
    fn trim(&self, id: Option<&str>, cap: u64) -> Result<Option<u64>, String> {
        let id = id.unwrap_or("own");
        let held = self
            .sizes
            .iter()
            .find(|(at, _)| *at == id)
            .map_or(0, |s| s.1);
        if held <= cap {
            return Ok(None);
        }
        match self.cargo_in.contains(&id) {
            true => Err(format!("a cargo runs in {id}")),
            false => self.note(format!("trim {id}")).map(|()| Some(held)),
        }
    }
    fn unsaved(&self, _id: &str) -> Result<Option<String>, String> {
        Ok(self.unsaved.borrow().clone())
    }
    fn deploy(&self) -> Result<String, String> {
        self.note("deploy")?;
        if self.deploy_fails.get() {
            return Err("cargo: error[E0425]\n  --> src/lib.rs".into());
        }
        Ok("8a81cbd serves".into())
    }
    fn own_pushes(&self, _id: &str, _since_ms: i64) -> Result<bool, String> {
        self.own.get().ok_or("git: no reflog".into())
    }
    fn diff(&self, id: &str) -> Result<String, String> {
        self.note(format!("diff {id}"))?;
        Ok(take(&self.diffs).expect("a diff").into())
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
        Pick::Start {
            id,
            account,
            pool: None,
        }
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

const PACE: Pace = Pace::TEST;

/// The script's list synced into the store, as a pass does before it picks: the list itself is never asked.
fn ports<'a>(tasks: &'a mut Store, s: &'a Script) -> Ports<'a> {
    let task = |id: &&str| Queued {
        held: None,
        id: id.to_string(),
        reopened_ms: s.reopened_ms,
        layer: s.layer.map(String::from),
        ..Queued::default()
    };
    let tasks_listed = s.queue.iter().map(task).collect();
    tasks.listed = Some(Queue {
        seats: 5,
        seats_why: String::new(),
        tasks: tasks_listed,
    });
    Ports {
        tasks,
        list: s,
        terminal: s,
        launcher: s,
        agent: s,
        forge: s,
        policy: s,
        step: &|_, _| {},
        free_memory: &|| None,
        load: &|| None,
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
        models: vec![],
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
    let Pick::Start { id, account, .. } = pick else {
        panic!("{pick:?}");
    };
    assert_eq!(
        (id.as_str(), account.as_deref()),
        ("t1", Some("spent-first"))
    );
    let finished =
        run_task(&id, account.as_deref(), &mut ports(&mut tasks, &s), PACE).expect("recorded");

    // Its pane closed while CI ran: the merge has none left to close.
    let (pane, session) = (None, Some("s1".into()));
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
         send the brief of t1, key Enter, key Enter, judge, base, checks, send leave, key Enter, close p1, \
         base, checks, base, checks, merge 7 at abc, merged t1 https://pr/7, restack 7, leave t1, deploy"
    );
    let began = "begun, no end recorded: its run stopped in the deploy";
    let said = [
        ("t1".into(), began.into()),
        ("t1".into(), "8a81cbd serves".into()),
    ];
    assert_eq!(tasks.deployed, said, "the sha that serves, on its row");
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
            "clean_closed",
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
            "restack",
            "leave",
            "deploy"
        ]
    );
}

/// Task rust-orch-pick-dynamic, done-when 3: `plan_now`, what `dash orch next` prints, is the pick a pass makes, the free memory
/// read once for it, and it changes nothing.
#[test]
fn plan_now_is_the_pick_a_pass_makes_and_reads_the_memory_once() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    let s = script(&[], &[]);
    let reads = Cell::new(0);
    let memory = || {
        reads.set(reads.get() + 1);
        Some(10)
    };
    let mut ports = ports(&mut tasks, &s);
    ports.free_memory = &memory;
    let (queue, rows) = (
        ports.tasks.queue().expect("queue"),
        ports.tasks.standing().expect("rows"),
    );
    let planned = plan_now(&ports, queue, &rows).expect("a plan");
    assert_eq!(reads.get(), 1);
    assert_eq!(s.did(), Vec::<String>::new(), "nothing done");
    let picked = next(&mut ports).expect("a look");
    assert_eq!(planned.next, picked);
    assert!(matches!(picked, Pick::Start { ref id, .. } if id == "t1"));
    assert_eq!(reads.get(), 2, "a pass reads it once too");
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
fn a_crashed_run_s_claim_whose_session_left_is_freed_and_the_task_is_picked_again() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", Some("p9"), Some("s1"), 1);
    let s = script(&[], &[Gone]);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(pick, Pick::Start { id, .. } if id == "t1"));
    assert_eq!(s.did(), ["page t1 over"], "a wait it crashed in ends");
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

/// rust-skeleton-live's case (2026-11-09): its PR merged, then its session left (a pane closed by hand). The run is a merge,
/// recorded finished, the list told, its worktree gone to the pool: never a second try of merged work.
#[test]
fn a_session_gone_after_its_pr_merged_is_a_merge_never_a_retry() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.rows.insert("t1".into(), ("open".into(), 1));
    let s = script(&["up"], &[Gone]);
    *s.pr.borrow_mut() = Some(pr(true));
    let ran = run_with(&mut tasks, &s, PACE);
    assert_eq!(ran.outcome, merged());
    assert_eq!(tasks.rows["t1"], ("finished".to_string(), 2));
    assert!(s.did().iter().any(|d| d.starts_with("merged t1")));
    // its worktree leaves with its build, into the pool: no clean before (task rust-worktree-pool)
    assert_eq!((s.count("leave t1"), s.count("clean t1")), (1, 0));
}

/// The same once its run died too: the next look records the merge, closes its own pane, and picks nothing.
#[test]
fn a_crashed_run_whose_pr_merged_is_recorded_merged_not_tried_again() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", Some("p9"), Some("s1"), 1);
    let s = script(&[], &[Idle]);
    *s.pr.borrow_mut() = Some(pr(true));
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(pick, Pick::Wait(_)), "{pick:?}");
    assert_eq!(tasks.rows["t1"].0, "finished");
    assert_eq!(tasks.run.as_ref().map(|r| &r.outcome), Some(&merged()));
    let did = s.did().join(", ");
    assert!(
        did.contains("close p9")
            && did.contains("clean t1")
            && did.contains("merged t1 https://pr/7")
            && did.contains("no deploy or restack ran for it"),
        "{did}"
    );
    let none = &tasks.deployed[0].1;
    assert!(
        none.starts_with("none: merged after its run died"),
        "{none}"
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

/// Done-when 3 of rust-task-names-layer, with a stand-in judge that says met each time: the brief names the layer, and a diff
/// that draws a page outside the design system holds the run, nudged, until its next diff keeps to it.
#[test]
fn a_run_naming_a_layer_is_held_while_its_diff_draws_a_page_outside_it() {
    let _budget = TestBudget::new();
    let s = Script {
        layer: Some("design:board"),
        ..script(&["up"], &[Idle, Idle, Working, Idle, Gone])
    };
    let page_local = "diff --git a/h b/h\n+++ b/rust/crates/dash/src/home.rs\n@@ -1 +1 @@\n+    rsx! { div { class: \"mine\" } }\n";
    let inside = "diff --git a/c b/c\n+++ b/rust/crates/surface/src/components.rs\n@@ -1 +1 @@\n+    rsx! { div { class: \"li-card\" } }\n";
    *s.diffs.borrow_mut() = [page_local, inside].into_iter().collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    let did = s.did();
    let brief = did
        .iter()
        .find(|d| d.starts_with("send the brief of t1"))
        .expect("told");
    assert!(
        brief.ends_with(&dash_orch::layer_line("design:board")),
        "{brief}"
    );
    assert_eq!((s.count("judge"), s.count("diff t1")), (2, 2), "{did:?}");
    let nudge =
        "send not yet: it names design:board, and its diff draws a page outside the design system";
    assert_eq!(
        did.iter().filter(|d| d.starts_with(nudge)).count(),
        1,
        "{did:?}"
    );
    // a task that names no layer: its diff is never read
    let s = script(&["up"], &[Idle, Gone]);
    *s.diffs.borrow_mut() = [page_local].into_iter().collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!(s.count("diff t1"), 0);
}

/// Done-when 3 of rust-task-rfc-judge, the PM check's half: a met run whose diff Jev scores under 0.5 against its section is
/// handed back, nudged with the score and the rule, until its next diff scores over; each score is a note on its row.
#[test]
fn the_pm_check_hands_back_a_diff_jev_scores_under_the_bar() {
    let _budget = TestBudget::new();
    let s = Script {
        layer: Some("rfc:ui-lib"),
        ..script(&["up"], &[Idle, Idle, Working, Idle, Gone])
    };
    let diff = "diff --git a/m b/m\n+++ b/rust/crates/dash/src/main.rs\n@@ -1 +1 @@\n+const MENU_JS: &str = \"x\";\n";
    *s.diffs.borrow_mut() = [diff, diff].into_iter().collect();
    *s.rfc.borrow_mut() = [0.1, 0.93].into_iter().collect();
    let mut tasks = Store::default();
    assert_eq!(run_with(&mut tasks, &s, PACE).outcome, merged());
    let did = s.did();
    assert_eq!(s.count("rfc-check t1 rfc:ui-lib"), 2, "{did:?}");
    let nudge = "send not yet: Jev scores its diff 0.10 against rfc-rust-unified#ui-lib, under 0.5";
    assert_eq!(
        did.iter().filter(|d| d.starts_with(nudge)).count(),
        1,
        "{did:?}"
    );
    let notes: Vec<&str> = tasks.notes.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(
        notes,
        [
            "rfc-check rfc:ui-lib: 0.10, it breaks rfc-rust-unified#ui-lib",
            "rfc-check rfc:ui-lib: 0.93, it keeps rfc-rust-unified#ui-lib"
        ],
        "{did:?}"
    );
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

/// Two PRs green apart broke main together (E0425), so a green head that lacks main's newest commit never merges as it is. A met
/// run's is updated from main here, its CI awaited on the new head, then merged at that head and deployed, its worktree left first
/// so the build has the disk. Its session is told nothing: #1274 ended « not met after 3 nudges » only because main moved.
#[test]
fn a_met_run_s_pr_behind_main_is_updated_ci_awaited_then_merged_at_the_new_head() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    let bases = [Base::Behind, Base::Behind, Base::Holds, Base::Holds];
    *s.bases.borrow_mut() = bases.into_iter().collect();
    let checks = [
        Checks::Pending,
        Checks::Green,
        Checks::Pending,
        Checks::Green,
    ];
    *s.checks.borrow_mut() = checks.into_iter().map(Ok).collect();
    let mut tasks = Store::default();
    assert_eq!(run_with(&mut tasks, &s, PACE).outcome, merged());
    let freed = (
        "t1".to_string(),
        FREED.trim_start_matches("note t1: ").to_string(),
    );
    assert_eq!(tasks.notes, [freed], "the seat freed, on its page");
    let did = s.did().join(", ");
    // Its CI on the old head settles first: an update per move of main would start a build per move.
    let path = "base, checks, base, checks, update t1 7 at abc, base, checks, base, checks, merge 7 at def, merged t1";
    assert!(
        did.contains(path) && did.ends_with("leave t1, deploy"),
        "{did}"
    );
    assert_eq!(
        s.count("merge 7 at abc"),
        0,
        "never the head that lacks main"
    );
    let told = s
        .did()
        .into_iter()
        .filter(|d| d.starts_with("send not yet"));
    assert_eq!(told.count(), 0, "{did}");
}

const FREED: &str = "note t1: met: its session ended to free its seat while CI runs; its run drives the pull request alone";

/// Done-when 1 of rust-met-frees-seat: four met runs held seats on 2026-11-09 waiting on CI, and the line cap's fix had none to
/// start. At the first wait on CI the session is told to leave and its pane closed, once; the claim drops its pane, which the
/// seats no longer count; the run alone waits, merges and deploys.
#[test]
fn a_met_run_waiting_on_ci_ends_its_session_frees_its_seat_and_lands_its_pr_alone() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Gone]);
    let checks = [Checks::Pending, Checks::Pending, Checks::Green];
    *s.checks.borrow_mut() = checks.into_iter().map(Ok).collect();
    let mut tasks = Store {
        note: Some(s.did.clone()),
        ..Store::default()
    };
    let finished = run_with(&mut tasks, &s, PACE);
    assert_eq!(finished.outcome, merged());
    assert_eq!(finished.pane, None, "no pane left to close");
    let did = s.did().join(", ");
    let path = format!(
        "judge, base, checks, send leave, close p1, left, {FREED}, base, checks, base, checks, merge 7 at abc, record, merged t1"
    );
    assert!(did.contains(&path), "{did}");
    // Task rust-orch-flags-only-stuck, done-when 1: the freed seat is a note on its page, never a flag.
    assert!(!s.did().iter().any(|d| d.starts_with("flag ")), "{did}");
    assert_eq!(s.count("send leave"), 1, "stopped once: {did}");
    assert_eq!(s.count("deploy"), 1, "{did}");
    // The claim as the wait left it: its session kept, its pane dropped. The seats count none for it, as they count one for a
    // claim whose session is not listed yet.
    let claim = tasks.claim_of("t1");
    assert_eq!(
        (claim.pane.as_deref(), claim.session.as_deref()),
        (None, Some("s1"))
    );
    assert_eq!(seats_taken(&s, std::slice::from_ref(&claim)), Ok(0));
    let starting = Claim {
        pane: Some("p1".into()),
        ..claim
    };
    assert_eq!(seats_taken(&s, &[starting]), Ok(1));
}

/// A close that fails after its session left (code review of rust-met-frees-seat): gone is what the look says, so a PR red later
/// is told to no one, and the page's note says the pane stayed open.
#[test]
fn a_session_that_left_with_its_pane_unclosed_is_gone_and_told_nothing() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Gone]);
    s.close_fails.set(1);
    let checks = [Checks::Pending, Checks::Red("test".into())];
    *s.checks.borrow_mut() = checks.into_iter().map(Ok).collect();
    let mut tasks = Store {
        note: Some(s.did.clone()),
        ..Store::default()
    };
    let finished = run_with(&mut tasks, &s, PACE);
    assert!(
        matches!(&finished.outcome, Outcome::Ended(e) if e.starts_with("met, its session ended while CI ran")),
        "{finished:?}"
    );
    let open = "but its pane stayed open: herdr: no answer";
    assert!(s.did().iter().any(|d| d.contains(open)), "{:?}", s.did());
    let told = |d: &String| d.starts_with("send not yet");
    assert!(!s.did().iter().any(told), "{:?}", s.did());
}

/// Done-when 2 of rust-met-frees-seat: with its session gone, a PR that turns red has no one to tell. The run ends, its PR open,
/// and hands its task back itself, its row reopened here (task rust-orch-flags-only-stuck): a new run starts in the same
/// worktree, told what to fix first, and lands.
#[test]
fn a_pr_red_after_its_met_session_left_is_handed_back_to_a_new_run_in_the_same_worktree() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    let s = script(&["up"], &[Idle, Idle, Gone]);
    let red = || Checks::Red("test".into());
    // The first run's wait, then red; its hand-back's read; the new run's first words; its landing.
    let checks = [Checks::Pending, red(), red(), red(), Checks::Green];
    *s.checks.borrow_mut() = checks.into_iter().map(Ok).collect();
    let first = run_with(&mut tasks, &s, PACE);
    let fix = "CI is red on https://pr/7: test. Fix it and push";
    let why = format!("met, its session ended while CI ran, then: {fix}");
    assert_eq!(first.outcome, Outcome::Ended(why));
    let told = |d: &String| d.starts_with("send not yet");
    assert!(!s.did().iter().any(told), "nothing told to a gone session");
    assert_eq!(tasks.rows["t1"], ("open".to_string(), 0), "reopened here");
    assert_eq!(s.count("reopen t1"), 0, "the list never told it ended");
    assert_eq!(hand_backs(&mut ports(&mut tasks, &s)).expect("a pass"), []);
    *s.looks.borrow_mut() = [Idle, Idle, Gone].into_iter().collect();
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    let id = "t1".to_string();
    assert_eq!(
        pick,
        Pick::Start {
            id,
            account: None,
            pool: None
        }
    );
    let second = run_with(&mut tasks, &s, PACE);
    assert_eq!(second.outcome, merged());
    assert_eq!(
        s.count("open /work/rust-t1 rust t1"),
        2,
        "the same worktree"
    );
    let words = format!(
        "send Its pull request needs this first: {fix}.\n\nThis task was opened again: https://pr/7"
    );
    assert!(
        s.did().iter().any(|d| d.starts_with(&words)),
        "{:?}",
        s.did()
    );
}

/// Each `TaskList::flag` call, and each `TaskList::ended`, which the older app flags: the stand-in writes both as « flag <id>: ».
fn flags(s: &Script) -> Vec<String> {
    s.did()
        .into_iter()
        .filter(|d| d.starts_with("flag "))
        .collect()
}

const CONFLICT: &str = "https://pr/7 conflicts with main in a.rs. Merge the base branch into yours, resolve them and push";

/// Task rust-orch-flags-only-stuck, done-when 1, the hand_back path: the hand-back pass reopens an ended run whose PR conflicts,
/// writes a note on its page, and calls no `TaskList::flag`.
#[test]
fn hand_back_writes_a_page_note_and_calls_no_flag() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let s = script(&[], &[]);
    s.earlier.set(true);
    *s.bases.borrow_mut() = [Base::Conflicts(vec!["a.rs".into()])].into_iter().collect();
    let handed = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    assert_eq!(handed, [("t1".to_string(), CONFLICT.to_string())]);
    assert_eq!(flags(&s), Vec::<String>::new(), "no TaskList::flag call");
    let note = (
        "t1".to_string(),
        format!("handed back to a new run: {CONFLICT}"),
    );
    assert_eq!(tasks.notes, [note], "a note on its page");
}

/// Task rust-orch-flags-only-stuck, done-when 1, the freed-seat path: a met run waiting on CI ends its session to free its seat,
/// writes a note on its page, lands its PR and calls no `TaskList::flag`.
#[test]
fn a_freed_seat_writes_a_page_note_and_calls_no_flag() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Gone]);
    let checks = [Checks::Pending, Checks::Pending, Checks::Green];
    *s.checks.borrow_mut() = checks.into_iter().map(Ok).collect();
    let mut tasks = Store::default();
    assert_eq!(run_with(&mut tasks, &s, PACE).outcome, merged());
    assert_eq!(flags(&s), Vec::<String>::new(), "no TaskList::flag call");
    let freed = FREED.trim_start_matches("note t1: ").to_string();
    assert_eq!(
        tasks.notes,
        [("t1".to_string(), freed)],
        "a note on its page"
    );
}

/// Task rust-orch-flags-only-stuck, done-when 1, the ended-with-conflict path: a met run whose PR conflicts after its session left
/// ends, hands its task back itself with a note on its page, and calls no `TaskList::flag` (nor `ended`, which the older app flags).
#[test]
fn an_end_with_a_conflict_writes_a_page_note_and_calls_no_flag() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Gone]);
    let conflict = Base::Conflicts(vec!["a.rs".into()]);
    *s.bases.borrow_mut() = [Base::Holds, conflict].into_iter().collect();
    *s.checks.borrow_mut() = [Ok(Checks::Pending)].into_iter().collect();
    let mut tasks = Store::default();
    let why = format!("met, its session ended while CI ran, then: {CONFLICT}");
    assert_eq!(run_with(&mut tasks, &s, PACE).outcome, Outcome::Ended(why));
    assert_eq!(flags(&s), Vec::<String>::new(), "no TaskList::flag call");
    let note = (
        "t1".to_string(),
        format!("handed back to a new run: {CONFLICT}"),
    );
    assert_eq!(tasks.notes.last(), Some(&note), "a note on its page");
    assert_eq!(tasks.rows["t1"], ("open".to_string(), 0), "handed back");
}

/// Task rust-orch-flags-only-stuck, done-when 1, PR 1388's case: one conflict woke the orchestrator 3 times, for the seat freed,
/// the run ended and the hand-back, each a step this orchestrator moves itself. The same path now flags nothing: the seat and the
/// hand-back are notes on its page, the run's end is never told to the list (whose `ended` flags), and the next look picks it.
#[test]
fn a_conflict_after_its_met_session_left_is_handed_back_with_notes_and_no_flag() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Gone]);
    let conflict = || Base::Conflicts(vec!["a.rs".into()]);
    // The first look waits on CI, which frees the seat; then main moves under it.
    *s.bases.borrow_mut() = [Base::Holds, conflict()].into_iter().collect();
    *s.checks.borrow_mut() = [Ok(Checks::Pending)].into_iter().collect();
    let mut tasks = Store {
        note: Some(s.did.clone()),
        ..Store::default()
    };
    let first = run_with(&mut tasks, &s, PACE);
    let fix = "https://pr/7 conflicts with main in a.rs. Merge the base branch into yours, resolve them and push";
    assert_eq!(
        first.outcome,
        Outcome::Ended(format!("met, its session ended while CI ran, then: {fix}"))
    );
    assert_eq!(hand_backs(&mut ports(&mut tasks, &s)).expect("a pass"), []);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(
        matches!(&pick, Pick::Start { id, .. } if id == "t1"),
        "{pick:?}"
    );
    let did = s.did();
    assert!(!did.iter().any(|d| d.starts_with("flag ")), "{did:?}");
    let notes: Vec<&str> = tasks.notes.iter().map(|(_, n)| n.as_str()).collect();
    let freed = "met: its session ended to free its seat while CI runs; its run drives the pull request alone";
    assert_eq!(notes, [freed, &format!("handed back to a new run: {fix}")]);
    assert_eq!(tasks.handed["t1"], "abc 1", "counted toward the cap");
    assert!(tasks.back.contains("t1"), "picked as a hand-back");
    assert_eq!(
        tasks.holds[&7].holder, "run t1",
        "its PR held for the next run"
    );
}

/// Done-when 2: what this orchestrator cannot move still flags, once. At the cap, the run's end is told to the list in the cap's
/// words, the ones the hand-back pass flags later, which the list keeps once a day; a read refused at the hand-back (its PR's CI
/// unread) is told in its end's own words. Either way the list's `ended` is the one flag.
#[test]
fn a_conflict_at_the_hand_back_cap_or_with_a_read_refused_still_flags() {
    let _budget = TestBudget::new();
    let conflict = || Base::Conflicts(vec!["a.rs".into()]);
    let fix = "https://pr/7 conflicts with main in a.rs. Merge the base branch into yours, resolve them and push";

    let s = script(&["up"], &[Idle, Idle, Gone]);
    *s.bases.borrow_mut() = [Base::Holds, conflict()].into_iter().collect();
    *s.checks.borrow_mut() = [Ok(Checks::Pending)].into_iter().collect();
    let mut tasks = Store::default();
    tasks.handed.insert("t1".into(), "older 3".into());
    run_with(&mut tasks, &s, PACE);
    let cap = format!("flag t1: not handed back: 3 hand-backs left it so: {fix}");
    assert_eq!(flags(&s), std::slice::from_ref(&cap));
    assert_eq!(tasks.rows["t1"].0, "finished", "not reopened");
    // The pass after flags the same words, which the list keeps once a day.
    s.earlier.set(true);
    assert_eq!(hand_backs(&mut ports(&mut tasks, &s)).expect("a pass"), []);
    assert_eq!(flags(&s), [cap.clone(), cap]);

    let s = script(&["up"], &[Idle, Idle, Gone]);
    *s.bases.borrow_mut() = [Base::Holds, Base::Holds, Base::Behind]
        .into_iter()
        .collect();
    let red = Ok(Checks::Red("test".into()));
    *s.checks.borrow_mut() = [Ok(Checks::Pending), red, Err("gh: refused".into())]
        .into_iter()
        .collect();
    let mut tasks = Store::default();
    run_with(&mut tasks, &s, PACE);
    let refused = "flag t1: met, its session ended while CI ran, then: CI is red on https://pr/7: test. Fix it and push";
    assert_eq!(flags(&s), [refused]);
    assert_eq!(tasks.rows["t1"].0, "finished", "not reopened");

    // Unlisted meanwhile, the list closed or dropped it: told its end, never opened again here (/code-review of d3b5a3272).
    let mut s = script(&["up"], &[Idle, Idle, Gone]);
    s.queue = vec![];
    *s.bases.borrow_mut() = [Base::Holds, conflict()].into_iter().collect();
    *s.checks.borrow_mut() = [Ok(Checks::Pending)].into_iter().collect();
    let mut tasks = Store::default();
    run_with(&mut tasks, &s, PACE);
    assert_eq!(
        flags(&s),
        [format!(
            "flag t1: met, its session ended while CI ran, then: {fix}"
        )]
    );
    assert_eq!(tasks.rows["t1"].0, "finished", "not reopened");
    assert!(
        tasks
            .notes
            .iter()
            .all(|(_, n)| !n.starts_with("handed back"))
    );
}

/// A conflict with main goes back to its session naming the files, with no merge tried.
#[test]
fn a_head_conflicting_with_main_goes_back_naming_the_files() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Working, Idle, Gone]);
    let files = vec!["a.rs".into(), "b.rs".into()];
    *s.bases.borrow_mut() = [Base::Conflicts(files), Base::Holds].into_iter().collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    let told = "send not yet: https://pr/7 conflicts with main in a.rs, b.rs.";
    let did = s.did().join(", ");
    assert!(did.contains(&format!("base, {told}")), "{did}");
    assert_eq!(s.count("merge 7 at abc"), 1, "only once it holds main");
    assert_eq!(s.count("deploy"), 1);
}

fn ended(tasks: &mut Store, pane: Option<&str>) {
    tasks.rows.insert("t1".into(), ("finished".into(), 1));
    tasks.run = Some(Run {
        pane: pane.map(String::from),
        session: pane.map(|_| "s1".to_string()),
        outcome: Outcome::Ended("not met after 3 nudges".into()),
    });
}

/// PR 1250's case: its run ended, its session gone, and main moved under its PR until it conflicts. The hand-back pass reopens its
/// task through the list, as `dash task reopen` does; the next sync starts its row over and the look picks it: a new run in its
/// worktree, its first words naming the files, merged once it holds main.
#[test]
fn an_ended_run_s_conflicting_pr_is_reopened_for_a_new_session_told_what_to_fix() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let mut s = script(&["up"], &[Idle, Gone]);
    s.earlier.set(true);
    let conflict = Base::Conflicts(vec!["a.rs".into()]);
    *s.bases.borrow_mut() = [conflict.clone(), conflict, Base::Holds]
        .into_iter()
        .collect();
    let handed = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    let words = "https://pr/7 conflicts with main in a.rs. Merge the base branch into yours, resolve them and push";
    assert_eq!(handed, [("t1".to_string(), words.to_string())]);
    assert!(s.did().contains(&"reopen t1".to_string()));
    // Task rust-orch-flags-only-stuck, done-when 1: a hand-back is a note on its page, never a flag.
    let note = (
        "t1".to_string(),
        format!("handed back to a new run: {words}"),
    );
    assert_eq!(tasks.notes, [note]);
    assert!(
        !s.did().iter().any(|d| d.starts_with("flag ")),
        "{:?}",
        s.did()
    );
    assert_eq!(tasks.handed["t1"], "abc 1", "its head kept, and its count");
    // The list's reopen, as the next sync brings it.
    s.reopened_ms = Some(1);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(
        pick,
        Pick::Start {
            id: "t1".into(),
            account: None,
            pool: None,
        }
    );
    let ran = run_task("t1", None, &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(ran.outcome, merged());
    let first = format!(
        "send Its pull request needs this first: {words}.\n\nThis task was opened again: https://pr/7"
    );
    assert!(
        s.did().iter().any(|d| d.starts_with(&first)),
        "{:?}",
        s.did()
    );
    assert_eq!(s.count("merge 7 at abc"), 1);
}

/// Task rust-pick-handbacks-first, done-when 3, comments-board-pill's case: handed back on a conflict, then someone pushed the
/// fix before its pick. The pick reads its PR again first: the head moved and needs no fix, so no session starts, its row ends
/// again with its PR for the hand-back pass to watch, and the next task starts in the same look. A merge since ends it the same
/// way; a moved head still red starts as picked; a PR that cannot be read starts it in no look, the next task meanwhile.
#[test]
fn a_hand_back_whose_head_moved_and_needs_no_fix_starts_no_session() {
    let _budget = TestBudget::new();
    let hand_back = |tasks: &mut Store, s: &mut Script| {
        ended(tasks, None);
        s.earlier.set(true);
        *s.bases.borrow_mut() = [Base::Conflicts(vec!["a.rs".into()])].into_iter().collect();
        assert_eq!(hand_backs(&mut ports(tasks, s)).expect("a pass").len(), 1);
        assert_eq!(tasks.handed["t1"], "abc 1");
        s.reopened_ms = Some(1);
        // the fix pushed by someone else: a new head that holds main
        s.pr.borrow_mut().as_mut().expect("a PR").head = "def".into();
        *s.bases.borrow_mut() = [Base::Holds].into_iter().collect();
    };
    let t2 = Pick::Start {
        id: "t2".into(),
        account: None,
        pool: None,
    };
    let mut tasks = Store::default();
    let mut s = script(&["up"], &[Idle, Gone]);
    s.queue = vec!["t2", "t1"];
    hand_back(&mut tasks, &mut s);
    *s.checks.borrow_mut() = [Ok(Checks::Pending)].into_iter().collect();
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    let why = "its PR head moved from abc to def since it was handed back, and https://pr/7 needs no fix now: no session started";
    assert_eq!(pick, t2, "the next task, in the same look");
    assert_eq!(
        tasks.rows["t1"].0, "finished",
        "ended again, its PR watched"
    );
    let run = tasks.run.as_ref().expect("recorded");
    assert_eq!(run.outcome, Outcome::Ended(why.into()));
    assert!(tasks.holds.is_empty(), "no run: its hand-back's hold ends");
    assert!(
        s.did().contains(&format!("flag t1: {why}")),
        "{:?}",
        s.did()
    );
    assert_eq!(s.count("worktree t1"), 0, "no session");
    // the list's reopen already applied: the stand-in reopens on any stamp, the store only on one newer than its end
    s.reopened_ms = None;
    let later = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(later, t2, "ended: no longer a hand-back, nor open");

    let mut tasks = Store::default();
    let mut s = script(&["up"], &[Idle, Gone]);
    s.queue = vec!["t2", "t1"];
    hand_back(&mut tasks, &mut s);
    s.pr.borrow_mut().as_mut().expect("a PR").merged = true;
    assert_eq!(next(&mut ports(&mut tasks, &s)).expect("a look"), t2);
    let why = "https://pr/7 merged since it was handed back at abc: no session started";
    let run = tasks.run.as_ref().expect("recorded");
    assert_eq!(
        run.outcome,
        Outcome::Ended(why.into()),
        "for hand_backs to judge again"
    );

    let mut tasks = Store::default();
    let mut s = script(&["up"], &[Idle, Gone]);
    s.queue = vec!["t2", "t1"];
    hand_back(&mut tasks, &mut s);
    s.pr_fails.set(true);
    assert_eq!(next(&mut ports(&mut tasks, &s)).expect("a look"), t2);
    assert_eq!(
        tasks.rows["t1"].0, "open",
        "unread: still a hand-back, for the next look"
    );
    s.pr_fails.set(false);

    let mut tasks = Store::default();
    let mut s = script(&["up"], &[Idle, Gone]);
    hand_back(&mut tasks, &mut s);
    *s.checks.borrow_mut() = [Ok(Checks::Red("test".into()))].into_iter().collect();
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    let id = "t1".to_string();
    assert_eq!(
        pick,
        Pick::Start {
            id,
            account: None,
            pool: None
        },
        "still red: it starts"
    );
}

/// Not reopened: a head already handed back, which no session moved since; a head only behind main, which would start a run per
/// move of main; a run whose last session is still in its pane, as a failed stop leaves it, so no second session starts beside it.
#[test]
fn a_pr_handed_at_this_head_only_behind_or_beside_its_live_session_is_not_reopened() {
    let _budget = TestBudget::new();
    let conflict = || Base::Conflicts(vec!["a.rs".into()]);
    type Case<'a> = (Option<&'a str>, Option<&'a str>, Base, &'a [Look]);
    let cases: [Case; 3] = [
        (Some("abc"), None, conflict(), &[]),
        (None, None, Base::Behind, &[]),
        (None, Some("p0"), conflict(), &[Idle]),
    ];
    for (handed, pane, base, looks) in cases {
        let mut tasks = Store::default();
        ended(&mut tasks, pane);
        if let Some(head) = handed {
            tasks.handed.insert("t1".into(), head.into());
        }
        let s = script(&[], looks);
        s.earlier.set(true);
        *s.bases.borrow_mut() = [base.clone()].into_iter().collect();
        let got = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
        assert_eq!(got, [], "{handed:?} {pane:?} {base:?}");
        assert_eq!(s.count("reopen t1"), 0);
        let held = s
            .did()
            .iter()
            .any(|d| d.starts_with("flag t1: not handed back: a session is still"));
        assert_eq!(held, pane.is_some(), "said where it is read");
    }
}

/// Red and behind main is still red: gate stops at main, so the hand-back reads the checks itself (review of PR 1256).
#[test]
fn a_red_pr_behind_main_is_handed_back() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let s = script(&[], &[]);
    s.earlier.set(true);
    *s.bases.borrow_mut() = [Base::Behind].into_iter().collect();
    *s.checks.borrow_mut() = [Ok(Checks::Red("test".into()))].into_iter().collect();
    let got = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    assert_eq!(got.len(), 1, "{:?}", s.did());
    assert_eq!(got[0].1, "CI is red on https://pr/7: test. Fix it and push");
    assert_eq!(s.count("reopen t1"), 1);
}

/// A reopen starts the tries over, so the hand-backs are counted: past three, a PR its sessions keep red is flagged, not reopened.
#[test]
fn a_pr_handed_back_three_times_is_flagged_not_reopened_again() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    tasks.handed.insert("t1".into(), "older 3".into());
    let s = script(&[], &[]);
    s.earlier.set(true);
    *s.checks.borrow_mut() = [Ok(Checks::Red("test".into()))].into_iter().collect();
    let got = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    assert_eq!(got, []);
    assert_eq!(s.count("reopen t1"), 0);
    let flag = "flag t1: not handed back: 3 hand-backs left it so: CI is red on https://pr/7: test";
    assert!(s.did().iter().any(|d| d.starts_with(flag)), "{:?}", s.did());
}

/// Four tasks gave up after 3 nudges, then their PRs merged, and nothing judged them again (rust-ended-merged-rejudged). The pass
/// judges such a run again while it is listed: met, it lands as a merge, the list told, its verdict kept for `dash task show`.
/// Unlisted, the list closed it: never judged, as three tasks done that morning were reopened (2026-11-09).
#[test]
fn a_run_given_up_on_whose_pr_merged_later_is_judged_again_and_lands_met() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let mut s = script(&[], &[]);
    *s.pr.borrow_mut() = Some(pr(true));
    let queue = std::mem::take(&mut s.queue);
    assert_eq!(hand_backs(&mut ports(&mut tasks, &s)).expect("a pass"), []);
    assert_eq!(
        (s.count("judge"), s.count("reopen t1")),
        (0, 0),
        "unlisted: closed by the list"
    );
    s.queue = queue;
    let got = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    let words = "met: its pull request https://pr/7 merged after its run ended";
    assert_eq!(got, [("t1".to_string(), words.to_string())]);
    assert_eq!(
        *s.said.borrow(),
        "its pull request https://pr/7 merged after its run ended"
    );
    assert_eq!(
        tasks.run.as_ref().map(|r| r.outcome.clone()),
        Some(merged())
    );
    assert!(
        s.did().contains(&"merged t1 https://pr/7".to_string()),
        "{:?}",
        s.did()
    );
    assert_eq!(tasks.rejudged["t1"], format!("abc {words}"));
    assert_eq!(s.count("clean t1"), 1, "its build output goes");
    // Merged now: no later pass judges it again.
    assert_eq!(hand_backs(&mut ports(&mut tasks, &s)).expect("a pass"), []);
    assert_eq!(s.count("judge"), 1);
}

/// The list told of a merge judged met even when the first tell fails: the record is untold, so the next look tells it. Blocked
/// is flagged, never reopened: a new run would meet the same block (/code-review of rust-ended-merged-rejudged).
#[test]
fn a_merge_judged_again_is_told_at_the_next_look_and_a_blocked_one_only_flagged() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    tasks.untold = None;
    let s = script(&[], &[]);
    *s.pr.borrow_mut() = Some(pr(true));
    s.merged_fails.set(true);
    assert_eq!(
        hand_backs(&mut ports(&mut tasks, &s))
            .expect("a pass")
            .len(),
        1
    );
    assert_eq!(tasks.untold, Some(merged()), "not told yet");
    s.merged_fails.set(false);
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(
        (s.count("merged t1 https://pr/7"), tasks.untold.clone()),
        (2, None)
    );

    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let s = script(&[], &[]);
    *s.pr.borrow_mut() = Some(pr(true));
    *s.verdicts.borrow_mut() = [Ok(Verdict::Blocked("his key".into()))]
        .into_iter()
        .collect();
    let got = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    let words = "blocked, not reopened: its pull request https://pr/7 merged after its run ended, and it needs his key";
    assert_eq!(got, [("t1".to_string(), words.to_string())]);
    assert_eq!(s.count("reopen t1"), 0);
    assert!(
        s.did().contains(&format!("flag t1: judged again, {words}")),
        "{:?}",
        s.did()
    );
}

/// Not met: its task reopens with the reason on its card. The same merged head is never judged twice, a new run that ends on it
/// included; a session still in its last pane holds the judge, as it holds a hand-back.
#[test]
fn a_run_given_up_on_whose_merged_pr_is_judged_not_met_reopens_with_the_reason_once() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let s = script(&[], &[]);
    *s.pr.borrow_mut() = Some(pr(true));
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("part 2".into()))].into_iter().collect();
    let got = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    let words =
        "not met, reopened: its pull request https://pr/7 merged after its run ended, but part 2";
    assert_eq!(got, [("t1".to_string(), words.to_string())]);
    assert_eq!(s.count("reopen t1"), 1);
    assert!(
        s.did().contains(&format!("flag t1: judged again, {words}")),
        "{:?}",
        s.did()
    );
    assert_eq!(tasks.rejudged["t1"], format!("abc {words}"));
    assert_eq!(hand_backs(&mut ports(&mut tasks, &s)).expect("a pass"), []);
    assert_eq!(
        (s.count("judge"), s.count("reopen t1")),
        (1, 1),
        "its head was judged"
    );

    let mut tasks = Store::default();
    ended(&mut tasks, Some("p0"));
    let s = script(&[], &[Idle]);
    *s.pr.borrow_mut() = Some(pr(true));
    assert_eq!(hand_backs(&mut ports(&mut tasks, &s)).expect("a pass"), []);
    assert_eq!((s.count("judge"), s.count("reopen t1")), (0, 0));
    let held = "flag t1: not judged again: a session is still in its last pane p0";
    assert!(s.did().contains(&held.to_string()), "{:?}", s.did());
}

/// A merge whose pane did not close may still have its session at work in the worktree: it stays, but not its build output, and
/// main still goes live.
#[test]
fn a_merge_whose_pane_stayed_open_keeps_its_worktree_and_still_deploys() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle]);
    s.close_fails.set(u32::MAX);
    let ran = run_task("t1", None, &mut ports(&mut Store::default(), &s), PACE);
    assert!(ran.expect_err("named").contains("its pane stayed open"));
    assert_eq!(s.count("leave t1"), 0);
    assert_eq!(s.count("clean t1"), 1, "its build output goes all the same");
    assert_eq!(s.count("deploy"), 1);
}

/// A merge whose deploy fails stays a merge, its task closed, and the failure is named and counted on its row for the look's
/// retry, flagging no one yet.
#[test]
fn a_deploy_that_fails_after_a_merge_is_named_and_counted() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    s.deploy_fails.set(true);
    let mut tasks = Store::default();
    let ran = run_task("t1", None, &mut ports(&mut tasks, &s), PACE);
    let e = ran.expect_err("named");
    assert!(
        e.contains("t1 merged, but main is not live: cargo: error[E0425]"),
        "{e}"
    );
    assert_eq!(
        tasks.run.as_ref().map(|r| r.outcome.clone()),
        Some(merged())
    );
    assert!(s.did().iter().any(|d| d.starts_with("merged t1")));
    assert!(
        !s.did().iter().any(|d| d.contains("not live")),
        "no flag at the first failure"
    );
    // One line on the row, as `dash task show` prints a field a line: the whole error is the flag's.
    let last = tasks.deployed.last().map(|(_, w)| w.as_str());
    let row = "failed 1: t1 merged, but main is not live: cargo: error[E0425] --> src/lib.rs";
    assert_eq!(last, Some(row), "the error, on its row");
    assert!(
        deploy_pending(&tasks.standing().expect("rows")),
        "the look retries it"
    );
}

/// Done-when 2 of rust-deploy-fetch-retry: a failed deploy is retried at each look until main serves, flagged once, at the
/// third failure; once main serves, no look deploys again.
#[test]
fn a_failed_deploy_is_retried_at_each_look_until_main_serves_and_flagged_at_the_third_failure() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    s.deploy_fails.set(true);
    let mut tasks = Store::default();
    let _ = run_task("t1", None, &mut ports(&mut tasks, &s), PACE);
    let flags = |s: &Script| {
        s.did()
            .iter()
            .filter(|d| d.contains("deploys failed"))
            .count()
    };
    let row = |tasks: &Store| {
        tasks
            .deployed
            .last()
            .map(|(_, w)| w.clone())
            .unwrap_or_default()
    };
    let e = redeploy(&mut ports(&mut tasks, &s)).expect_err("a second failure");
    assert!(
        e.starts_with("t1 merged, but main is not live: cargo"),
        "{e}"
    );
    assert!(row(&tasks).starts_with("failed 2: t1 merged, but main is not live"));
    assert_eq!(flags(&s), 0, "two failures flag no one");
    redeploy(&mut ports(&mut tasks, &s)).expect_err("a third failure");
    assert!(row(&tasks).starts_with("failed 3: "), "{}", row(&tasks));
    let flag = "flag t1: t1 merged, but main is not live: cargo: error[E0425]\n  --> src/lib.rs (3 deploys failed; each look tries again until main serves)";
    assert!(s.did().iter().any(|d| d == flag), "{:?}", s.did());
    redeploy(&mut ports(&mut tasks, &s)).expect_err("still red");
    assert!(row(&tasks).starts_with("failed 4: "), "{}", row(&tasks));
    assert_eq!(flags(&s), 1, "flagged once");
    s.deploy_fails.set(false);
    let served = redeploy(&mut ports(&mut tasks, &s)).expect("main serves");
    assert_eq!(served.as_deref(), Some("8a81cbd serves"));
    let row_said = "8a81cbd serves (after 4 failed deploys)";
    assert_eq!(
        row(&tasks),
        row_said,
        "its row says what serves, and what came before"
    );
    assert!(!deploy_pending(&tasks.standing().expect("rows")));
    let deploys = s.count("deploy");
    assert_eq!(
        redeploy(&mut ports(&mut tasks, &s)),
        Ok(None),
        "nothing to retry"
    );
    assert_eq!(s.count("deploy"), deploys, "no deploy once main serves");
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
    assert_eq!(s.count("key Escape"), 0, "a question is no limit menu");
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

    // A crashed run's merged PR is its own, landed just before the crash: the look records the merge, and no retry starts.
    let already = script(&["up"], &[Idle, Gone]);
    *already.pr.borrow_mut() = Some(pr(true));
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", None, None, 1);
    let pick = next(&mut ports(&mut tasks, &already)).expect("a look");
    assert!(matches!(pick, Pick::Wait(_)), "{pick:?}");
    assert_eq!(tasks.run.map(|r| r.outcome), Some(merged()));
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
    // Its work is met: main moving is the run's to follow, never words to it (rust-merge-updates-stale-head).
    let base = |d: &String| d.to_lowercase().contains("merge the base branch");
    assert!(!refused.did().iter().any(base), "{:?}", refused.did());
    assert!(
        !matches!(&finished.outcome, Outcome::Ended(e) if e.to_lowercase().contains("merge the base branch"))
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
    // Its own session still there is closed by none: a new run adopts it (on its last try, `a_failed_close_holds_...`).
    for (saved, look, adopted) in [
        (Some("s1"), Taken, false),
        (Some("s1"), Gone, false),
        (Some("s1"), Idle, true),
        // Nothing saved: nothing here can name what that pane holds.
        (None, Idle, false),
        (None, Gone, false),
    ] {
        let mut tasks = Store::default();
        tasks.left_by_a_crash("t1", Some("p9"), saved, 1);
        let s = script(&[], &[look]);
        let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
        match adopted {
            true => assert_eq!(pick, Pick::Adopt("t1".into())),
            false => assert!(matches!(pick, Pick::Start { id, .. } if id == "t1")),
        }
        assert_eq!(s.count("close p9"), 0, "{saved:?}");
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
    assert_eq!(
        pick,
        Pick::Adopt("t1".into()),
        "once its id shows, it is adopted"
    );
    assert_eq!(s.count("close p9"), 0);
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

    // On its last try: an earlier one's session is adopted, never closed (`after_a_crash_only_...`).
    tasks.rows.get_mut("t1").expect("its row").1 = 3;
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
    assert_eq!(
        look(&mut tasks),
        Pick::Wait("nothing".into()),
        "out of tries"
    );
    assert_eq!(s.count("close p1"), 1);
    assert_eq!(tasks.rows["t1"].0, "finished");
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
    // Its own account full twice, and no other with room to move to.
    let full = vec![limit("a", 100.0), limit("b", 100.0)];
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

/// Its login unknown (no reading at its start), it is not known to have room while any login is full: it moves to one that has,
/// never waits days (/code-review of this task).
#[test]
fn a_run_on_no_named_account_continues_when_every_login_has_room_else_moves() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Idle, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    // A key's row spent to its cap is a headless run's, never this session's: it holds no wait.
    let key = Limit {
        key: true,
        ..limit("a-key", 100.0)
    };
    *s.limits.borrow_mut() = [vec![limit("a", 0.0), key]].into_iter().collect();
    assert_eq!(run(&s, PACE).outcome, merged());
    assert_eq!((s.limit_reads.get(), s.count("send continue")), (1, 1));
    assert_eq!(s.count("open /work/rust-t1 rust t1"), 1, "no move");

    let s = script(
        &["up"],
        &[Idle, Idle, Idle, Gone, Gone, Idle, Working, Idle, Gone],
    );
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    *s.limits.borrow_mut() = [vec![limit("a", 100.0), limit("b", 0.0)]]
        .into_iter()
        .collect();
    let mut tasks = Store::default();
    assert_eq!(run_with(&mut tasks, &s, PACE).outcome, merged());
    assert_eq!(s.limit_reads.get(), 1, "no wait");
    assert_eq!(s.count("run resume s1 of t1 in /work/rust-t1 on b"), 1);
    assert_eq!(tasks.logins, ["b"]);
}

/// /code-review of this task: a session whose id the run cannot name (a /clear) may be this work, so none starts beside it.
#[test]
fn no_resume_starts_beside_a_session_its_pane_still_lists_and_this_run_cannot_name() {
    let _budget = TestBudget::new();
    // Its stop; the look before Escape; then its pane lists another id, for good.
    let s = script(&["up"], &[Idle, Idle, Taken]);
    *s.outcomes.borrow_mut() = [limited()].into_iter().collect();
    *s.limits.borrow_mut() = [vec![limit("a", 100.0), limit("b", 0.0)]]
        .into_iter()
        .collect();
    let mut tasks = Store::default();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("cannot name")),
        "{:?}",
        finished.outcome
    );
    assert_eq!(s.count("run resume s1 of t1 in /work/rust-t1 on b"), 0);
    assert_eq!(s.count("open /work/rust-t1 rust t1"), 1, "no second pane");
}

/// /code-review of this task: a gap in the listing while its own login is back is looked at again, never a move; a session
/// gone before its « continue » is resumed as one gone during the wait.
#[test]
fn a_gap_in_the_listing_moves_nothing_and_a_session_gone_before_continue_is_resumed() {
    let _budget = TestBudget::new();
    // Its stop; a look that missed it; listed again; listed for the words; at work; its report; gone.
    let s = script(&["up"], &[Idle, Gone, Idle, Idle, Working, Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    let none = vec![limit("a", 100.0), limit("b", 100.0)];
    let back = vec![limit("a", 10.0), limit("b", 0.0)];
    *s.limits.borrow_mut() = [none, back].into_iter().collect();
    let finished = run_task("t1", Some("a"), &mut ports(&mut Store::default(), &s), PACE);
    assert_eq!(finished.expect("recorded").outcome, merged());
    assert_eq!(s.count("send continue"), 1, "{:?}", s.did());
    assert!(!s.did().iter().any(|d| d.starts_with("run resume")));

    // Its stop; gone through the look before Escape, `tell`'s three, `resume`'s, its stop's two and the one after; listed in its
    // new pane; at work; its report; gone.
    let looks = [
        Idle, Gone, Gone, Gone, Gone, Gone, Gone, Gone, Gone, Idle, Working, Idle, Gone,
    ];
    let s = script(&["up"], &looks);
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    *s.limits.borrow_mut() = [vec![limit("a", 10.0)]].into_iter().collect();
    let finished = run_task("t1", Some("a"), &mut ports(&mut Store::default(), &s), PACE);
    assert_eq!(finished.expect("recorded").outcome, merged());
    assert_eq!(s.count("run resume s1 of t1 in /work/rust-t1 on a"), 1);
    assert_eq!(s.count("send continue"), 1, "{:?}", s.did());
}

/// /code-review of this task: a launcher that resumes nothing (Codex's one login) fails the run once its session is gone, never
/// after days of waiting for room.
#[test]
fn a_session_gone_that_its_launcher_cannot_resume_fails_at_once() {
    let _budget = TestBudget::new();
    let s = script(&["up"], &[Idle, Gone]);
    *s.outcomes.borrow_mut() = [limited()].into_iter().collect();
    *s.limits.borrow_mut() = [vec![limit("a", 100.0)]].into_iter().collect();
    s.resume_fails.set(true);
    let finished = run_task("t1", Some("a"), &mut ports(&mut Store::default(), &s), PACE);
    let finished = finished.expect("recorded");
    assert!(
        matches!(&finished.outcome, Outcome::Failed(e) if e.contains("none moves")),
        "{:?}",
        finished.outcome
    );
    assert_eq!(
        s.limit_reads.get(),
        4,
        "its stop_polls looks, then one reading"
    );
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
    // Its stop, the look before Escape, listed for the words, at work.
    let turn = [Idle, Idle, Idle, Working];
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

/// Six runs sat on second's limit menu from 11:25 to its reset two days on, default at 4% (2026-11-09; @nha: « fix the root
/// cause not the surface one-time thing. »): the session moves to the login with room, never a new one.
#[test]
fn a_limited_run_resumes_on_a_login_with_room_in_its_worktree_its_session_and_menu_closed() {
    let _budget = TestBudget::new();
    // Its stop; its menu, Escape; closed; the stop's look; gone after it and once closed; listed in its new pane; at work; its
    // report; gone.
    let s = script(
        &["up", "menu"],
        &[
            Idle, Asking, Idle, Idle, Gone, Gone, Idle, Working, Idle, Gone,
        ],
    );
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    // The spent login is the one the pick named; the other's week resets later, as default's did.
    let other = Limit {
        week_resets: 9,
        ..limit("b", 4.0)
    };
    *s.limits.borrow_mut() = [vec![limit("a", 100.0), other]].into_iter().collect();
    let mut tasks = Store::default();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(finished.outcome, merged());
    let did = s.did();
    let step = |what: &str| {
        let at = did.iter().position(|d| d == what);
        at.unwrap_or_else(|| panic!("no {what}: {did:?}"))
    };
    let path = [
        "key Escape",
        "send leave",
        "close p1",
        "run resume s1 of t1 in /work/rust-t1 on b",
        "send continue",
    ];
    let at: Vec<usize> = path.iter().map(|p| step(p)).collect();
    assert!(at.is_sorted(), "{did:?}");
    assert_eq!(s.count("worktree t1"), 1, "the same worktree");
    assert_eq!(s.count("open /work/rust-t1 rust t1"), 2, "a new pane there");
    assert_eq!(s.count("run start t1 on a"), 1, "never a new session");
    assert_eq!(
        tasks.saved["t1"].1.as_deref(),
        Some("s1"),
        "the same session"
    );
    assert_eq!(tasks.logins, ["a", "b"]);
    assert!(did.iter().any(|d| d.contains("limited on a: resumed on b")));
    assert_eq!(s.limit_reads.get(), 1, "no wait");
    assert_eq!(s.count("judge"), 1, "its report alone");
    assert_eq!(tasks.rows["t1"], ("finished".to_string(), 1), "no retry");
}

/// Task rust-pools-fable (/code-review finding 1): a run on Fable that Fable's own allowance stopped is never told to go on where
/// Fable is spent. A login with Fable left takes it on Fable; with none left anywhere, it resumes on the login's own model, its
/// route's next pool, and its week's room alone is read.
#[test]
fn a_run_its_fable_allowance_stopped_moves_to_fable_left_else_to_the_login_s_own_model() {
    let _budget = TestBudget::new();
    let fable = |used: f64| {
        vec![dash_core::tasks::Scoped {
            model: "fable".into(),
            used,
            resets: u64::MAX,
        }]
    };
    let run = |b_fable: f64| {
        let s = script(
            &["up", "menu"],
            &[
                Idle, Asking, Idle, Idle, Gone, Gone, Idle, Working, Idle, Gone,
            ],
        );
        *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
            .into_iter()
            .collect();
        // a's week has room, its Fable none; b's week resets later
        let a = Limit {
            models: fable(100.0),
            ..limit("a", 10.0)
        };
        let b = Limit {
            week_resets: 9,
            models: fable(b_fable),
            ..limit("b", 4.0)
        };
        *s.limits.borrow_mut() = [vec![a, b]].into_iter().collect();
        let mut tasks = Store::default();
        let ran = run_task_on(
            "t1",
            Some("a"),
            Some("fable"),
            &mut ports(&mut tasks, &s),
            PACE,
        );
        assert_eq!(ran.expect("recorded").outcome, merged());
        s
    };
    let left = run(30.0);
    assert_eq!(left.count("run start t1 on a with fable"), 1);
    assert_eq!(
        left.count("send continue"),
        1,
        "never told to go on on a: {:?}",
        left.did()
    );
    assert_eq!(
        left.count("run resume s1 of t1 in /work/rust-t1 on b with fable"),
        1,
        "{:?}",
        left.did()
    );
    let none = run(100.0);
    assert_eq!(
        none.count("run resume s1 of t1 in /work/rust-t1 on a"),
        1,
        "Fable spent everywhere: the login's own model, on the week that resets first: {:?}",
        none.did()
    );
}

/// Claude's limit menu (« Stop and wait for limit to reset ») is a question to the terminal: merge-gate's run sat on it as one,
/// 2026-11-09. It is read at once as the limit, never at the stuck limit, closed before « continue », and read again the same
/// way when it comes back with the same words.
#[test]
fn a_limit_menu_reads_as_limited_never_a_question() {
    let _budget = TestBudget::new();
    // After its start, the pane shows the menu.
    let s = script(&["up", "menu"], &[]);
    // Its words, the brief's look; the menu; asking still, Escape; closed; listed for the words; the menu back; the same; at
    // work; its report; gone.
    *s.looks.borrow_mut() = [
        Idle, Asking, Asking, Idle, Idle, Asking, Asking, Idle, Idle, Working, Idle, Gone,
    ]
    .into_iter()
    .collect();
    // Each menu with its transcript's limit row.
    *s.outcomes.borrow_mut() = [limited(), limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    *s.limits.borrow_mut() = [vec![limit("a", 10.0)]].into_iter().collect();
    let mut tasks = Store::default();
    let started = std::time::Instant::now();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert!(started.elapsed() < PACE.stuck_limit, "read at once");
    assert_eq!(finished.outcome, merged());
    let did = s.did();
    assert!(!did.iter().any(|d| d.contains("question")), "{did:?}");
    assert!(did.iter().any(|d| d == "page t1 limited"), "{did:?}");
    let keys: Vec<&String> = did
        .iter()
        .filter(|d| *d == "key Escape" || *d == "send continue")
        .collect();
    assert_eq!(
        keys,
        ["key Escape", "send continue", "key Escape", "send continue"],
        "{did:?}"
    );
    assert_eq!(s.count("judge"), 1, "its report alone");
    assert_eq!(tasks.rows["t1"], ("finished".to_string(), 1), "no retry");
}

/// merge-gate's run sat on second's limit menu from 11:27 on 2026-11-09, flagged as a question by a binary older than the menu's
/// read, through a restart: its run's process outlives one, so a restart ends it, and the next look adopts its session, never
/// closes it. The adopted run reads the menu as the limit and moves the session to the login with room.
#[test]
fn a_run_flagged_as_asking_before_a_restart_is_adopted_and_its_limit_menu_moves_it() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    // Its run gone with the restart, on its first try, its session asking on the menu.
    tasks.left_by_a_crash("t1", Some("p9"), Some("s1"), 1);
    // The look's; the menu; Escape; closed; the stop's look; gone once closed; listed in its new pane; at work; its report; gone.
    let s = script(
        &["menu"],
        &[
            Asking, Asking, Asking, Idle, Idle, Gone, Gone, Idle, Working, Idle, Gone,
        ],
    );
    s.did.borrow_mut().clear();
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    *s.limits.borrow_mut() = [vec![limit("second", 100.0), limit("default", 4.0)]]
        .into_iter()
        .collect();
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(pick, Pick::Adopt("t1".into()));
    assert!(
        typed(&s).is_empty() && s.count("close p9") == 0,
        "{:?}",
        s.did()
    );

    let started = std::time::Instant::now();
    let finished = adopt_task("t1", &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert!(started.elapsed() < PACE.stuck_limit, "read at once");
    assert_eq!(finished.outcome, merged());
    let did = s.did();
    let step = |what: &str| {
        let at = did.iter().position(|d| d == what);
        at.unwrap_or_else(|| panic!("no {what}: {did:?}"))
    };
    let path = [
        "page t1 limited",
        "key Escape",
        "send leave",
        "close p9",
        "run resume s1 of t1 in /work/rust-t1 on default",
        "send continue",
    ];
    let at: Vec<usize> = path.iter().map(|p| step(p)).collect();
    assert!(at.is_sorted(), "{did:?}");
    assert!(!did.iter().any(|d| d.contains("question")), "{did:?}");
    assert!(
        !did.iter().any(|d| d.starts_with("run start")),
        "never a new session"
    );
    assert!(
        !did.iter().any(|d| d.contains("the brief of t1")),
        "no brief again"
    );
    assert_eq!(
        tasks.saved["t1"].1.as_deref(),
        Some("s1"),
        "the same session"
    );
    assert_eq!(tasks.logins, ["default"]);
    assert_eq!(
        tasks.rows["t1"],
        ("finished".to_string(), 2),
        "the restart's try"
    );
}

/// Review of 946a54e3a: a failed PR read proves no PR open, and a merged one's worktree would start over under the session.
/// Neither is adopted: the crash's path as before. Two adoptable claims are adopted one a look, the rest of each look dealt with.
#[test]
fn a_crashed_session_is_adopted_only_on_a_pr_read_that_shows_no_merge_and_one_a_look() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", Some("p9"), Some("s1"), 1);
    let s = script(&[], &[Idle]);
    s.pr_fails.set(true);
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(
        matches!(pick, Pick::Start { ref id, .. } if id == "t1"),
        "{pick:?}"
    );
    assert_eq!(s.count("close p9"), 1);

    let mut tasks = Store::default();
    tasks.left_by_a_crash("t1", Some("p8"), Some("s1"), 1);
    tasks.left_by_a_crash("t2", Some("p9"), Some("s1"), 1);
    let s = script(&[], &[Idle]);
    let first = next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(matches!(first, Pick::Adopt(_)), "{first:?}");
    assert_eq!(s.count("close p8") + s.count("close p9"), 0);
}

/// The same in one run: a question flagged, then the pane shows the limit menu. A flag is no verdict that it is not the menu.
#[test]
fn a_question_flagged_then_the_limit_menu_reads_as_limited() {
    let _budget = TestBudget::new();
    let stuck_limit = Duration::ZERO;
    let pace = Pace {
        stuck_limit,
        ..PACE
    };
    // Its start; a question, read for the menu and flagged; then the menu.
    let s = script(&["up", "question", "question", "menu"], &[]);
    // Its words; the question; the menu; Escape; closed; listed for « continue »; at work; its report; gone.
    *s.looks.borrow_mut() = [
        Idle, Asking, Asking, Asking, Idle, Idle, Working, Idle, Gone,
    ]
    .into_iter()
    .collect();
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    *s.limits.borrow_mut() = [vec![limit("a", 10.0)]].into_iter().collect();
    let mut tasks = Store::default();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), pace).expect("recorded");
    assert_eq!(finished.outcome, merged());
    let did = s.did();
    let flags: Vec<&String> = did.iter().filter(|d| d.starts_with("flag")).collect();
    assert_eq!(flags.len(), 1, "{did:?}");
    assert!(flags[0].contains("on a question; the pane shows: question"));
    let after: Vec<&String> = did
        .iter()
        .skip_while(|d| !d.starts_with("flag"))
        .filter(|d| *d == "page t1 limited" || *d == "key Escape" || *d == "send continue")
        .collect();
    assert_eq!(
        after,
        ["page t1 limited", "key Escape", "send continue"],
        "{did:?}"
    );
}

#[test]
fn a_limited_run_whose_pane_is_gone_waits_flagged_once_with_the_reset_then_resumes_the_same_way() {
    let _budget = TestBudget::new();
    // Its stop, then gone: five looks of its wait, `resume`'s, its stop's two and the one after; listed in its new pane; at work;
    // its report; gone.
    let s = script(
        &["up"],
        &[
            Idle, Gone, Gone, Gone, Gone, Gone, Gone, Gone, Gone, Gone, Idle, Working, Idle, Gone,
        ],
    );
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    // second's reset on 11-09, 2026-11-11 16:00 UTC; the other's a day later.
    let a = Limit {
        week_resets: 1_794_412_800,
        ..limit("a", 100.0)
    };
    let b = Limit {
        week_resets: 1_794_499_200,
        ..limit("b", 100.0)
    };
    let none = vec![a.clone(), b.clone()];
    let room = vec![limit("a", 0.0), b];
    *s.limits.borrow_mut() = [&none, &none, &none, &none, &none, &room]
        .into_iter()
        .cloned()
        .collect();
    let mut tasks = Store::default();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), PACE).expect("recorded");
    assert_eq!(finished.outcome, merged());
    assert_eq!(s.limit_reads.get(), 6, "until a login had room");
    let flags = s
        .did()
        .into_iter()
        .filter(|d| d.contains("no login has room"));
    let flags: Vec<String> = flags.collect();
    assert_eq!(flags.len(), 1, "{flags:?}");
    assert!(flags[0].contains("2026-11-11 16:00 UTC"), "{flags:?}");
    assert_eq!(s.count("key Escape"), 0, "no menu in a pane gone");
    assert_eq!(s.count("close p1"), 0, "a pane gone may be another's");
    assert_eq!(s.count("run resume s1 of t1 in /work/rust-t1 on a"), 1);
    assert_eq!(s.count("send continue"), 1);
    assert_eq!(tasks.logins, ["a", "a"]);
    assert_eq!(tasks.rows["t1"], ("finished".to_string(), 1), "no retry");
}

#[test]
fn failed_limit_readings_are_flagged_once() {
    let _budget = TestBudget::new();
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

/// dash task close on a run Rust holds (rust-task-close-rust-runs): refused while its agent works or its worktree holds work no
/// remote has, nothing touched; else its agent told to leave, its pane closed, its row ended for good and the list told.
#[test]
fn close_ends_a_held_run_and_its_pane_refused_while_it_works_or_holds_unpushed_work() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.running("t1", Some("p1"), Some("s1"));
    let s = script(&[], &[Working]);
    let said = close("t1", "regroup", &mut ports(&mut tasks, &s), PACE).expect_err("works");
    assert!(
        said.contains("refused, its agent works in pane p1"),
        "{said}"
    );
    *s.looks.borrow_mut() = [Idle].into_iter().collect();
    *s.unsaved.borrow_mut() = Some("2 commits on no remote branch".into());
    let said = close("t1", "regroup", &mut ports(&mut tasks, &s), PACE).expect_err("unpushed");
    assert!(
        said.contains("its worktree holds 2 commits on no remote branch: push it first"),
        "{said}"
    );
    *s.unsaved.borrow_mut() = None;
    // Another session in its pane (a /clear's id, or a reused pane id's): it may be this work (/code-review of baa197583).
    *s.looks.borrow_mut() = [Taken].into_iter().collect();
    let said = close("t1", "regroup", &mut ports(&mut tasks, &s), PACE).expect_err("not named");
    assert!(
        said.contains("lists a session its run cannot name"),
        "{said}"
    );
    // A task that waits on it would never be freed by a merge.
    *s.looks.borrow_mut() = [Idle].into_iter().collect();
    tasks.waits.insert("t2".into(), vec!["t1".into()]);
    tasks.rows.insert("t2".into(), ("open".into(), 0));
    let said = close("t1", "regroup", &mut ports(&mut tasks, &s), PACE).expect_err("waited on");
    assert!(said.contains("t2 waits on it"), "{said}");
    tasks.rows.remove("t2");
    // Its launch in flight: no pane yet.
    tasks.running("t3", None, None);
    let said = close("t3", "regroup", &mut ports(&mut tasks, &s), PACE).expect_err("starting");
    assert!(said.contains("opens no pane yet"), "{said}");
    tasks.rows.remove("t3");
    assert!(
        s.did().is_empty(),
        "a refusal sends, closes and flags nothing: {:?}",
        s.did()
    );
    assert_eq!(tasks.status("t1"), Ok(Some("claimed".into())));
    assert!(tasks.locks.contains("t1"), "its run still holds it");

    // Idle when looked at, still there when told to leave, then gone.
    *s.looks.borrow_mut() = [Idle, Idle, Idle, Gone].into_iter().collect();
    let said = close("t1", "regroup", &mut ports(&mut tasks, &s), PACE).expect("closed");
    assert_eq!(said, "t1: its run ended, its pane p1 closed");
    assert_eq!(
        s.did(),
        [
            "send leave",
            "key Enter",
            "close p1",
            "flag t1: closed by dash task close: regroup",
            "clean t1",
        ]
    );
    assert_eq!(tasks.status("t1"), Ok(Some("finished".into())));
    let run = tasks.run.clone().expect("recorded");
    assert_eq!(
        run.outcome,
        Outcome::Ended("closed by dash task close: regroup".into())
    );
    assert_eq!(tasks.untold, None, "the list was told");
    // Nothing left for a look to free and retry.
    assert!(tasks.crashed().expect("claims").is_empty());

    let said = close("t1", "again", &mut ports(&mut tasks, &s), PACE).expect_err("not held");
    assert_eq!(said, format!("t1: {NOT_HELD}"));
}

/// Task rust-pr-one-owner, done-when 1: a PR has one holder. The orchestrator holds #7 for a fix of its own: the hand-back pass
/// skips it, flagged, nothing reopened. Released, it is handed back and its run holds it; the orchestrator may still take it while
/// that run waits on a seat (@nha, 2026-11-10: « urgent tasks may require that »), and the pick then starts no session beside it.
/// A run at work keeps its own: the orchestrator is refused.
#[test]
fn a_held_pr_is_not_handed_back_and_a_hand_back_s_pick_waits_on_another_s_hold() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let mut s = script(&["up"], &[Idle, Gone]);
    s.earlier.set(true);
    *s.bases.borrow_mut() = [Base::Conflicts(vec!["a.rs".into()])].into_iter().collect();
    tasks
        .hold_pr(7, ORCHESTRATOR, "an urgent fix")
        .expect("taken");
    let handed = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    assert!(handed.is_empty(), "{handed:?}");
    assert_eq!(s.count("reopen t1"), 0);
    let held = "flag t1: not handed back: #7 is held by orchestrator: an urgent fix";
    assert!(s.did().contains(&held.to_string()), "{:?}", s.did());

    tasks.unhold(7).expect("released");
    let handed = hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    assert_eq!(handed.len(), 1, "free: handed back");
    assert_eq!(tasks.holds[&7].holder, "run t1", "its run holds it");

    // Its task reopened, its run not started: the orchestrator takes it, and the pick waits.
    s.reopened_ms = Some(1);
    let _ = next(&mut ports(&mut tasks, &s));
    tasks.rows.insert("t1".into(), ("open".into(), 0));
    let took = tasks.hold_pr(7, ORCHESTRATOR, "pushing the fix itself");
    assert_eq!(took.expect("taken").expect("from its run").holder, "run t1");
    s.queue = vec!["t1", "t2"];
    let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
    let t2 = Pick::Start {
        id: "t2".into(),
        account: None,
        pool: None,
    };
    assert_eq!(pick, t2, "t1 not started beside the orchestrator");
    let skipped =
        "flag t1: handed back, not started: #7 is held by orchestrator: pushing the fix itself";
    assert!(s.did().contains(&skipped.to_string()), "{:?}", s.did());

    // A run at work keeps its hold.
    tasks.holds.insert(
        7,
        Hold {
            pr: 7,
            holder: "run t1".into(),
            why: "w".into(),
            touched_ms: 0,
        },
    );
    tasks.rows.insert("t1".into(), ("claimed".into(), 1));
    let refused = tasks
        .hold_pr(7, ORCHESTRATOR, "late")
        .expect_err("its run works");
    assert_eq!(refused, "#7 is held by run t1: w");
}

/// Task rust-pr-one-owner, done-when 2: a run's hold ends with its run, a merge included; a merge judged again ends anyone's.
#[test]
fn a_run_s_hold_ends_at_its_end_and_a_merge_ends_any_hold() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    let s = script(&["up"], &[Idle, Gone]);
    s.earlier.set(true);
    let ran = run_with(&mut tasks, &s, PACE);
    assert_eq!(ran.outcome, merged());
    assert!(tasks.holds.is_empty(), "{:?}", tasks.holds);

    let mut tasks = Store::default();
    ended(&mut tasks, None);
    let s = script(&[], &[]);
    *s.pr.borrow_mut() = Some(pr(true));
    tasks.hold_pr(7, ORCHESTRATOR, "landing it").expect("taken");
    hand_backs(&mut ports(&mut tasks, &s)).expect("a pass");
    assert!(
        tasks.holds.is_empty(),
        "the merge ends it: {:?}",
        tasks.holds
    );
}

/// Task rust-pr-one-owner, done-when 3: each run's end says whether it pushed a commit of its own. One handed back at abc that
/// pushed nothing while its PR moved to def met another's fix: a duplicate, its tokens kept. One that pushed is none; one whose
/// push could not be read is no duplicate either.
#[test]
fn a_run_that_only_met_another_s_fix_is_recorded_a_duplicate() {
    let _budget = TestBudget::new();
    let handed_then_run = |own: Option<bool>| {
        let mut tasks = Store::default();
        ended(&mut tasks, None);
        let mut s = script(&["up"], &[Idle, Gone]);
        s.earlier.set(true);
        *s.bases.borrow_mut() = [Base::Conflicts(vec!["a.rs".into()])].into_iter().collect();
        assert_eq!(
            hand_backs(&mut ports(&mut tasks, &s))
                .expect("a pass")
                .len(),
            1
        );
        s.reopened_ms = Some(1);
        *s.checks.borrow_mut() = [Ok(Checks::Red("test".into())), Ok(Checks::Green)]
            .into_iter()
            .collect();
        let pick = next(&mut ports(&mut tasks, &s)).expect("a look");
        assert!(matches!(pick, Pick::Start { .. }), "{pick:?}");
        // someone else's fix lands on its PR while it works
        s.pr.borrow_mut().as_mut().expect("a PR").head = "def".into();
        *s.bases.borrow_mut() = [Base::Holds].into_iter().collect();
        s.own.set(own);
        run_with(&mut tasks, &s, PACE);
        tasks.ends.last().cloned().expect("its end")
    };
    let (id, end) = handed_then_run(Some(false));
    assert_eq!(id, "t1");
    let dup = End {
        pushed: Some(false),
        duplicate: true,
        tokens: Some(1_200),
        nudges: 0,
    };
    assert_eq!(end, dup);
    let own = handed_then_run(Some(true)).1;
    assert_eq!((own.pushed, own.duplicate), (Some(true), false));
    let unread = handed_then_run(None).1;
    assert_eq!((unread.pushed, unread.duplicate), (None, false));
}

/// Task rust-clean-closed-builds, done-when 1: each look cleans the build of a worktree whose task the list holds done or dropped,
/// and only that: an open task keeps its build for its next run, a claimed one is its run's to clean, and none is asked of the list
/// when no worktree holds a build.
#[test]
fn each_look_cleans_the_build_of_a_worktree_whose_task_the_list_closed() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.running("t2", Some("p2"), Some("s2"));
    let mut s = script(&[], &[Idle]);
    s.queue = vec![];
    s.built = vec!["t1", "t2", "t3", "t4"];
    s.closed = vec!["t2", "t3", "t4"];
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(s.did(), ["closed? t1 t3 t4", "clean t3"], "one a look");
    // A build that will not go is flagged, and the next one goes in the same look.
    s.did.borrow_mut().clear();
    s.clean_fails = vec!["t3"];
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(
        s.did(),
        [
            "closed? t1 t3 t4",
            "clean t3",
            "flag t3: closed, but its build stays: EACCES",
            "clean t4"
        ]
    );
    s.did.borrow_mut().clear();
    s.built = vec![];
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert!(s.did().is_empty(), "{:?}", s.did());
}

/// Task rust-clean-closed-builds, done-when 2: `dash land` closes as a run closes its own on a merge. PR 1350's second session
/// pushed after the merge: its agent is told to leave though it works, its pane closed, its build gone, and its claim kept for
/// its run, which reads the merge as its end. A task no run of Rust's holds loses its build alone.
#[test]
fn dash_land_stops_the_task_s_pane_and_cleans_its_build() {
    let _budget = TestBudget::new();
    let mut tasks = Store::default();
    tasks.running("t1", Some("p1"), Some("s1"));
    let s = script(&[], &[Working, Working, Gone]);
    let said = landed("t1", &mut ports(&mut tasks, &s), PACE).expect("closed");
    assert_eq!(
        said,
        "no agent of its run left in pane p1, its build output gone"
    );
    assert_eq!(s.did(), ["send leave", "key Enter", "close p1", "clean t1"]);
    assert_eq!(tasks.status("t1"), Ok(Some("claimed".into())));

    s.did.borrow_mut().clear();
    let said = landed("t9", &mut ports(&mut tasks, &s), PACE).expect("cleaned");
    assert_eq!(said, "no pane of Rust's runs it, its build output gone");
    assert_eq!(s.did(), ["clean t9"]);
}

/// A pane recorded from herdr on 2026-11-11: an orchestrator done with its turn (herdr's « done ») while its wake-wait shell runs.
const SHELL_PANE: &str = include_str!("panes/done-with-shell.txt");

/// The same pane once its shell ended: the counts gone from its footer.
fn shell_ended() -> &'static str {
    let ended = SHELL_PANE
        .replace(" · 1 shell still running", "")
        .replace(" · 1 shell", "");
    ended.leak()
}

/// Task rust-wait-background-shell, done-when 1: a stop whose pane shows its background shell running (a push, a test) waits on
/// it, no judge, no nudge, no stuck limit, so no reopen; the shell ended, its report is judged once. Before, it was judged at
/// once and nudged « not yet » on a report its shell had not finished proving.
#[test]
fn a_stop_on_its_running_background_shell_waits_for_it_never_nudged_nor_reopened() {
    let _budget = TestBudget::new();
    let s = script(
        &["up", SHELL_PANE, SHELL_PANE, SHELL_PANE, shell_ended()],
        &[Idle, Idle, Idle, Idle, Gone],
    );
    let mut tasks = Store::default();
    // No stuck limit to spare it: a stop held on its shell is no wait on anyone.
    let pace = Pace {
        stuck_limit: Duration::ZERO,
        ..PACE
    };
    let finished = run_with(&mut tasks, &s, pace);
    assert_eq!(finished.outcome, merged(), "{:?}", s.did());
    // Its one judge came after each look at the shell running: only the pane with its shell ended is left unread.
    assert_eq!(
        *s.screens.borrow(),
        [shell_ended()],
        "judged while its shell ran"
    );
    assert_eq!(
        s.count("judge"),
        1,
        "judged once its shell ended: {:?}",
        s.did()
    );
    let nudged = s
        .did()
        .into_iter()
        .filter(|d| d.starts_with("send not yet"))
        .count();
    assert_eq!(nudged, 0, "no nudge while its shell ran: {:?}", s.did());
    assert!(
        !s.did().iter().any(|d| d.starts_with("reopen")),
        "{:?}",
        s.did()
    );
    assert_eq!(
        tasks.ends.last().expect("its end").1.nudges,
        0,
        "its end row counts none"
    );
}

/// The same pane past `shell_cap`: a forgotten shell hides no stop, which is judged and nudged as before, the nudge on its end row.
#[test]
fn a_shell_running_past_its_cap_hides_no_stop_and_its_nudge_is_counted() {
    let _budget = TestBudget::new();
    let s = script(&["up", SHELL_PANE], &[Idle, Idle, Working, Idle, Gone]);
    *s.verdicts.borrow_mut() = [Ok(Verdict::NotMet("part 2".into())), Ok(Verdict::Met)]
        .into_iter()
        .collect();
    let mut tasks = Store::default();
    let pace = Pace {
        shell_cap: Duration::ZERO,
        ..PACE
    };
    assert_eq!(run_with(&mut tasks, &s, pace).outcome, merged());
    assert_eq!(s.count("send not yet: part 2"), 1, "{:?}", s.did());
    assert_eq!(tasks.ends.last().expect("its end").1.nudges, 1);
}

/// Task rust-wait-background-shell, done-when 2: `PM.at_work?/1`'s rule (task busy-one-check's four cases), ported: working,
/// a shell under the cap, a shell past it, idle.
#[test]
fn at_work_is_working_or_a_shell_under_its_cap() {
    let _budget = TestBudget::new();
    let seen = |idle, asking| Seen {
        idle,
        asking,
        session: None,
    };
    let cap = Duration::from_secs(2 * 3600);
    assert!(at_work(&seen(false, false), None, cap), "working");
    assert!(
        at_work(&seen(true, false), Some(Duration::from_secs(60)), cap),
        "a shell under the cap"
    );
    assert!(
        !at_work(&seen(true, false), Some(cap), cap),
        "a shell past the cap"
    );
    assert!(!at_work(&seen(true, false), None, cap), "idle");
    assert!(
        !at_work(&seen(false, true), None, cap),
        "a question waits on someone"
    );
}

/// `PM.shells_footer?/1`'s cases, ported: the recorded pane and the footers the older app's tests hold.
#[test]
fn a_footer_shows_a_running_shell_monitor_or_agent_and_prose_shows_none() {
    let _budget = TestBudget::new();
    assert!(shells_running(SHELL_PANE), "the recorded pane");
    assert!(!shells_running(shell_ended()), "its shell ended");
    for running in [
        "✻ Baked for 5s · 1 shell still running\n❯ \n  ⏵⏵ bypass permissions on · 1 shell\n",
        "❯ \n  ⏵⏵ bypass permissions on · 1 shell · ↓ to manage\n",
        "❯ \n  ⏵⏵ bypass permissions on · 2 shells, 1 monitor\n",
        "✻ Waiting for 1 background agent to finish\n❯ \n",
        "· 2 background tasks still running\n❯ \n",
        "❯ \n  ⏵⏵ bypass permissions on · 1 local agent\n",
        "✻ Baked (1 shell still running)\n❯ \n",
    ] {
        assert!(shells_running(running), "{running}");
    }
    let quoted = format!(
        "review: it printed 1 shell still running\n{}❯ \n",
        "line\n".repeat(12)
    );
    for none in [
        "⏺ the 3 agents ran · 3 agents\n❯ \n  ⏵⏵ bypass permissions on\n",
        "❯ \n  ⏵⏵ bypass permissions on · 3 agents ran\n",
        "❯ \n  ⏵⏵ bypass permissions on\n",
        quoted.as_str(),
    ] {
        assert!(!shells_running(none), "{none}");
    }
}

/// A limit hit while a background shell runs is that limit's stop, never held on the shell: it waits for room and is told to
/// continue as any limit (/code-review of this task).
#[test]
fn a_limit_hit_while_its_shell_runs_is_the_limit_s_stop() {
    let _budget = TestBudget::new();
    // Every look shows its shell: only its transcript's limit moves it, then its time does.
    let s = script(
        &["up", SHELL_PANE],
        &[Idle, Idle, Idle, Idle, Working, Idle],
    );
    *s.outcomes.borrow_mut() = [limited(), Outcome::Done(DONE.into())]
        .into_iter()
        .collect();
    let full = vec![limit("a", 100.0), limit("b", 100.0)];
    let room = vec![limit("a", 10.0), limit("b", 100.0)];
    *s.limits.borrow_mut() = [full, room].into_iter().collect();
    let pace = Pace {
        limit: Duration::from_secs(1),
        shell_cap: Duration::from_secs(3600),
        ..PACE
    };
    let mut tasks = Store::default();
    let finished = run_task("t1", Some("a"), &mut ports(&mut tasks, &s), pace).expect("recorded");
    assert_eq!(finished.outcome, merged(), "{:?}", s.did());
    assert_eq!(s.count("send continue"), 1, "{:?}", s.did());
}

/// Task rust-disk-caps, done-when 1 and 2: with no closed build to clean, a look takes the repository's own build past 15 GB, else
/// an open task's no run claims past 5 GB, one a look; a cargo running in one keeps it, and the next goes. A claimed task's build,
/// whatever its size, is its run's.
#[test]
fn each_look_caps_the_repository_s_own_build_and_an_unclaimed_open_task_s() {
    let _budget = TestBudget::new();
    const GB: u64 = 1 << 30;
    let mut tasks = Store::default();
    tasks.running("t2", Some("p2"), Some("s2"));
    let mut s = script(&[], &[Idle]);
    s.queue = vec![];
    s.built = vec!["t1", "t2", "t3", "t4"];
    s.sizes = vec![
        ("own", 16 * GB),
        ("t1", 6 * GB),
        ("t2", 40 * GB),
        ("t3", 4 * GB),
        ("t4", 7 * GB),
    ];
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(
        s.did(),
        ["closed? t1 t3 t4", "trim own"],
        "the own build first, alone"
    );

    s.did.borrow_mut().clear();
    s.sizes[0].1 = 15 * GB;
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(
        s.did(),
        ["closed? t1 t3 t4", "trim t1"],
        "at 15 GB it stays; t1 past 5 goes"
    );

    s.did.borrow_mut().clear();
    (s.sizes[0].1, s.cargo_in) = (20 * GB, vec!["own", "t1"]);
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(
        s.did(),
        [
            "closed? t1 t3 t4",
            "flag t1: past 5 GB, its build stays: a cargo runs in t1",
            "trim t4"
        ],
        "a cargo keeps own and t1, t1 flagged; t3 is under, t2 claimed"
    );

    s.did.borrow_mut().clear();
    s.closed = vec!["t3"];
    next(&mut ports(&mut tasks, &s)).expect("a look");
    assert_eq!(
        s.did(),
        ["closed? t1 t3 t4", "clean t3"],
        "a closed build goes first, whatever its size"
    );
}
