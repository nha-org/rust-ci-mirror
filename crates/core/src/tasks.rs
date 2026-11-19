//! Section 20b's ports for a task's run, as thin as phase 2 needs. Each tool sits in an adapter crate; crates/gate fails if
//! this file names one.

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub folder: String,
    pub prompt: String,
    /// The model its pool names (`--model`); `None`: the login's own.
    pub model: Option<String>,
}

/// What a task's brief is written from, as the list files it (task brief-tests-prepush-skip): its row's words, his open
/// comments on it, its Linear key and his overall focuses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filed {
    pub id: String,
    pub title: String,
    pub why: String,
    pub done_when: String,
    /// Its run's prompt.
    pub prompt: String,
    /// Its run's kind, else its own (« writing », « data »), empty for none.
    pub kind: String,
    pub epic: String,
    pub plan_first: bool,
    /// The turns its run may take, `None` for the default.
    pub turns: Option<u32>,
    /// Its Linear issue's key (AAA-117802), `None` before a sync linked one.
    pub linear: Option<String>,
    pub comments: Vec<Comment>,
    /// His overall focuses in order, `(id, title)`.
    pub focus: Vec<(String, String)>,
}

/// One open comment of his on a task: its id, the page it sits on, what he quoted there and his words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comment {
    pub id: String,
    pub file: String,
    pub quote: String,
    pub text: String,
}

/// What the page serves against main's rust/ (`Forge::serving`), in words, its sha first.
#[derive(Clone, Debug, PartialEq)]
pub enum Serving {
    /// The commit the page names holds main's rust/.
    Main(String),
    /// It names one of main's past: a merge no deploy took live, whoever merged it (task rust-deploy-any-merge).
    Behind(String),
    /// It names one main does not hold, a branch deployed by hand: a deploy builds main, the look leaves it.
    Other(String),
    /// It names none, down or slow: no build starts on that guess.
    Unnamed(String),
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

/// A claim of the older app's PM loop (its `task_claims`), as Rust's landed pass reads it (task rust-lane-landed-port): `at` as
/// written, `age_s` since then.
#[derive(Clone, Debug, PartialEq)]
pub struct OlderClaim {
    pub task: String,
    pub project: String,
    pub pane: Option<String>,
    pub at: String,
    pub age_s: i64,
}

/// What the landed pass reads of a claimed task's list row: `runs_here`, the older app starts it (`Tasks.runs_here?/1`);
/// `work_left`, no report since its last reopen or one saying its work goes on (`Reports.work_left?/1`); `past_prs`, the
/// PRs its earlier periods merged, never this run's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LandedRow {
    pub open: bool,
    pub runs_here: bool,
    pub review_of: bool,
    pub work_left: bool,
    pub past_prs: Vec<String>,
}

/// A claimed row. `pane` is what it opened, `session` what it saw there: a terminal reuses pane ids, so only the two together
/// name this run's session.
#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub id: String,
    pub pane: Option<String>,
    pub session: Option<String>,
    pub attempts: u32,
    /// The login its session runs on, as `login` last wrote it.
    pub account: Option<String>,
    /// The model its pool named at its start (RFC 20h), as `model` wrote it: an adopted session resumes on it.
    pub model: Option<String>,
}

/// Who works on a PR now, its task's run or the orchestrator, never both (task rust-pr-one-owner; @nha, 2026-11-10: « How will
/// you fix systematically the wasted effort time and tokens of duplicate work ? »; two reruns found the fix pushed). `touched_ms`:
/// its last take, Unix milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Hold {
    pub pr: u64,
    pub holder: String,
    pub why: String,
    pub touched_ms: i64,
}

/// The orchestrator's name as a holder: `dash pr hold`, `dash land`.
pub const ORCHESTRATOR: &str = "orchestrator";

/// A hold untouched this long ends, a run's while its task is claimed excepted: a holder that forgot it frees it.
pub const HOLD_MS: i64 = 30 * 60 * 1000;

/// The PR lane's name as a holder: `dash review next`'s claim (task rust-review-next-port).
pub const LANE: &str = "review-lane";
/// The lane's hold once it handed a PR over or left it waiting on him: passed over, as the older app's `PrLane.parked`.
pub const LANE_PARKED: &str = "review-lane parked";
/// A lane hold untouched this long ends, the older app's claim length (`PM.claim_hours`, 6): a review outlasts `HOLD_MS`.
pub const LANE_HOLD_MS: i64 = 6 * 3600 * 1000;

/// A session holding the PR: its name (a task id, `review-<id>`) and the terminal pane it works in.
#[derive(Debug, PartialEq)]
pub struct Holder {
    pub name: String,
    pub pane: Option<String>,
}

/// The sessions holding the PR among Rust's rows and holds: a claimed run of the branch's review (`review-<id>`), or one handed
/// back that the app resumes (`handed_back`, task helper-review-claims), and a reviewer's run holding one of `prs`, the PRs the
/// key names. Never the task's own run.
pub fn holders(
    branch: Option<&str>,
    prs: &[u64],
    rows: &[Standing],
    holds: &[Hold],
) -> Vec<Holder> {
    let pane = |id: &str| {
        rows.iter()
            .find(|r| r.id == id)
            .and_then(|r| r.pane.clone())
    };
    let claimed = |id: &str| rows.iter().any(|r| r.id == id && r.status == "claimed");
    let mut out: Vec<Holder> = vec![];
    let mut add = |name: String| {
        if !out.iter().any(|h| h.name == name) {
            let pane = pane(&name);
            out.push(Holder { name, pane });
        }
    };
    if let Some(id) = branch.and_then(|b| b.strip_prefix("task/")) {
        let name = format!("review-{id}");
        if claimed(&name) || handed_back(rows, &name).is_some() {
            add(name);
        }
    }
    for h in holds.iter().filter(|h| prs.contains(&h.pr)) {
        let run = h.holder.strip_prefix("run ");
        if let Some(id) = run.filter(|id| id.starts_with("review-")) {
            add(id.to_string());
        }
    }
    out
}

/// A run's name as a holder.
pub fn run_holder(id: &str) -> String {
    format!("run {id}")
}

/// What a run's end cost (task rust-pr-one-owner): whether it pushed a commit of its own (`None`: unread), whether it only met
/// another's fix (its PR moved while it pushed nothing), its session's tokens (`None`: unread), and the nudges it was sent: what
/// counts the stops judged too soon, as on a running background shell (task rust-wait-background-shell).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct End {
    pub pushed: Option<bool>,
    pub duplicate: bool,
    pub tokens: Option<u64>,
    pub nudges: u32,
}

/// This orchestrator's runs. It never starts anything.
pub trait TaskStore {
    /// Takes a task no run holds, atomically: of two claims, one fails. How many claims it has had, this one included.
    fn claim(&mut self, id: &str) -> Result<u32, String>;
    /// The pane a claim opened, written at once: a run that crashes leaves it to close.
    fn opened(&mut self, id: &str, pane: &str) -> Result<(), String>;
    /// The session that pane first listed, written at once: what a later look must find there before anything acts on the pane.
    fn adopted(&mut self, id: &str, session: &str) -> Result<(), String>;
    /// Its session stopped and its pane closed while its run still drives its PR (a met run waiting on CI): the claim keeps its
    /// session and drops its pane, which `seats_taken` reads as no seat. Nothing else leaves a claim so.
    fn left(&mut self, id: &str) -> Result<(), String>;
    /// The login its session runs on, at its start and at each move a limit made: what `dash task show` names.
    fn login(&mut self, id: &str, account: &str) -> Result<(), String>;
    /// The model its run started on (RFC 20h), kept for an adoption's resume. A store that keeps none forgets it.
    fn model(&mut self, _id: &str, _model: &str) -> Result<(), String> {
        Ok(())
    }
    fn record(&mut self, id: &str, run: &Run) -> Result<(), String>;
    /// What this run's session waits on (`Limited`, `Offline`) for a page to read, `None` once it is told to go on.
    fn waiting(&mut self, id: &str, on: Option<&Outcome>) -> Result<(), String>;
    /// The run failed and may try again: recorded, its row open.
    fn release(&mut self, id: &str, run: &Run) -> Result<(), String>;
    /// A crashed claim taken by a new run that watches its session on (`Pick::Adopt`): its lock taken and kept, its row still
    /// claimed, its pane, session and login kept, one try spent as any crash spends one. Its waits are not read: its session
    /// already works. Err: a run holds it, or it is not claimed.
    fn hold(&mut self, id: &str) -> Result<Claim, String>;
    /// Every claim whose run no longer lives, its row still claimed: a session it left may still work in its worktree.
    fn crashed(&self) -> Result<Vec<Claim>, String>;
    /// A crashed claim's row open again, once nothing of its run is left. False: a run holds it, or it is no longer claimed.
    fn free(&mut self, id: &str) -> Result<bool, String>;
    /// A claim's run ended from outside (`dash task close`): its process stopped and its claim held here, so what this store
    /// records next is how it ended and nothing retries it. Err: not claimed, or its run would not stop.
    fn take_over(&mut self, id: &str) -> Result<(), String>;
    /// Each claim: a run still starting holds a seat no terminal lists yet.
    fn claims(&self) -> Result<Vec<Claim>, String>;
    /// The list opened this task again at `at_ms` (Unix milliseconds): a row that finished before then starts over, its tries
    /// and its last run forgotten. Anything else stays as it is. True: the reopen is held, the cap keeps it finished for now, so
    /// what its old run said is no longer the list's to hear.
    fn reopen_held(&mut self, id: &str, at_ms: i64) -> Result<bool, String>;
    /// Each open row a run the network ended released (`Outcome::Offline`), oldest first, with when (Unix milliseconds): what
    /// waits on the network before any start, and what the wake once it answers names. A store that keeps none has none.
    fn offline(&self) -> Result<Vec<(String, i64)>, String> {
        Ok(vec![])
    }
    /// Each row `dash task reopen` held at the open cap started over while the cap has room, oldest id first: what started.
    fn reopen_waiting(&mut self) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    /// The PR head its task was last handed back at, then the count: a head no session moved since is not handed back again.
    /// Marks the row a live hand-back too (`Standing::back`), until a run on it finishes or its merge is judged again.
    fn handed(&mut self, id: &str, head: &str) -> Result<(), String>;
    /// An ended run's PR that merged later, judged again: its head, then the verdict in words, as `dash task show` prints it. A
    /// head judged already is not judged again, a reopen included.
    fn rejudged(&mut self, id: &str, words: &str) -> Result<(), String>;
    /// What became of main after its merge, as `dash task show` prints it: the sha that serves, the deploy's error, or why none
    /// ran. Written once before the deploy too, so a run that died in it says so.
    fn deployed(&mut self, id: &str, words: &str) -> Result<(), String>;
    /// A line on its page's Activity, waking no one: a step this orchestrator moved itself (a hand-back, a seat freed), where
    /// `TaskList::flag` is kept for what only whoever coordinates moves (task rust-orch-flags-only-stuck).
    fn note(&mut self, id: &str, words: &str) -> Result<(), String>;
    /// `pr` taken for `holder`, or its hold touched. A live hold of another's refuses it (Err names whose), unless the orchestrator
    /// takes it from a run whose task is not claimed (a hand-back not started): Ok names the hold it took over.
    fn hold_pr(&mut self, pr: u64, holder: &str, why: &str) -> Result<Option<Hold>, String>;
    /// Its hold ended: a merge, a release, its run's end.
    fn unhold(&mut self, pr: u64) -> Result<(), String>;
    /// Each live hold: touched within `HOLD_MS`, or a run's whose task is claimed.
    fn holds(&self) -> Result<Vec<Hold>, String>;
    /// A run's end, kept apart from its row, which the next run overwrites: what /watchers counts a day.
    fn run_end(&mut self, id: &str, end: &End) -> Result<(), String>;
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
    /// Why his drag on /tasks holds it, a card he moved to Backlog or In Review (task rust-board-linear-order): no pick starts it.
    pub held: Option<String>,
    /// The model, /design component or view, or RFC section it changes, as `dash task add --layer` filed it (task
    /// rust-task-names-layer): no two runs change one at once.
    pub layer: Option<String>,
    /// Labelled one of `BUILDS_NOTHING`: planning, a draft, a review or a doc, work that compiles nothing, so it may start while
    /// memory is short for a build (RFC 20e rule 4; task rust-orch-pick-dynamic).
    pub builds_nothing: bool,
    /// What `rank` orders it by, each read from the list at each sync: the order itself is never stored.
    pub rank: Rank,
    /// Its epic and labels as the list files them: what /tasks?layout=graph filters by (task rust-graph-why-these).
    pub epic: Option<String>,
    pub labels: Vec<String>,
    /// The kind of work a free model may take, from its labels (RFC 20e's table; the policy names each kind's model); `None`:
    /// seats only.
    pub free: Option<FreeKind>,
    /// What it needs, from its row (RFC 20h): `run.demand`, else hard when its `run.effort` is high.
    pub demand: Demand,
    /// Its demand's route as the store's row sets it, the pools it may spend best first: the pick takes the first with room.
    /// `None`: no row, the policy's own route for its demand. Empty: no pool, the login's own model.
    pub route: Option<Vec<String>>,
    /// The files its title, why and done-when name (`dash_store::similar::files_named`): a start waits while a started task names one
    /// (task task-dedupe).
    pub files: Vec<String>,
}

