//! The binary's wiring of section 20b's orchestrator: the store, the older app's list and judge, herdr, the approved
//! `make claude`, Claude Code, git and GitHub, on orch. One process per run: a run that crashes takes its claim's lock with it
//! and nothing else, and `dash orch` frees it on its next look.
/// Where the browser finds the front door, the older app's pages behind it: a link the window opens there, and the live page a
/// deploy reads « Built from » on. The Makefile's RUST_PORT.
pub(crate) const FRONT_DOOR: &str = "http://127.0.0.1:7788";
use crate::kinds::{Agents, ByKind};
use crate::reporting::{reporting, say_memory};
use dash_agent::ClaudeCode;
use dash_agent::codex::Codex;
use dash_bridge::{Dash, Issue, Open};
use dash_core::tasks::{
    Agent, Claim, Forge, Launcher, Limit, Outcome, Pick, Plan, Queue, Standing, TaskList,
    TaskStore, Terminal, plan,
};
use dash_forge::GitHub;
use dash_launcher::{ApprovedCodex, ApprovedMake, mxc};
use dash_meter::cap::STOP_CODE;
use dash_orch::{Pace, Ports, hand_backs, next, run_task, seats_taken};
use dash_store::tasks::{self as store, AgentRow, Event, Row, SqliteTasks};
use dash_surface::{self as surface, Drawing, TaskView};
use dash_terminal::Herdr;
use dash_views::Views;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PACE: Pace = Pace {
    poll: Duration::from_secs(2),
    // session.sh's wait for the prompt.
    start_limit: Duration::from_secs(3 * 60),
    limit: Duration::from_secs(8 * 60 * 60),
    // Past it nothing moves the session: a minute of Enters, then a margin for a slow start.
    stuck_limit: Duration::from_secs(3 * 60),
    stop_polls: 15,
    // Once a minute, as dash ci wait: its 3 s kin hit GitHub's rate limit (gh-meter).
    ci_poll: Duration::from_secs(60),
    ci_limit: Duration::from_secs(3 * 60 * 60),
    judge_poll: Duration::from_secs(60),
    // A wait on him can last a night: each look spawns a tool and rereads a whole transcript.
    patient_poll: Duration::from_secs(60),
    // A limit its readings show waits on them; this bounds one they do not.
    limit_retry: Duration::from_secs(30 * 60),
};

/// A look for what to start. A task runs long, so nothing here needs to be faster than a person's look.
const ORCH_POLL: Duration = Duration::from_secs(60);
/// Looks between two hand-back passes: each reads every ended row's PR on GitHub, too dear at each minute's look.
const HAND_BACK_EVERY: u32 = 10;
/// Looks between two worktree reaps, an hour: each reads every idle clean worktree's PR on GitHub. Its run's own `leave` is the
/// quick path; this one catches the rest within the hour (task worktrees-under-5g).
const REAP_EVERY: u32 = 60;

fn or_exit<T>(result: Result<T, String>) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    })
}

/// The tools of the real thing. `DASH_REPO`, `DASH_WORKTREES`, `DASH_CLI` name another checkout and app; the app's own
/// CLI takes `DASHBOARD_URL`, so a branch's app answers it.
struct Tools {
    list: Dash,
    terminal: Herdr,
    launcher: ApprovedMake,
    codex: ApprovedCodex,
    agents: Agents,
    forge: GitHub,
    policy: ByKind<fn(&str) -> bool>,
}

fn home() -> String {
    or_exit(std::env::var("HOME").map_err(|e| format!("HOME: {e}")))
}

fn path(var: &str, default: &str) -> PathBuf {
    std::env::var(var).map_or_else(|_| Path::new(&home()).join(default), PathBuf::from)
}

pub(crate) fn repo() -> PathBuf {
    path("DASH_REPO", "repo/site_perso")
}

/// The folder the task worktrees sit in, and with them each claim's lock.
fn worktrees() -> PathBuf {
    path("DASH_WORKTREES", ".herdr/worktrees/site_perso")
}

fn this_folder() -> PathBuf {
    or_exit(std::env::current_dir().map_err(|e| format!("this folder: {e}")))
}

pub(crate) fn open_tasks(dir: &Path) -> Result<SqliteTasks, String> {
    store::open(dir, &worktrees())
}

/// The older app's database, where it files tasks today (its config/dev.exs, under `DASHBOARD_LIVE` the main checkout).
pub(crate) fn older_db() -> PathBuf {
    repo().join("private/dashboard/dashboard_dev.db")
}

/// One sync of the list `db` holds into `tasks`, the first step of each pass: what the older app filed is pickable within it.
/// A read that fails leaves the last list synced.
pub(crate) fn sync_list(tasks: &mut SqliteTasks, db: &Path) -> Result<usize, String> {
    tasks.sync(&dash_store::older::queue(db)?)
}

/// The data folder of the running tool, outside git: what `dash serve --port <n>` reads, and `make rust/serve` names.
pub fn live_data() -> PathBuf {
    repo().join("private/live/rust/data")
}

pub(crate) fn older_app() -> Dash {
    let bin = std::env::var("DASH_CLI");
    let bin = bin.unwrap_or_else(|_| {
        repo()
            .join("private/dashboard/bin/dash")
            .display()
            .to_string()
    });
    Dash { bin }
}

fn claude_code(home: &str) -> ClaudeCode {
    let claude = Path::new(home).join(".claude");
    ClaudeCode {
        projects: claude.join("projects"),
        limits: claude.join("weekly-limit"),
    }
}

fn codex(home: &str) -> Codex {
    Codex {
        bin: "codex".into(),
        sessions: Path::new(home).join(".codex/sessions"),
    }
}

/// A failed read is Claude's, the kind a row names by default: the run reads its kind again, and refuses one it cannot read.
fn is_codex(id: &str) -> bool {
    older_app().agent(id).is_ok_and(|kind| kind == "codex")
}

fn tools(data: &Path) -> Tools {
    let home = home();
    let repo = repo();
    Tools {
        list: older_app(),
        terminal: Herdr::from_env(),
        launcher: ApprovedMake { home: home.clone() },
        codex: ApprovedCodex {
            home: home.clone(),
            opener: dash_agent::codex::OPENER,
        },
        agents: Agents::new(claude_code(&home), codex(&home), data),
        forge: GitHub {
            repo,
            root: worktrees(),
            base: "main".into(),
            // the build line alone (task rust-home-prompt): Home lost it, and no page's layout moves it again
            page: format!("{FRONT_DOOR}/built"),
            cli: "gh".into(),
        },
        policy: ByKind { is_codex },
    }
}

/// `agent` and `launcher`: a run's kind, or the picker's, which reads every kind's limits and launches nothing.
fn ports<'a>(
    tasks: &'a mut SqliteTasks,
    tools: &'a Tools,
    agent: &'a dyn Agent,
    launcher: &'a dyn Launcher,
    step: &'a dyn Fn(&str, Duration),
) -> Ports<'a> {
    Ports {
        tasks,
        list: &tools.list,
        terminal: &tools.terminal,
        launcher,
        agent,
        forge: &tools.forge,
        policy: &tools.policy,
        step,
    }
}

