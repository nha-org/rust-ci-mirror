//! Section 20b's ports for a task's run, as thin as phase 2 needs. Each tool sits in an adapter crate; crates/gate fails if
//! this file names one.

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub folder: String,
    pub prompt: String,
}

/// How a run ended, with the words that say so. `Done` and `Blocked` are what a session said at the end of a turn: judged,
/// never final. `Limited`: a turn its account's limit ended: no word of the session's, so never judged. `Merged`: its PR's
/// link. `Ended`: given up on, its reason. `Failed`: the run itself failed.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done(String),
    Blocked(String),
    Limited(String),
    Ended(String),
    Failed(String),
    Merged(String),
}

impl Outcome {
    /// Its kind as a word, and what it said.
    pub fn parts(&self) -> (&'static str, &str) {
        match self {
            Outcome::Done(said) => ("done", said),
            Outcome::Blocked(said) => ("blocked", said),
            Outcome::Limited(said) => ("limited", said),
            Outcome::Ended(said) => ("ended", said),
            Outcome::Failed(said) => ("failed", said),
            Outcome::Merged(said) => ("merged", said),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub pane: Option<String>,
    pub session: Option<String>,
    pub outcome: Outcome,
}

/// A claimed row. `pane` is what it opened, `session` what it saw there: a terminal reuses pane ids, so only the two together
/// name this run's session.
#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub id: String,
    pub pane: Option<String>,
    pub session: Option<String>,
    pub attempts: u32,
}

/// This orchestrator's runs. It never starts anything.
pub trait TaskStore {
    /// Takes a task no run holds, atomically: of two claims, one fails. How many claims it has had, this one included.
    fn claim(&mut self, id: &str) -> Result<u32, String>;
    /// The pane a claim opened, written at once: a run that crashes leaves it to close.
    fn opened(&mut self, id: &str, pane: &str) -> Result<(), String>;
    /// The session that pane first listed, written at once: what a later look must find there before anything acts on the pane.
    fn adopted(&mut self, id: &str, session: &str) -> Result<(), String>;
    fn record(&mut self, id: &str, run: &Run) -> Result<(), String>;
    /// The run failed and may try again: recorded, its row open.
    fn release(&mut self, id: &str, run: &Run) -> Result<(), String>;
    /// Every claim whose run no longer lives, its row still claimed: a session it left may still work in its worktree.
    fn crashed(&self) -> Result<Vec<Claim>, String>;
    /// A crashed claim's row open again, once nothing of its run is left. False: a run holds it, or it is no longer claimed.
    fn free(&mut self, id: &str) -> Result<bool, String>;
    /// Each claim: a run still starting holds a seat no terminal lists yet.
    fn claims(&self) -> Result<Vec<Claim>, String>;
    /// The list opened this task again at `at_ms` (Unix milliseconds): a row that finished before then starts over, its tries
    /// and its last run forgotten. Anything else stays as it is.
    fn reopened(&mut self, id: &str, at_ms: i64) -> Result<(), String>;
    /// Its row's status, `None` with no row: only an open row or none can be picked.
    fn status(&self, id: &str) -> Result<Option<String>, String>;
    /// How its finished run ended, while the list has not been told of it.
    fn untold(&self, id: &str) -> Result<Option<Outcome>, String>;
    fn told(&mut self, id: &str) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Queued {
    pub id: String,
    pub title: String,
    /// When the list last opened it again after it was done (Unix milliseconds), if it ever did.
    pub reopened_ms: Option<i64>,
}

/// What may be picked, and the sessions he allows at once.
#[derive(Clone, Debug, PartialEq)]
pub struct Queue {
    pub seats: usize,
    pub tasks: Vec<Queued>,
}

/// A judge's word on a task's report.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Met,
    NotMet(String),
    Blocked(String),
    /// Not this orchestrator's any more: handed back or gone. No later look changes it.
    NotOurs(String),
}

/// The list of tasks and the judge, both the older app's until a stream moves (section 20b's bridge).
pub trait TaskList {
    fn queue(&self) -> Result<Queue, String>;
    /// The words a session starts from.
    fn brief(&self, id: &str) -> Result<String, String>;
    /// The verdict on its newest report, else on what its session last `said`. Err: no answer yet, the same is read again.
    fn judge(&self, id: &str, said: &str) -> Result<Verdict, String>;
    fn merged(&self, id: &str, pr: &str) -> Result<(), String>;
    /// A run over without a merge: flagged, and its open PR is the list's to drive from here.
    fn ended(&self, id: &str, why: &str) -> Result<(), String>;
    /// Tells whoever coordinates of what nothing here will move.
    fn flag(&self, id: &str, what: &str) -> Result<(), String>;
}

/// An agent the terminal sees in a pane. `asking`: waiting on a question it put. Neither that nor `idle`: working.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub idle: bool,
    pub asking: bool,
    pub session: Option<String>,
}

