//! Section 20b's ports for a task's run, as thin as phase 2 needs. Each tool sits in an adapter crate; crates/gate fails if
//! this file names one.

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub folder: String,
    pub prompt: String,
}

/// How a run ended, with the words that say so. `Done` and `Blocked` are what a session said at the end of a turn: judged,
/// never final. `Limited`: a turn its account's limit ended, `Offline`: one the API's absence ended: no word of the session's,
/// so never judged. `Merged`: its PR's link. `Ended`: given up on, its reason. `Failed`: the run itself failed.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done(String),
    Blocked(String),
    Limited(String),
    Offline(String),
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
            Outcome::Offline(said) => ("offline", said),
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
    /// What this run's session waits on (`Limited`, `Offline`) for a page to read, `None` once it is told to go on.
    fn waiting(&mut self, id: &str, on: Option<&Outcome>) -> Result<(), String>;
    /// The run failed and may try again: recorded, its row open.
    fn release(&mut self, id: &str, run: &Run) -> Result<(), String>;
    /// Every claim whose run no longer lives, its row still claimed: a session it left may still work in its worktree.
    fn crashed(&self) -> Result<Vec<Claim>, String>;
    /// A crashed claim's row open again, once nothing of its run is left. False: a run holds it, or it is no longer claimed.
    fn free(&mut self, id: &str) -> Result<bool, String>;
    /// Each claim: a run still starting holds a seat no terminal lists yet.
    fn claims(&self) -> Result<Vec<Claim>, String>;
    /// The list opened this task again at `at_ms` (Unix milliseconds): a row that finished before then starts over, its tries
    /// and its last run forgotten. Anything else stays as it is. True: the reopen is held, the cap keeps it finished for now, so
    /// what its old run said is no longer the list's to hear.
    fn reopen_held(&mut self, id: &str, at_ms: i64) -> Result<bool, String>;
    /// The PR head its task was last handed back at: a head no session moved since is not handed back again.
    fn handed(&mut self, id: &str, head: &str) -> Result<(), String>;
    /// An ended run's PR that merged later, judged again: its head, then the verdict in words, as `dash task show` prints it. A
    /// head judged already is not judged again, a reopen included.
    fn rejudged(&mut self, id: &str, words: &str) -> Result<(), String>;
    /// Its row's status, `None` with no row: only an open row or none can be picked.
    fn status(&self, id: &str) -> Result<Option<String>, String>;
    /// How its finished run ended, while the list has not been told of it.
    fn untold(&self, id: &str) -> Result<Option<Outcome>, String>;
    fn told(&mut self, id: &str) -> Result<(), String>;
    /// Every row, as a plan reads it.
    fn standing(&self) -> Result<Vec<Standing>, String>;
    /// What may be picked and his seats, as the last sync of the list wrote them here: a pick asks the list nothing.
    fn queue(&self) -> Result<Queue, String>;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queued {
    pub id: String,
    pub title: String,
    /// When the list last opened it again after it was done (Unix milliseconds), if it ever did.
    pub reopened_ms: Option<i64>,
    /// The tasks the list says it waits on, those it lists as done or dropped left out: met once each is merged here too.
    pub waits: Vec<String>,
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
    /// The words a session starts from.
    fn brief(&self, id: &str) -> Result<String, String>;
    /// The verdict on its newest report, else on what its session last `said`. Err: no answer yet, the same is read again.
    fn judge(&self, id: &str, said: &str) -> Result<Verdict, String>;
    fn merged(&self, id: &str, pr: &str) -> Result<(), String>;
    /// A PR that merged after its run ended, judged met again: the same close, its note saying so.
    fn merged_later(&self, id: &str, pr: &str) -> Result<(), String> {
        self.merged(id, pr)
    }
    /// A run over without a merge: flagged, and its open PR is the list's to drive from here.
    fn ended(&self, id: &str, why: &str) -> Result<(), String>;
    /// Opens it again, as `dash task reopen` does: its next sync starts its row over (`TaskStore::reopen_held`).
    fn reopen(&self, id: &str) -> Result<(), String>;
    /// Tells whoever coordinates of what nothing here will move.
    fn flag(&self, id: &str, what: &str) -> Result<(), String>;
    /// What this run's session waits on (`Limited`, `Offline`), for the task's page; `None` once it is told to go on.
    fn waiting(&self, id: &str, on: Option<&Outcome>) -> Result<(), String>;
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

/// One account's spend, in percent of each window, and when each resets (Unix seconds). A window with no reading is 0, its
/// reset too: a page tells « no reading yet » from 0 % by that reset alone.
/// `key`: an API key's row, its `week` our count of its runs' spend against its cap, which no reset brings back: a headless run's,
/// never a pane's (the orchestrator's answer in rust-key-agent's question dialog, 2026-11-08: « A: headless run »).
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub account: String,
    pub five_hours: f64,
    pub five_hours_resets: u64,
    pub week: f64,
    pub week_resets: u64,
    pub key: bool,
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
    /// Whether what this agent talks to answers: an `Offline` stop waits for it.
    fn reachable(&self) -> bool;
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

/// A PR's head against its base branch's newest commit. Only `Holds` lands: CI built the head, so it built what main becomes. Two
/// PRs green apart broke main together when each was only checked for files its base changed (E0425, 84c04e6ec and 409a1c0e9).
#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    Holds,
    /// Main has commits the head lacks where the build reads, merging cleanly.
    Behind,
    /// The files a merge of main into the head conflicts in.
    Conflicts(Vec<String>),
}