/// `dash limits`: each kind's limits as the picker reads them; exit 1 when a kind's read failed, after printing the others.
pub fn limits(mut out: File) {
    let home = home();
    let keys = dash_store::keys::open(&live_data()).and_then(|k| k.list());
    let reads = [
        ("claude", claude_code(&home).limits()),
        ("codex", codex(&home).limits()),
        (
            "key",
            keys.map(|keys| keys.iter().map(|k| k.limit()).collect()),
        ),
    ];
    let mut failed = false;
    for (kind, read) in reads {
        match read {
            Ok(limits) => {
                for l in limits {
                    let line = match l.key {
                        true => format!("key {}: {}% of its cap spent", l.account, l.week),
                        false => format!(
                            "{kind} {}: 5 hours {}%, week {}%, week resets {}",
                            l.account, l.five_hours, l.week, l.week_resets
                        ),
                    };
                    writeln!(out, "{line}").expect("stdout");
                }
            }
            Err(e) => {
                failed = true;
                writeln!(out, "{kind}: {e}").expect("stdout");
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// Rust's list, and when it was last synced (Unix milliseconds).
type QueueAt = (Result<Queue, String>, i64);

#[derive(Default)]
struct QueueRead {
    last: Option<QueueAt>,
    /// What a task's page shows that this store does not hold, read with it.
    issues: Option<Result<Vec<Issue>, String>>,
    /// The sessions, each kind's spend and which queued tasks are Codex's, read with it: what a pick reads beside the queue.
    seats: Option<Result<Seats, String>>,
    reading: bool,
}

/// One of the older app's reads as it printed it, or why it did not.
type Words = Result<String, String>;

/// The older app's reads the pages show, each polled apart (task rust-home-prompt): /inbox's handoff and daily, /plan's roadmap,
/// /insights' feedback count and cost fit.
#[derive(Default)]
struct Reads {
    handoff: Arc<Mutex<Polled<Words>>>,
    progress: Arc<Mutex<Polled<Words>>>,
    roadmap: Arc<Mutex<Polled<Words>>>,
    feedback: Arc<Mutex<Polled<Words>>>,
    cost: Arc<Mutex<Polled<Words>>>,
}

/// A read apart from the queue's, started only by the page that shows it: a page's reads, the board's every task. A /tasks
/// tab or the window costs the older app no /inbox read, /inbox no `task ls`, and no read waits on another's (/code-review of
/// rust-nav-board and of rust-nav-polish).
struct Polled<T> {
    last: Option<T>,
    at: i64,
    reading: bool,
}

impl<T> Default for Polled<T> {
    fn default() -> Self {
        Polled {
            last: None,
            at: 0,
            reading: false,
        }
    }
}

/// Every project's open tasks, as `dash task ls` lists them (task rust-nav-polish).
type EveryRead = Result<Vec<Open>, String>;

/// A read that failed keeps the last words read: one slow answer never blanks what waits on him.
fn kept(
    new: Result<String, String>,
    old: Option<&Result<String, String>>,
) -> Result<String, String> {
    match (new, old) {
        (Err(_), Some(Ok(said))) => Ok(said.clone()),
        (new, _) => new,
    }
}

type Seats = (usize, Vec<Limit>, HashSet<String>);

/// What the page at / reads: this store's runs, and what the older app says Rust may pick next.
pub struct Board {
    tasks: SqliteTasks,
    /// Section 6's views, fed by this store's op log: /tasks' columns are the board view's. Started again if their thread ends.
    views: Mutex<Views>,
    queue: Arc<Mutex<QueueRead>>,
    every: Arc<Mutex<Polled<EveryRead>>>,
    data: PathBuf,
}

impl Board {
    pub fn open(dir: &Path) -> Result<Board, String> {
        let tasks = open_tasks(dir)?;
        let views = Mutex::new(board_views(open_tasks(dir)?)?);
        let (queue, every) = (Arc::default(), Arc::default());
        let data = dir.to_path_buf();
        Ok(Board {
            tasks,
            views,
            queue,
            every,
            data,
        })
    }

    /// The data folder the board was opened on: the server's, where /drafts keeps its table and /watchers reads its beats.
    pub(crate) fn data(&self) -> &Path {
        &self.data
    }

    /// The board view's rows. Views that fail are started again once, so one failure never takes /tasks down until a restart.
    fn board_view(&self) -> Result<Vec<dash_views::rules::Out>, String> {
        let mut views = self.views.lock().unwrap_or_else(|e| e.into_inner());
        views.rows("board").or_else(|why| {
            eprintln!("dash serve: the views failed ({why}), started again");
            *views = board_views(open_tasks(&self.data)?)?;
            views.rows("board")
        })
    }

    fn look(&self) -> Result<Look, String> {
        let rows = self.tasks.rows()?;
        let columns = columns(self.board_view()?);
        let standing = dash_core::tasks::TaskStore::standing(&self.tasks)?;
        let now_ms = now_ms();
        let (queue, issues, seats) = {
            let mut queue = self.queue.lock().expect("the queue");
            let stale = queue
                .last
                .as_ref()
                .is_none_or(|(_, at)| now_ms - at >= ORCH_POLL.as_millis() as i64);
            if stale && !queue.reading {
                let claims = dash_core::tasks::TaskStore::claims(&self.tasks)?;
                queue.reading = self.read_queue(claims);
            }
            (
                queue.last.clone(),
                queue.issues.clone(),
                queue.seats.clone(),
            )
        };
        let plan = match (&queue, seats) {
            (Some((Ok(queue), _)), Some(Ok((sessions, limits, codex)))) => Some(Ok(plan(
                &ByKind {
                    is_codex: |id: &str| codex.contains(id),
                },
                queue.clone(),
                &standing,
                sessions,
                limits,
            ))),
            (Some((Ok(_), _)), Some(Err(e))) => Some(Err(e)),
            _ => None,
        };
        Ok(Look {
            rows,
            columns,
            standing,
            now_ms,
            queue,
            issues,
            plan,
            every: None,
        })
    }

    /// /tasks as the board or the list (task rust-nav-board): every project's open tasks, or `project`'s alone. The graph is
    /// `graph`'s.
    pub fn tasks(&self, view: TaskView, project: Option<&str>) -> Result<Drawing, String> {
        let mut look = self.look()?;
        look.every = self.read_every();
        // only site_perso's board shows what runs (/code-review of rust-home-prompt)
        let runs = match project.is_none_or(|p| p == "site_perso") {
            true => self.runs(&look),
            false => String::new(),
        };
        Ok(list_drawing(&look, view, project, &runs))
    }

    /// The board's dependency view, at /tasks?layout=graph and /graph.
    pub fn graph(&self) -> Result<Drawing, String> {
        Ok(graph_drawing(&self.look()?))
    }

    /// The page at /tasks/<id>; `None`: no open task of that id.
    pub fn task(&self, id: &str) -> Result<Option<Drawing>, String> {
        let look = self.look()?;
        let page = |tree| Drawing::Tree {
            title: format!("Rust task: {id}"),
            tree,
        };
        // a history not read leaves the rest of the page standing, and says so (/code-review of rust-task-page-linear)
        let history = self.tasks.history(id);
        Ok(task_tree(&look, id, &history).map(page))
    }

    /// Whether `id` is a task this board lists, from what has answered: only such a task's old link opens its page here. Before
    /// the older app's list answers, an id it alone would list is not one yet (/code-review of rust-task-page-linear).
    pub fn holds(&self, id: &str) -> Result<bool, String> {
        Ok(listed(&self.look()?)
            .iter()
            .any(|t| t.id == id && t.other.is_none()))
    }

    /// Only while someone reads the page, so a closed page costs the older app nothing. On its own thread: a read can take its
    /// 60 s limit, and the store's loop is serial. A page's skeleton fills once it ends; true when the thread started.
    fn read_queue(&self, claims: Vec<Claim>) -> bool {
        let shared = Arc::clone(&self.queue);
        let data = self.data.clone();
        let started = std::thread::Builder::new().spawn(move || {
            // As the picker's pass: this store's list, synced first. A sync that fails keeps the last list and the time it was
            // synced, so the page's note ages and says the sync stopped.
            let synced = open_tasks(&data).and_then(|mut tasks| {
                let sync = sync_list(&mut tasks, &older_db());
                TaskStore::queue(&tasks).map(|queue| (queue, sync))
            });
            let last_at = match &shared.lock().expect("the queue").last {
                Some((Ok(_), at)) => Some(*at),
                _ => None,
            };
            let (read, at) = match (synced, last_at) {
                (Ok((queue, Ok(_))), _) => (Ok(queue), now_ms()),
                (Ok((queue, Err(_))), Some(at)) => (Ok(queue), at),
                // Its reason shown where it is read: the picker keeps the stored list, and the orch line says so too.
                (Ok((_, Err(e))), None) => (Err(format!("its sync failed: {e}")), now_ms()),
                (Err(e), _) => (Err(e), now_ms()),
            };
            let answered = read.is_ok();
            let ids: Vec<String> = read
                .iter()
                .flat_map(|q| &q.tasks)
                .map(|t| t.id.clone())
                .collect();
            // Shown at once: a slow terminal never holds back the older app's line.
            shared.lock().expect("the queue").last = Some((read, at));
            // No queue, no plan: nothing else is asked of the terminal.
            let seats = answered.then(|| {
                let sessions = seats_taken(&Herdr::from_env(), &claims)?;
                // No HOME is no spend known. `home()` would exit, and from this thread that ends the whole server.
                let home = std::env::var("HOME");
                // The picker's own agent, so the page names the start the picker makes. ponytail: a Codex read and a kind read
                // per queued task at each of the page's looks, a minute apart; the picker's cache if those show.
                let limits = home.map(|home| {
                    let agents = Agents::new(claude_code(&home), codex(&home), &data);
                    agents.limits().unwrap_or_default()
                });
                let codex = ids.into_iter().filter(|id| is_codex(id)).collect();
                Ok((sessions, limits.unwrap_or_default(), codex))
            });
            shared.lock().expect("the queue").seats = seats;
            // Last, so a slow answer holds back no Next line; not after a queue that failed, whose wait would only double.
            let issues = answered.then(|| older_app().issues());
            let mut queue = shared.lock().expect("the queue");
            // A queue that failed keeps the last words read: one slow answer never blanks every page (/code-review of #1197).
            match issues {
                Some(issues) => queue.issues = Some(issues),
                None => {
                    queue
                        .issues
                        .get_or_insert_with(|| Err("its queue did not answer".into()));
                }
            }
            queue.reading = false;
        });
        started.is_ok()
    }

    /// The last list of every project's open tasks, a new one started on its own thread once a minute while /tasks is read;
    /// a read that failed keeps the last rows read.
    fn read_every(&self) -> Option<EveryRead> {
        let mut state = self.every.lock().expect("every");
        let stale = state.last.is_none() || now_ms() - state.at >= ORCH_POLL.as_millis() as i64;
        if stale && !state.reading {
            let shared = Arc::clone(&self.every);
            let started = std::thread::Builder::new().spawn(move || {
                let read = older_app().every_task();
                let mut state = shared.lock().expect("every");
                if read.is_ok() || !matches!(state.last, Some(Ok(_))) {
                    state.last = Some(read);
                }
                state.at = now_ms();
                state.reading = false;
            });
            state.reading = started.is_ok();
        }
        state.last.clone()
    }

    /// What runs: the agents, what the plan holds back, the runs table and the build line, last.
    fn runs(&self, look: &Look) -> String {
        let plan = match &look.plan {
            // the tree's own Next line says the rest, a plan not known included: never twice (/code-review)
            Some(Ok(plan)) => waiting_text(plan),
            _ => String::new(),
        };
        // The agents' read failing costs their lines only, never the runs'.
        let agents = match self.tasks.agents() {
            Ok(agents) => agents_text(&agents, look.now_ms),
            Err(e) => format!("Agents: {}\n\n", not_known(&e)),
        };
        let above = agents + &plan;
        page(
            &look.rows,
            look.queue.as_ref(),
            &above,
            look.now_ms,
            option_env!("DASH_COMMIT"),
        )
    }
}

/// The reads outside the board, so /inbox, /plan and /insights never wait on its lock (/code-review of rust-home-prompt).
static READS: std::sync::LazyLock<Reads> = std::sync::LazyLock::new(Reads::default);

/// How often /insights' reads start again: `dash feedback count` took 48 s on 2026-11-09, and the older app collects its
/// tables every five minutes, so a minute's poll would keep it busy (/code-review of rust-home-prompt).
const SLOW_READ: Duration = Duration::from_secs(600);

/// /inbox: what waits on him, then today's daily (task rust-home-prompt: Home's first two parts).
pub fn inbox_page() -> Drawing {
    let handoff = read_words(&READS.handoff, ORCH_POLL, || older_app().handoff());
    let progress = read_words(&READS.progress, ORCH_POLL, || older_app().progress());
    let waits = |s: &str| crate::home::cards(s, crate::home::handoff_link);
    let daily = |s: &str| crate::home::cards(s, crate::home::progress_link);
    let parts: [Part; 2] = [
        ("waits", "Waiting on you", handoff.as_ref(), &waits),
        ("daily", "Today's daily", progress.as_ref(), &daily),
    ];
    read_page("Inbox", &parts)
}

/// /plan: his focuses by horizon, Urgent, Now, Next, Later (task rust-home-prompt).
pub fn plan_page() -> Drawing {
    let roadmap = read_words(&READS.roadmap, ORCH_POLL, || older_app().roadmap());
    let parts: [Part; 1] = [(
        "roadmap",
        "The roadmap",
        roadmap.as_ref(),
        &crate::home::roadmap,
    )];
    read_page("Plan", &parts)
}

/// /insights: his feedback against the work done, and what a task costs per estimate point (task rust-home-prompt).
pub fn insights_page() -> Drawing {
    let feedback = read_words(&READS.feedback, SLOW_READ, || older_app().feedback_count());
    let cost = read_words(&READS.cost, SLOW_READ, || older_app().cost_fit());
    let parts: [Part; 2] = [
        (
            "feedback",
            "His feedback and the work done",
            feedback.as_ref(),
            &crate::home::facts,
        ),
        (
            "cost",
            "Cost per estimate point",
            cost.as_ref(),
            &crate::home::facts,
        ),
    ];
    read_page("Insights", &parts)
}

/// One read's last words, a new read started on its own thread once `every` has passed while its page is read. A read that
/// failed, or panicked, keeps the last words read: one slow answer never blanks a page, and a panic never stops the polling.
fn read_words(
    state: &Arc<Mutex<Polled<Words>>>,
    every: Duration,
    read: fn() -> Words,
) -> Option<Words> {
    let mut held = state.lock().expect("a read");
    // a read that failed tries again on the minute's poll, never waits a slow read's interval (task rust-home-prompt's live check)
    let every = match held.last {
        Some(Err(_)) => every.min(ORCH_POLL),
        _ => every,
    };
    let stale = held.last.is_none() || now_ms() - held.at >= every.as_millis() as i64;
    if stale && !held.reading {
        let shared = Arc::clone(state);
        let started = std::thread::Builder::new().spawn(move || {
            let new =
                std::panic::catch_unwind(read).unwrap_or_else(|_| Err("its read panicked".into()));
            let mut held = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            held.last = Some(kept(new, held.last.as_ref()));
            held.at = now_ms();
            held.reading = false;
        });
        held.reading = started.is_ok();
    }
    held.last.clone()
}

/// What both pages read, in one look. `plan`: `None` until the queue has answered, an error when the sessions did not.
struct Look {
    rows: Vec<Row>,
    /// The board view's column of each Rust task, by its id on one line.
    columns: HashMap<String, &'static str>,
    standing: Vec<Standing>,
    now_ms: i64,
    queue: Option<QueueAt>,
    /// `None` until the older app has answered.
    issues: Option<Result<Vec<Issue>, String>>,
    plan: Option<Result<Plan, String>>,
    /// `None` until the older app has listed every project's open tasks.
    every: Option<Result<Vec<Open>, String>>,
}

/// The views on their own thread, fed by this store's op log (`tasks` its own connection: the views read while a page writes).
fn board_views(tasks: SqliteTasks) -> Result<Views, String> {
    Views::start(Box::new(move |after| tasks.ops_after(after, 10_000)))
}

/// The board view's rows as /tasks' columns, whose words are the surface's statuses too.
fn columns(board: Vec<dash_views::rules::Out>) -> HashMap<String, &'static str> {
    let column = |word: &str| dash_views::rules::COLUMNS.into_iter().find(|c| *c == word);
    let ids = board
        .into_iter()
        .map(|(_, id, word, _)| (one_line(&id), column(&word)));
    ids.filter_map(|(id, c)| Some((id, c?))).collect()
}

pub(crate) fn now_ms() -> i64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    now.map_or(0, |d| d.as_millis() as i64)
}

fn age(ms: i64) -> String {
    match ms / 1000 {
        s if s < 60 => format!("{} s ago", s.max(0)),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86400),
    }
}

/// The first pull request link in what a run said: a merge says only that, a session's report says it among other words,
/// often in markdown. Exactly `<owner>/<repo>/pull/<digits>`, cut there, so `/pull/12/files` and `(…/pull/12).` are #12.
/// ponytail: a report citing another PR first shows that one; a `pr` field in the store once a run records it.
fn pr_in(said: &str) -> Option<&str> {
    const SITE: &str = "https://github.com/";
    let name = |w: &str| {
        !w.is_empty()
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    said.match_indices(SITE).find_map(|(start, _)| {
        let rest = &said[start..];
        let mut parts = rest[SITE.len()..].splitn(4, '/');
        let (owner, repo, pull, tail) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let digits = tail.chars().take_while(char::is_ascii_digit).count();
        let ends = !tail[digits..].starts_with(|c: char| c.is_ascii_alphanumeric());
        let good = name(owner) && name(repo) && pull == "pull" && digits > 0 && ends;
        good.then(|| &rest[..SITE.len() + owner.len() + repo.len() + "//pull/".len() + digits])
    })
}

/// Words a session or the older app wrote, on one line: a control character (newline, tab, a terminal's escape), a line separator
/// or an invisible format character (bidi override, zero width) is a space, so they cannot fake a row or rewrite what is shown.
/// std has no Unicode category test, so these are spelled. crates/gate's `graph::one_line` is its copy: dash links gate only to build.
fn one_line(words: &str) -> String {
    let hidden = |c: char| {
        c.is_control()
            || matches!(c, '\u{ad}' | '\u{61c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
    };
    words
        .chars()
        .map(|c| if hidden(c) { ' ' } else { c })
        .collect()
}

/// A line, tab-separated as /search answers. Every field goes through `one_line`: an id comes from the older app's queue or an
/// argument, `words`, last, is a session's first line or a queued task's title.
fn line(
    id: &str,
    state: &str,
    tries: u32,
    last: &str,
    outcome: &str,
    pr: &str,
    words: &str,
) -> String {
    let tries = tries.to_string();
    let fields = [id, state, &tries, last, outcome, pr, words].map(one_line);
    format!("{}\n", fields.join("\t"))
}

/// A claim in a word: whatever its row says, one whose lock is free runs nowhere.
fn claim_word(gone: bool) -> &'static str {
    if gone { "run gone" } else { "running" }
}

/// What a run's session waits on, for a claimed row: it is alive and nothing has ended.
fn waits_on(outcome: Option<&str>) -> Option<&'static str> {
    match outcome {
        Some("offline") => Some("waits on the network"),
        Some("limited") => Some("waits on its limit"),
        _ => None,
    }
}

/// A row's last run: what it said, how it ended, when.
struct LastRun<'a> {
    said: &'a str,
    outcome: &'a str,
    when: String,
}

fn last_run(row: &Row, now_ms: i64) -> LastRun<'_> {
    // claim() keeps the last attempt's outcome and words: a running task shows neither. A claim whose lock is free runs nowhere,
    // whatever its row says: a crash, or a reboot with no `dash orch` to free it.
    let running = row.status == "claimed";
    let said = if running {
        ""
    } else {
        row.said.as_deref().unwrap_or_default()
    };
    let outcome = if row.run_gone {
        claim_word(true)
    } else if running {
        waits_on(row.outcome.as_deref()).unwrap_or(claim_word(false))
    } else {
        row.outcome.as_deref().unwrap_or("none yet")
    };
    // No try, no run: what `add` wrote, or what a reopen forgot.
    let when = if row.attempts == 0 {
        "never".to_string()
    } else {
        age(now_ms - row.last_ms)
    };
    LastRun {
        said,
        outcome,
        when,
    }
}

fn row_line(row: &Row, now_ms: i64) -> String {
    let run = last_run(row, now_ms);
    let first: String = run
        .said
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect();
    let pr = pr_in(run.said).unwrap_or_default();
    line(
        &row.id,
        &row.status,
        row.attempts,
        &run.when,
        run.outcome,
        pr,
        &first,
    )
}

fn not_known(e: &str) -> String {
    format!(
        "Next: not known, the sessions did not answer: {}",
        one_line(e)
    )
}

fn next_line(plan: &Plan) -> String {
    match &plan.next {
        Pick::Start { id, .. } => format!("Next: {}", one_line(id)),
        Pick::Wait(why) => format!("Next: nothing, {}", one_line(why)),
    }
}

/// What runs next and why the others wait, as `plan` says it: the words the picker acts on. The page says each half apart.
#[cfg(test)]
fn plan_text(plan: &Plan) -> String {
    format!("{}\n{}", next_line(plan), waiting_text(plan))
}

/// What the plan holds back and why, one a line: the list's tree has its Next line already.
fn waiting_text(plan: &Plan) -> String {
    let waiting: String = plan
        .waiting
        .iter()
        .map(|held| format!("{}\t{}\n", one_line(&held.id), one_line(&held.why)))
        .collect();
    format!("{waiting}\n")
}