/// What a task needs (RFC 20h's demand): hard work goes to the best pool while it has room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Demand {
    Hard,
    #[default]
    Normal,
}

impl Demand {
    pub fn name(self) -> &'static str {
        match self {
            Demand::Hard => "hard",
            Demand::Normal => "normal",
        }
    }

    /// A row's word: `hard`, any case, with or without EDN's colon; anything else is normal.
    pub fn of(word: Option<&str>) -> Demand {
        match word.map(|w| w.trim_start_matches(':').to_ascii_lowercase()) {
            Some(w) if w == "hard" => Demand::Hard,
            _ => Demand::Normal,
        }
    }
}

/// The work a free model is good at (@nha, 2026-11-09: « let's use them, throttling needs to be considered and usage too but I
/// suggest we use them for what they are good at. »), as a task's labels name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeKind {
    /// Backlog and planning: sorting, labelling, briefs, RFC reviews, plain-English passes.
    Planning,
    /// Code generation, and anything past 200,000 tokens (logs, the whole backlog).
    Build,
    /// Small scripts and syntax fixes.
    Script,
    /// Bug hunts.
    BugHunt,
}

/// Each kind's label: a build goes free only when labelled so (RFC 20e: free models for specified builds and triage).
pub const FREE_LABELS: [(FreeKind, &str); 4] = [
    (FreeKind::Planning, "Docs"),
    (FreeKind::Build, "Free"),
    (FreeKind::Script, "Script"),
    (FreeKind::BugHunt, "Bug"),
];

impl FreeKind {
    /// The kind its labels name, the first of `FREE_LABELS` that matches, any case; Research is planning too.
    pub fn of(labels: &[String]) -> Option<FreeKind> {
        let has = |l: &str| labels.iter().any(|x| x.eq_ignore_ascii_case(l));
        if has("Research") {
            return Some(FreeKind::Planning);
        }
        FREE_LABELS
            .iter()
            .find(|(_, label)| has(label))
            .map(|row| row.0)
    }

    /// Its label, what a store keeps and `of` reads back.
    pub fn label(self) -> &'static str {
        let row = FREE_LABELS.iter().find(|row| row.0 == self);
        row.expect("every kind has a label").1
    }
}

/// A task's place, computed at each plan, never stored (@nha, 2026-11-10, on a reorder command: « why should it ? should it
/// not be evergreen ? »). `horizon`: its roadmap item's (urgent, now, next, later), `item` that item; `deadline` as `dash task
/// add --deadline` filed it; `priority` 1 (urgent) to 4; `estimate` in points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rank {
    pub horizon: Option<String>,
    pub item: Option<String>,
    pub deadline: Option<String>,
    pub priority: Option<u8>,
    pub estimate: Option<u32>,
    /// What it retires and meets, as `dash task add --impact` counted it (task rust-task-impact): equal priorities go by it.
    pub impact: Option<Impact>,
    /// Its why quotes him (`quotes_him`): at priority 1 it starts beside its layer's running task (`jumps`).
    pub his: bool,
}

/// A why that opens with his words, cited the one way the repo cites him: « (@nha, <date>: « … ») ». Only an opening quote
/// counts: a why citing an older remark of his further on, for context, is ours (/code-review of rust-his-priority-jumps).
pub fn quotes_him(why: &str) -> bool {
    why.trim_start().starts_with("(@nha, ")
}

/// A priority-1 task whose why quotes him passes the one-task-per-layer rule beside its layer's running task (task
/// rust-his-priority-jumps: a tool's task he asked to prioritize waited two days behind rfc:tools' queue of 5; his words are
/// in RFC 20h).
pub fn jumps(task: &Queued) -> bool {
    task.rank.priority == Some(1) && task.rank.his
}

/// The rule a jump passes by, as the plan's order and its start's Activity line say it.
pub fn jumped_words(holder: &str, layer: &str) -> String {
    format!(
        "beside {holder}, at work on {layer}: priority 1 in his words passes the one-task-per-layer rule (task rust-his-priority-jumps)"
    )
}

impl Queue {
    /// The rule `id`'s claim just passed by, when it jumped: priority 1 in his words, another task at work on its layer. What its
    /// run notes on its Activity once claimed, never before, as a row written before its claim slips past the open cap
    /// (/code-review of rust-his-priority-jumps' fixes).
    pub fn jumped_at_claim(&self, rows: &[Standing], id: &str) -> Option<String> {
        let task = self
            .tasks
            .iter()
            .find(|t| t.id == id)
            .filter(|t| jumps(t))?;
        let layer = task.layer.as_deref()?;
        let claimed = |t: &&Queued| row(rows, &t.id).is_some_and(|r| r.status == "claimed");
        let holder = (self.tasks.iter())
            .filter(|t| t.id != id && t.layer.as_deref() == Some(layer))
            .find(claimed)?;
        Some(jumped_words(&holder.id, layer))
    }
}

/// What a Rust task unlocks (task rust-task-impact; @nha, 2026-11-12: « do we need something like "expected impact" or
/// something ? for example /documents allows moving documents to rust. which is a big deal imo. »): the Elixir modules or
/// routes it retires and their lines, counted when it is filed, and the rows of rfc-rust-unified's section 20d it meets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Impact {
    pub retires: Vec<String>,
    pub lines: u32,
    pub rows: Vec<String>,
}

impl Impact {
    /// A 20d row is named by its number (`10`, `3a`, `20b`); a module (`Dashboard.Documents`) or a route (`/documents`) is
    /// retired.
    pub fn is_row(item: &str) -> bool {
        item.starts_with(|c: char| c.is_ascii_digit())
    }

    /// Read back from what `stored` wrote: its lines, then each item a space apart. `none` for a task that retires and meets
    /// nothing.
    pub fn of(text: &str) -> Option<Impact> {
        let mut words = text.split_whitespace();
        let lines = words.next()?.parse().ok()?;
        let (rows, retires) = words.map(String::from).partition(|w| Impact::is_row(w));
        Some(Impact {
            retires,
            lines,
            rows,
        })
    }

    pub fn stored(&self) -> String {
        let items = self.retires.iter().chain(&self.rows);
        std::iter::once(self.lines.to_string())
            .chain(items.cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// What it weighs in the pick: Elixir lines retired, then 20d rows met; `none` weighs nothing, as no impact kept.
    pub fn weight(&self) -> (u32, usize) {
        (self.lines, self.rows.len())
    }

    /// What `dash orch next` and the task's page say.
    pub fn said(&self) -> String {
        if self.retires.is_empty() && self.rows.is_empty() {
            return "none named".into();
        }
        let mut parts = vec![];
        if !self.retires.is_empty() {
            let what = self.retires.join(", ");
            parts.push(format!("retires {} Elixir lines ({what})", self.lines));
        }
        if !self.rows.is_empty() {
            let n = self.rows.len();
            parts.push(format!("meets {n} 20d row(s) ({})", self.rows.join(", ")));
        }
        parts.join(", ")
    }
}

/// The roadmap's horizons, first to last; a task on no item comes after them all.
pub const HORIZONS: [&str; 4] = ["urgent", "now", "next", "later"];

/// The PR head `id` was handed back at, while its row is open on it (task rust-pick-handbacks-first): a fix of work already
/// made, which starts before any new task and holds its layer (`plan`). `None`: no live hand-back.
pub fn handed_back<'a>(rows: &'a [Standing], id: &str) -> Option<&'a str> {
    let r = row(rows, id).filter(|r| r.status == "open" && r.back)?;
    r.handed.as_deref()?.split(' ').next()
}

/// Orders `tasks` hand-backs first (`handed_back`; @nha, 2026-11-09: « It MUST be clear algorithmically at any point in time
/// what is the next best task to pick »: PR 1363, handed back, ranked 22nd), then by roadmap tier, deadline, priority, impact
/// (`Impact`: Elixir lines retired, then 20d rows met), how many
/// of them wait on it (through a chain), then estimate, the id last so two equal tasks keep one order; a task with a deadline
/// is urgent whatever its item (RFC 20e: urgent tasks are time-bound). Returns each task's reason, in that order: what `dash
/// orch next` prints beside its rank.
pub fn rank(tasks: &mut [Queued], rows: &[Standing]) -> Vec<(String, String)> {
    use std::collections::{BTreeMap, BTreeSet};
    // who waits on whom, its row's waits and the list's, as `plan` reads them: id -> the tasks that wait on it
    let mut waiters: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (at, t) in tasks.iter().enumerate() {
        for w in all_waits(row(rows, &t.id), Some(t)) {
            waiters.entry(w).or_default().push(at);
        }
    }
    // every task its merge frees, through the chain: what waits on what waits on it counts too
    let freed = |at: usize| {
        let (mut seen, mut next) = (BTreeSet::from([at]), vec![at]);
        while let Some(one) = next.pop() {
            for &w in waiters.get(&tasks[one].id).into_iter().flatten() {
                if seen.insert(w) {
                    next.push(w);
                }
            }
        }
        seen.remove(&at);
        seen
    };
    // tier, deadline, priority; none sorts after every value: `(is_none, value)`
    let head = |t: &Queued| {
        let tier = match (&t.rank.deadline, &t.rank.horizon) {
            (Some(_), _) => 0,
            (None, Some(h)) => (HORIZONS.iter().position(|x| x == h)).unwrap_or(HORIZONS.len()),
            (None, None) => HORIZONS.len(),
        };
        let deadline = (t.rank.deadline.is_none(), t.rank.deadline.clone());
        // equal priorities by impact: Elixir lines retired first, then 20d rows met; none after any (task rust-task-impact)
        // `--impact none` names nothing: it weighs as a task that has none kept, never above it (/code-review)
        let impact = std::cmp::Reverse(
            t.rank
                .impact
                .as_ref()
                .map(Impact::weight)
                .unwrap_or_default(),
        );
        (
            tier,
            deadline,
            (
                t.rank.priority.is_none(),
                t.rank.priority.unwrap_or_default(),
            ),
            impact,
        )
    };
    // a task ranks at least as high as anything it frees, so an urgent task never waits behind its own low blocker: the
    // source of its head is that task's
    let keyed: Vec<_> = (0..tasks.len())
        .map(|at| {
            let all = freed(at);
            let by = all
                .iter()
                .copied()
                .filter(|&w| head(&tasks[w]) < head(&tasks[at]));
            let source = by
                .min_by_key(|&w| (head(&tasks[w]), tasks[w].id.clone()))
                .unwrap_or(at);
            // a hand-back, or what one waits on: its fix starts first, its blocker with it
            let own = handed_back(rows, &tasks[at].id);
            let lifts = all
                .iter()
                .any(|&w| handed_back(rows, &tasks[w].id).is_some());
            let back = (own, lifts);
            (
                (own.is_none() && !lifts),
                head(&tasks[source]),
                all.len(),
                source,
                at,
                back,
            )
        })
        .collect();
    let mut keyed = keyed;
    let estimate = |t: &Queued| {
        (
            t.rank.estimate.is_none(),
            t.rank.estimate.unwrap_or_default(),
        )
    };
    keyed.sort_by(|(ba, ha, na, _, a, _), (bb, hb, nb, _, b, _)| {
        (
            ba,
            ha,
            std::cmp::Reverse(na),
            estimate(&tasks[*a]),
            &tasks[*a].id,
        )
            .cmp(&(
                bb,
                hb,
                std::cmp::Reverse(nb),
                estimate(&tasks[*b]),
                &tasks[*b].id,
            ))
    });
    let mut why = vec![];
    let mut ranked = vec![];
    for (_, _, n, source, at, (own, lifts)) in keyed {
        let (task, from) = (&tasks[at], &tasks[source]);
        let r = &from.rank;
        let tier = match (&r.deadline, &r.horizon, &r.item) {
            (Some(day), _, _) => format!("urgent, due {day}"),
            (None, Some(h), Some(item)) => format!("{h} ({item})"),
            (None, Some(h), None) => h.clone(),
            (None, None, _) => "on no roadmap item".into(),
        };
        let or = |v: Option<String>| v.unwrap_or_else(|| "none".into());
        let lifted = (source != at).then(|| format!(", as {} waits on it", from.id));
        let back = match own {
            Some(head) => {
                format!(
                    "handed back on its open PR at {head}: a fix of work already made starts first; "
                )
            }
            None if lifts => "a hand-back waits on it, so it starts first; ".into(),
            None => String::new(),
        };
        let reason = format!(
            "{}{tier}, priority {}{}, {n} wait on it, estimate {}, impact {}",
            back,
            or(r.priority.map(|p| p.to_string())),
            lifted.unwrap_or_default(),
            or(task.rank.estimate.map(|e| e.to_string())),
            // the source's, as its order read it: a blocker lifted says the impact it ranks by (/code-review)
            or(r.impact.as_ref().map(Impact::said)),
        );
        why.push((task.id.clone(), reason));
        ranked.push(task.clone());
    }
    tasks.clone_from_slice(&ranked);
    why
}