/// A task's worktree and its pull request.
pub trait Forge {
    /// Its worktree's real path, made once, kept across a retry.
    fn worktree(&self, id: &str) -> Result<String, String>;
    /// Its branch's newest pull request, of any state.
    fn pr(&self, id: &str) -> Result<Option<Pr>, String>;
    fn checks(&self, pr: &Pr) -> Result<Checks, String>;
    /// Err: no verdict.
    fn base(&self, pr: &Pr) -> Result<Base, String>;
    /// Merges exactly `pr.head`. Err: refused, or no answer, which only a later look tells apart.
    fn merge(&self, pr: &Pr) -> Result<(), String>;
    /// Its worktree gone, once its PR is merged and its session stopped.
    fn leave(&self, id: &str) -> Result<(), String>;
    /// After `merged` squash-merged: every open PR stacked on its branch rebased onto main, its base main, so it conflicts with
    /// none of the squash. What each one got, said in words.
    /// `busy`: the branches a session is at work on, left alone.
    fn restack(&self, merged: &Pr, busy: &[String]) -> Result<Vec<String>, String>;
    /// Its build output gone, once its PR is merged, whatever else stays.
    fn clean(&self, id: &str) -> Result<(), String>;
    /// The base branch's newest commit live, after a merge.
    fn deploy(&self) -> Result<(), String>;
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

/// Open tasks Rust's store may hold, those a run holds included: he caps tasks, the seats cap sessions (@nha, 2026-11-06: « we
/// will cap the rust tasks at say 50 for now too »).
pub const OPEN_CAP: usize = 50;

/// A row of Rust's store as a plan and a write read it. `waits`: the tasks it names, met once each is merged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Standing {
    pub id: String,
    pub status: String,
    /// Its claims so far.
    pub attempts: u32,
    /// Its last run's pane and the session it saved there: one a stop left at work may still be in it.
    pub pane: Option<String>,
    pub session: Option<String>,
    /// The PR head it was last handed back at.
    pub handed: Option<String>,
    /// The merged PR head an ended run was judged again at, then the verdict.
    pub rejudged: Option<String>,
    pub outcome: Option<String>,
    pub waits: Vec<String>,
}

fn open_count(rows: &[Standing]) -> usize {
    rows.iter().filter(|r| r.status != "finished").count()
}

fn row<'a>(rows: &'a [Standing], id: &str) -> Option<&'a Standing> {
    rows.iter().find(|r| r.id == id)
}

/// What holds a new row back, once the cap is reached.
pub fn at_cap(rows: &[Standing]) -> Option<String> {
    (open_count(rows) >= OPEN_CAP)
        .then(|| format!("{OPEN_CAP} Rust tasks are open, the cap: one must finish first"))
}

/// The line that refuses one more open task, once the cap is reached.
pub fn over_cap(rows: &[Standing], id: &str) -> Option<String> {
    at_cap(rows).map(|why| format!("{id}: refused, {why}"))
}

/// `id -> a -> id`, when waiting on `waits` would close a loop. It follows what each row names, and a loop a merge of two replicas
/// made elsewhere does not hold it up. A finished row's edges count: a reopen keeps its waits, so a met wait can hold again.
fn cycle(rows: &[Standing], id: &str, waits: &[String]) -> Option<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut paths: Vec<Vec<&str>> = waits.iter().map(|w| vec![id, w.as_str()]).collect();
    while let Some(path) = paths.pop() {
        let last = path[path.len() - 1];
        if last == id {
            return Some(path.join(" -> "));
        }
        if seen.insert(last)
            && let Some(row) = row(rows, last)
        {
            for wait in &row.waits {
                let mut longer = path.clone();
                longer.push(wait);
                paths.push(longer);
            }
        }
    }
    None
}

/// Why `id` may not be written as waiting on `waits`, read under the store's write lock: its row new past the cap, a name no row
/// has, or a loop. A loop would wait for ever, and only the write can still say which edge made it.
pub fn refused(rows: &[Standing], id: &str, waits: &[String]) -> Option<String> {
    let capped = row(rows, id).is_none().then(|| over_cap(rows, id));
    let unknown = waits.iter().find(|w| *w != id && row(rows, w).is_none());
    let unknown = unknown.map(|w| format!("{id}: refused, it names {w}, no such task"));
    let looped = cycle(rows, id, waits).map(|c| format!("{id}: refused, a cycle: {c}"));
    capped.flatten().or(unknown).or(looped)
}