/// Each open task and what it waits on, the plan's own words on each. A finished task is drawn only as what an open one names, with
/// no edge of its own: the past stays out of the picture.
fn graph_tasks(look: &Look) -> Vec<Value> {
    let plan = look.plan.as_ref().and_then(|p| p.as_ref().ok());
    let queue = look.queue.as_ref().and_then(|(q, _)| q.as_ref().ok());
    // By id on one line, the drawn name: two raw ids that read the same are one task, the first row standing for both, since a
    // graph refuses a name twice.
    let mut rows = HashMap::new();
    for row in &look.standing {
        rows.entry(one_line(&row.id)).or_insert(row);
    }
    let mut whys = HashMap::new();
    for held in plan.iter().flat_map(|p| &p.waiting) {
        whys.entry(one_line(&held.id)).or_insert(held.why.as_str());
    }
    // Finished as the store says: a reopen is the store's to judge (`reopen_held`, with the open-task cap), and the next
    // `dash orch` pass resets the row; a copy of that rule here disagreed with it (second /simplify of #1197).
    let finished = |id: &str| rows.get(id).is_some_and(|r| r.status == "finished");
    let next = plan.and_then(|p| match &p.next {
        Pick::Start { id, .. } => Some(id.as_str()),
        Pick::Wait(_) => None,
    });
    let held = plan.iter().flat_map(|p| &p.waiting).map(|h| h.id.as_str());
    let open = look.standing.iter().map(|r| r.id.as_str());
    let offered = queue.iter().flat_map(|q| &q.tasks).map(|t| t.id.as_str());
    let mut ids: Vec<String> = vec![];
    let mut seen = HashSet::new();
    for id in next.into_iter().chain(held).chain(open).chain(offered) {
        let id = one_line(id);
        if !finished(&id) && seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    // Grows as it goes: what a task names may name more.
    let mut at = 0;
    while let Some(id) = ids.get(at) {
        let row = rows.get(id.as_str()).filter(|r| r.status != "finished");
        for wait in row.iter().flat_map(|r| &r.waits).map(|w| one_line(w)) {
            if seen.insert(wait.clone()) {
                ids.push(wait);
            }
        }
        at += 1;
    }
    let next = next.map(one_line);
    let pending = look.plan.is_none() && !queue_failed(look);
    let says = |id: &str| -> (&str, String) {
        let row = rows.get(id);
        if next.as_deref() == Some(id) {
            return ("next", "the next pick".into());
        }
        if finished(id) {
            let outcome = row.and_then(|r| r.outcome.as_deref());
            let merged = outcome == Some("merged");
            let state = if merged { "finished" } else { "ended" };
            return (state, outcome.unwrap_or("finished").into());
        }
        if row.is_some_and(|r| r.status == "claimed") {
            let gone = (look.rows.iter()).any(|r| r.run_gone && one_line(&r.id) == id);
            let waits = waits_on(row.and_then(|r| r.outcome.as_deref())).filter(|_| !gone);
            if let Some(waits) = waits {
                return ("waiting", waits.into());
            }
            let state = if gone { "waiting" } else { "running" };
            return (state, claim_word(gone).into());
        }
        let known = match plan {
            Some(_) => "no row",
            None if pending => "not known yet",
            None => "not known",
        };
        ("waiting", whys.get(id).copied().unwrap_or(known).into())
    };
    let task = |id: &String| {
        let (state, words) = says(id);
        let row = rows.get(id.as_str()).filter(|_| !finished(id));
        let waits: Vec<String> = row
            .iter()
            .flat_map(|r| &r.waits)
            .map(|w| one_line(w))
            .collect();
        json!({"id": id, "state": state, "says": one_line(&words), "waits": waits})
    };
    ids.iter().map(task).collect()
}

fn queue_failed(look: &Look) -> bool {
    matches!(look.queue, Some((Err(_), _)))
}

/// /graph's page.
fn graph_drawing(look: &Look) -> Drawing {
    Drawing::Tree {
        title: "Rust tasks: the graph".into(),
        tree: graph_tree(look),
    }
}

/// /tasks' board or list, `project`'s tasks alone when named.
fn list_drawing(look: &Look, view: TaskView, project: Option<&str>, runs: &str) -> Drawing {
    Drawing::Tree {
        title: "Tasks".into(),
        tree: list_tree(look, view, project, runs),
    }
}

/// The screen at /graph as a UI tree: what runs next, how old that is, the graph, then each waiting task's whole why, which a node
/// may clip. With no graph to draw it says why in its place, and the rest stands.
fn graph_tree(look: &Look) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let tasks = graph_tasks(look);
    let graph = match tasks.len() {
        0 => text(none_open(look).into()),
        n if n > surface::GRAPH_LIMIT => text(format!(
            "{n} tasks: past the {} a graph draws. /, the list, shows them.",
            surface::GRAPH_LIMIT
        )),
        _ => json!({"type": "task-graph", "props": {"tasks": tasks}}),
    };
    let mut elements = Map::new();
    let heading = json!({"type": "heading", "props": {"text": "Rust tasks"}});
    elements.insert("title".into(), heading);
    elements.insert("views".into(), views(TaskView::Graph, None));
    elements.insert("next".into(), next_words(look));
    elements.insert("note".into(), note(look));
    elements.insert("graph".into(), graph);
    let mut children: Vec<String> = ["title", "views", "next", "note", "graph"]
        .map(String::from)
        .into();
    let held = look.plan.iter().flatten().flat_map(|p| &p.waiting);
    for (i, h) in held.enumerate() {
        let key = format!("why-{i}");
        let words = format!("{}: {}", one_line(&h.id), one_line(&h.why));
        elements.insert(key.clone(), text(words));
        children.push(key);
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// What runs next as the page's element: a skeleton while the plan is read (task rust-skeleton-live).
fn next_words(look: &Look) -> Value {
    let words = match &look.plan {
        Some(Ok(plan)) => next_line(plan),
        Some(Err(e)) => not_known(e),
        None if queue_failed(look) => "Next: not known, Rust's list is not known.".into(),
        None => return surface::skeleton("Next"),
    };
    json!({"type": "text", "props": {"text": words}})
}

fn none_open(look: &Look) -> &'static str {
    match look.queue {
        Some((Ok(_), _)) => "No Rust task open.",
        _ => "This store has no task open, and Rust's list is not known yet.",
    }
}

/// An open task as the list shows it. `status`: a surface `issue-status`.
struct Listed {
    id: String,
    status: &'static str,
    says: String,
    /// Another project's task (or one of site_perso's Rust does not own): its project and title, as `dash task ls` lists it.
    other: Option<(String, String)>,
}

/// A project's status word as the board's column: each project names its statuses its own way (« open », « To Do »,
/// « Active »). ponytail: the words `dash task ls` prints today; a new word lands in Backlog until it is named here.
fn column_of(word: &str) -> &'static str {
    match word.to_ascii_lowercase().as_str() {
        "in review" | "in-review" | "review" => "in-review",
        "in progress" | "in-progress" | "active" | "ongoing" | "claimed" | "running" => {
            "in-progress"
        }
        "open" | "to do" | "todo" => "todo",
        _ => "backlog",
    }
}

/// The older app's words on a task, whichever of its ids reads as this one.
fn issue_of<'a>(look: &'a Look, id: &str) -> Option<&'a Issue> {
    let issues = look.issues.as_ref().and_then(|i| i.as_ref().ok());
    issues.and_then(|issues| issues.iter().find(|i| one_line(&i.id) == id))
}

/// Its title: the older app's list, else its queue, which answers first; else its id.
fn title_of(look: &Look, id: &str) -> String {
    let queue = look.queue.as_ref().and_then(|(q, _)| q.as_ref().ok());
    let queued = queue.and_then(|q| q.tasks.iter().find(|t| one_line(&t.id) == id));
    let title = issue_of(look, id).map(|i| i.title.as_str());
    let title = title.or(queued.map(|t| t.title.as_str()));
    one_line(title.filter(|t| !t.is_empty()).unwrap_or(id))
}

fn row_of<'a>(look: &'a Look, id: &str) -> Option<&'a Row> {
    look.rows.iter().find(|r| one_line(&r.id) == id)
}

/// Each open task, in pick order: the plan's own, as the graph reads it (next, then what waits), its status beside it, its words
/// the plan's; with no plan known, the queue's order, which the plan would follow. After them what the older app holds open that no
/// plan names: held there, or its run over here and not yet merged.
fn listed(look: &Look) -> Vec<Listed> {
    let queue = look.queue.as_ref().and_then(|(q, _)| q.as_ref().ok());
    let mut all = vec![];
    for task in graph_tasks(look) {
        let word = |field: &str| task[field].as_str().unwrap_or_default().to_string();
        let (id, state) = (word("id"), word("state"));
        let status = match state.as_str() {
            // Named by an open task, its own run over: the past stays out, unless the older app holds it open, below.
            "finished" | "ended" => continue,
            // The board view's (section 6): a claimed run, gone or not, in progress; offered with its waits merged, to do.
            _ => look.columns.get(&id).copied().unwrap_or("backlog"),
        };
        let says = word("says");
        all.push(Listed {
            id,
            status,
            says,
            other: None,
        });
    }
    if !matches!(look.plan, Some(Ok(_))) {
        let tasks = queue.iter().flat_map(|q| &q.tasks).enumerate();
        let order: HashMap<String, usize> = tasks.map(|(at, t)| (one_line(&t.id), at)).collect();
        all.sort_by_key(|t: &Listed| order.get(&t.id).copied().unwrap_or(usize::MAX));
    }
    let issues = look.issues.as_ref().and_then(|i| i.as_ref().ok());
    for issue in issues.into_iter().flatten() {
        let id = one_line(&issue.id);
        // an issue with no id has no page to link to (/code-review of #1197)
        if id.is_empty() || all.iter().any(|t: &Listed| t.id == id) {
            continue;
        }
        let run = row_of(look, &id).map(|row| last_run(row, look.now_ms));
        let (status, says) = match run {
            Some(run) => match pr_in(run.said) {
                Some(pr) => ("in-review", format!("{}, {}", run.outcome, pr_name(pr))),
                None => ("backlog", format!("its run ended: {}", run.outcome)),
            },
            None => ("backlog", "the list does not offer it".into()),
        };
        let says = one_line(&says);
        all.push(Listed {
            id,
            status,
            says,
            other: None,
        });
    }
    all
}

fn pr_name(url: &str) -> String {
    format!("PR #{}", url.rsplit('/').next().unwrap_or_default())
}

/// Words a person or a session wrote, their lines kept: each as safe as `one_line`.
fn lines_kept(words: &str) -> String {
    let lines: Vec<String> = words.lines().map(one_line).collect();
    lines.join("\n")
}

/// Why a task's words are missing, when they are; none while the older app's list is read, the page's skeleton then.
fn no_words(look: &Look) -> Option<String> {
    Some(match &look.issues {
        None => return None,
        // A bad answer's error quotes all of it: a line here, the whole in the log of whoever runs the command.
        Some(Err(e)) => {
            let e: String = one_line(e).chars().take(200).collect();
            format!("The older app did not answer: {e}")
        }
        Some(Ok(_)) => "The older app holds no such task.".into(),
    })
}

fn views(current: TaskView, project: Option<&str>) -> Value {
    json!({"type": "task-views", "props": {"current": current, "project": project}})
}

/// The open tasks /tasks lists: Rust's own first, with what their runs say, then every other open task `dash task ls`
/// lists, each in its project's column (task rust-nav-polish; the orchestrator's answer, 2026-11-08: « 1, and each sidebar
/// project opens Rust's board filtered to it »). `project`: that project's alone. Rust's own are site_perso's.
fn every_listed(look: &Look, project: Option<&str>) -> Vec<Listed> {
    let ours = project.is_none_or(|p| p == "site_perso");
    let mut all = if ours { listed(look) } else { vec![] };
    let every = look.every.as_ref().and_then(|e| e.as_ref().ok());
    for open in every.into_iter().flatten() {
        let id = one_line(&open.id);
        let mine =
            open.project == "site_perso" && all.iter().any(|t| t.other.is_none() && t.id == id);
        if id.is_empty() || mine || project.is_some_and(|p| p != open.project) {
            continue;
        }
        // the column says the status; unfiltered, the row names its project
        let says = one_line(if project.is_some() {
            &open.status
        } else {
            &open.project
        });
        let other = Some((open.project.clone(), one_line(&open.title)));
        all.push(Listed {
            id,
            status: column_of(&open.status),
            says,
            other,
        });
    }
    all
}

/// The screen at /tasks as a UI tree: the views, what runs next, how old that is, then the open tasks by status, each in pick
/// order. The board puts the statuses side by side, each one there even when empty, as Linear's columns; the list stacks the
/// ones that hold a task. A task on a comment says so on its row, as `Dashboard.Board.Inbox` on the older board: no panel
/// beside the list (done-when 3 of rust-linear-view). `project`: the one project /tasks is kept to.
/// `runs`: what runs and the build line, site_perso's, last (task rust-home-prompt: Home's last part, which `make rust/serves`
/// reads here).
fn list_tree(look: &Look, view: TaskView, project: Option<&str>, runs: &str) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut elements = Map::new();
    let title = match project {
        None => "Tasks".to_string(),
        Some(p) => format!("Tasks: {}", one_line(p)),
    };
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": title}}),
    );
    elements.insert("views".into(), views(view, project));
    let mut children = vec!["title".to_string(), "views".into()];
    // what Rust runs next is site_perso's: another project's board has no such line
    if project.is_none_or(|p| p == "site_perso") {
        elements.insert("next".into(), next_words(look));
        elements.insert("note".into(), note(look));
        children.extend(["next", "note"].map(String::from));
        if let Some(Err(_)) = &look.issues {
            elements.insert("words".into(), text(no_words(look).unwrap_or_default()));
            children.push("words".into());
        }
    }
    let others = match &look.every {
        Some(Ok(_)) => None,
        None => Some(surface::skeleton("The other projects' tasks")),
        Some(Err(e)) => {
            let e: String = one_line(e).chars().take(200).collect();
            Some(text(format!(
                "The other projects' tasks are not shown: the older app did not answer: {e}"
            )))
        }
    };
    if let Some(others) = others {
        elements.insert("every".into(), others);
        children.push("every".into());
    }
    let tasks = every_listed(look, project);
    let board = view == TaskView::Board;
    let mut groups = vec![];
    for status in ["in-review", "in-progress", "todo", "backlog"] {
        let rows: Vec<(usize, &Listed)> = (tasks.iter().enumerate())
            .filter(|(_, t)| t.status == status)
            .collect();
        if rows.is_empty() && !board {
            continue;
        }
        let mut names = vec![];
        for (i, task) in rows {
            let (project, title) = match &task.other {
                Some((project, title)) => (Some(project.as_str()), title.clone()),
                None => (None, title_of(look, &task.id)),
            };
            let comments = if project.is_some() {
                0
            } else {
                issue_of(look, &task.id).map_or(0, |i| i.comments.len())
            };
            // a card's label is its project and its assignee who runs it: Rust's own run here, through `dash orch`; another
            // project's task is no one's here (task rust-board-linear). Cards only: the list's rows show neither, and its text
            // says what its HTML shows (/code-review). ponytail: no owner or label is read from the older app.
            let labels: Vec<&str> = board
                .then(|| project.unwrap_or("site_perso"))
                .into_iter()
                .collect();
            let assignee = (board && project.is_none()).then_some("Rust");
            let row = json!({"type": "issue-row", "props": {
                "id": task.id, "title": title, "status": status, "says": task.says, "comments": comments, "project": project,
                "labels": labels, "assignee": assignee,
            }});
            let name = format!("row-{i}");
            elements.insert(name.clone(), row);
            names.push(name);
        }
        let group = json!({"type": "issue-group", "props": {"status": status}, "children": names});
        elements.insert(status.into(), group);
        groups.push(status.to_string());
    }
    if board {
        elements.insert("board".into(), json!({"type": "board", "children": groups}));
        children.push("board".into());
    } else {
        children.extend(groups);
    }
    if tasks.is_empty() {
        // The queue leaves out what is held: only the list can say none is open.
        let none = match look.issues {
            Some(Ok(_)) => none_open(look),
            _ => "This store has no task open, and the older app's list has not answered.",
        };
        elements.insert("none".into(), text(none.into()));
        children.push("none".into());
    }
    if project.is_none_or(|p| p == "site_perso") {
        let quote = json!({"type": "quote", "props": {"text": runs.trim_end()}});
        elements.insert("runs-text".into(), quote);
        let section =
            json!({"type": "section", "props": {"title": "What runs"}, "children": ["runs-text"]});
        elements.insert("runs".into(), section);
        children.push("runs".into());
    }
    let page = json!({"type": "stack", "children": children});
    elements.insert("page".into(), page);
    json!({"root": "page", "elements": elements}).to_string()
}