/// Panes and what runs in them. Nothing here takes his focus.
pub trait Terminal {
    /// A new pane in `folder`: its id.
    fn open(&self, folder: &str, label: &str) -> Result<String, String>;
    /// A command line, as typed at the pane's shell.
    fn run(&self, pane: &str, line: &str) -> Result<(), String>;
    fn agent(&self, pane: &str) -> Result<Option<Seen>, String>;
    /// The sessions open, whoever started them: what the seats count.
    fn sessions(&self) -> Result<usize, String>;
    fn read(&self, pane: &str) -> Result<String, String>;
    /// Text, arriving whole however long.
    fn send(&self, pane: &str, text: &str) -> Result<(), String>;
    fn key(&self, pane: &str, key: &str) -> Result<(), String>;
    fn close(&self, pane: &str) -> Result<(), String>;
}

/// The command that starts this agent, sandboxed, in the task's folder, on `account` when one is named.
pub trait Launcher {
    fn command(&self, task: &Task, account: Option<&str>) -> Result<String, String>;
}

/// What a pane's screen means while an agent starts. `Keys` answer a start dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    Starting,
    Ready,
    Keys(Vec<String>),
}

/// One account's spend, in percent of each window, and when its week resets (Unix seconds). A window with no reading is 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub account: String,
    pub five_hours: f64,
    pub week: f64,
    pub week_resets: u64,
}

impl Limit {
    /// Both windows below 100%: he spends an account to its end, so only that stops a start or holds a session.
    pub fn room(&self) -> bool {
        self.five_hours < 100.0 && self.week < 100.0
    }
}

/// An agent kind: all the core knows of one.
pub trait Agent {
    fn screen(&self, shown: &str) -> Screen;
    /// How `session` ended, once it ended a turn; `None` before.
    fn outcome(&self, folder: &str, session: &str) -> Result<Option<Outcome>, String>;
    fn stop_words(&self) -> String;
    /// Each account's limits, as last read. None known means nothing says stop.
    fn limits(&self) -> Result<Vec<Limit>, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pr {
    pub number: u64,
    pub url: String,
    pub head: String,
    pub merged: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Checks {
    Green,
    Red(String),
    Pending,
}

/// A task's worktree and its pull request.
pub trait Forge {
    /// Its worktree's real path, made once, kept across a retry.
    fn worktree(&self, id: &str) -> Result<String, String>;
    /// Its branch's newest pull request, of any state.
    fn pr(&self, id: &str) -> Result<Option<Pr>, String>;
    fn checks(&self, pr: &Pr) -> Result<Checks, String>;
    /// The PR's files its base branch changed since its CI ran: a refusal to merge. Err: no verdict.
    fn stale(&self, pr: &Pr) -> Result<Vec<String>, String>;
    /// Merges exactly `pr.head`. Err: refused, or no answer, which only a later look tells apart.
    fn merge(&self, pr: &Pr) -> Result<(), String>;
    /// Its worktree gone, once its PR is merged.
    fn leave(&self, id: &str) -> Result<(), String>;
}

/// All a pick reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub queue: Queue,
    pub sessions: usize,
    pub limits: Vec<Limit>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Start { id: String, account: Option<String> },
    Wait(String),
}

/// What follows a verdict. `Land`: to its PR. `Nudge`: these words to the session. `Keep`: watch on, it may still move.
/// `GiveUp`: end the run, saying why.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Land,
    Nudge(String),
    Keep,
    GiveUp(String),
}

/// The decisions, pure: a snapshot in, a decision out.
pub trait Policy {
    fn pick(&self, snapshot: &Snapshot) -> Pick;
    /// `nudges`: how many this run has sent. `past_limit`: the session did not stop, its time did, so it is working.
    fn after(&self, verdict: &Verdict, nudges: u32, past_limit: bool) -> Step;
    /// Whether a run that failed after `attempts` claims tries again.
    fn retry(&self, attempts: u32) -> bool;
}