/// The labels of the list's set whose work builds nothing: a task carrying one is light on memory.
pub const BUILDS_NOTHING: [&str; 2] = ["Docs", "Research"];

/// What may be picked, and the sessions he allows at once.
#[derive(Clone, Debug, PartialEq)]
pub struct Queue {
    pub seats: usize,
    /// What set `seats`, in his words: day or night, the forecast's boost, rust-seats' cap, a drain (task rust-seats-follow-room).
    pub seats_why: String,
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

/// What an update's Err holds when its push had no answer within its limit, its merge undone: five met runs ended on 2026-11-14
/// told to « commit or drop » what was a pre-push hook building dash at load (task hooks-run-built-dash, done-when 3).
pub const PUSH_TIMED_OUT: &str = "its push had no answer";

/// The older app's judge's words when a stop holds neither a report nor a TASK_DONE block (`PM.verdict/4`, frozen by no-elixir).
pub const NO_REPORT: &str = "your report (dash task report";

impl Verdict {
    /// Not met only for want of a report: what a run that ended before its report leaves, whose PR merged after.
    pub fn unreported(&self) -> bool {
        matches!(self, Verdict::NotMet(why) if why.starts_with(NO_REPORT))
    }
}

/// The list of tasks and the judge, both the older app's until a stream moves (section 20b's bridge).
pub trait TaskList {
    /// The words a session starts from, in its worktree `folder`, whose sandbox profile says what it may write.
    fn brief(&self, id: &str, folder: &str) -> Result<String, String>;
    /// The verdict on its newest report, else on what its session last `said`. Err: no answer yet, the same is read again.
    fn judge(&self, id: &str, said: &str) -> Result<Verdict, String>;
    fn merged(&self, id: &str, pr: &str) -> Result<(), String>;
    /// A PR that merged after its run ended, judged met again: the same close, its note saying so.
    fn merged_later(&self, id: &str, pr: &str) -> Result<(), String> {
        self.merged(id, pr)
    }
    /// Its report written from its merged PR `pr` (`dash task report`), its run having ended before it wrote one. No default:
    /// a list that filed nothing would let `rejudge` judge the same empty stop again (/code-review).
    fn report_merged(&self, id: &str, pr: &str) -> Result<(), String>;
    /// A run over without a merge: flagged, and its open PR is the list's to drive from here.
    fn ended(&self, id: &str, why: &str) -> Result<(), String>;
    /// Opens it again, as `dash task reopen` does: its next sync starts its row over (`TaskStore::reopen_held`).
    fn reopen(&self, id: &str) -> Result<(), String>;
    /// Tells whoever coordinates of what nothing here will move.
    fn flag(&self, id: &str, what: &str) -> Result<(), String>;
    /// What this run's session waits on (`Limited`, `Offline`), for the task's page; `None` once it is told to go on.
    fn waiting(&self, id: &str, on: Option<&Outcome>) -> Result<(), String>;
    /// Of `ids`, those it holds done or dropped: a look cleans their worktrees' build (task rust-clean-closed-builds). A list
    /// that cannot say closes none.
    fn closed(&self, _ids: &[String]) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    /// Jev's score of a met run's `diff` against the RFC section its `layer` names, 0 to 1, and the rule it was held to; `None`
    /// when no section is judged for that layer, or this list judges none (task rust-task-rfc-judge).
    fn rfc_check(&self, _id: &str, _layer: &str, _diff: &str) -> Result<Option<RfcScore>, String> {
        Ok(None)
    }
    /// Whether its row says `review true`: its PR then lands only after a reviewer's verdict (`review_needed`).
    fn review(&self, _id: &str) -> Result<bool, String> {
        Ok(false)
    }
}

/// A diff's score against its RFC section, and the rule it was held to.
#[derive(Clone, Debug, PartialEq)]
pub struct RfcScore {
    pub score: f64,
    pub rule: String,
}

/// Under this, a diff breaks its RFC section: through `dash task rfc-check`, the back-test of 2026-11-11 scored the 10 breaches
/// 0.17 to 0.41 and the 11 keepers 0.68 to 0.84 but one at 0.38 (rfc-rust-unified#conformance). Measured again on the next 20
/// Rust PRs.
pub const RFC_BAR: f64 = 0.5;

/// An agent the terminal sees in a pane. `asking`: waiting on a question it put. Neither that nor `idle`: working.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub idle: bool,
    pub asking: bool,
    pub session: Option<String>,
}

/// A pane's status as its program reported it, never read off its screen (task rust-watch-osc7501).
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub pane: String,
    /// idle, working, done, blocked or error.
    pub state: String,
    /// question, permission or auth, when a blocked program said which.
    pub kind: Option<String>,
    /// Moves each time the word changes: a stop is one by its pane and this.
    pub at: i64,
    /// Its agent session id, where the transcript is, when the runtime keeps one.
    pub session: Option<String>,
}

/// Panes and what runs in them. Nothing here takes his focus.
pub trait Terminal {
    /// A new pane in `folder`: its id.
    fn open(&self, folder: &str, label: &str) -> Result<String, String>;
    /// A command line, as typed at the pane's shell.
    fn run(&self, pane: &str, line: &str) -> Result<(), String>;
    /// The agent the pane holds, `None` when none. `Seen.session` is the agent's own session id: the orchestrator knows a run
    /// by it, never by the pane id, which a terminal reuses and a reboot starts over. A backend that cannot read it returns `None`
    /// there, and its runs are then judged by the pane alone (rust-hc-terminal's review found this rule unstated).
    fn agent(&self, pane: &str) -> Result<Option<Seen>, String>;
    /// The sessions open, whoever started them: what the seats count.
    fn sessions(&self) -> Result<usize, String>;
    /// Of the agent `seen` in `pane`: `None` when it is no session `sessions` counts (another kind of agent), else whether
    /// it shows a question dialog, never a permission prompt answered in seconds (task rust-seats-skip-his-waits). The default
    /// reads every agent as counted and every prompt as a question.
    fn question(&self, _pane: &str, seen: &Seen) -> Result<Option<bool>, String> {
        Ok(Some(seen.asking))
    }
    fn read(&self, pane: &str) -> Result<String, String>;
    /// Text, arriving whole however long.
    fn send(&self, pane: &str, text: &str) -> Result<(), String>;
    fn key(&self, pane: &str, key: &str) -> Result<(), String>;
    fn close(&self, pane: &str) -> Result<(), String>;
}

/// The command that starts this agent, sandboxed, in the task's folder, on `account` when one is named.
pub trait Launcher {
    fn command(&self, task: &Task, account: Option<&str>) -> Result<String, String>;
    /// The same, resuming `session` in the task's folder rather than starting a new one: a limit moves it to another login.
    /// `from`: the pane it leaves, whose supervisor may hold the session for a moment more.
    fn resume(
        &self,
        task: &Task,
        account: Option<&str>,
        session: &str,
        from: &str,
    ) -> Result<String, String>;
    /// What a session on a login about to run out is told for task `id`: finish its step, push, leave a handover note, end with
    /// its block. None by default: this agent winds down nothing.
    fn wind_down(&self, _id: &str) -> Option<String> {
        None
    }
    /// The handover note a wound-down session left in `folder` for task `id`, written at or after `since` (Unix seconds), taken so
    /// a later block of its own is never read as one: its path relative to `folder`. None by default: no note.
    fn handover(&self, _folder: &str, _id: &str, _since: u64) -> Option<String> {
        None
    }
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
/// `models`: each allowance one model has of its own inside the week, as `get_usage` answers it (Fable's, RFC 20h).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limit {
    pub account: String,
    pub five_hours: f64,
    pub five_hours_resets: u64,
    pub week: f64,
    pub week_resets: u64,
    pub key: bool,
    pub models: Vec<Scoped>,
}

/// One model's own allowance on a login: its name as the agent's model flag takes it, percent used, its reset (Unix seconds).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scoped {
    pub model: String,
    pub used: f64,
    pub resets: u64,
}

impl Limit {
    /// Both windows below 100%: he spends an account to its end, so only that stops a start or holds a session.
    pub fn room(&self) -> bool {
        self.five_hours < 100.0 && self.week < 100.0
    }

    /// `room`, and `model`'s own allowance under 100% too when one is read and not past its reset at `now` (RFC 20h): what a
    /// start on that model, or a session on it, may spend.
    pub fn room_for(&self, model: Option<&str>, now: u64) -> bool {
        self.room() && model.is_none_or(|m| self.scoped(m, now).is_none_or(|s| s.used < 100.0))
    }