/// A part of a read page: its key, its title, the older app's words, and how they become cards.
type Part<'a> = (
    &'a str,
    &'a str,
    Option<&'a Words>,
    &'a dyn Fn(&str) -> crate::home::Read,
);

/// One of the older app's reads as cards under `part`, its parts' headings sections of their own; else why it is missing.
fn read_part(
    elements: &mut Map<String, Value>,
    part: &str,
    // its section's title, what its skeleton says is read (/code-review: two « Its cards » told no part from the other)
    title: &str,
    read: Option<&Words>,
    parse: &dyn Fn(&str) -> crate::home::Read,
) -> Vec<String> {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut put = |name: String, value: Value| {
        elements.insert(name.clone(), value);
        name
    };
    let said = match read {
        Some(Ok(said)) => said,
        Some(Err(e)) => {
            let e: String = one_line(e).chars().take(200).collect();
            return vec![put(
                format!("{part}-text"),
                text(format!("The older app did not answer: {e}")),
            )];
        }
        None => {
            return vec![put(format!("{part}-text"), surface::skeleton(title))];
        }
    };
    let read = parse(said);
    let mut children = vec![];
    if !read.lead.is_empty() {
        children.push(put(format!("{part}-lead"), text(one_line(&read.lead))));
    }
    for (g, group) in read.groups.iter().enumerate() {
        let mut cards = vec![];
        for (c, card) in group.cards.iter().enumerate() {
            let props = json!({"title": one_line(&card.title), "line": one_line(&card.line), "href": card.href});
            cards.push(put(
                format!("{part}-{g}-{c}"),
                json!({"type": "card", "props": props}),
            ));
        }
        // a horizon of /plan with no focus says so under its title
        let cards = match cards.is_empty() {
            true => put(format!("{part}-{g}"), text("Nothing.".into())),
            false => put(
                format!("{part}-{g}"),
                json!({"type": "cards", "children": cards}),
            ),
        };
        children.push(match group.title.as_str() {
            "" => cards,
            title => put(format!("{part}-{g}-part"), json!({"type": "section", "props": {"title": one_line(title)}, "children": [cards]})),
        });
    }
    if children.is_empty() {
        children.push(put(format!("{part}-text"), text("Nothing.".into())));
    }
    children
}

/// Home at / (task rust-home-prompt; @nha, 2026-11-09: « Home should be clean. Not a full clutter of stuff. in fact a
/// google/openai-like prompt should be all, plus a couple visual suggestions. »): the compass's prompt, whose words go to the
/// comment queue as a comment on /, and three cards to the pages that took Home's parts. It reads nothing, so it never waits.
pub fn home_page() -> Drawing {
    let card = |title: &str, line: &str, href: &str| json!({"type": "card", "props": {"title": title, "line": line, "href": href}});
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["ask", "suggestions"]},
        "ask": {"type": "compose", "props": {
            "action": "/compass", "send": "Ask", "placeholder": "Ask dash, or tell it what to do",
            "hidden": [["page", "/"]],
        }},
        "suggestions": {"type": "cards", "children": ["inbox", "plan", "insights"]},
        "inbox": card("What waits on you", "Drafts, approvals and today's daily", "/inbox"),
        "plan": card("The plan", "Urgent, now, next and later", "/plan"),
        "insights": card("Insights", "Your feedback against the work done", "/insights"),
    }});
    Drawing::Tree {
        title: "Home".into(),
        tree: tree.to_string(),
    }
}

/// A page of the older app's reads under its heading, each part a section of cards, a skeleton while it is read.
fn read_page(title: &str, parts: &[Part]) -> Drawing {
    let mut elements = Map::new();
    let mut children = vec!["title".to_string()];
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": title}}),
    );
    for (key, part_title, read, parse) in parts {
        let inside = read_part(&mut elements, key, part_title, *read, *parse);
        let section =
            json!({"type": "section", "props": {"title": part_title}, "children": inside});
        elements.insert(key.to_string(), section);
        children.push(key.to_string());
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    Drawing::Tree {
        title: title.into(),
        tree: json!({"root": "page", "elements": elements}).to_string(),
    }
}

/// The screen at /tasks/<id> as a UI tree, the older app's issue page in Linear's components (task rust-task-page-linear; @nha,
/// 2026-11-09: « the tasks page need to match the linear design »): its title and status, its why, its checks, its Activity
/// (`history`, then his comments on it), its last run, its pull request beside them. `None`: the older app's list has answered
/// and no open task reads as `id`. Before it has, a task may still be one: the page says its words are not known, never that it
/// does not exist.
fn task_tree(look: &Look, id: &str, history: &Result<Vec<Event>, String>) -> Option<String> {
    let tasks = listed(look);
    let unknown = Listed {
        id: id.to_string(),
        status: "backlog",
        says: no_words(look).unwrap_or_default(),
        other: None,
    };
    let task = match tasks.iter().find(|t| t.id == id) {
        Some(task) => task,
        None if matches!(look.issues, Some(Ok(_))) => return None,
        None => &unknown,
    };
    let issue = issue_of(look, id);
    // a task read with no words says so; None only while the older app is read (/code-review of the merge)
    let why = issue.map_or_else(
        || no_words(look),
        |i| match lines_kept(&i.why) {
            w if w.is_empty() => Some("None written.".into()),
            w => Some(w),
        },
    );
    let checks: Vec<Value> = (issue.iter().flat_map(|i| &i.checks))
        .map(|c| json!({"text": one_line(&c.text), "met": c.met}))
        .collect();
    // None while the older app has not answered: the template draws a skeleton (task rust-skeleton-live)
    let no_checks = issue.map_or_else(|| no_words(look), |_| Some("None written.".into()));
    let unread = history.as_ref().err().map_or(String::new(), |e| {
        format!("Its history is not read: {}", one_line(e))
    });
    let events: Vec<Value> = (history.as_deref().unwrap_or_default().iter())
        .map(|e| json!({"what": event_words(e), "when": dash_core::stamp(e.at_ms)}))
        .collect();
    // his comments: the older app gives each one's words alone
    let comments: Vec<Value> = (issue.iter().flat_map(|i| &i.comments))
        .map(|c| json!({"said": [{"who": "Nicolas", "words": lines_kept(c)}]}))
        .collect();
    let (mut run, mut pr, mut said) = ("Never run.".to_string(), None, String::new());
    if let Some(row) = row_of(look, id).filter(|r| r.attempts > 0) {
        let last = last_run(row, look.now_ms);
        run = one_line(&format!(
            "Try {}, {}: {}.",
            row.attempts, last.when, last.outcome
        ));
        pr = pr_in(last.said);
        said = lines_kept(last.said);
    }
    // The task brief's template lays it out (task rust-doc-types): the page gives its fields alone.
    let brief = json!({"type": "document", "props": {
        "kind": "task-brief", "id": task.id, "title": title_of(look, id), "status": task.status, "says": task.says,
        "why": why, "checks": checks, "no_checks": no_checks,
        "events": events, "unread": unread, "comments": comments, "run": run, "pr": pr, "said": said,
    }});
    Some(json!({"root": "issue", "elements": {"issue": brief}}).to_string())
}

/// A write of its status in words, as its Activity lists it: a claim its try, an end its outcome.
fn event_words(event: &Event) -> String {
    match event.status.as_str() {
        // a reopen, a release or a crash freed: its outcome said (/code-review of rust-task-page-linear)
        "open" => event
            .outcome
            .as_deref()
            .map_or("Opened".into(), |o| format!("Opened: {}", one_line(o))),
        "claimed" => event
            .attempts
            .map_or("Started".into(), |n| format!("Started, try {n}")),
        "finished" => event
            .outcome
            .as_deref()
            .map_or("Finished".into(), |o| format!("Finished: {}", one_line(o))),
        other => one_line(other),
    }
}

/// How old Rust's list is, as the page's element: a skeleton while it is first synced.
fn note(look: &Look) -> Value {
    match &look.queue {
        Some(queue) => json!({"type": "text", "props": {"text": queue_note(queue, look.now_ms)}}),
        None => surface::skeleton("Rust's list"),
    }
}

fn queue_note(queue: &QueueAt, now_ms: i64) -> String {
    match queue {
        (Ok(_), at) => format!("Rust's list, synced {}.", age(now_ms - at)),
        (Err(e), at) => format!(
            "Rust's list is not known, {}: {}",
            age(now_ms - at),
            one_line(e)
        ),
    }
}

const AGENTS_SHOWN: usize = 10;

/// The agents `dash claude` started, above the tasks: none, no lines.
fn agents_text(agents: &[AgentRow], now_ms: i64) -> String {
    if agents.is_empty() {
        return String::new();
    }
    // The newest first: past sessions never push the tasks down the page.
    let lines = agents.iter().take(AGENTS_SHOWN).map(|a| {
        // Every field through one_line, the pane's id and folder too: one agent, one line (/code-review).
        format!(
            "{}\t{}\t{}\t{}\n",
            one_line(&a.pane),
            one_line(&a.state),
            age(now_ms - a.last_ms),
            one_line(&a.folder)
        )
    });
    let more = match agents.len().saturating_sub(AGENTS_SHOWN) {
        0 => String::new(),
        n => format!("and {n} more, older\n"),
    };
    format!(
        "Agent\tState\tLast write\tFolder\n{}{more}\n",
        lines.collect::<String>()
    )
}

/// Plain text, /tasks' last part: the drawn screen is `graph_tree`'s. `make rust/deploy` matches the whole « Built from » line.
/// `above`: the lines over the tasks table, the agents and the plan.
fn page(
    rows: &[Row],
    queue: Option<&QueueAt>,
    above: &str,
    now_ms: i64,
    commit: Option<&str>,
) -> String {
    let mut table: String = rows.iter().map(|r| row_line(r, now_ms)).collect();
    if let Some((Ok(queue), _)) = queue {
        let waiting = queue
            .tasks
            .iter()
            .filter(|t| !rows.iter().any(|r| r.id == t.id));
        for task in waiting {
            table += &line(&task.id, "queued", 0, "never", "", "", &task.title);
        }
    }
    if table.is_empty() {
        table = "No Rust task yet.\n".into();
    }
    let commit = commit.unwrap_or("a local build");
    format!(
        "Tasks: /tasks\nCode map: /deps\n\n{above}Task\tState\tTries\tLast run\tOutcome\tPR\tWords\n{table}\nBuilt from {commit}.\n"
    )
}

/// The build line alone, at /built: what `make rust/serves` and forge's deploy read (task rust-home-prompt; Home's redesign
/// had moved the line a deploy looked for). /tasks still ends with it.
pub(crate) fn built() -> String {
    let commit = option_env!("DASH_COMMIT").unwrap_or("a local build");
    format!("Built from {commit}.\n")
}

fn names_in(waits: &str) -> Vec<String> {
    let names = waits.split(',').filter(|w| !w.is_empty());
    names.map(String::from).collect()
}

/// A write, then its row, then the verdict as the last line: « <done> <id> », or « refused: <why> » with exit 1. Whatever
/// else a later slice prints (similar commits, a hint) goes above it: the old app printed those after « added », and agents
/// read the add as refused (review-handover-waits-on-room, 4 tries; task rust-task-add-verdict-last). One stream, stdout, so
/// the last line is the verdict whichever way it went.
fn written(
    dir: &Path,
    id: &str,
    done: &str,
    mut out: File,
    write: impl FnOnce(&mut SqliteTasks) -> Result<(), String>,
) {
    let row = open_tasks(dir).and_then(|mut tasks| {
        write(&mut tasks)?;
        tasks
            .show(id)?
            .ok_or(format!("{id}: written, yet no row reads back"))
    });
    match row {
        Ok(row) => writeln!(out, "{row}\n{done} {id}").expect("stdout"),
        Err(why) => {
            writeln!(out, "refused: {}", why.replace('\n', " ")).expect("stdout");
            std::process::exit(1);
        }
    }
}

/// A new open task.
pub fn add(dir: &Path, id: &str, waits: &str, out: File) {
    written(dir, id, "added", out, |tasks| {
        tasks.add(id, &names_in(waits))
    });
}

/// The list a task waits on replaced, emptied by an empty word.
pub fn waits(dir: &Path, id: &str, waits: &str, out: File) {
    written(dir, id, "waits set", out, |tasks| {
        tasks.waits(id, &names_in(waits))
    });
}

/// An open task given up on, out from under the cap.
pub fn close(dir: &Path, id: &str, why: &str, out: File) {
    written(dir, id, "closed", out, |tasks| tasks.close(id, why));
}

pub fn show(dir: &Path, id: &str, mut out: File) {
    let row = or_exit(or_exit(open_tasks(dir)).show(id));
    let row = or_exit(row.ok_or(format!("{id}: no such task")));
    writeln!(out, "{row}").expect("stdout");
}

/// Exit 0 on a task merged; any other outcome prints the same, with exit 1. A stop at the memory cap prints no outcome: exit
/// `STOP_CODE`, which `orchestrate` flags by name.
pub fn run(dir: &Path, id: &str, account: Option<&str>, mut out: File) {
    let report = reporting(dir, "run");
    let step = |name: &str, took: Duration| report.step_took(name, took);
    let tools = tools(dir);
    // Before its claim: a kind not read, or none Rust runs, starts nothing.
    let kind = or_exit(tools.list.agent(id));
    let (agent, launcher): (&dyn Agent, &dyn Launcher) = match kind.as_str() {
        "claude" => (&tools.agents.claude, &tools.launcher),
        "codex" => (&tools.agents.codex, &tools.codex),
        other => or_exit(Err(format!(
            "{id}: Rust runs claude and codex, not {other}"
        ))),
    };
    let mut tasks = or_exit(open_tasks(dir));
    let run = run_task(
        id,
        account,
        &mut ports(&mut tasks, &tools, agent, launcher, &step),
        PACE,
    );
    say_memory(&report);
    let run = or_exit(run);
    let (kind, said) = run.outcome.parts();
    let (pane, session) = (
        run.pane.unwrap_or_default(),
        run.session.unwrap_or_default(),
    );
    writeln!(
        out,
        "pane: {pane}\nsession: {session}\noutcome: {kind}\nsaid: {said}"
    )
    .expect("stdout");
    if !matches!(run.outcome, Outcome::Merged(_)) {
        std::process::exit(1);
    }
}

/// `dash claude`: one Claude Code session in a herdr pane of its own, in this folder, isolated by MXC (rust-dash-claude). The
/// pane runs the live dash, which records the agent on `dir` for the page at /. Prints the pane.
pub fn claude(dir: &Path, mut out: File) {
    let _report = reporting(dir, "claude");
    let folder = this_folder();
    let herdr = Herdr::from_env();
    let pane = or_exit(herdr.open(&folder.display().to_string(), "rust claude"));
    let dash = repo().join("private/live/rust/dash");
    let line = mxc::pane_line(&dash.display().to_string(), &dir.display().to_string());
    or_exit(herdr.run(&pane, &line));
    writeln!(out, "pane: {pane}").expect("stdout");
}