/// A task that does not run now, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub id: String,
    pub why: String,
}

/// What runs next and why the others wait.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub next: Pick,
    pub waiting: Vec<Held>,
}

/// `waits on a (open), b (ended)`, the names not met yet; `None` once each is merged.
/// ponytail: a PR merged outside Rust after its run ended leaves its row `ended` until the hand-back pass judges it again (met or
/// not ours: merged), up to ten looks on; `dash task waits` frees what waits on it meanwhile. A row `add` made that the older
/// app finishes itself stays open the same way, holding a place under the cap: `dash task close` frees it, and a new task held
/// at the cap is flagged.
pub fn waits_unmet(rows: &[Standing], waits: &[String]) -> Option<String> {
    let met = |id: &&String| {
        row(rows, id)
            .is_some_and(|r| r.status == "finished" && r.outcome.as_deref() == Some("merged"))
    };
    let says = |id: &String| {
        // No row here: the older app's task or words, met only once it marks it done there (/code-review of rust-own-queue).
        let state = row(rows, id).map_or("not a Rust task", |r| match r.outcome.as_deref() {
            Some(outcome) if r.status == "finished" => outcome,
            _ => r.status.as_str(),
        });
        format!("{id} ({state})")
    };
    let unmet: Vec<String> = waits.iter().filter(|w| !met(w)).map(says).collect();
    (!unmet.is_empty()).then(|| format!("waits on {}", unmet.join(", ")))
}

/// What `id` waits on: its row's own waits, then the list's. The one merge `plan` and the page read, so they never disagree.
pub fn all_waits(standing: Option<&Standing>, queued: Option<&Queued>) -> Vec<String> {
    let own = standing.into_iter().flat_map(|r| &r.waits);
    let mut all: Vec<String> = vec![];
    for w in own.chain(queued.into_iter().flat_map(|t| &t.waits)) {
        if !all.contains(w) {
            all.push(w.clone());
        }
    }
    all
}

/// The one answer the picker and the page both read, so the picture differs from what gets picked by at most one look: `next`
/// frees crashed claims and applies reopens first, and the page reads what those left. Only what the list offers can start: its
/// own holds stand. A row claimed or finished is no candidate, and what remains goes to `policy`; with only held tasks left, the
/// first one's reason is the answer.
pub fn plan(
    policy: &dyn Policy,
    queue: Queue,
    rows: &[Standing],
    sessions: usize,
    limits: Vec<Limit>,
) -> Plan {
    let (mut ready, mut held) = (vec![], vec![]);
    for task in &queue.tasks {
        let standing = row(rows, &task.id);
        let why = match standing.map(|r| r.status.as_str()) {
            // A claim would add its row.
            None => at_cap(rows).or_else(|| waits_unmet(rows, &task.waits)),
            Some("open") => waits_unmet(rows, &all_waits(standing, Some(task))),
            _ => continue,
        };
        match why {
            Some(why) => held.push(Held {
                id: task.id.clone(),
                why,
            }),
            None => ready.push(task.clone()),
        }
    }
    let all_held = held
        .first()
        .filter(|_| ready.is_empty())
        .map(|first: &Held| Pick::Wait(format!("{}: {}", first.id, first.why)));
    let unlisted = rows
        .iter()
        .filter(|r| r.status == "open" && !queue.tasks.iter().any(|t| t.id == r.id));
    for r in unlisted {
        let why = waits_unmet(rows, &r.waits).unwrap_or("the list does not offer it".into());
        let id = r.id.clone();
        held.push(Held { id, why });
    }
    let ids: Vec<String> = ready.iter().map(|t| t.id.clone()).collect();
    let next = all_held.unwrap_or_else(|| {
        policy.pick(&Snapshot {
            queue: Queue {
                seats: queue.seats,
                tasks: ready,
            },
            sessions,
            limits,
        })
    });
    let behind = ids.iter().enumerate().filter_map(|(at, id)| {
        let why = match &next {
            Pick::Start { id: picked, .. } if picked == id => return None,
            Pick::Start { id: picked, .. } => format!("behind {picked}"),
            Pick::Wait(why) if at == 0 => why.clone(),
            Pick::Wait(_) => format!("behind {}", ids[0]),
        };
        let id = id.clone();
        Some(Held { id, why })
    });
    let waiting = behind.chain(held).collect();
    Plan { next, waiting }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_wait_both_the_row_and_the_list_name_is_counted_once() {
        let _budget = TestBudget::new();
        let queued = Queued {
            id: "b".into(),
            title: String::new(),
            reopened_ms: None,
            waits: vec!["a".into(), "c".into(), "a".into()],
        };
        assert_eq!(all_waits(None, Some(&queued)), ["a", "c"]);
        assert_eq!(
            waits_unmet(&[], &["x".into()]).as_deref(),
            Some("waits on x (not a Rust task)")
        );
    }
}