    /// `model`'s own allowance on this login, read and not past its reset at `now`.
    pub fn scoped(&self, model: &str, now: u64) -> Option<&Scoped> {
        let own = self.models.iter().find(|m| m.model == model);
        own.filter(|m| m.resets > now)
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
    /// Whether its pane shows its own limit menu, which the terminal reads as a question. None by default.
    fn limit_menu(&self, _shown: &str) -> bool {
        false
    }
    /// The logins forecast to run out within `within_s` seconds at their burn: their sessions are told to wind down. None by default.
    fn ending(&self, _within_s: u64) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    /// The tokens `session` spent so far, cache reads left out: what a duplicate run cost. An agent that cannot read it says so.
    fn tokens(&self, _folder: &str, _session: &str) -> Result<u64, String> {
        Err("this agent keeps no tokens".into())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pr {
    pub number: u64,
    pub url: String,
    pub head: String,
    pub merged: bool,
}

/// The monitors' data, `dash monitors save`'s source, as git names it (`dash land`, the forge's deploy): a merge that changes
/// it has its deploy save them. A path, not a `use` of monitors.rs, which reaches back to the tasks through alerts.rs.
pub const MONITORS_DATA: &str = "rust/crates/dash/src/monitors.rs";

/// An open PR as the stall watcher reads it (dash_orch::stalled), the forge listing it (task rust-stalled-port).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenPr {
    pub pr: Pr,
    pub branch: String,
    /// The task its title names, else its branch `task/<id>`.
    pub task: Option<String>,
    /// Seconds since its head commit: none read is 0, no stall.
    pub head_age_s: i64,
}

/// The task a PR's title names, `[fix|feat|chore][<id>] …` (bin/pr-title-gate's convention), else `pr-<repo>-<n>` for a GitHub
/// PR: the id its reviewer's brief names in its title template (task review-brief-own-task-id: PR 900, titled
/// [fix][review-claim-after-release], got a brief naming [pr-site_perso-900]). None: neither a task id nor a GitHub PR link.
/// Ported from `Dashboard.Orch.PM.pr_task/1`, which never read the title. ponytail: any owner, where pm.ex keeps his
/// (`Forge.owners_re`); Rust's runs review site_perso's PRs only.
pub fn pr_task_id(title: &str, url: &str) -> Option<String> {
    title_task(title).or_else(|| {
        let mut parts = url.strip_prefix("https://github.com/")?.split('/').skip(1);
        let (repo, pull, n) = (parts.next()?, parts.next()?, parts.next()?);
        let n = n.split(['?', '#']).next()?;
        (pull == "pull" && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("pr-{repo}-{n}"))
    })
}

/// The task id a PR title names, exactly as bin/pr-title-gate passes it: `[fix|feat|chore][<id>] ` then a word.
pub fn title_task(title: &str) -> Option<String> {
    let (kind, rest) = title.strip_prefix('[')?.split_once("][")?;
    let (id, after) = rest.split_once(']')?;
    let worded = after
        .strip_prefix(' ')
        .is_some_and(|w| w.starts_with(|c: char| !c.is_ascii_whitespace()));
    let ok = matches!(kind, "fix" | "feat" | "chore")
        && worded
        && !id.is_empty()
        && (id.bytes()).all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
    ok.then(|| id.to_string())
}

/// Why a TASK_DONE is held on its PR's title, else None: status-guard.py's title check (removed in #1120), ported with its fault
/// fixed. A merged PR's title is no one's to fix any more, and a title naming any real task passes, its session's own or another's:
/// PR 1107, titled [feat][rust-registry-ext], held its reviewer review-pr-site_perso-1107 because the title was not that id.
pub fn title_refusal(title: &str, merged: bool, is_task: impl Fn(&str) -> bool) -> Option<String> {
    if merged {
        return None;
    }
    match title_task(title) {
        Some(id) if is_task(&id) => None,
        Some(id) => Some(format!("the title names « {id} », no task in the store")),
        None => Some(format!(
            "« {title} » must read « [fix|feat|chore][<task-id>] <title> »"
        )),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Checks {
    Green,
    Red(String),
    /// Red only on steps before any test (a runner never set up, checkout, cache, apt), each named: no fault of the code, so a
    /// met task stays met and its CI is rerun once that step runs green again (task rust-infra-red-reruns).
    Infra(String),
    Pending,
}

/// A reviewer's verdict on a PR, as `dash review verdict` posts it (task rust-land-waits-review): `Pass` lands it, `Fix` sends
/// it back with the reviewer's words.
#[derive(Clone, Debug, PartialEq)]
pub enum Review {
    Pass(String),
    Fix(String),
}

/// Why a PR waits on a reviewer's verdict before it lands, `None` when green CI alone lands it: the send paths it changes
/// (`Forge::sends`), or its task's `review true` (@nha, 2026-11-11: « I approved so this is up to you to send automatically. »;
/// PR 1490, the outbound gate, merged on green mid-review, and its review then found a double send and three fail-open paths).
pub fn review_needed(sends: &[String], flagged: bool) -> Option<String> {
    match (sends.is_empty(), flagged) {
        (false, _) => Some(format!("it touches a send path ({})", sends.join(", "))),
        (true, true) => Some("its task says review true".into()),
        (true, false) => None,
    }
}

/// Why a PR touching hot files (`hot`) waits on a reviewer, one on `HOT_MODEL` (task hot-files-fable); `None` when it
/// touches none.
pub fn hot_why(hot: &[(String, usize)]) -> Option<String> {
    let files: Vec<String> = hot.iter().map(|(f, n)| format!("{f}: {n}")).collect();
    (!files.is_empty()).then(|| {
        format!(
            "it touches a file fixed more than {HOT_FIXES} times in {HOT_DAYS} days ({})",
            files.join(", ")
        )
    })
}

/// A file is hot past this many [fix] PRs merged in `HOT_DAYS` days: a PR touching it is reviewed on `HOT_MODEL` (@nha,
/// 2026-10-29: « when a file / feature gets too many bugs get a bigger model to review and/or fix it (Fable) »).
pub const HOT_FIXES: usize = 3;
pub const HOT_DAYS: u32 = 14;
pub const HOT_MODEL: &str = "fable";

/// The forge CLI's arguments listing every file PR `number` changes, a moved one by its old name too, paged past the 100 a
/// PR view stops at. The one read of a PR's files, for the forge's holds and `dash review model` both.
pub fn pr_files(number: u64) -> Vec<String> {
    let path = format!("repos/{{owner}}/{{repo}}/pulls/{number}/files");
    let names = ".[] | .filename, (.previous_filename // empty)";
    ["api", &path, "--paginate", "-q", names]
        .map(String::from)
        .to_vec()
}

/// `git log`'s arguments for `fix_counts`: `base`'s commits of `HOT_DAYS` days, each its subject and files. The one reader of
/// the fix history, for the hot files' wake and the forge's review hold both (arch-review of hot-files-fable).
pub fn fix_log(base: &str) -> Vec<String> {
    let since = format!("--since={HOT_DAYS}.days.ago");
    let base = format!("origin/{base}");
    ["log", &since, "--format=%x1e%s", "--name-only", &base]
        .map(String::from)
        .to_vec()
}

/// Each file's count of [fix] PRs in `log`, as `fix_log` prints the base branch: a squash merge
/// a PR, its subject `[fix][<task>] <title> (#<n>)`. A commit pushed straight to main is no PR, so never counted. Most fixed
/// first, then by name.
pub fn fix_counts(log: &str) -> Vec<(String, usize)> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for commit in log.split('\x1e').skip(1) {
        let mut lines = commit.lines();
        let subject = lines.next().unwrap_or_default().trim_end();
        if !(subject.starts_with("[fix]") && subject.contains("(#") && subject.ends_with(')')) {
            continue;
        }
        let files: BTreeSet<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
        for file in files {
            *counts.entry(file).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().map(|(f, n)| (f.into(), n)).collect();
    // stable: equal counts keep their name order
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    ranked
}

/// The files of `files` that `counts` (`fix_counts`) names past `HOT_FIXES`, with their counts, most fixed first.
pub fn hot<'a>(
    counts: &[(String, usize)],
    files: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, usize)> {
    let files: Vec<&str> = files.into_iter().collect();
    (counts.iter())
        .filter(|(f, n)| *n > HOT_FIXES && files.contains(&f.as_str()))
        .cloned()
        .collect()
}

/// The first line of a verdict comment; the rest of that line is `pass|fix <head sha>`, the lines after it the reviewer's words.
pub const VERDICT: &str = "Review verdict:";

/// The PR comment `dash review verdict` posts on `head`.
pub fn verdict_comment(review: &Review, head: &str) -> String {
    let (word, words) = match review {
        Review::Pass(w) => ("pass", w),
        Review::Fix(w) => ("fix", w),
    };
    format!("{VERDICT} {word} {head}\n\n{}", words.trim())
}

/// A comment's verdict and the head it names; `None`: no verdict comment.
pub fn verdict_of(body: &str) -> Option<(Review, String)> {
    let (first, rest) = body
        .trim_start()
        .split_once('\n')
        .unwrap_or((body.trim_start(), ""));
    let mut w = first.strip_prefix(VERDICT)?.split_whitespace();
    let (word, head) = (w.next()?, w.next()?);
    let words = rest.trim().to_string();
    let review = match word {
        "pass" => Review::Pass(words),
        "fix" => Review::Fix(words),
        _ => return None,
    };
    Some((review, head.to_string()))
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

/// dash-gate's tests on a PR merged with main, before `dash land` merges it (task rust-land-gates-merged; @nha, 2026-11-13:
/// « Each PR alone was green… seems bad design. »): passed, red, or the files the merge conflicts in. Each in words.
/// `MergedGate::Red`'s first words: a hand-back on them reaches the new run's first words, which its green checks never give.
pub const MERGED_RED: &str = "dash-gate fails on ";

#[derive(Clone, Debug, PartialEq)]
pub enum MergedGate {
    Passed(String),
    Red(String),
    Conflicts(Vec<String>),
}

/// A task's worktree and its pull request.
pub trait Forge {
    /// Its worktree's real path, made once, kept across a retry.
    fn worktree(&self, id: &str) -> Result<String, String>;
    /// Its worktree as it stands, never started over from main for a merged PR: an adopted session works in it.
    fn worktree_as_is(&self, id: &str) -> Result<String, String> {
        self.worktree(id)
    }
    /// Its branch's newest pull request, of any state.
    fn pr(&self, id: &str) -> Result<Option<Pr>, String>;
    fn checks(&self, pr: &Pr) -> Result<Checks, String>;
    /// `pr`'s infra reds (`Checks::Infra`) rerun, each once a later run of its job got past that step: what was rerun, in words;
    /// none while the outage lasts.
    fn rerun(&self, _pr: &Pr) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    /// Err: no verdict.
    fn base(&self, pr: &Pr) -> Result<Base, String>;
    /// Merges exactly `pr.head`. Err: refused, or no answer, which only a later look tells apart.
    fn merge(&self, pr: &Pr) -> Result<(), String>;
    /// Main merged into its branch where its session works, and pushed: a met PR behind main is brought up to date here, never
    /// sent back (#1274 ended « not met after 3 nudges » only because main moved). Err: nothing pushed; one holding
    /// [`PUSH_TIMED_OUT`] is pushed again at the next pass, never counted as refused.
    fn update(&self, id: &str, pr: &Pr) -> Result<(), String>;
    /// Its worktree gone, once its PR is merged and its session stopped.
    fn leave(&self, id: &str) -> Result<(), String>;
    /// Its worktree back to the pool, its branch kept for a reopen, once its run is closed and nothing in it lacks a remote
    /// (`unsaved` said none). A stand-in keeps it.
    fn give_back(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }
    /// After `merged` squash-merged: every open PR stacked on its branch rebased onto main, its base main, so it conflicts with
    /// none of the squash. What each one got, said in words.
    /// `busy`: the branches a session is at work on, left alone.
    fn restack(&self, merged: &Pr, busy: &[String]) -> Result<Vec<String>, String>;
    /// Its build output gone, once its PR is merged, whatever else stays; never while a cargo builds there (Err says so).
    fn clean(&self, id: &str) -> Result<(), String>;
    /// The build output of its worktree (`None`: the repository's own checkout) gone once past `cap` bytes, never while a cargo
    /// builds there (Err says so): the bytes counted once past the cap, `None` when under it (task rust-disk-caps).
    fn trim(&self, _id: Option<&str>, _cap: u64) -> Result<Option<u64>, String> {
        Ok(None)
    }
    /// Each task whose worktree holds build output, whatever its state: what a look cleans once the list closed it.
    fn built(&self) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    /// What its worktree holds that no remote has, in words (changes not committed, commits on no remote branch); None: nothing.
    /// A forge that cannot look says so, never none: a close must not lose work on a guess.
    fn unsaved(&self, _id: &str) -> Result<Option<String>, String> {
        Err("this forge cannot look at a worktree".into())
    }
    /// Every worktree no run holds (`busy`: the claimed ids) whose PR merged or closed, its tree clean, nothing in it past that PR
    /// or a remote branch, untouched an hour: gone, whatever ended its run. Only a run that saw its own merge left before: 9 of
    /// 28 stayed past their merge, up to 18 GB each with their build (task worktrees-under-5g, 2026-11-09). What went, in words; a failed read keeps the worktree.
    fn reap(&self, _busy: &[String]) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    /// A finished task's worktree that the caller saw nothing work in (no claim, no session in it): released now, as `reap` would
    /// after its hour or day, or kept for what no remote has. What became of it, in words.
    fn release_done(&self, _id: &str) -> Result<String, String> {
        Err("this forge cannot release a worktree".into())
    }
    /// The base branch's newest commit live, after a merge: what serves then, in words, its sha first.
    fn deploy(&self) -> Result<String, String>;
    /// Main fetched, then what the page serves against its rust/. Err: git could not tell. No default: a forge that forgot it
    /// would never deploy a merge made outside a run, silently (/code-review of dc971a27c).
    fn serving(&self) -> Result<Serving, String>;
    /// After a deploy, the live binary's `dash monitors save` when the commit live changes the monitors' data since the last
    /// save: Ok(None) when it does not. No default, as `serving`: a forge that forgot it would leave the old monitors watching,
    /// silently (task rust-deploy-any-merge).
    fn save_monitors(&self) -> Result<Option<String>, String>;
    /// Its branch's change since it left the base branch, as `git diff` prints it: what the judge's layer check reads.
    fn diff(&self, id: &str) -> Result<String, String>;
    /// The files `pr` changes where something is sent in his name, or the gate that decides it: what `review_needed` reads. A
    /// stand-in lists none.
    fn sends(&self, _pr: &Pr) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    /// The files `pr` changes fixed more than `HOT_FIXES` times in `HOT_DAYS` days (`hot`): what puts its reviewer on
    /// `HOT_MODEL`. A stand-in lists none.
    fn hot(&self, _pr: &Pr) -> Result<Vec<(String, usize)>, String> {
        Ok(vec![])
    }
    /// `sends` and `hot` of `pr`, from one read of its files where a forge can (/code-review of PR 1692: two paged reads a
    /// head). The hot read's failure stays apart: it holds nothing.
    #[allow(clippy::type_complexity)]
    fn touches(
        &self,
        pr: &Pr,
    ) -> Result<(Vec<String>, Result<Vec<(String, usize)>, String>), String> {
        Ok((self.sends(pr)?, self.hot(pr)))
    }
    /// The newest reviewer's verdict on `pr` (`dash review verdict`'s comment) that still covers its head: the commit it names,
    /// or one only merges followed. `None`: none yet, or one on code that changed since.
    fn verdict(&self, _pr: &Pr) -> Result<Option<Review>, String> {
        Ok(None)
    }
    /// `pr`'s title, which names its task (`title_task`). A stand-in has none.
    fn title(&self, _pr: &Pr) -> Result<String, String> {
        Ok(String::new())
    }
    /// Whether its worktree made a commit since `since_ms` (Unix milliseconds) that a remote branch holds: a run's own push, never
    /// one it only pulled. A forge that cannot look says so.
    fn own_pushes(&self, _id: &str, _since_ms: i64) -> Result<bool, String> {
        Err("this forge cannot look at a worktree".into())
    }
    /// Whether its remote answers: a run the network ended starts again only then (`network_end`). A stand-in always does.
    fn reachable(&self) -> bool {
        true
    }
}

/// Whether a run's failure names the network, never the work: ssh, DNS or a fetch that found no host. Such an end spends no
/// try, as no try was made (task rust-infra-fail-reopens: 108 runs ended 00:00-06:00 on 2026-11-13 on « ssh: connect to host
/// github.com », each its task's last try). ponytail: words, as the tools print them; a code per failure if one ever lies.
pub fn network_end(said: &str) -> bool {
    const WORDS: [&str; 7] = [
        "ssh: connect to host",
        "Could not resolve host",
        "nodename nor servname",
        "Temporary failure in name resolution",
        "Network is unreachable",
        "error connecting to api.github.com",
        "dial tcp: lookup",
    ];
    WORDS.iter().any(|w| said.contains(w))
}

/// All a pick reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub queue: Queue,
    pub sessions: usize,
    pub limits: Vec<Limit>,
    /// The machine's free memory in percent, as last read; `None` unread, which holds nothing back.
    pub free_percent: Option<u8>,
    /// Unix seconds: what a pool's refill is measured from (RFC 20h rule 2).
    pub now: u64,
}

/// The pool a start spends (RFC 20h): the model it runs on and why that one.
#[derive(Clone, Debug, PartialEq)]
pub struct Spend {
    pub pool: String,
    pub why: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Start {
        id: String,
        account: Option<String>,
        /// `None`: no route, the login's own model.
        pool: Option<Spend>,
    },
    /// A crashed run's session, still in its pane: a new run watches it on (`TaskStore::hold`). Only `next` picks it, never a
    /// policy.
    Adopt(String),
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
    /// Why `task` may not start at this moment whatever else the queue holds, in plain words (memory short for a build); `None`:
    /// it may. `pick` and `plan` both read it, so the plan names the reason the pick acts on.
    fn holds(&self, _task: &Queued, _free_percent: Option<u8>) -> Option<String> {
        None
    }
}

/// The free memory a build needs, in percent: under it only work that builds nothing starts. The older app's start gate,
/// `Runner`'s `@low_free_pct` (RFC 20e rule 4; @nha, 2026-11-09: « Light in RAM but has a lot of LLM subscription left [...]
/// -> plan tasks, sort them, add labels, prepare drafts, review RFCs [...] »).
pub const BUILD_FREE_PERCENT: u8 = 25;

/// The 5-minute load per core past which nothing starts (task rust-seats-follow-room: rust-seats raised to 5 took the load from 14
/// to 60 on 10 cores; @nha, 2026-11-11: « Think about the system and what would improve it for the future. Spike, measure. »).
pub const LOAD_PER_CORE: f64 = 3.0;

/// The label of a task whose work lowers the load: past `LOAD_PER_CORE` it still starts, one at a time (task load-relief-starts;
/// @nha, 2026-11-14: « If CI is the bottleneck... use hostman as I asked. »: rust-hostman-ci-runner and hooks-run-built-dash,
/// tonight's two load fixes, were held by that load for hours). Rust's store keeps it: the older list's labels are frozen.
pub const RELIEVES_LOAD: &str = "relieves-load";

/// Why task `id` may not carry `RELIEVES_LOAD`: its why, the last `--why` of `args`, else the one `filed` reads, names no load,
/// CI or build cost, so the label would only skip the hold. A read that failed says so, never taken as an empty why. `None`: it
/// may.
pub fn relief_refused(
    id: &str,
    args: &[String],
    filed: impl FnOnce() -> Result<Option<String>, String>,
) -> Option<String> {
    let given = (args.windows(2)).rfind(|p| p[0] == "--why");
    let why = match given {
        Some(p) => p[1].clone(),
        None => match filed() {
            Ok(Some(why)) => why,
            // never read as an empty why: the refusal says what was missing (/code-review)
            Ok(None) => {
                return Some(format!(
                    "{id}: refused, the list holds no open task {id} to read its why from: give --why"
                ));
            }
            Err(e) => return Some(format!("{id}: refused, its why could not be read: {e}")),
        },
    };
    let lower = why.to_lowercase();
    let mut words = lower.split(|c: char| !c.is_alphanumeric());
    // whole words: « loading » or « builder » names no cost (/code-review)
    // ponytail: a word list, so « builds nothing » passes; read the cost's verb if a mislabel ever skips the hold
    const COSTS: [&str; 11] = [
        "load",
        "loads",
        "ci",
        "build",
        "builds",
        "compile",
        "compiles",
        "compilation",
        "cargo",
        "runner",
        "runners",
    ];
    let names = |w: &str| COSTS.contains(&w);
    (!words.any(names)).then(|| {
        format!(
            "{id}: refused, --labels {RELIEVES_LOAD} starts past the load hold, and its why names no load, CI or build \
             cost: say which it lowers in --why"
        )
    })
}

/// The seats this moment allows and what set them, from the list's `seats` and `why` and the machine's readings: `load`, the
/// 5-minute load per core, and `free_percent`; `None` unread. Past `LOAD_PER_CORE`, none. Under `BUILD_FREE_PERCENT` free the
/// seats stand and the policy's `holds` keeps builds back, light work still starting (the answer in rust-seats-follow-room's
/// question dialog, 2026-11-11: « Light tasks still start »). A fact of the machine, not a policy's choice: `plan_at` applies
/// it, so no planner can skip it (PR 1463's /simplify).
pub fn seats_now(
    seats: usize,
    why: &str,
    load: Option<f64>,
    free_percent: Option<u8>,
) -> (usize, String) {
    if let Some(load) = load.filter(|l| *l > LOAD_PER_CORE) {
        let otherwise = match why {
            "" => format!("{seats} otherwise"),
            why => format!("{seats} otherwise: {why}"),
        };
        let why = format!(
            "load: the 5-minute load is {load:.2} per core, past {LOAD_PER_CORE}, so nothing starts ({otherwise})"
        );
        return (0, why);
    }
    let memory = free_percent.filter(|f| *f < BUILD_FREE_PERCENT).map(|free| {
        format!("memory: {free}% free, under {BUILD_FREE_PERCENT}%, so only work that builds nothing starts")
    });
    let said = [Some(why.to_string()), memory].into_iter().flatten();
    let said: Vec<String> = said.filter(|w| !w.is_empty()).collect();
    (seats, said.join("; "))
}

/// A sending's status, Linear's triage: it arrived, he has not planned it, and no run claims it (task capture-extension; @nha,
/// 2026-10-29: « status in triage could be a good fit, linear-style »). Kept in Rust's store alone: the older list's statuses and
/// labels are frozen with its Elixir.
pub const TRIAGE: &str = "triage";
/// A sending's label: where it came from (@nha, 2026-10-29: « perhaps with an initial label to show the provenance »).
pub const FROM_EXTENSION: &str = "from-extension";

/// A task the Chrome extension sent: its words (the comment and the page's address), the files kept beside it, and the task whose
/// worktree served the page, if one did, and the sub-project its host names, if one does (flare.nicolas-ha.com's flare-site).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Triaged {
    pub id: String,
    pub title: String,
    pub text: String,
    pub page: String,
    pub files: Vec<String>,
    pub task: Option<String>,
    pub project: Option<String>,
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
    /// The PR head it was last handed back at, then the count.
    pub handed: Option<String>,
    /// Handed back and no run on it finished since, nor its merge judged again: what `handed_back` reads. Apart from `handed`,
    /// which outlives the hand-back as the head no session moved (/code-review of 6305f42: a later reopen read as a hand-back).
    pub back: bool,
    /// The merged PR head an ended run was judged again at, then the verdict.
    pub rejudged: Option<String>,
    /// What became of main after its merge (`TaskStore::deployed`): « failed <n>: … » while the look retries it.
    pub deployed: Option<String>,
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

/// The design system's crate: the one place a page's markup, styles and components are written (rule 6 of rust/'s rules).
pub const DESIGN_SYSTEM: &str = "rust/crates/surface/";

/// HTML elements a page might write by hand in a string: `"<div ` and its kind.
const TAGS: [&str; 24] = [
    "div", "span", "a", "p", "b", "i", "button", "input", "textarea", "select", "form", "label",
    "section", "nav", "ul", "ol", "li", "table", "tr", "td", "h1", "h2", "h3", "img",
];

/// Whether an added Rust line draws a page: an `rsx! {`, a `#[component]`, an rsx `class: "…"` or `style: "…"`, or a TAGS
/// element written in a string. A struct's `style: s`, a word in a doc comment or a usage string's `"<id>"` is none.
fn draws(code: &str) -> bool {
    let code = code.trim();
    let tag = |rest: &str| {
        TAGS.iter().any(|t| {
            rest.strip_prefix(t)
                .is_some_and(|r| r.starts_with([' ', '>', '/']))
        })
    };
    !code.starts_with("//")
        && (code.contains("rsx! {")
            || code.contains("rsx!{")
            || code.starts_with("#[component]")
            || code.contains("class: \"")
            || code.contains("style: \"")
            || code
                .match_indices("\"<")
                .any(|(at, _)| tag(&code[at + 2..])))
}

/// Each line a unified diff adds that draws a page outside the design system, as « file: line »: `draws`, or a whole .css,
/// .html or .js file. A run that names a layer changes it there; code like this elsewhere is a page's own one-off (task
/// rust-task-names-layer; @nha, 2026-11-09: « HTML is an impl detail of our components that are reusable. Components are
/// impl. details of our pages »). Read with the whole file as context (`git diff -U<large>`), so a top-level `#[cfg(test)]`
/// module, as the gate's markup rule reads it, and a tests/ folder are test code. A file's name is read from its header only,
/// between `diff --git` and its first `@@`, so an added line « ++ x » is never taken for one.
pub fn page_local_lines(diff: &str) -> Vec<String> {
    let (mut file, mut header, mut tests) = ("", false, false);
    let mut found = vec![];
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            (file, header, tests) = ("", true, false);
            continue;
        }
        if header {
            if let Some(path) = line.strip_prefix("+++ ") {
                file = path.strip_prefix("b/").unwrap_or(path);
            }
            header = !line.starts_with("@@");
            continue;
        }
        let (Some(mark), body) = (line.chars().next(), line.get(1..).unwrap_or_default()) else {
            continue;
        };
        tests |= mark != '-' && body.starts_with("#[cfg(test)]");
        // a vendored crate is a dependency kept as published plus its patch, no page of ours: blitz-dom's user-agent
        // stylesheet held rust-ui-hit-panic's run
        let ours = file.starts_with("rust/")
            && !file.starts_with(DESIGN_SYSTEM)
            && !file.starts_with("rust/vendor/")
            && !file.contains("/tests/");
        if mark != '+' || tests || !ours {
            continue;
        }
        let markup_file = [".css", ".html", ".js"].iter().any(|e| file.ends_with(e));
        if markup_file || (file.ends_with(".rs") && draws(body)) {
            found.push(format!("{file}: {}", body.trim()));
        }
    }
    found
}

/// A judge's verdict, held while the run's diff draws a page outside the design system: a met run whose task names `layer`
/// is not met until that code moves into it. Any other verdict stands as it is.
pub fn held_by_layer(verdict: Verdict, layer: Option<&str>, diff: &str) -> Verdict {
    let (Verdict::Met, Some(layer)) = (&verdict, layer) else {
        return verdict;
    };
    let found = page_local_lines(diff);
    if found.is_empty() {
        return verdict;
    }
    let shown: Vec<&str> = found.iter().take(5).map(String::as_str).collect();
    Verdict::NotMet(format!(
        "it names {layer}, and its diff draws a page outside the design system ({DESIGN_SYSTEM}), {} line(s): {}; \
         move it into the component and use it",
        found.len(),
        shown.join("; ")
    ))
}

/// A task that does not run now, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub id: String,
    pub why: String,
    /// Ready, held only because another ready task comes first: no wait of its own, so its why is « queued », never
    /// « behind <id> », and no page draws an edge for it (task rust-graph-queue-edges; @nha, 2026-11-09: « that's not a graph
    /// this is a list of cards »). Set where `plan` builds that hold, never read from `why`, whose words a list hold may share.
    pub queued: bool,
}

/// What a task held only by queue order says: its place is the list's order, the Next line names what comes first.
pub const QUEUED: &str = "queued";

/// What runs next and why the others wait.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The seats this plan held to and what set them, after the policy's `seats`.
    pub seats: usize,
    pub seats_why: String,
    pub next: Pick,
    pub waiting: Vec<Held>,
    /// Every task the list offers, ranked, each with its reason (`rank`).
    pub order: Vec<(String, String)>,
    /// The panes left out of the seats, each task waiting on his answer and why (`his_wait` or a dialog open): set by
    /// whoever counted the sessions, as `plan_at` reads only their number (task rust-seats-skip-his-waits).
    pub his: Vec<(String, String)>,
    /// Each ready task that passed its layer's rule only by `jumps`, and the rule in words (`jumped_words`).
    pub jumped: Vec<(String, String)>,
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

/// The wait on his answer `id`'s task names, its row's or the list's: a `:blocked-by` « his: <what he does> » (the
/// task-writing skill). Its pane holds no seat while he decides (task rust-seats-skip-his-waits: two panes idle 7 h on his
/// PR 1476 and 1477 choices held 2 of 5 seats, 2026-11-12).
pub fn his_wait(id: &str, rows: &[Standing], queue: &[Queued]) -> Option<String> {
    let queued = queue.iter().find(|t| t.id == id);
    let waits = all_waits(row(rows, id), queued);
    waits
        .into_iter()
        .find(|w| w.trim_start().starts_with("his:"))
}

/// What set the seats, as a wait's closing words: « (day 5) », or nothing when nothing was said.
pub fn said(seats_why: &str) -> String {
    match seats_why {
        "" => String::new(),
        why => format!(" ({why})"),
    }
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
    free_percent: Option<u8>,
) -> Plan {
    plan_at(policy, queue, rows, sessions, limits, free_percent, None, 0)
}

/// `plan` at `now`, Unix seconds, what a pool's reset and refill are measured from (RFC 20h rule 2): the core reads no clock, so
/// its caller does. `plan`'s 0 reads no reset as past and waits for no refill. `load`, the machine's 5-minute load per core, and
/// `free_percent` set the seats first, through the policy's `seats` (task rust-seats-follow-room).
#[allow(clippy::too_many_arguments)]
pub fn plan_at(
    policy: &dyn Policy,
    queue: Queue,
    rows: &[Standing],
    sessions: usize,
    limits: Vec<Limit>,
    free_percent: Option<u8>,
    load: Option<f64>,
    now: u64,
) -> Plan {
    let mut queue = queue;
    // his own seats, which the relief's one seat past the load hold never passes (/code-review)
    let his_seats = queue.seats;
    (queue.seats, queue.seats_why) = seats_now(queue.seats, &queue.seats_why, load, free_percent);
    let order = rank(&mut queue.tasks, rows);
    // only what a pick may start: a claimed or finished row is no candidate, as below
    let pickable = |id: &String| row(rows, id).is_none_or(|r| r.status == "open");
    let mut order: Vec<(String, String)> =
        order.into_iter().filter(|(id, _)| pickable(id)).collect();
    let (mut ready, mut held) = (vec![], vec![]);
    // A layer a run changes now holds every other task naming it (task rust-task-names-layer: two runs on one component
    // write over each other), and so does a hand-back that could start, which starts first. A hand-back is never held by its
    // layer: its PR changed it already (task rust-pick-handbacks-first: PR 1363 waited an hour behind its domain's next task).
    // ponytail: a claimed task the list no longer offers lends no layer; its run ends within a pass.
    let claimed = |t: &Queued| row(rows, &t.id).is_some_and(|r| r.status == "claimed");
    let busy = |task: &Queued, layer: &str| {
        if handed_back(rows, &task.id).is_some() {
            return None;
        }
        // a hand-back that could start: not held by him, its waits met, the policy willing
        let startable = |t: &Queued| {
            let waits = all_waits(row(rows, &t.id), Some(t));
            t.held.is_none()
                && waits_unmet(rows, &waits).is_none()
                && policy.holds(t, free_percent).is_none()
        };
        // a hand-back first, wherever it ranks: it starts first, and no jump passes it
        let on = |t: &&Queued| t.layer.as_deref() == Some(layer);
        let back = |t: &&Queued| handed_back(rows, &t.id).is_some() && startable(t);
        let tasks = || queue.tasks.iter().filter(on);
        (tasks().find(back))
            .map(|t| {
                (
                    t,
                    format!("{} was handed back on {layer} and starts first", t.id),
                )
            })
            .or_else(|| {
                (tasks().find(|t| claimed(t))).map(|t| (t, format!("{} changes {layer} now", t.id)))
            })
    };
    // His priority 1 passes beside a run at work on its layer, never beside another such jump, at work or picked earlier in
    // this plan, nor a hand-back about to start. ponytail: a jump is read from today's rank; one whose priority dropped after
    // its start lends its layer to the next jump.
    let jump_at_work = |layer: &str| {
        (queue.tasks.iter()).any(|t| t.layer.as_deref() == Some(layer) && claimed(t) && jumps(t))
    };
    // each jump this plan lets pass: its layer, id and rule
    let mut jumped: Vec<(&str, String, String)> = vec![];
    // A file a started task names holds every other task naming it: two runs on one file build the same thing twice or step
    // on each other (task task-dedupe; incident-2026-11-03-arrow-not-ready, problem J; @nha, 2026-11-03: « this is either
    // double work or stepping in the exact same area in parallel »). Started: its run at work, its PR ready for review, or
    // handed back; a task never started holds nothing, or two naming one file would hold each other. A hand-back is never
    // held: its PR changed the file already.
    let overlap = |task: &Queued| {
        if handed_back(rows, &task.id).is_some() {
            return None;
        }
        let started = |t: &&Queued| {
            row(rows, &t.id).is_some_and(|r| match r.status.as_str() {
                "claimed" => true,
                "finished" => r.outcome.as_deref() == Some("done"),
                _ => handed_back(rows, &t.id).is_some(),
            })
        };
        (queue.tasks.iter())
            .filter(|t| t.id != task.id)
            .filter(started)
            .find_map(|t| {
                let f = t.files.iter().find(|f| task.files.contains(f))?;
                Some(format!(
                    "{} names {f} too and is at work or in review",
                    t.id
                ))
            })
    };
    for task in &queue.tasks {
        let standing = row(rows, &task.id);
        let why = match standing.map(|r| r.status.as_str()) {
            // A claim would add its row.
            None => (task.held.clone())
                .or_else(|| at_cap(rows))
                .or_else(|| waits_unmet(rows, &task.waits)),
            Some("open") => {
                (task.held.clone()).or_else(|| waits_unmet(rows, &all_waits(standing, Some(task))))
            }
            _ => continue,
        };
        let mut jump = None;
        let why = why
            .or_else(|| {
                let layer = task.layer.as_deref()?;
                let (holder, why) = busy(task, layer)?;
                if !jumps(task) || !claimed(holder) || jump_at_work(layer) {
                    return Some(why);
                }
                if let Some((_, first, _)) = jumped.iter().find(|(l, _, _)| *l == layer) {
                    return Some(format!("{first} jumps {layer} first"));
                }
                jump = Some((layer, jumped_words(&holder.id, layer)));
                None
            })
            .or_else(|| overlap(task))
            .or_else(|| policy.holds(task, free_percent));
        match why {
            Some(why) => held.push(Held {
                id: task.id.clone(),
                why,
                queued: false,
            }),
            None => {
                if let Some((layer, words)) = jump {
                    jumped.push((layer, task.id.clone(), words));
                }
                ready.push(task.clone());
            }
        }
    }
    // Past the load hold, a task labelled RELIEVES_LOAD still gets one seat while no other such task runs, and nothing else
    // starts (task load-relief-starts).
    let relief = |t: &Queued| t.labels.iter().any(|l| l == RELIEVES_LOAD);
    // ponytail: a claimed relief the list no longer offers is not counted, as a layer's; its run ends within a pass
    let relief_seat = load.is_some_and(|l| l > LOAD_PER_CORE)
        && sessions < his_seats
        && !queue.tasks.iter().any(|t| relief(t) && claimed(t))
        && ready.iter().any(relief);
    if relief_seat {
        let (relieving, rest): (Vec<Queued>, Vec<Queued>) = ready.into_iter().partition(relief);
        ready = relieving;
        let why = format!(
            "{}; only a task labelled {RELIEVES_LOAD} starts",
            queue.seats_why
        );
        held.extend(rest.into_iter().map(|t| Held {
            id: t.id,
            why: why.clone(),
            queued: false,
        }));
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
        held.push(Held {
            id,
            why,
            queued: false,
        });
    }
    let ids: Vec<String> = ready.iter().map(|t| t.id.clone()).collect();
    // No seat, no start, whatever the policy picks: the load guard holds every planner here (PR 1463's /code-review). In plain
    // words, never « 1 of 0 » (task rust-graph-queue-edges; @nha, 2026-11-09: « that's not a graph this is a list of cards here »).
    let no_seat = || {
        let first = ids.first().filter(|_| queue.seats == 0 && !relief_seat)?;
        Some(Pick::Wait(format!(
            "{first} waits: no session may start, 0 seats allowed{}",
            said(&queue.seats_why)
        )))
    };
    let next = all_held.or_else(no_seat).unwrap_or_else(|| {
        policy.pick(&Snapshot {
            queue: Queue {
                // the relief's one seat, above the sessions open
                seats: if relief_seat {
                    sessions + 1
                } else {
                    queue.seats
                },
                seats_why: queue.seats_why.clone(),
                tasks: ready,
            },
            sessions,
            limits,
            free_percent,
            now,
        })
    });
    let behind = ids.iter().enumerate().filter_map(|(at, id)| {
        let (why, queued) = match &next {
            Pick::Start { id: picked, .. } | Pick::Adopt(picked) if picked == id => return None,
            Pick::Wait(why) if at == 0 => (why.clone(), false),
            // Behind the pick, or behind a head a global hold (seats, load) stops: the Next line names that one.
            Pick::Start { .. } | Pick::Adopt(_) | Pick::Wait(_) => (QUEUED.to_string(), true),
        };
        let id = id.clone();
        Some(Held { id, why, queued })
    });
    let waiting = behind.chain(held).collect();
    // a jump's rule beside its rank, where `dash orch next` prints the order (task rust-his-priority-jumps)
    for (id, why) in &mut order {
        if let Some((_, _, words)) = jumped.iter().find(|(_, j, _)| j == id) {
            *why = format!("{why}; {words}");
        }
    }
    Plan {
        seats: queue.seats,
        seats_why: queue.seats_why,
        next,
        waiting,
        order,
        his: vec![],
        jumped: jumped
            .into_iter()
            .map(|(_, id, words)| (id, words))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Only the older judge's no-report words read as unreported (/code-review of rust-rejudge-merged-closes); dash-bridge's
    /// `no_report_is_the_older_judge_s_own_words` ties them to pm.ex.
    #[dash_test]
    fn only_the_no_report_words_read_as_unreported() {
        let words = format!("{NO_REPORT}, as your brief says) or the TASK_DONE block");
        assert!(Verdict::NotMet(words).unreported());
        assert!(!Verdict::NotMet("part 2".into()).unreported());
        assert!(!Verdict::Met.unreported());
    }

    #[dash_test]
    fn a_wait_both_the_row_and_the_list_name_is_counted_once() {
        let queued = Queued {
            held: None,
            id: "b".into(),
            title: String::new(),
            reopened_ms: None,
            waits: vec!["a".into(), "c".into(), "a".into()],
            layer: None,
            builds_nothing: false,
            rank: Default::default(),
            free: None,
            demand: Default::default(),
            route: None,
            ..Queued::default()
        };
        assert_eq!(all_waits(None, Some(&queued)), ["a", "c"]);
        assert_eq!(
            waits_unmet(&[], &["x".into()]).as_deref(),
            Some("waits on x (not a Rust task)")
        );
    }

    /// Done-when 5 of rust-orch-pick-dynamic: the order is computed from each task's fields, never the list's stored order.
    /// Real open Rust tasks of 2026-11-10, their priority, estimate and epic as the older app holds them, filed here in its
    /// stored order (`pos`), which the rank ignores: one given a deadline comes first, then Now's by priority, what waits on
    /// them and estimate, then a Later item's, then one on no item. A blocker ranks as high as what waits on it.
    #[dash_test]
    fn the_order_is_tier_deadline_priority_waits_then_estimate_whatever_the_list_s_order() {
        let real =
            |id: &str, item: Option<(&str, &str)>, priority: u8, estimate: Option<u32>| Queued {
                id: id.into(),
                rank: Rank {
                    horizon: item.map(|i| i.1.to_string()),
                    item: item.map(|i| i.0.to_string()),
                    priority: Some(priority),
                    estimate,
                    deadline: None,
                    impact: None,
                    his: false,
                },
                ..Queued::default()
            };
        let now = Some(("rust-poc", "now"));
        let later = Some(("dash-sqlite", "later"));
        let mut tasks = vec![
            real("search-reaches-flare", later, 2, Some(3)),
            real("rust-merge-gate-one-rule", None, 1, None),
            real("rust-orch-pick-dynamic", now, 1, Some(5)),
            real("rust-table-sortable", now, 1, Some(5)),
            real("rust-tasks-no-sync-note", now, 2, Some(1)),
            real("rust-menu-text", now, 2, Some(2)),
            real("rust-ui-controls", now, 1, Some(5)),
            real("rust-graph-queue-edges", now, 2, Some(5)),
            real("schedules-visible", later, 2, Some(5)),
        ];
        // rust-orch-pick-dynamic was blocked by rust-graph-queue-edges: the priority 2 blocker ranks at its priority 1
        tasks[2].waits = vec!["rust-graph-queue-edges".into()];
        // an urgent task: priority 1 with a deadline, whatever its item, waiting on a Later task, which goes first
        tasks[3].rank.deadline = Some("2026-11-12".into());
        tasks[3].waits = vec!["search-reaches-flare".into()];
        let why = rank(&mut tasks, &[]);
        let said: Vec<String> = why.iter().map(|(id, why)| format!("{id}: {why}")).collect();
        assert_eq!(
            said,
            [
                "search-reaches-flare: urgent, due 2026-11-12, priority 1, as rust-table-sortable waits on it, 1 wait on it, \
                 estimate 3, impact none",
                "rust-table-sortable: urgent, due 2026-11-12, priority 1, 0 wait on it, estimate 5, impact none",
                "rust-graph-queue-edges: now (rust-poc), priority 1, as rust-orch-pick-dynamic waits on it, 1 wait on it, \
                 estimate 5, impact none",
                "rust-orch-pick-dynamic: now (rust-poc), priority 1, 0 wait on it, estimate 5, impact none",
                "rust-ui-controls: now (rust-poc), priority 1, 0 wait on it, estimate 5, impact none",
                "rust-tasks-no-sync-note: now (rust-poc), priority 2, 0 wait on it, estimate 1, impact none",
                "rust-menu-text: now (rust-poc), priority 2, 0 wait on it, estimate 2, impact none",
                "schedules-visible: later (dash-sqlite), priority 2, 0 wait on it, estimate 5, impact none",
                "rust-merge-gate-one-rule: on no roadmap item, priority 1, 0 wait on it, estimate none, impact none",
            ]
        );
        let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
        // the same tasks in any stored order rank the same: nothing of the list's order is read
        tasks.reverse();
        rank(&mut tasks, &[]);
        assert_eq!(tasks.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), ids);
        // the plan picks the head, its order printed; a claimed task leaves the order, as it leaves the pick
        let claimed = Standing {
            id: "rust-ui-controls".into(),
            status: "claimed".into(),
            ..Standing::default()
        };
        let plan = plan(
            &First,
            Queue {
                seats: 5,
                seats_why: String::new(),
                tasks,
            },
            &[claimed],
            0,
            vec![],
            None,
        );
        assert_eq!(
            plan.next,
            Pick::Start {
                id: "search-reaches-flare".into(),
                account: None,
                pool: None,
            }
        );
        let open: Vec<_> = why
            .into_iter()
            .filter(|(id, _)| id != "rust-ui-controls")
            .collect();
        assert_eq!(plan.order, open);
    }

    /// Done-when 2 of rust-task-impact: of two tasks at one priority, the one retiring more Elixir lines goes first, whatever
    /// its estimate or name; at equal lines, more 20d rows met; one with no impact after both. A higher priority still wins.
    #[dash_test]
    fn equal_priorities_go_by_impact_elixir_lines_retired_first() {
        let task = |id: &str, priority: u8, impact: Option<&str>| Queued {
            id: id.into(),
            rank: Rank {
                priority: Some(priority),
                estimate: Some(1),
                impact: impact.and_then(Impact::of),
                ..Rank::default()
            },
            ..Queued::default()
        };
        let mut tasks = vec![
            task("a-rows", 2, Some("0 3 10 20b")),
            task("b-none", 2, None),
            task(
                "c-documents",
                2,
                Some("1200 Dashboard.Documents /documents 10"),
            ),
            task("d-urgent", 1, None),
            task("e-small", 2, Some("40 Dashboard.Small")),
            // `--impact none` sorts with a task that has none kept, by name, never above it
            task("a-named-none", 2, Some("0")),
        ];
        let why = rank(&mut tasks, &[]);
        let ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "d-urgent",
                "c-documents",
                "e-small",
                "a-rows",
                "a-named-none",
                "b-none"
            ]
        );
        let said: Vec<String> = why.iter().map(|(id, why)| format!("{id}: {why}")).collect();
        assert_eq!(
            said[1],
            "c-documents: on no roadmap item, priority 2, 0 wait on it, estimate 1, impact retires 1200 Elixir lines \
             (Dashboard.Documents, /documents), meets 1 20d row(s) (10)"
        );
        assert!(
            said[3].ends_with("impact meets 3 20d row(s) (3, 10, 20b)"),
            "{}",
            said[3]
        );
        assert!(said[4].ends_with("impact none named"), "{}", said[4]);
        assert!(said[5].ends_with("impact none"), "{}", said[5]);
        // what the store keeps reads back the same
        let kept = Impact::of("1200 Dashboard.Documents /documents 10").expect("read");
        assert_eq!(kept.stored(), "1200 Dashboard.Documents /documents 10");
        assert_eq!(
            Impact::of("0").map(|i| i.said()).as_deref(),
            Some("none named")
        );
    }

    /// Task rust-pick-handbacks-first, done-when 2: a hand-back's layer never holds it back, a run on that layer included; the
    /// task it shares the layer with waits instead, ranked first or not, until the hand-back starts. One he holds lends none.
    #[dash_test]
    fn a_hand_back_passes_its_layer_lock_and_the_task_sharing_it_waits() {
        let task = |id: &str, priority: u8| Queued {
            id: id.into(),
            layer: Some("model:task".into()),
            rank: Rank {
                priority: Some(priority),
                ..Rank::default()
            },
            ..Queued::default()
        };
        let row = |id: &str, status: &str, handed: Option<&str>| Standing {
            id: id.into(),
            status: status.into(),
            handed: handed.map(String::from),
            back: handed.is_some(),
            ..Standing::default()
        };
        let tasks = vec![task("new", 1), task("running", 1), task("pr-1363", 4)];
        let rows = [
            row("running", "claimed", None),
            row("pr-1363", "open", Some("abc 1")),
            // finished with a hand-back head kept from before: not live, so no hold
            row("old", "finished", Some("zzz 1")),
        ];
        let queue = |tasks: Vec<Queued>| Queue {
            seats: 5,
            seats_why: String::new(),
            tasks,
        };
        let plan_of = |tasks| plan(&First, queue(tasks), &rows, 0, vec![], None);
        let got = plan_of(tasks.clone());
        let id = "pr-1363".to_string();
        assert_eq!(
            got.next,
            Pick::Start {
                id,
                account: None,
                pool: None
            }
        );
        let why = |plan: &Plan, id: &str| {
            let held = plan.waiting.iter().find(|h| h.id == id);
            held.map(|h| h.why.clone())
        };
        assert_eq!(
            why(&got, "new").as_deref(),
            Some("pr-1363 was handed back on model:task and starts first")
        );
        // with no run on the layer, the new task still waits for the hand-back
        let rows_free = [rows[1].clone()];
        let free = plan(&First, queue(tasks.clone()), &rows_free, 0, vec![], None);
        assert!(matches!(&free.next, Pick::Start { id, .. } if id == "pr-1363"));
        assert_eq!(
            why(&free, "new").as_deref(),
            Some("pr-1363 was handed back on model:task and starts first")
        );
        let starts =
            |plan: &Plan, want: &str| matches!(&plan.next, Pick::Start { id, .. } if id == want);
        // a hand-back he holds (dragged to Backlog) holds no one: the new task starts
        let mut held = tasks.clone();
        held[2].held = Some("he moved it to Backlog".into());
        let got = plan(&First, queue(held), &rows_free, 0, vec![], None);
        assert!(starts(&got, "new"), "{:?}", got.next);
        // nor one whose wait is unmet, which could not start
        let mut waiting = tasks.clone();
        waiting[2].waits = vec!["x".into()];
        let got = plan(&First, queue(waiting.clone()), &rows_free, 0, vec![], None);
        assert!(starts(&got, "new"), "{:?}", got.next);
        // what a hand-back waits on is lifted with it, ahead of new work, and says why
        waiting.push(Queued {
            id: "x".into(),
            rank: Rank {
                priority: Some(4),
                ..Rank::default()
            },
            ..Queued::default()
        });
        let order = plan(&First, queue(waiting), &rows_free, 0, vec![], None).order;
        let ids: Vec<&str> = order.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["x", "pr-1363", "new", "running"]);
        assert!(
            order[0]
                .1
                .starts_with("a hand-back waits on it, so it starts first; "),
            "{order:?}"
        );
        // a head kept from a hand-back whose run finished since is no hand-back: it ranks by priority, holding nothing
        let stale = [Standing {
            back: false,
            ..rows[1].clone()
        }];
        let got = plan(&First, queue(tasks), &stale, 0, vec![], None);
        assert!(starts(&got, "new"), "{:?}", got.next);
        assert!(!got.order[1].1.contains("handed back"), "{:?}", got.order);
    }

    /// The first task offered, whatever the seats.
    struct First;

    impl Policy for First {
        fn pick(&self, snapshot: &Snapshot) -> Pick {
            let first = snapshot.queue.tasks.first();
            first.map_or(Pick::Wait("nothing".into()), |t| Pick::Start {
                id: t.id.clone(),
                account: None,
                pool: None,
            })
        }
        fn after(&self, _: &Verdict, _: u32, _: bool) -> Step {
            Step::Keep
        }
        fn retry(&self, _: u32) -> bool {
            false
        }
    }

    /// Done-when 4 of rust-task-names-layer: the picker never runs two tasks naming the same layer at once.
    #[dash_test]
    fn a_layer_a_run_changes_holds_every_other_task_naming_it() {
        let task = |id: &str, layer: Option<&str>| Queued {
            id: id.into(),
            layer: layer.map(String::from),
            ..Queued::default()
        };
        let queue = Queue {
            seats: 5,
            seats_why: String::new(),
            tasks: vec![
                task("a", Some("design:board")),
                task("b", Some("design:board")),
                task("c", Some("model:task")),
                task("d", None),
            ],
        };
        let standing = |id: &str, status: &str| Standing {
            id: id.into(),
            status: status.into(),
            ..Standing::default()
        };
        let rows = [standing("a", "claimed"), standing("b", "open")];
        let plan = plan(&First, queue.clone(), &rows, 1, vec![], None);
        let start = |id: &str| Pick::Start {
            id: id.into(),
            account: None,
            pool: None,
        };
        assert_eq!(plan.next, start("c"));
        let b = plan.waiting.iter().find(|h| h.id == "b").expect("b held");
        assert_eq!(b.why, "a changes design:board now");
        // the run over, its layer is free again
        let rows = [standing("a", "finished"), standing("b", "open")];
        assert_eq!(plan_of(&queue, &rows), start("b"));
        // nothing claimed: the first task naming it may start
        assert_eq!(plan_of(&queue, &[]), start("a"));
    }

    /// Done-when 1 of rust-his-priority-jumps: a priority-1 task whose why quotes him starts beside its layer's running task;
    /// his words at another priority, or priority 1 without them, wait as before, and one jump holds the next one's.
    #[dash_test]
    fn his_priority_one_starts_beside_its_layer_s_running_task() {
        let task = |id: &str, priority: u8, why: &str| Queued {
            id: id.into(),
            layer: Some("rfc:tools".into()),
            rank: Rank {
                priority: Some(priority),
                his: quotes_him(why),
                ..Rank::default()
            },
            ..Queued::default()
        };
        let his = "(@nha, 2026-11-14: « an awful long time »)";
        let standing = |id: &str, status: &str| Standing {
            id: id.into(),
            status: status.into(),
            ..Standing::default()
        };
        let start = |id: &str| Pick::Start {
            id: id.into(),
            account: None,
            pool: None,
        };
        let queue = |tasks| Queue {
            seats: 5,
            seats_why: String::new(),
            tasks,
        };
        // rfc:tools' run at work: its priority-1 task in his words starts, and the plan names the rule it passed by
        let jumping = queue(vec![
            task("run", 2, ""),
            task(
                "not-his",
                1,
                "a why of ours, citing him further on (@nha, 2026-11-01: « later »)",
            ),
            task("his-p2", 2, his),
            task("his-p1", 1, his),
        ]);
        let rows = [standing("run", "claimed")];
        let picked = plan(&First, jumping.clone(), &rows, 1, vec![], None);
        assert_eq!(picked.next, start("his-p1"));
        let rule = "beside run, at work on rfc:tools: priority 1 in his words passes the one-task-per-layer rule (task rust-his-priority-jumps)";
        assert_eq!(
            picked.jumped,
            vec![("his-p1".to_string(), rule.to_string())]
        );
        // and beside its rank, as `dash orch next` prints the order
        let (_, ranked) = picked
            .order
            .iter()
            .find(|(id, _)| id == "his-p1")
            .expect("ranked");
        assert!(ranked.ends_with(&format!("; {rule}")), "{ranked}");
        // once claimed beside the run, its run names the same rule; a task that did not jump names none
        let both = [standing("run", "claimed"), standing("his-p1", "claimed")];
        assert_eq!(
            jumping.jumped_at_claim(&both, "his-p1"),
            Some(rule.to_string())
        );
        assert_eq!(jumping.jumped_at_claim(&both, "not-his"), None);
        assert_eq!(
            jumping.jumped_at_claim(&[standing("his-p1", "claimed")], "his-p1"),
            None
        );
        // priority 1 without his words, or his words at priority 2: held by the layer as before
        for id in ["not-his", "his-p2"] {
            let held = picked.waiting.iter().find(|h| h.id == id).expect("held");
            assert_eq!(held.why, "run changes rfc:tools now", "{id}");
        }
        // nothing at work on it: nothing jumps, the rule is not named
        let free = plan(
            &First,
            queue(vec![task("his-p1", 1, his)]),
            &[],
            0,
            vec![],
            None,
        );
        assert_eq!((free.next, free.jumped), (start("his-p1"), vec![]));
        // the jump at work: a second one waits behind it, one jump a layer
        let rows = [standing("run", "claimed"), standing("his-p1", "claimed")];
        let second = queue(vec![
            task("run", 2, ""),
            task("his-p1", 1, his),
            task("next", 1, his),
        ]);
        let held = plan(&First, second, &rows, 2, vec![], None);
        let next = held.waiting.iter().find(|h| h.id == "next").expect("held");
        assert_eq!(next.why, "his-p1 changes rfc:tools now");
        assert_eq!(held.jumped, vec![]);
        // two jumps in one plan: the first passes, the second waits for it
        let rows = [standing("run", "claimed")];
        let two = queue(vec![
            task("run", 2, ""),
            task("a", 1, his),
            task("b", 1, his),
        ]);
        let one = plan(&First, two, &rows, 1, vec![], None);
        assert_eq!(one.next, start("a"));
        let b = one.waiting.iter().find(|h| h.id == "b").expect("held");
        assert_eq!(b.why, "a jumps rfc:tools first");
        // a hand-back about to start on its layer: no jump passes it
        let back = Standing {
            back: true,
            handed: Some("abc 1".into()),
            ..standing("back", "open")
        };
        let rows = [standing("run", "claimed"), back];
        let behind = queue(vec![
            task("run", 2, ""),
            task("back", 3, ""),
            task("his-p1", 1, his),
        ]);
        let waits = plan(&First, behind, &rows, 1, vec![], None);
        let held = waits
            .waiting
            .iter()
            .find(|h| h.id == "his-p1")
            .expect("held");
        assert_eq!(
            held.why,
            "back was handed back on rfc:tools and starts first"
        );
        assert_eq!(waits.jumped, vec![]);
    }

    /// Done-when 3 of task-dedupe: a start is refused while another open task naming the same file is at work, its PR in
    /// review, or handed back; a task never started holds nothing, so two naming one file never hold each other.
    #[dash_test]
    fn a_file_a_started_task_names_holds_every_other_task_naming_it() {
        let task = |id: &str, files: &[&str]| Queued {
            id: id.into(),
            files: files.iter().map(|f| f.to_string()).collect(),
            ..Queued::default()
        };
        let queue = Queue {
            seats: 5,
            seats_why: String::new(),
            tasks: vec![
                task("a", &["rust/crates/dash/src/main.rs"]),
                task("b", &["rust/crates/dash/src/main.rs", "x.rs"]),
                task("c", &["rust/crates/dash/src/other.rs"]),
            ],
        };
        let standing = |id: &str, status: &str, outcome: Option<&str>| Standing {
            id: id.into(),
            status: status.into(),
            outcome: outcome.map(String::from),
            ..Standing::default()
        };
        let start = |id: &str| Pick::Start {
            id: id.into(),
            account: None,
            pool: None,
        };
        let held = |rows: &[Standing]| {
            let plan = plan(&First, queue.clone(), rows, 1, vec![], None);
            let b = plan
                .waiting
                .iter()
                .find(|h| h.id == "b")
                .map(|h| h.why.clone());
            (plan.next, b)
        };
        let why = Some(
            "a names rust/crates/dash/src/main.rs too and is at work or in review".to_string(),
        );
        // at work
        assert_eq!(
            held(&[standing("a", "claimed", None)]),
            (start("c"), why.clone())
        );
        // its PR ready for review
        assert_eq!(held(&[standing("a", "finished", Some("done"))]).1, why);
        // handed back: its PR is open
        let mut back = standing("a", "open", None);
        (back.back, back.handed) = (true, Some("abc 1".into()));
        assert_eq!(held(&[back.clone()]).1, why);
        // ... and a hand-back is never held by the file itself
        let mut b_back = standing("b", "open", None);
        (b_back.back, b_back.handed) = (true, Some("def 1".into()));
        let both = [standing("a", "claimed", None), b_back];
        let held_b = plan(&First, queue.clone(), &both, 1, vec![], None);
        assert!(
            held_b
                .waiting
                .iter()
                .all(|h| h.id != "b" || !h.why.contains("names")),
            "{:?}",
            held_b.waiting
        );
        // queued, never started (its row open, or no row): a holds nothing, so two naming one file never hold each other; a
        // starts, b waits only behind the pick
        for rows in [vec![standing("a", "open", None)], vec![]] {
            let queued = plan(&First, queue.clone(), &rows, 1, vec![], None);
            let whys: Vec<(&str, &str)> = (queued.waiting.iter())
                .map(|h| (h.id.as_str(), h.why.as_str()))
                .collect();
            println!(
                "never started, rows {rows:?}: next {:?}, waiting {whys:?}",
                queued.next
            );
            assert_eq!(queued.next, start("a"));
            assert_eq!(whys, [("b", QUEUED), ("c", QUEUED)]);
        }
        // merged, failed or never started: nothing held, the first may start
        assert_eq!(
            held(&[standing("a", "finished", Some("merged"))]),
            (start("b"), None)
        );
        assert_eq!(plan_of(&queue, &[]), start("a"));
    }

    fn plan_of(queue: &Queue, rows: &[Standing]) -> Pick {
        plan(&First, queue.clone(), rows, 0, vec![], None).next
    }

    /// Done-when 3 of rust-task-names-layer: a run whose diff draws a page outside the design system is held, whatever the
    /// judge said, once its task names a layer.
    #[dash_test]
    fn a_met_run_whose_diff_draws_a_page_outside_the_design_system_is_held() {
        let diff = "diff --git a/rust/crates/dash/src/home.rs b/rust/crates/dash/src/home.rs
--- a/rust/crates/dash/src/home.rs
+++ b/rust/crates/dash/src/home.rs
@@ -1,2 +1,12 @@
 fn page() {}
+    rsx! { div { class: \"my-card\", \"hi\" } }
+    let x = \"<span>own</span>\";
+    let y = 2 < 3;
+    /// an `rsx! {` named in a doc comment, a `class: \"x\"` too
+    let usage = \"<id> --title <text>\";
+    let shape = Shape { style: s, class: c };
+    let words = [\"rsx!\", \"#[component]\", \"class:\"];
+++ x
+    rsx! { b { \"after a line that reads as a header\" } }
 #[cfg(test)]
 mod tests {
+    rsx! { div { class: \"in-a-test\" } }
 }
diff --git a/rust/crates/surface/src/components.rs b/rust/crates/surface/src/components.rs
--- a/rust/crates/surface/src/components.rs
+++ b/rust/crates/surface/src/components.rs
@@ -1 +1,2 @@
+    rsx! { div { class: \"li-card\" } }
diff --git a/rust/crates/dash/src/page.css b/rust/crates/dash/src/page.css
--- /dev/null
+++ b/rust/crates/dash/src/page.css
@@ -0,0 +1 @@
+.mine { color: red }
diff --git a/rust/crates/dash/tests/pages.rs b/rust/crates/dash/tests/pages.rs
+++ b/rust/crates/dash/tests/pages.rs
@@ -0,0 +1 @@
+    rsx! { button {} }
diff --git a/rust/vendor/blitz-dom/assets/default.css b/rust/vendor/blitz-dom/assets/default.css
--- /dev/null
+++ b/rust/vendor/blitz-dom/assets/default.css
@@ -0,0 +1 @@
+@namespace url(http://www.w3.org/1999/xhtml);
";
        let found = page_local_lines(diff);
        assert_eq!(
            found,
            [
                "rust/crates/dash/src/home.rs: rsx! { div { class: \"my-card\", \"hi\" } }",
                "rust/crates/dash/src/home.rs: let x = \"<span>own</span>\";",
                "rust/crates/dash/src/home.rs: rsx! { b { \"after a line that reads as a header\" } }",
                "rust/crates/dash/src/page.css: .mine { color: red }",
            ]
        );
        let held = held_by_layer(Verdict::Met, Some("design:board"), diff);
        let Verdict::NotMet(why) = held else {
            panic!("held: {held:?}")
        };
        assert!(why.starts_with("it names design:board, and its diff draws a page outside"));
        assert!(why.contains("4 line(s)"), "{why}");
        // the design system's own change, or a task that names none, or a verdict other than met: as the judge said
        let inside = "diff --git a/x b/x\n+++ b/rust/crates/surface/src/components.rs\n@@ -1 +1 @@\n+ rsx! {}\n";
        assert_eq!(
            held_by_layer(Verdict::Met, Some("design:board"), inside),
            Verdict::Met
        );
        assert_eq!(held_by_layer(Verdict::Met, None, diff), Verdict::Met);
        let blocked = Verdict::Blocked("his login".into());
        assert_eq!(
            held_by_layer(blocked.clone(), Some("design:board"), diff),
            blocked
        );
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task hot-files-fable: files ranked by [fix] PRs (squash merges), a file counted once a PR, a feat or a straight push to
    /// main never; past 3 a PR touching it waits on a reviewer and says why.
    #[dash_test]
    fn fix_prs_rank_files_and_one_past_three_holds_its_pr() {
        let commit =
            |subject: &str, files: &[&str]| format!("\x1e{subject}\n\n{}\n", files.join("\n"));
        let mut log = String::new();
        for n in 1..=4 {
            log += &commit(&format!("[fix][t{n}] a fix (#{n})"), &["a.rs", "b.rs"]);
        }
        log += &commit("[fix][t5] a fix (#5)", &["b.rs", "c.rs"]);
        log += &commit("[feat][t6] a feat (#6)", &["c.rs"]);
        log += &commit("[fix] pushed to main", &["c.rs"]);
        let counts = fix_counts(&log);
        let named = |v: &[(String, usize)]| {
            v.iter()
                .map(|(f, n)| format!("{f}:{n}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(named(&counts), ["b.rs:5", "a.rs:4", "c.rs:1"]);
        let held = hot(&counts, ["a.rs", "c.rs"]);
        assert_eq!(named(&held), ["a.rs:4"]);
        assert_eq!(
            hot_why(&held).as_deref(),
            Some("it touches a file fixed more than 3 times in 14 days (a.rs: 4)")
        );
        let three = [("d.rs".to_string(), 3)];
        assert_eq!(hot(&three, ["d.rs"]), [], "3 is not past 3");
    }

    /// Task rust-land-waits-review: a send path or `review true` names why a PR waits on its reviewer; a verdict comment reads
    /// back as posted, and any other comment is none.
    #[dash_test]
    fn a_send_path_needs_review_and_a_verdict_reads_back_as_posted() {
        let files = |f: &[&str]| f.iter().map(|f| f.to_string()).collect::<Vec<_>>();
        assert_eq!(review_needed(&[], false), None);
        assert_eq!(
            review_needed(&files(&["rust/crates/mail/src/jmap.rs"]), true).as_deref(),
            Some("it touches a send path (rust/crates/mail/src/jmap.rs)")
        );
        assert_eq!(
            review_needed(&[], true).as_deref(),
            Some("its task says review true")
        );
        for review in [
            Review::Pass("clean".into()),
            Review::Fix("a double send\nand more".into()),
        ] {
            let body = verdict_comment(&review, "abc123");
            assert_eq!(verdict_of(&body), Some((review, "abc123".into())));
        }
        assert_eq!(verdict_of("LGTM"), None);
        assert_eq!(verdict_of("Review verdict: maybe abc"), None);
        assert_eq!(
            verdict_of("Review verdict: pass"),
            None,
            "no head, no verdict"
        );
    }
}

#[cfg(test)]
mod pr_title_tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 1 and 2 of review-brief-own-task-id: PR 900's title names its own task, which its reviewer's brief takes; a PR
    /// whose title names none falls back to pr-<repo>-<n>, and a link that is no GitHub PR to none.
    #[dash_test]
    fn pr_900_s_title_names_its_own_task_else_pr_repo_n() {
        let url = "https://github.com/nha-org/site_perso/pull/900";
        let title =
            "[fix][review-claim-after-release] dash review claim takes a PR whose reviewer ended";
        assert_eq!(
            pr_task_id(title, url).as_deref(),
            Some("review-claim-after-release")
        );
        assert_eq!(
            pr_task_id("Bump deps", url).as_deref(),
            Some("pr-site_perso-900")
        );
        assert_eq!(
            pr_task_id("[bug][x] T", "https://github.com/nha/klar/pull/12?x=1").as_deref(),
            Some("pr-klar-12")
        );
        assert_eq!(
            pr_task_id("[fix][a b] T", "https://example.com/pull/3"),
            None
        );
        // bin/pr-title-gate's own pattern: a word after « ] », and only a github.com link falls back
        assert_eq!(title_task("[fix][foo]"), None);
        assert_eq!(title_task("[fix][foo]bar"), None);
        assert_eq!(title_task("[fix][foo] bar").as_deref(), Some("foo"));
        assert_eq!(pr_task_id("", "https://notgithub.com/x/y/pull/3"), None);
    }

    /// Done-when 3: PR 1107's title names rust-registry-ext, another real task than its reviewer's review-pr-site_perso-1107:
    /// it passes, and so does any merged PR; an unknown id or no id is held while open.
    #[dash_test]
    fn pr_1107_s_title_naming_another_real_task_passes_and_any_merged_pr() {
        let title = "[feat][rust-registry-ext] A crate, a Steel script and an MCP pod each add an extractor";
        let real = |id: &str| id == "rust-registry-ext";
        assert_eq!(title_refusal(title, false, real), None);
        assert_eq!(title_refusal("Bump deps", true, real), None);
        assert_eq!(
            title_refusal("[fix][gone-task] T", false, real).as_deref(),
            Some("the title names « gone-task », no task in the store")
        );
        assert!(title_refusal("Bump deps", false, real).is_some_and(|w| w.contains("must read")));
    }

    /// Task load-relief-starts, done-when 2's word check (/code-review): whole words name the cost, a why given wins over the
    /// filed one, and a task the list holds no open row for is told to give --why, never read as an empty why.
    #[dash_test]
    fn relief_needs_a_why_naming_load_ci_or_build_in_whole_words() {
        let args = |why: &str| vec!["--why".to_string(), why.to_string()];
        let none = || Ok(None);
        assert!(relief_refused("t", &args("CI holds every PR an hour."), none).is_none());
        assert!(relief_refused("t", &args("the 5-minute load past 3"), none).is_none());
        let no = relief_refused("t", &args("He wants the page loading faster"), none);
        assert!(no.is_some_and(|w| w.contains("names no load, CI or build cost")));
        let filed = || Ok(Some("cargo builds twice".to_string()));
        assert!(relief_refused("t", &[], filed).is_none());
        let unread = relief_refused("t", &[], none).expect("refused");
        assert!(
            unread.contains("no open task t to read its why from: give --why"),
            "{unread}"
        );
    }
}