/// `dash claude --in-pane`, the pane's line: the session under the approved copy's policy for the claude job, recorded on
/// `dir` as it starts and as it ends. Exits with the session's code, 1 when it never started.
pub fn claude_in_pane(dir: &Path) {
    let _report = reporting(dir, "claude-in-pane");
    let folder = this_folder();
    let folder = folder.display().to_string();
    // herdr's own name for the pane, as sandbox-apply's DASH_SESSION falls back to it.
    let pane =
        std::env::var("HERDR_PANE_ID").unwrap_or_else(|_| format!("pid-{}", std::process::id()));
    // Its row by pane and process: a pane id herdr hands out again never overwrites an earlier agent (/code-review).
    let row = format!("{pane} {}", std::process::id());
    let mut tasks = open_tasks(dir).unwrap_or_else(|why| held(&why));
    tasks
        .agent(&row, &folder, "running")
        .unwrap_or_else(|why| held(&why));
    let home = home();
    // The approved policy is Seatbelt's: elsewhere a clear refusal, never mxc's spawn error (/code-review).
    let mac = if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err("dash claude runs on macOS only: the approved policy is Seatbelt's".to_string())
    };
    let ran = mac
        .and_then(|()| mxc::approved_job(&home, &folder, "claude"))
        .and_then(|job| {
            let policy = mxc::approved_profile(&home, &folder, "claude")?;
            let env = mxc::environment(&job, &pane, std::env::vars());
            mxc::in_terminal(mxc::request(&policy, &folder, &job.command, env))
        });
    let (state, code) = match &ran {
        Ok(code) => (format!("ended {code}"), *code),
        Err(why) => (format!("refused: {why}"), 1),
    };
    // The row first: after a closed pane, a write to its terminal fails, and nothing may stop the row being written.
    or_exit(tasks.agent(&row, &folder, &state));
    if let Err(why) = &ran {
        held(why);
    }
    std::process::exit(code);
}

/// The pane's line is `exec dash claude --in-pane`: an exit closes it, so `why` waits for Enter to be read, then exits 1
/// (/code-review).
fn held(why: &str) -> ! {
    let _ = writeln!(std::io::stderr(), "{why}\nEnter closes this pane.");
    if std::io::stdin().is_terminal() {
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(1);
}

/// `dash orch --sync`: a pass's first step alone, then the list as the store holds it, his seats first and one task a line with
/// what it waits on: what the next pick reads, with nothing started. Exit 1 when the sync or the read fails.
pub fn sync_once(dir: &Path, mut out: File) {
    let mut tasks = or_exit(open_tasks(dir));
    let written = or_exit(sync_list(&mut tasks, &older_db()));
    let queue = or_exit(TaskStore::queue(&tasks));
    let mut text = format!("{written} fields written; seats {}\n", queue.seats);
    for task in &queue.tasks {
        let waits = task
            .waits
            .iter()
            .map(|w| one_line(w))
            .collect::<Vec<_>>()
            .join(", ");
        text += &format!("{}\t{}\n", one_line(&task.id), waits);
    }
    write!(out, "{text}").expect("stdout");
}

/// What `forge.reap` took, one line each, past every claimed task's worktree. A failed read is its line.
fn reap(tasks: &SqliteTasks, forge: &dyn Forge) -> String {
    let busy = TaskStore::claims(tasks).map(|c| c.into_iter().map(|c| c.id).collect::<Vec<_>>());
    let said = busy.and_then(|busy| forge.reap(&busy));
    said.unwrap_or_else(|e| vec![format!("reap: {e}")])
        .iter()
        .map(|line| format!("{line}\n"))
        .collect()
}

/// `dash orch --reap`: one reap pass, as the look runs each hour, what went printed.
pub fn reap_once(dir: &Path, mut out: File) {
    let tasks = or_exit(open_tasks(dir));
    write!(out, "{}", reap(&tasks, &tools(dir).forge)).expect("stdout");
}

/// Picks and starts, each run a process of its own. `once`: the first pick's run, its exit this one's, nothing else started.
pub fn orchestrate(dir: &Path, once: bool, began: Instant, mut out: File) {
    let report = reporting(dir, "orch");
    let step = |name: &str, took: Duration| report.step_took(name, took);
    let tools = tools(dir);
    let mut tasks = or_exit(open_tasks(dir));
    let mut first = true;
    let mut running: Vec<(String, Child)> = vec![];
    let mut said = String::new();
    let mut looks = 0u32;
    loop {
        running.retain_mut(|(id, c)| {
            let Ok(Some(status)) = c.try_wait() else {
                return true;
            };
            // The next look reads this run as a crash and retries within its tries: only this exit code says why.
            if let Some(what) = stopped_at_cap(status) {
                let _ = tools.list.flag(id, what);
            }
            false
        });
        let (agent, launcher) = (&tools.agents, &tools.launcher);
        // Before the sync: the list's reopen of a task handed back reaches the store in this very look, and its pick.
        if looks.is_multiple_of(HAND_BACK_EVERY) {
            let handed = report.step("hand_back", || {
                hand_backs(&mut ports(&mut tasks, &tools, agent, launcher, &step))
            });
            for (id, words) in handed.unwrap_or_else(|e| vec![("hand-back".into(), e)]) {
                writeln!(out, "{id}: {words}").expect("stdout");
            }
        }
        if looks.is_multiple_of(REAP_EVERY) {
            let reaped = report.step("reap", || reap(&tasks, &tools.forge));
            write!(out, "{reaped}").expect("stdout");
        }
        looks = looks.wrapping_add(1);
        let synced = report.step("sync", || sync_list(&mut tasks, &older_db()));
        let look = report.step("pass", || {
            next(&mut ports(&mut tasks, &tools, agent, launcher, &step))
        });
        // To its first useful answer: a look that failed is none.
        if first && look.is_ok() {
            first = false;
            report.started(began);
        }
        let line = match look {
            Ok(Pick::Start { id, account }) => {
                let child = start(dir, &id, account.as_deref());
                if once {
                    let status = or_exit(child).wait().map_err(|e| e.to_string());
                    say_memory(&report);
                    std::process::exit(or_exit(status).code().unwrap_or(1));
                }
                match child {
                    Ok(child) => running.push((id.clone(), child)),
                    Err(e) => eprintln!("{id}: {e}"),
                }
                format!(
                    "started {id} on {}",
                    account.unwrap_or("its default account".into())
                )
            }
            Ok(Pick::Wait(why)) if once => {
                writeln!(out, "{why}").expect("stdout");
                say_memory(&report);
                std::process::exit(3);
            }
            Ok(Pick::Wait(why)) => why,
            // A look that failed is no verdict: the app may be restarting.
            Err(e) => e,
        };
        // Its picks went on from the last list: said beside them, so a sync that stopped is read where the picks are.
        let line = match synced {
            Ok(_) => line,
            Err(e) => format!("{line} (the list's sync failed, its last copy stands: {e})"),
        };
        // Once per change: a day's waits are one line.
        if line != said {
            writeln!(out, "{line}").expect("stdout");
            said = line;
        }
        std::thread::sleep(ORCH_POLL);
    }
}

fn stopped_at_cap(status: ExitStatus) -> Option<&'static str> {
    (status.code() == Some(STOP_CODE)).then_some("its run stopped at the memory cap")
}

fn start(dir: &Path, id: &str, account: Option<&str>) -> Result<Child, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(me);
    command.args(["task", "run", "--data"]).arg(dir).arg(id);
    if let Some(account) = account {
        command.args(["--account", account]);
    }
    detached(&mut command).spawn().map_err(|e| e.to_string())
}

/// A run in its own process group outlives this job: launchd ends the job's group, and a restart must not close a healthy session
/// and spend one of its tries.
fn detached(command: &mut Command) -> &mut Command {
    std::os::unix::process::CommandExt::process_group(command, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_bridge::Check;
    use dash_core::tasks::{Queued, Run};
    use dash_meter::memory::TestBudget;
    use dash_policy::Defaults;

    #[test]
    fn only_the_cap_s_exit_code_reads_as_a_stop_at_the_cap() {
        let _budget = TestBudget::new();
        let exit = |code: i32| {
            Command::new("sh")
                .args(["-c", &format!("exit {code}")])
                .status()
                .expect("sh")
        };
        assert!(stopped_at_cap(exit(STOP_CODE)).is_some());
        assert_eq!(stopped_at_cap(exit(1)), None);
        assert_eq!(stopped_at_cap(exit(0)), None);
    }

    #[test]
    fn a_run_leads_its_own_process_group() {
        let _budget = TestBudget::new();
        let mut perl = Command::new("perl");
        perl.args(["-e", "print getpgrp"])
            .stdout(std::process::Stdio::piped());
        let child = detached(&mut perl).spawn().expect("perl");
        let pid = child.id().to_string();
        let out = child.wait_with_output().expect("its group");
        assert_eq!(String::from_utf8_lossy(&out.stdout), pid);
    }

    fn row(said: &str) -> Row {
        Row {
            id: "a-task".into(),
            status: "finished".into(),
            attempts: 2,
            outcome: Some("merged".into()),
            said: Some(said.into()),
            last_ms: 1_000,
            run_gone: false,
        }
    }

    #[test]
    fn a_page_shows_a_run_with_its_pr_and_what_a_session_said() {
        let _budget = TestBudget::new();
        let said = "TASK_DONE\npr: https://github.com/o/r/pull/12, ready";
        let text = page(&[row(said)], None, "", 301_000, Some("abc123"));
        let line =
            "a-task\tfinished\t2\t5 min ago\tmerged\thttps://github.com/o/r/pull/12\tTASK_DONE\n";
        assert!(text.contains(line), "{text}");
        assert!(text.contains("Code map: /deps\n"), "{text}");
        assert!(text.ends_with("\nBuilt from abc123.\n"), "{text}");
    }

    #[test]
    fn the_page_lists_each_agent_dash_claude_started_and_none_says_nothing() {
        let _budget = TestBudget::new();
        assert_eq!(agents_text(&[], 0), "");
        let agent = |pane: &str, state: &str| AgentRow {
            pane: pane.into(),
            folder: "/w".into(),
            state: state.into(),
            last_ms: 1_000,
        };
        let text = agents_text(
            &[agent("p1", "running"), agent("p2", "refused: no\npolicy")],
            61_000,
        );
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[0], "Agent\tState\tLast write\tFolder");
        assert!(
            lines[1].starts_with("p1\trunning\t") && lines[1].ends_with("\t/w"),
            "{text}"
        );
        assert!(
            lines[2].starts_with("p2\trefused: no policy\t"),
            "one line a row: {text}"
        );
        let many: Vec<_> = (0..12)
            .map(|n| agent(&format!("p{n}"), "ended 0"))
            .collect();
        let text = agents_text(&many, 0);
        let shown = text.lines().count();
        assert_eq!(
            shown,
            1 + AGENTS_SHOWN + 2,
            "the newest 10, a title, the rest named and a gap"
        );
        assert!(text.contains("and 2 more, older\n"));
    }

    #[test]
    fn a_row_that_never_ran_shows_no_time() {
        let _budget = TestBudget::new();
        let mut added = row("");
        added.attempts = 0;
        let text = page(&[added], None, "", 301_000, None);
        assert!(text.contains("a-task\tfinished\t0\tnever\t"), "{text}");
    }

    #[test]
    fn words_cannot_fake_a_row() {
        let _budget = TestBudget::new();
        let queue = Ok(Queue {
            seats: 1,
            tasks: vec![Queued {
                id: "ne\nxt\x1b[2J".into(),
                title: "its\nfaked\trow\x1b[2J\r\u{202e}\u{2028}".into(),
                reopened_ms: None,
                waits: vec![],
                layer: None,
            }],
        });
        let text = page(&[], Some(&(queue, 0)), "", 0, None);
        assert!(text.contains("\nne xt [2J\tqueued\t"), "{text}");
        assert!(text.contains("\tits faked row [2J   \n"), "{text}");
        let down = (Err("down\nBuilt from x.".into()), 0);
        let failed = page(&[], Some(&down), "", 0, Some("y"));
        assert!(queue_note(&down, 0).ends_with("0 s ago: down Built from x."));
        assert!(!failed.contains("\nBuilt from x.\n"), "{failed}");
        let forged = "\nBuilt from x.\n";
        let queue = Ok(Queue {
            seats: 1,
            tasks: vec![Queued {
                id: forged.into(),
                title: forged.into(),
                reopened_ms: None,
                waits: vec![],
                layer: None,
            }],
        });
        let quoted = page(&[], Some(&(queue, 0)), "", 0, Some("y"));
        assert!(!quoted.contains("\nBuilt from x.\n"), "{quoted}");
    }

    #[test]
    fn a_task_on_its_next_try_shows_nothing_of_the_last_one() {
        let _budget = TestBudget::new();
        let mut retried = row("failed at https://github.com/o/r/pull/12");
        retried.status = "claimed".into();
        retried.outcome = Some("failed".into());
        let text = page(&[retried], None, "", 1_000, None);
        assert!(text.contains("running"), "{text}");
        assert!(!text.contains("failed"), "{text}");
        assert!(!text.contains("pull/12"), "{text}");
    }

    #[test]
    fn a_claim_whose_lock_is_free_says_run_gone_never_running() {
        let _budget = TestBudget::new();
        let mut gone = row("");
        gone.status = "claimed".into();
        gone.run_gone = true;
        let text = page(std::slice::from_ref(&gone), None, "", 1_000, None);
        assert!(text.contains("run gone"), "{text}");
        assert!(!text.contains("running"), "{text}");
        gone.run_gone = false;
        let text = page(&[gone], None, "", 1_000, None);
        assert!(text.contains("running"), "{text}");
        assert!(!text.contains("run gone"), "{text}");
    }

    #[test]
    fn a_session_waiting_on_the_network_reads_so_never_ended_and_a_gone_run_stays_gone() {
        let _budget = TestBudget::new();
        let mut waiting = row("API Error: Can't reach the API server");
        waiting.status = "claimed".into();
        waiting.outcome = Some("offline".into());
        let text = page(std::slice::from_ref(&waiting), None, "", 1_000, None);
        assert!(text.contains("waits on the network"), "{text}");
        assert!(
            !text.contains("ended") && !text.contains("running"),
            "{text}"
        );
        waiting.outcome = Some("limited".into());
        let text = page(std::slice::from_ref(&waiting), None, "", 1_000, None);
        assert!(text.contains("waits on its limit"), "{text}");
        waiting.outcome = Some("offline".into());
        waiting.run_gone = true;
        let text = page(&[waiting], None, "", 1_000, None);
        assert!(text.contains("run gone"), "{text}");
        assert!(!text.contains("waits on the network"), "{text}");
    }

    #[test]
    fn a_queued_task_shows_once_and_a_queue_that_failed_says_so() {
        let _budget = TestBudget::new();
        let queued = |id: &str| Queued {
            id: id.into(),
            title: "its words".into(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
        };
        let queue = Ok(Queue {
            seats: 1,
            tasks: vec![queued("a-task"), queued("next")],
        });
        let at = (queue, 1_000);
        let text = page(&[row("")], Some(&at), "", 121_000, None);
        // the note is the list tree's line, never the runs' too
        assert!(queue_note(&at, 121_000).contains("list, synced 2 min ago."));
        assert!(!text.contains("list, synced"), "{text}");
        assert_eq!(text.matches("a-task\t").count(), 1, "{text}");
        assert!(
            text.contains("next\tqueued\t0\tnever\t\t\tits words\n"),
            "{text}"
        );
        let down = (Err("down now".into()), 0);
        let failed = page(&[], Some(&down), "", 0, None);
        assert!(queue_note(&down, 0).contains("is not known, 0 s ago: down now"));
        assert!(failed.contains("No Rust task yet."), "{failed}");
    }

    /// The picker's half of this same case is run_on_store's the_picker_starts_what_the_page_names.
    #[test]
    fn the_page_names_what_plan_runs_next_and_why_the_others_wait() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("tasks-page-plan");
        let mut tasks = store::open(&dir.0, &dir.0).expect("open");
        tasks.add("first", &[]).expect("add");
        tasks.add("after", &["first".into()]).expect("add");
        let queued = |id: &str| Queued {
            id: id.into(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
        };
        let queue = Queue {
            seats: 5,
            tasks: vec![queued("after"), queued("first"), queued("also")],
        };
        let rows = dash_core::tasks::TaskStore::standing(&tasks).expect("rows");
        let plan = plan_text(&plan(&Defaults, queue, &rows, 0, vec![]));
        let text = page(&[], None, &plan, 0, None);
        assert!(
            text.contains(
                "Next: first\nalso\tbehind first\nafter\twaits on first (open)\n\nTask\t"
            ),
            "{text}"
        );
        let full = Plan {
            next: Pick::Wait("a waits: 5 of 5 sessions open".into()),
            waiting: vec![],
        };
        assert!(plan_text(&full).starts_with("Next: nothing, a waits: 5 of 5"));
    }

    fn queued(ids: &[&str], seats: usize) -> Queue {
        let queued = |id: &&str| Queued {
            id: id.to_string(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
        };
        Queue {
            seats,
            tasks: ids.iter().map(queued).collect(),
        }
    }

    /// What the store says of a task the fixture makes: only these fields reach the graph.
    fn stood(id: &str, status: &str, outcome: Option<&str>, waits: &[&str]) -> Standing {
        Standing {
            id: id.into(),
            status: status.into(),
            outcome: outcome.map(String::from),
            waits: waits.iter().map(|w| w.to_string()).collect(),
            ..Standing::default()
        }
    }

    /// The board view of these rows and this list, through the views' engine: the ops the store would have logged.
    fn viewed(standing: &[Standing], queue: Option<&Queue>) -> HashMap<String, &'static str> {
        use dash_core::Value::{Int, Text};
        let mut ops = vec![];
        let mut put = |table: &str, row: &str, field: &str, value: dash_core::Value| {
            let (table, row, field) = (table.into(), row.into(), field.into());
            let write = dash_core::FieldWrite {
                table,
                row,
                field,
                value,
            };
            let stamp = ops.len() as i64;
            ops.push(dash_core::Op {
                stamp,
                writer: "t".into(),
                write,
            });
        };
        for r in standing {
            put("tasks", &r.id, "status", Text(r.status.clone()));
            if let Some(outcome) = &r.outcome {
                put("tasks", &r.id, "outcome", Text(outcome.clone()));
            }
            put("tasks", &r.id, "waits", Text(r.waits.join(" ")));
        }
        for (pos, t) in queue.iter().flat_map(|q| &q.tasks).enumerate() {
            put("listed", &t.id, "title", Text(t.title.clone()));
            put("listed", &t.id, "waits", Text(t.waits.join("\n")));
            put("listed", &t.id, "pos", Int(pos as i64));
        }
        let mut engine = dash_views::Engine::new();
        engine.commit(&ops);
        columns(engine.rows("board"))
    }

    fn look(standing: Vec<Standing>, queue: Queue, sessions: usize, limits: Vec<Limit>) -> Look {
        let plan = plan(&Defaults, queue.clone(), &standing, sessions, limits);
        Look {
            rows: vec![],
            columns: viewed(&standing, Some(&queue)),
            standing,
            now_ms: 0,
            queue: Some((Ok(queue), 0)),
            issues: None,
            plan: Some(Ok(plan)),
            every: None,
        }
    }

    fn drawn(look: &Look) -> String {
        surface::screen::page("t", &graph_tree(look)).expect("a page")
    }

    /// The node of `id`, from its opening to the end of its words.
    fn node<'a>(page: &'a str, id: &str) -> &'a str {
        let start = page
            .find(&format!(r#"data-task="{id}""#))
            .unwrap_or_else(|| panic!("no node {id}: {page}"));
        // Its class sits just before: from the node's own class word on.
        let class = page[..start].rfind("class=").expect("its class");
        let says = start + page[start..].find(r#"class="says">"#).expect("its words");
        let end = says + page[says + 1..].find('<').expect("their end");
        &page[class..=end]
    }

    /// Done-when 1 of rust-page-forms: the server's `drawn` makes /graph's one drawing its SVG page or the tree's text.
    #[test]
    fn graph_is_one_drawing_the_server_makes_html_or_text() {
        let _budget = TestBudget::new();
        let standing = vec![
            stood("base", "open", None, &[]),
            stood("top", "open", None, &["base"]),
        ];
        let look = look(standing, queued(&["base"], 5), 1, vec![]);
        let (_, kind, html) = crate::drawn(true, Some(Ok(graph_drawing(&look))));
        assert_eq!(kind, crate::HTML);
        assert!(html.contains(r#"data-from="base" data-to="top""#), "{html}");
        let (_, kind, text) = crate::drawn(false, Some(Ok(graph_drawing(&look))));
        assert_eq!(kind, crate::TEXT);
        assert!(
            text.contains("\ntop\t") && text.contains("\twaits on base\n") && !text.contains('<'),
            "{text}"
        );
    }

    /// Done-when 1 of rust-page-forms: /tasks' board and list are one drawing each, the Dioxus page or the same tree's text.
    #[test]
    fn tasks_is_one_drawing_the_server_makes_html_or_text() {
        let _budget = TestBudget::new();
        let look = look(
            vec![stood("base", "open", None, &[])],
            queued(&["base"], 5),
            1,
            vec![],
        );
        for view in [TaskView::Board, TaskView::List] {
            let (_, kind, html) = crate::drawn(true, Some(Ok(list_drawing(&look, view, None, ""))));
            assert_eq!(kind, crate::HTML);
            assert!(html.contains(r#"class="tabs""#), "{html}");
            let (_, kind, text) =
                crate::drawn(false, Some(Ok(list_drawing(&look, view, None, ""))));
            assert_eq!(kind, crate::TEXT);
            assert_eq!(
                text,
                surface::text(&list_tree(&look, view, None, "")).expect("a text")
            );
            assert!(
                text.starts_with("Tasks\nViews: ") && !text.contains('<'),
                "{text}"
            );
        }
    }

    /// Done-when 1: five open tasks, each drawn with its edges, and the plan's next pick marked.
    #[test]
    fn the_graph_draws_five_tasks_with_their_dependencies_and_marks_the_next_pick() {
        let _budget = TestBudget::new();
        let standing = vec![
            stood("base", "open", None, &[]),
            stood("also", "open", None, &[]),
            stood("middle", "open", None, &["base"]),
            stood("top", "open", None, &["middle", "also"]),
            stood("busy", "claimed", None, &[]),
        ];
        let queue = queued(&["top", "middle", "base", "also"], 5);
        let page = drawn(&look(standing, queue, 1, vec![]));
        for id in ["base", "also", "middle", "top", "busy"] {
            assert!(
                page.contains(&format!(r#"data-task="{id}""#)),
                "{id}: {page}"
            );
        }
        for (from, to) in [("base", "middle"), ("middle", "top"), ("also", "top")] {
            let edge = format!(r#"data-from="{from}" data-to="{to}""#);
            assert!(page.contains(&edge), "{edge}: {page}");
        }
        assert_eq!(page.matches(r#"class="edge""#).count(), 3, "{page}");
        // base is the first the queue offers that nothing holds: the plan's answer, the one marked.
        assert_eq!(page.matches(r#"class="node next""#).count(), 1, "{page}");
        assert!(
            node(&page, "base").contains(r#"class="node next""#),
            "{page}"
        );
        assert!(node(&page, "base").contains("the next pick"), "{page}");
        assert!(page.contains("Next: base"), "{page}");
        assert!(
            node(&page, "busy").contains(r#"class="node running""#),
            "{page}"
        );
        assert!(node(&page, "also").contains("behind base"), "{page}");
    }

    /// Done-when 2: a dependency, the seats and the load, each in the plan's own words on the waiting task.
    #[test]
    fn a_waiting_task_says_it_waits_on_a_dependency_on_seats_or_on_load() {
        let _budget = TestBudget::new();
        let open = |id: &str, waits: &[&str]| stood(id, "open", None, waits);
        let on_dep = look(
            vec![open("first", &[]), open("after", &["first"])],
            queued(&["after", "first"], 5),
            0,
            vec![],
        );
        let page = drawn(&on_dep);
        let after = node(&page, "after");
        assert!(after.contains(r#"class="node waiting""#), "{after}");
        assert!(after.contains("waits on first (open)"), "{after}");
        assert!(
            page.contains("after: waits on first (open)"),
            "the list: {page}"
        );

        let on_seats = look(vec![open("a", &[])], queued(&["a", "b"], 2), 2, vec![]);
        let page = drawn(&on_seats);
        let says = "a waits: 2 of 2 sessions open";
        assert!(node(&page, "a").contains(says), "{page}");
        assert!(node(&page, "b").contains("behind a"), "{page}");
        assert!(page.contains(&format!("Next: nothing, {says}")), "{page}");
        assert!(!page.contains(r#"class="node next""#), "{page}");

        let spent = Limit {
            account: "one".into(),
            five_hours: 100.0,
            five_hours_resets: 1,
            week: 10.0,
            week_resets: 1,
            key: false,
        };
        let on_load = look(vec![open("a", &[])], queued(&["a"], 5), 0, vec![spent]);
        let page = drawn(&on_load);
        let says = "a waits: every account is at its limit";
        assert!(node(&page, "a").contains(says), "{page}");
        assert!(page.contains(&format!("Next: nothing, {says}")), "{page}");
    }

    #[test]
    fn what_a_task_names_is_drawn_finished_and_the_past_stays_out() {
        let _budget = TestBudget::new();
        let standing = vec![
            stood("old", "finished", Some("merged"), &[]),
            stood("failed", "finished", Some("ended"), &["old"]),
            stood("gone-by", "finished", Some("merged"), &[]),
            stood("now", "open", None, &["old", "failed"]),
        ];
        let page = drawn(&look(standing, queued(&["now"], 5), 0, vec![]));
        assert!(
            node(&page, "old").contains(r#"class="node finished""#),
            "{page}"
        );
        assert!(
            node(&page, "failed").contains(r#"class="node ended""#),
            "{page}"
        );
        assert!(
            page.contains(r#"class="edge met" data-from="old""#),
            "{page}"
        );
        assert!(
            page.contains(r#"class="edge" data-from="failed""#),
            "{page}"
        );
        assert!(
            node(&page, "now").contains("waits on failed (ended)"),
            "{page}"
        );
        assert!(!page.contains(r#"data-task="gone-by""#), "{page}");
        assert!(
            !page.contains(r#"data-from="old" data-to="failed""#),
            "{page}"
        );
    }

    #[test]
    fn before_the_plan_is_known_the_graph_says_so_and_a_gone_run_never_reads_running() {
        let _budget = TestBudget::new();
        let mut gone = row("");
        gone.status = "claimed".into();
        gone.run_gone = true;
        let standing = vec![
            stood("a-task", "claimed", None, &[]),
            stood("b", "open", None, &[]),
        ];
        let mut no_plan = Look {
            rows: vec![gone],
            columns: viewed(&standing, None),
            standing,
            now_ms: 0,
            queue: None,
            issues: None,
            plan: None,
            every: None,
        };
        // a gone run is still In Progress until `dash orch` frees it: « run gone » says so, as the older board's « dead »
        assert_eq!(no_plan.columns.get("a-task"), Some(&"in-progress"));
        let page = drawn(&no_plan);
        assert!(page.contains(r#"list: being read""#), "{page}");
        assert!(node(&page, "a-task").contains("run gone"), "{page}");
        assert!(!node(&page, "a-task").contains("running"), "{page}");
        assert!(node(&page, "b").contains("not known yet"), "{page}");
        let standing = no_plan.standing[0].clone();
        no_plan.standing[0] = stood("a-task", "claimed", Some("offline"), &[]);
        let graph = drawn(&no_plan);
        assert!(node(&graph, "a-task").contains("run gone"), "{graph}");
        no_plan.rows[0].run_gone = false;
        let graph = drawn(&no_plan);
        assert!(
            node(&graph, "a-task").contains("waits on the network"),
            "{graph}"
        );
        assert!(!node(&graph, "a-task").contains("ended"), "{graph}");
        no_plan.rows[0].run_gone = true;
        no_plan.standing[0] = standing;
        no_plan.plan = Some(Err("down\nNext: fake".into()));
        let page = drawn(&no_plan);
        assert!(page.contains("did not answer: down Next: fake"), "{page}");
    }

    #[test]
    fn the_graph_says_how_old_its_answer_is_and_when_the_pick_is_still_coming() {
        let _budget = TestBudget::new();
        let mut known = look(vec![], queued(&["a"], 5), 0, vec![]);
        known.now_ms = 9 * 3_600_000;
        let page = drawn(&known);
        assert!(page.contains("Next: a"), "{page}");
        assert!(page.contains("list, synced 9 h ago."), "{page}");
        known.plan = None;
        let page = drawn(&known);
        assert!(page.contains(r#"aria-label="Next: being read""#), "{page}");
        assert!(node(&page, "a").contains("not known yet"), "{page}");
    }

    #[test]
    fn a_queue_that_failed_promises_no_reload_and_never_says_nothing_is_open() {
        let _budget = TestBudget::new();
        let failed = |standing: Vec<Standing>, queue: Option<QueueAt>| Look {
            rows: vec![],
            columns: viewed(&standing, None),
            standing,
            now_ms: 0,
            queue,
            issues: None,
            plan: None,
            every: None,
        };
        let down = || Some((Err("down".to_string()), 0));
        let page = drawn(&failed(vec![stood("b", "open", None, &[])], down()));
        assert!(page.contains("list is not known."), "{page}");
        assert!(!page.contains("next reload"), "{page}");
        assert!(node(&page, "b").contains("not known"), "{page}");
        assert!(!node(&page, "b").contains("not known yet"), "{page}");
        for queue in [down(), None] {
            let page = drawn(&failed(vec![], queue));
            assert!(!page.contains("No Rust task open."), "{page}");
            assert!(page.contains("is not known yet"), "{page}");
        }
    }

    #[test]
    fn with_nothing_to_draw_or_too_much_the_page_still_stands() {
        let _budget = TestBudget::new();
        let empty = drawn(&look(vec![], queued(&[], 5), 0, vec![]));
        assert!(empty.contains("No Rust task open."), "{empty}");
        assert!(!empty.contains("task-graph\""), "{empty}");
        let ids: Vec<String> = (0..=surface::GRAPH_LIMIT)
            .map(|i| format!("t{i}"))
            .collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let many = drawn(&look(vec![], queued(&ids, 5), 0, vec![]));
        assert!(many.contains("501 tasks: past the 500"), "the words");
        assert!(many.contains("Next: t0"), "the next line");
        assert!(many.contains("t500: behind t0"), "the list");
    }

    #[test]
    fn two_ids_that_read_the_same_never_lose_the_page() {
        let _budget = TestBudget::new();
        let page = drawn(&look(vec![], queued(&["a\nb", "a b"], 5), 0, vec![]));
        assert_eq!(page.matches(r#"data-task="a b""#).count(), 1, "{page}");
        // The row of the id it reads as gives it its waits: what they name is no node adrift.
        let standing = vec![
            stood("a b", "open", None, &["c"]),
            stood("c", "open", None, &[]),
        ];
        let page = drawn(&look(standing, queued(&["a\nb"], 5), 0, vec![]));
        assert!(page.contains(r#"data-from="c" data-to="a b""#), "{page}");
    }

    fn issue(id: &str, title: &str, comments: &[&str]) -> Issue {
        Issue {
            id: id.into(),
            title: title.into(),
            why: format!("why of {id}"),
            checks: vec![],
            comments: comments.iter().map(|c| c.to_string()).collect(),
        }
    }

    /// The list as / serves a browser: the Dioxus component's server render.
    fn list_page(look: &Look) -> String {
        surface::screen::page("t", &list_tree(look, TaskView::List, None, "")).expect("a page")
    }

    /// The words dash ui's window shows of the same page: its HTML, drawn as the window draws it.
    fn list_window(look: &Look) -> String {
        let props = dash_ui::PageProps {
            html: list_page(look),
        };
        dash_ui::shown(dioxus::prelude::VirtualDom::new_with_props(
            dash_ui::Page,
            props,
        ))
    }

    fn task_page(look: &Look, id: &str) -> String {
        let tree = task_tree(look, id, &Ok(vec![])).unwrap_or_else(|| panic!("no task {id}"));
        surface::screen::page("t", &tree).expect("a page")
    }

    /// The ids of a status's rows, in order.
    fn rows_of(page: &str, status: &str) -> Vec<String> {
        let start = format!(r#"data-status="{status}">"#);
        let Some(at) = page.find(&start) else {
            return vec![];
        };
        let group = &page[at + start.len()..];
        let end = group.find(r#"class="issue-group""#).unwrap_or(group.len());
        group[..end]
            .split(r#"data-task=""#)
            .skip(1)
            .map(|r| r[..r.find('"').expect("its id")].to_string())
            .collect()
    }

    /// Done-when 1: a fixture of each status, each group in pick order, the groups in the Linear view's order.
    #[test]
    fn the_list_shows_open_tasks_by_status_in_pick_order() {
        let _budget = TestBudget::new();
        let open = |id: &str, waits: &[&str]| stood(id, "open", None, waits);
        let standing = vec![
            open("first", &[]),
            open("held", &["first"]),
            open("second", &[]),
            stood("busy", "claimed", None, &[]),
            stood("gone", "claimed", None, &[]),
            stood("reviewed", "finished", Some("done"), &[]),
            stood("failed", "finished", Some("ended"), &[]),
            stood("merged", "finished", Some("merged"), &[]),
            open("unlisted", &[]),
        ];
        let queue = queued(&["third", "first", "held", "second"], 5);
        let mut look = look(standing, queue, 1, vec![]);
        let mut gone = row("");
        gone.id = "gone".into();
        gone.status = "claimed".into();
        gone.run_gone = true;
        let mut reviewed = row("TASK_DONE pr: https://github.com/o/r/pull/12, ready");
        reviewed.id = "reviewed".into();
        reviewed.outcome = Some("done".into());
        let mut failed = row("gave up");
        failed.id = "failed".into();
        failed.outcome = Some("ended".into());
        look.rows = vec![gone, reviewed, failed];
        look.issues = Some(Ok(vec![
            issue("", "No id, no row", &[]),
            issue("reviewed", "Its review", &[]),
            issue("failed", "Its failure", &[]),
            issue("only-there", "Held by the older app", &[]),
            issue("third", "The third, <first>", &[]),
        ]));
        let page = list_page(&look);
        assert!(page.contains("--color-accent:"), "its theme: {page}");
        assert!(page.contains("Next: third"), "{page}");
        assert_eq!(rows_of(&page, "in-review"), ["reviewed"], "{page}");
        assert_eq!(rows_of(&page, "in-progress"), ["busy", "gone"], "{page}");
        assert_eq!(
            rows_of(&page, "todo"),
            ["third", "first", "second"],
            "next, then what is behind it, the queue's order: {page}"
        );
        assert_eq!(
            rows_of(&page, "backlog"),
            ["held", "unlisted", "failed", "only-there"],
            "{page}"
        );
        assert!(!page.contains(r#"data-task="merged""#), "{page}");
        let at = |s: &str| page.find(&format!(r#"data-status="{s}""#)).expect(s);
        assert!(at("in-review") < at("in-progress"), "{page}");
        assert!(at("in-progress") < at("todo"), "{page}");
        assert!(at("todo") < at("backlog"), "{page}");
        assert!(page.contains(r#"class="badge count">2"#), "{page}");
        assert!(page.contains("The third, &#60;first&#62;"), "{page}");
        assert!(
            page.contains("waits on first (open)"),
            "the plan's words: {page}"
        );
        assert!(page.contains("done, PR #12"), "{page}");
        assert!(page.contains("run gone"), "{page}");
        assert!(page.contains("its run ended: ended"), "{page}");
        // done-when 1 of rust-dioxus-only-app: the window draws the same component, so the same rows in the same order
        let window = list_window(&look);
        let rows = [
            "reviewed", "busy", "gone", "third", "first", "second", "held", "unlisted",
        ];
        let mut at_row = rows.iter().map(|r| {
            window
                .find(&format!("\n{r}\n"))
                .unwrap_or_else(|| panic!("{r}: {window}"))
        });
        assert!(
            at_row
                .clone()
                .zip(at_row.by_ref().skip(1))
                .all(|(a, b)| a < b),
            "{window}"
        );
        assert!(window.contains("\nThe third, <first>\n"), "{window}");
        let unlisted = &page[page.find(r#"data-task="unlisted""#).expect("its row")..];
        assert!(
            unlisted[..unlisted.find("data-task=\"failed").expect("the next row")]
                .contains("does not offer it"),
            "an open row never reads as none: {page}"
        );
    }

    #[test]
    fn with_no_plan_known_the_list_keeps_the_queue_s_order() {
        let _budget = TestBudget::new();
        let open = |id: &str| stood(id, "open", None, &[]);
        let mut look = look(
            vec![open("a"), open("z")],
            queued(&["z", "a"], 5),
            0,
            vec![],
        );
        assert_eq!(rows_of(&list_page(&look), "todo"), ["z", "a"]);
        look.plan = None;
        assert_eq!(rows_of(&list_page(&look), "todo"), ["z", "a"]);
        look.plan = Some(Err("the sessions are down".into()));
        assert_eq!(rows_of(&list_page(&look), "todo"), ["z", "a"]);
    }

    #[test]
    fn a_task_is_no_404_until_the_older_app_s_list_says_so() {
        let _budget = TestBudget::new();
        let mut look = look_empty();
        look.queue = None;
        look.plan = None;
        let page = task_page(&look, "bookmarked");
        assert!(
            page.contains(r#"class="li-issue-title">bookmarked"#),
            "{page}"
        );
        assert!(
            page.contains(r#"aria-label="Its words: being read""#),
            "{page}"
        );
        look.issues = Some(Err("down".into()));
        assert!(task_page(&look, "bookmarked").contains("did not answer: down"));
        look.issues = Some(Ok(vec![]));
        assert!(task_tree(&look, "bookmarked", &Ok(vec![])).is_none());
    }

    /// Done-when 2.
    #[test]
    fn a_task_opens_to_its_title_why_checks_and_last_run() {
        let _budget = TestBudget::new();
        let mut look = look(
            vec![stood("a-task", "open", None, &[])],
            queued(&["a-task"], 5),
            0,
            vec![],
        );
        let mut ran = row("TASK_BLOCKED\nneeds: a <tap>\nhttps://github.com/o/r/pull/12");
        ran.status = "open".into();
        ran.outcome = Some("blocked".into());
        look.rows = vec![ran];
        look.now_ms = 301_000;
        let mut words = issue("a-task", "Its <odd> name", &[]);
        words.why = "(@nha: « because »)\nA second line".into();
        words.checks = vec![
            Check {
                text: "One thing: a test".into(),
                met: Some(true),
            },
            Check {
                text: "Another: a test".into(),
                met: Some(false),
            },
            Check {
                text: "A third".into(),
                met: None,
            },
        ];
        look.issues = Some(Ok(vec![words]));
        // The list opens it.
        assert!(list_page(&look).contains(r#"href="/tasks/a-task""#));
        let page = task_page(&look, "a-task");
        assert!(
            page.contains(r#"href="/tasks">Tasks"#),
            "the way back: {page}"
        );
        assert!(
            page.contains(r#"class="li-issue-title">Its &#60;odd&#62; name"#),
            "{page}"
        );
        assert!(page.contains(r#"data-status="todo""#), "{page}");
        assert!(page.contains("the next pick"), "{page}");
        assert!(
            page.contains("(@nha: « because »)\nA second line"),
            "{page}"
        );
        assert!(page.contains("Done when: 1 of 3 met"), "{page}");
        assert!(
            page.contains(r#"class="li-check met" title="met""#),
            "{page}"
        );
        assert!(
            page.contains(r#"class="li-check unmet" title="not met""#),
            "{page}"
        );
        assert!(page.contains("A third"), "{page}");
        assert!(page.contains("Last run"), "{page}");
        assert!(page.contains("Try 2, 5 min ago: blocked."), "{page}");
        assert!(page.contains(r#">#12"#), "{page}");
        assert!(
            page.contains("needs: a &#60;tap&#62;\n"),
            "its lines kept: {page}"
        );
        assert!(task_tree(&look, "no-such", &Ok(vec![])).is_none());
        // its Activity: each write of its status in words, when (task rust-task-page-linear)
        let event = |at_ms, status: &str, attempts, outcome: Option<&str>| Event {
            at_ms,
            status: status.into(),
            attempts,
            outcome: outcome.map(String::from),
        };
        let history = [
            event(0, "claimed", Some(1), None),
            event(60_000, "finished", None, Some("failed")),
            event(120_000, "open", None, Some("failed")),
            event(180_000, "open", None, None),
        ];
        let tree = task_tree(&look, "a-task", &Ok(history.to_vec())).expect("its page");
        let said = surface::text(&tree).expect("its text");
        let told = "Activity\nStarted, try 1\t1970-01-01 00:00 UTC\nFinished: failed\t1970-01-01 00:01 UTC\nOpened: failed\t1970-01-01 00:02 UTC\nOpened\t1970-01-01 00:03 UTC\n";
        assert!(said.contains(told), "{said}");
        // its pull request among its properties, the same tree drawn
        let page = surface::screen::page("t", &tree).expect("a page");
        let pr = r#"class="li-tag li-pr" href="https://github.com/o/r/pull/12""#;
        assert!(page.contains(pr), "{page}");
        // a history not read: the page stands, and says so
        let unread = task_tree(&look, "a-task", &Err("locked".into())).expect("its page");
        let said = surface::text(&unread).expect("its text");
        assert!(
            said.contains("Activity\nIts history is not read: locked\n"),
            "{said}"
        );
        assert!(said.contains("Done when: 1 of 3 met"), "{said}");
    }

    #[test]
    fn a_task_that_never_ran_or_whose_words_are_not_read_says_so() {
        let _budget = TestBudget::new();
        let mut look = look(vec![], queued(&["a"], 5), 0, vec![]);
        if let Some((Ok(queue), _)) = &mut look.queue {
            queue.tasks[0].title = "From the queue".into();
        }
        let page = task_page(&look, "a");
        assert!(
            page.contains(r#"class="li-issue-title">From the queue"#),
            "{page}"
        );
        if let Some((Ok(queue), _)) = &mut look.queue {
            queue.tasks[0].title = String::new();
        }
        let page = task_page(&look, "a");
        assert!(page.contains("Never run."), "{page}");
        assert!(
            page.contains(r#"aria-label="Its words: being read""#),
            "{page}"
        );
        assert!(page.contains(r#"class="li-issue-title">a"#), "{page}");
        look.issues = Some(Err("x".repeat(300)));
        assert!(
            !list_page(&look).contains(&"x".repeat(201)),
            "a line, not the whole answer"
        );
        look.issues = Some(Err("down\nNext: fake".into()));
        let page = task_page(&look, "a");
        assert!(page.contains("did not answer: down Next: fake"), "{page}");
        let list = list_page(&look);
        assert!(list.contains("did not answer: down Next: fake"), "{list}");
        look.issues = Some(Ok(vec![issue("a", "t", &[])]));
        let page = task_page(&look, "a");
        assert!(page.contains("None written."), "{page}");
        let mut empty = look_empty();
        let unread = list_page(&empty);
        assert!(!unread.contains("No Rust task open."), "{unread}");
        assert!(unread.contains("list has not answered"), "{unread}");
        empty.issues = Some(Ok(vec![]));
        let empty = list_page(&empty);
        assert!(empty.contains("No Rust task open."), "{empty}");
        assert!(!empty.contains(r#"class="issue-group""#), "{empty}");
    }

    /// Done-when 1 of rust-skeleton-live: a section not read yet is a skeleton under the id its filled section has, so the
    /// page's refill (surface's LIVE_JS) swaps one for the other with no reload; neither form promises a reload.
    #[test]
    fn an_unread_section_is_a_skeleton_its_read_drawing_replaces_in_place() {
        let _budget = TestBudget::new();
        let mut look = look(vec![], queued(&["a"], 5), 0, vec![]);
        look.issues = None;
        let unread = task_page(&look, "a");
        let skeleton = r#"id="issue-why" class="skeleton" aria-busy="true" role="status" aria-label="Its words: being read""#;
        assert!(unread.contains(skeleton), "{unread}");
        // its two skeletons (its words, its checks), one refill loop
        assert_eq!(unread.matches("window.dashLive=1").count(), 1, "{unread}");
        let text =
            surface::text(&task_tree(&look, "a", &Ok(vec![])).expect("a page")).expect("a text");
        assert!(text.contains("Its words: being read."), "{text}");
        look.issues = Some(Ok(vec![issue("a", "t", &[])]));
        let read = task_page(&look, "a");
        assert!(
            read.contains(r#"class="doc-text" id="issue-why">why of a<"#),
            "{read}"
        );
        assert!(
            !read.contains("aria-busy") && !read.contains("window.dashLive"),
            "{read}"
        );
        // the checks' skeleton sits in the part the read drawing holds by id, so the refill swaps its title too (/code-review of
        // the merge: a skeleton whose id the filled drawing lacked was removed, its checks never shown); its shape: surface's
        // a_brief_being_read_swaps_whole_parts
        assert!(unread.contains(r#"id="issue-checks-read""#), "{unread}");
        let mut judged = issue("a", "t", &[]);
        judged.checks = vec![dash_bridge::Check {
            text: "c".into(),
            met: Some(true),
        }];
        judged.why = String::new();
        look.issues = Some(Ok(vec![judged]));
        let read = task_page(&look, "a");
        assert!(
            read.contains(r#"id="issue-checks""#) && read.contains("Done when: 1 of 1 met"),
            "{read}"
        );
        // a task read with no words says so, never that the older app lacks it
        assert!(read.contains(r#"id="issue-why">None written.<"#), "{read}");
        // what he and an agent read: the refill script's own `location.reload()` is no word on the page
        for words in [
            surface::forms::text_of(&unread),
            surface::forms::text_of(&read),
            text,
        ] {
            assert!(!words.to_lowercase().contains("reload"), "{words}");
        }
    }

    /// Done-when 1 of rust-home-prompt: / is the compass's prompt and three suggestion cards, nothing else, in both forms.
    #[test]
    fn home_is_one_prompt_and_three_suggestions() {
        let _budget = TestBudget::new();
        let Drawing::Tree { tree, .. } = home_page() else {
            panic!("a tree")
        };
        let text = surface::text(&tree).expect("a text");
        assert_eq!(
            text,
            "\nAsk: POST /compass, his words as text\n\
             What waits on you\tDrafts, approvals and today's daily\t/inbox\n\
             The plan\tUrgent, now, next and later\t/plan\n\
             Insights\tYour feedback against the work done\t/insights\n"
        );
        let html = surface::screen::page("Home", &tree).expect("a page");
        let body = &html[html.find(r#"class="app""#).expect("the app")..];
        assert!(
            body.contains(r#"method="post" id="ask" action="/compass""#)
                && body.contains(r#"name="page" type="hidden" value="/""#)
                && body.contains(r#"name="text""#),
            "{body}"
        );
        assert_eq!(body.matches(r#"class="card""#).count(), 3, "{body}");
        for href in ["/inbox", "/plan", "/insights"] {
            assert!(
                body.contains(&format!(r#"class="card" href="{href}""#)),
                "{href}: {body}"
            );
        }
        for gone in [
            "Waiting on you",
            "Today's daily",
            "What runs",
            "Built from",
            r#"class="quote""#,
        ] {
            assert!(!body.contains(gone), "{gone}: {body}");
        }
    }

    /// Done-when 2 of rust-home-prompt: /inbox holds what Home showed first, what waits on him then his daily, each a skeleton
    /// while read, the older app's silence said, a step a card with its link (done-when 1 of rust-nav-polish).
    #[test]
    fn inbox_shows_his_waits_then_his_daily() {
        let _budget = TestBudget::new();
        let waits = |s: &str| crate::home::cards(s, crate::home::handoff_link);
        let daily = |s: &str| crate::home::cards(s, crate::home::progress_link);
        let shown = |handoff: Option<&Words>, progress: Option<&Words>| {
            let parts: [Part; 2] = [
                ("waits", "Waiting on you", handoff, &waits),
                ("daily", "Today's daily", progress, &daily),
            ];
            let Drawing::Tree { tree, .. } = read_page("Inbox", &parts) else {
                panic!("a tree")
            };
            tree
        };
        let reading = surface::text(&shown(None, None)).expect("a text");
        assert_eq!(
            reading,
            "Inbox\n\nWaiting on you\nWaiting on you: being read.\n\nToday's daily\nToday's daily: being read.\n"
        );
        let (handoff, down): (Words, Words) = (
            Ok("## His three steps\n- Connect\u{1b}[2J Datadog\n".into()),
            Err("down\nnow".into()),
        );
        let inbox = surface::text(&shown(Some(&handoff), Some(&down))).expect("a text");
        assert_eq!(
            inbox,
            "Inbox\n\nWaiting on you\n\nHis three steps\nConnect [2J Datadog\n\nToday's daily\nThe older app did not answer: down now\n"
        );
        let (nothing, today): (Words, Words) = (
            Ok("  \n".into()),
            Ok("Forward progress, today: review.".into()),
        );
        let quiet = surface::text(&shown(Some(&nothing), Some(&today))).expect("a text");
        assert!(
            quiet.ends_with(
                "Waiting on you\nNothing.\n\nToday's daily\nForward progress, today: review\n"
            ),
            "{quiet}"
        );
        let draft: Words = Ok("## Drafts to review (1)\n- acme-intro -> Ann (Acme)\n".into());
        let tree = shown(Some(&draft), Some(&today));
        let html = surface::screen::page("Inbox", &tree).expect("a page");
        let card = r#"id="waits-0-0" class="card" href="/d/acme-intro">"#;
        assert!(
            html.contains(card) && html.contains(r#"class="title">Ann (Acme)"#),
            "{html}"
        );
        assert!(!html.contains("## ") && !html.contains("- acme"), "{html}");
        assert!(
            surface::text(&tree)
                .expect("a text")
                .contains("\nDrafts to review (1)\nAnn (Acme)\tacme-intro\t/d/acme-intro\n")
        );
    }

    /// Task rust-home-prompt's live check: a failed read tries again on the minute's poll; words read wait the read's own
    /// interval.
    #[test]
    fn a_failed_read_tries_again_on_the_minute() {
        let _budget = TestBudget::new();
        let state = Arc::new(Mutex::new(Polled {
            last: Some(Err("down".into())),
            at: now_ms() - 61_000,
            reading: false,
        }));
        let settled = |state: &Arc<Mutex<Polled<Words>>>| {
            for _ in 0..100 {
                if !state.lock().expect("a read").reading {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            state.lock().expect("a read").last.clone()
        };
        read_words(&state, SLOW_READ, || Ok("up".into()));
        assert_eq!(settled(&state), Some(Ok("up".into())));
        state.lock().expect("a read").at = now_ms() - 61_000;
        read_words(&state, SLOW_READ, || Ok("again".into()));
        assert!(!state.lock().expect("a read").reading);
        assert_eq!(settled(&state), Some(Ok("up".into())));
    }

    /// Done-when 2 of rust-home-prompt: what runs and the build line, Home's last part, end /tasks, where `make rust/serves`
    /// reads it; another project's board has none.
    #[test]
    fn what_runs_ends_the_tasks_page() {
        let _budget = TestBudget::new();
        let look = look_empty();
        let tasks = surface::text(&list_tree(
            &look,
            TaskView::Board,
            None,
            "runs\nBuilt from y.\n",
        ))
        .expect("a text");
        assert!(
            tasks.ends_with("\nWhat runs\nruns\nBuilt from y.\n"),
            "{tasks}"
        );
        let klar = list_tree(
            &look,
            TaskView::Board,
            Some("klar"),
            "runs\nBuilt from y.\n",
        );
        assert!(!surface::text(&klar).expect("a text").contains("Built from"));
    }

    /// Done-when 2 of rust-nav-polish: the board lists every project's open tasks, each in its column, and ?project= keeps
    /// it to one; Rust's own task is listed once.
    #[test]
    fn the_board_lists_every_project_s_tasks_and_keeps_to_one() {
        let _budget = TestBudget::new();
        let mut look = look(
            vec![stood("a-task", "open", None, &[])],
            queued(&["a-task"], 5),
            0,
            vec![],
        );
        let open = |p: &str, id: &str, s: &str| Open {
            project: p.into(),
            id: id.into(),
            status: s.into(),
            title: format!("{id} title"),
        };
        look.every = Some(Ok(vec![
            open("site_perso", "a-task", "open"),
            open("site_perso", "now-note", "in review"),
            open("klar", "12", "In Progress"),
            open("graal", "3", "Backlog"),
        ]));
        let board =
            |look: &Look, project: Option<&str>| list_tree(look, TaskView::Board, project, "");
        let shown = |project: Option<&str>| surface::text(&board(&look, project)).expect("a text");
        let all = shown(None);
        assert!(
            all.starts_with("Tasks\nViews: Board /tasks (shown), "),
            "{all}"
        );
        assert!(
            all.contains("\nIn Review (1)\nnow-note\tnow-note title\tsite_perso\t#site_perso\n"),
            "{all}"
        );
        assert!(
            all.contains("\nIn Progress (1)\n12\t12 title\tklar\t#klar\n"),
            "{all}"
        );
        assert!(
            all.contains("\nBacklog (1)\n3\t3 title\tgraal\t#graal\n"),
            "{all}"
        );
        assert_eq!(
            all.matches("\na-task\t").count(),
            1,
            "Rust's own once: {all}"
        );
        let klar = shown(Some("klar"));
        assert!(klar.starts_with("Tasks: klar\nViews: Board /tasks?project=klar (shown), List /tasks?layout=list&project=klar, "), "{klar}");
        assert!(
            klar.contains("\nIn Progress (1)\n12\t12 title\tIn Progress\t#klar\n")
                && !klar.contains("a-task")
                && !klar.contains("Next:"),
            "{klar}"
        );
        assert!(
            shown(Some("site_perso")).contains("a-task\t"),
            "Rust's own are site_perso's"
        );
        // done-when 1 of rust-board-linear: each card's label is its project, and Rust's own are Rust's to run
        let own = all
            .lines()
            .find(|l| l.starts_with("a-task\t"))
            .expect("Rust's own");
        assert!(own.ends_with("\t#site_perso\t@Rust"), "{own}");
        // the list's rows show no label nor assignee, so neither does its text
        let list = surface::text(&list_tree(&look, TaskView::List, None, "")).expect("a text");
        assert!(
            !list.contains("#site_perso") && !list.contains("@Rust"),
            "{list}"
        );
        let html = surface::screen::page("t", &list_tree(&look, TaskView::Board, None, ""))
            .expect("a page");
        assert!(
            html.contains(r#"data-task="12" href="/projects/klar?issue=12""#),
            "another project's task opens on its board: {html}"
        );
        look.every = Some(Err("down".into()));
        let down = surface::text(&board(&look, None)).expect("a text");
        assert!(
            down.contains(
                "The other projects' tasks are not shown: the older app did not answer: down"
            ),
            "{down}"
        );
    }

    #[test]
    fn a_read_that_failed_keeps_the_last_words() {
        let _budget = TestBudget::new();
        let said: Result<String, String> = Ok("2 things wait".into());
        assert_eq!(kept(Err("timed out".into()), Some(&said)), said);
        assert_eq!(
            kept(Ok("1 thing".into()), Some(&said)),
            Ok("1 thing".into())
        );
        assert_eq!(kept(Err("down".into()), None), Err("down".into()));
        assert_eq!(
            kept(Err("down".into()), Some(&Err("before".into()))),
            Err("down".into())
        );
    }

    /// Done-when 1 and 4 of rust-views-differential: /tasks' columns are the board view's, fed by the store's own commits.
    #[test]
    fn the_board_view_follows_the_stores_commits() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("tasks-board-view");
        let locks = dir.0.join("worktrees");
        let views = board_views(store::open(&dir.0, &locks).expect("open")).expect("views");
        let mut tasks = store::open(&dir.0, &locks).expect("a writer");
        let mut queue = queued(&["a", "b"], 2);
        queue.tasks[1].waits = vec!["a".into()];
        tasks.sync(&queue).expect("synced");
        let board = || columns(views.rows("board").expect("the board"));
        let want = |pairs: &[(&str, &'static str)]| {
            (pairs.iter())
                .map(|(id, c)| (id.to_string(), *c))
                .collect::<HashMap<_, _>>()
        };
        assert_eq!(board(), want(&[("a", "todo"), ("b", "backlog")]));
        TaskStore::claim(&mut tasks, "a").expect("claimed");
        assert_eq!(board(), want(&[("a", "in-progress"), ("b", "backlog")]));
        let merged = Run {
            pane: None,
            session: None,
            outcome: Outcome::Merged("pr".into()),
        };
        TaskStore::record(&mut tasks, "a", &merged).expect("recorded");
        assert_eq!(
            board(),
            want(&[("b", "todo")]),
            "a merged: off the board, and b's wait met"
        );
    }

    fn look_empty() -> Look {
        look(vec![], queued(&[], 5), 0, vec![])
    }

    /// Done-when 3: the comment on the working task's own card, and nothing beside the list.
    #[test]
    fn a_task_on_a_comment_shows_on_its_card_and_no_panel_lists_working_tasks() {
        let _budget = TestBudget::new();
        let standing = vec![
            stood("busy", "claimed", None, &[]),
            stood("quiet", "open", None, &[]),
        ];
        let mut look = look(standing, queued(&["quiet"], 5), 1, vec![]);
        look.issues = Some(Ok(vec![
            issue("busy", "Busy", &["fix <this>", "and that"]),
            issue("quiet", "Quiet", &[]),
        ]));
        let page = list_page(&look);
        let card = |id: &str| {
            let at = page.find(&format!(r#"data-task="{id}""#)).expect(id);
            let after = at + 1;
            let end = page[after..]
                .find("data-task=")
                .map_or(page.len(), |n| after + n);
            page[at..end].to_string()
        };
        assert!(
            card("busy").contains(r#"data-comments="2">2 comments"#),
            "{page}"
        );
        assert!(!card("quiet").contains("data-comments"), "{page}");
        // The working task once, on its card in its group: no aside, no panel, no second list of it.
        assert_eq!(page.matches(r#"data-task="busy""#).count(), 1, "{page}");
        assert!(!page.contains("aside"), "{page}");
        assert!(!page.contains("panel"), "{page}");
        let opened = task_page(&look, "busy");
        // each in its Activity, a comment of his
        assert!(opened.contains("Activity"), "{opened}");
        let said = r#"class="doc-words">fix &#60;this&#62;"#;
        assert!(opened.contains(said), "{opened}");
        assert!(!task_page(&look, "quiet").contains(r#"class="doc-thread""#));
    }

    #[test]
    fn only_a_pull_request_link_counts() {
        let _budget = TestBudget::new();
        assert_eq!(
            pr_in("see https://github.com/o/r/issues/3 then https://github.com/o/r/pull/4."),
            Some("https://github.com/o/r/pull/4")
        );
        assert_eq!(pr_in("https://evil.example/pull/4"), None);
        let twelve = Some("https://github.com/o/r/pull/12");
        for said in [
            "pr: [#12](https://github.com/o/r/pull/12)",
            "pr:https://github.com/o/r/pull/12, ready",
            "<https://github.com/o/r/pull/12>",
            "https://github.com/o/r/pull/12/files",
        ] {
            assert_eq!(pr_in(said), twelve, "{said}");
        }
        assert_eq!(pr_in("https://github.com/o/r/pull/"), None);
        for not_one in [
            "https://github.com/o/r/blob/main/pull/12",
            "https://github.com/pull/12",
            "https://github.com/o/r/pull/12abc",
        ] {
            assert_eq!(pr_in(not_one), None, "{not_one}");
        }
        assert_eq!(
            pr_in("[i](https://github.com/o/r/issues/3),[p](https://github.com/o/r/pull/4)"),
            Some("https://github.com/o/r/pull/4")
        );
        assert_eq!(pr_in(""), None);
    }
}
