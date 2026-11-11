//! The binary's wiring of section 20b's orchestrator: the store, the older app's list and judge, herdr, the approved
//! `make claude`, Claude Code, git and GitHub, on orch. One process per run: a run that crashes takes its claim's lock with it
//! and nothing else, and `dash orch` frees it on its next look.
/// Where the browser finds the front door, the older app's pages behind it: a link the window opens there, and the live page a
/// deploy reads « Built from » on. The Makefile's RUST_PORT.
pub(crate) const FRONT_DOOR: &str = "http://127.0.0.1:7788";
use crate::kinds::{Agents, ByKind};
use crate::pages::{self, Asked, Page};
use crate::reporting::{reporting, say_memory};
use dash_agent::ClaudeCode;
use dash_agent::codex::Codex;
use dash_bridge::{Dash, Issue, Open};
use dash_core::tasks::{
    Agent, Claim, Demand, Forge, Launcher, Limit, ORCHESTRATOR, Outcome, Pick, Plan, Queue, Queued,
    Standing, TaskList, TaskStore, Terminal, Verdict, all_waits, plan_at,
};
use dash_forge::GitHub;
use dash_launcher::{ApprovedCodex, ApprovedMake, mxc};
use dash_meter::cap::STOP_CODE;
use dash_orch::{
    Pace, Ports, adopt_task, deploy_pending, hand_backs, next, plan_now, redeploy, run_task_on,
    seats_taken,
};
use dash_policy::{BUILD_FREE_PERCENT, default_route, free, route_of};
use dash_store::older::{OWNERS, settable_labels};
use dash_store::tasks::{self as store, AgentRow, Event, Row, SqliteTasks};
use dash_surface::{self as surface, Drawing, TaskView};
use dash_views::Views;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    // `PM.run_cap_s/0`: a shell still running past 2 hours is a hang, and its stop is judged.
    shell_cap: Duration::from_secs(2 * 60 * 60),
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
    list: List,
    terminal: Box<dyn Terminal>,
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
    crate::pages::repo()
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

/// A read started at `started` with each of his picks noted since kept over it: one read before his pick never puts the old
/// value back (/code-review of PR 1399). Picks it read are forgotten.
fn kept_picks(mut read: Sides, queue: &mut QueueRead, started: i64) -> Sides {
    let last = queue.sides.as_deref();
    queue.noted.retain(|id, at| {
        let kept = *at >= started;
        if let Some(side) = last.and_then(|l| l.tasks.get(id)).filter(|_| kept) {
            read.tasks.insert(id.clone(), side.clone());
        }
        kept
    });
    read
}

/// Every task's sidebar and the epic list, off the older app's file (task rust-issue-sidebar-menus).
fn read_sides() -> Result<Sides, String> {
    let db = older_db();
    Ok(Sides {
        tasks: dash_store::older::sidebars(&db)?,
        epics: dash_store::older::epics(&db)?,
    })
}

/// One sync of the list `db` holds into `tasks`, the first step of each pass: what the older app filed is pickable within it.
/// A read that fails leaves the last list synced.
pub(crate) fn sync_list(tasks: &mut SqliteTasks, db: &Path) -> Result<usize, String> {
    tasks.sync(&dash_store::older::queue(db)?)
}

/// The data folder of the running tool, outside git: what `dash serve --port <n>` reads, and `make rust/serve` names.
pub fn live_data() -> PathBuf {
    crate::pages::live_data()
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

/// The older app's list of tasks and its judge, with Rust's own brief (task brief-tests-prepush-skip; @nha, 2026-11-10:
/// « remember we move to rust. no more elixir, so if something needs fixing port and then fix »): `dash task brief`, the
/// older app's `Launch.argv/2` through herdr's `session.sh brief`, is no longer asked.
struct List(Dash);

impl TaskList for List {
    fn brief(&self, id: &str, folder: &str) -> Result<String, String> {
        let repo = repo();
        brief(&older_db(), &repo, id, &writable(&repo, Path::new(folder)))
    }

    fn judge(&self, id: &str, said: &str) -> Result<Verdict, String> {
        self.0.judge(id, said)
    }

    fn merged(&self, id: &str, pr: &str) -> Result<(), String> {
        self.0.merged(id, pr)
    }

    fn merged_later(&self, id: &str, pr: &str) -> Result<(), String> {
        self.0.merged_later(id, pr)
    }

    fn ended(&self, id: &str, why: &str) -> Result<(), String> {
        self.0.ended(id, why)
    }

    fn reopen(&self, id: &str) -> Result<(), String> {
        self.0.reopen(id)
    }

    fn flag(&self, id: &str, what: &str) -> Result<(), String> {
        self.0.flag(id, what)
    }

    fn waiting(&self, id: &str, on: Option<&Outcome>) -> Result<(), String> {
        self.0.waiting(id, on)
    }

    fn closed(&self, ids: &[String]) -> Result<Vec<String>, String> {
        let places = dash_store::older::places(&older_db(), ids)?;
        Ok(places
            .into_iter()
            .filter(|(_, p)| p.closed)
            .map(|(id, _)| id)
            .collect())
    }
}

/// The folders a session in `folder` may write, read from the approved copy's profile for the claude job, the one `dash
/// claude` runs it under: those under the projects' folder (`repo`'s) and the worktrees' (`folder`'s). None, said on stderr,
/// when the profile cannot be read: the brief then names no folder rather than a guess (task rust-live-copy).
fn writable(repo: &Path, folder: &Path) -> Vec<String> {
    match mxc::approved_profile(&home(), &folder.display().to_string(), "claude") {
        Ok(profile) => {
            let roots: Vec<String> = [repo.parent(), folder.parent()]
                .into_iter()
                .flatten()
                .map(|p| p.display().to_string())
                .collect();
            let roots: Vec<&str> = roots.iter().map(String::as_str).collect();
            dash_launcher::brief::writable(&profile, &roots)
        }
        Err(why) => {
            eprintln!("brief: the folders it may write: {why}");
            vec![]
        }
    }
}

/// Task `id`'s brief, written from its row in the older app's database `db`, with the decided rules of each RFC it cites
/// read from `repo`'s private/docs: a page not there cites none, as the older app read it. `writable`: `writable`'s folders.
pub(crate) fn brief(
    db: &Path,
    repo: &Path,
    id: &str,
    writable: &[String],
) -> Result<String, String> {
    let filed =
        dash_store::older::brief(db, id)?.ok_or(format!("brief: no task {id} in the list"))?;
    // as `Launch.argv/2` refused a task with no :run: a brief with no prompt would start a session on nothing
    if filed.prompt.trim().is_empty() {
        return Err(format!(
            "brief: task {id} has no :run :prompt yet: nothing to launch"
        ));
    }
    let mut rules = vec![];
    for name in dash_launcher::brief::cited(&filed) {
        let Ok(html) = std::fs::read_to_string(repo.join(format!("private/docs/{name}.html")))
        else {
            continue;
        };
        let each = dash_launcher::brief::rules_of(&html).into_iter();
        rules.extend(each.map(|(rule, text)| (format!("{name}#{rule}"), text)));
    }
    let (base, repo) = (github().base, repo.display().to_string());
    Ok(dash_launcher::brief::brief(
        &filed, &rules, &base, &repo, writable,
    ))
}

/// `dash task brief <id>`: the brief a run of it starts from, printed, its folders those of a session in this folder (a run
/// reads its own worktree's, made when it starts).
pub fn print_brief(id: &str, mut out: File) {
    let repo = repo();
    let text = or_exit(brief(
        &older_db(),
        &repo,
        id,
        &writable(&repo, &this_folder()),
    ));
    writeln!(out, "{text}").expect("stdout");
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
    Tools {
        list: List(older_app()),
        terminal: dash_terminal::from_env(),
        launcher: ApprovedMake { home: home.clone() },
        codex: ApprovedCodex {
            home: home.clone(),
            opener: dash_agent::codex::OPENER,
        },
        agents: Agents::new(claude_code(&home), codex(&home), data),
        forge: github(),
        policy: ByKind { is_codex },
    }
}

/// The one forge: the orchestrator's runs and `dash land` deploy through it, so each merge restarts the look on the new binary.
fn github() -> GitHub {
    GitHub {
        repo: repo(),
        root: worktrees(),
        base: "main".into(),
        // the build line alone (task rust-home-prompt): Home lost it, and no page's layout moves it again
        page: format!("{FRONT_DOOR}/built"),
        cli: "gh".into(),
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
        terminal: &*tools.terminal,
        launcher,
        agent,
        forge: &tools.forge,
        policy: &tools.policy,
        step,
        free_memory: &free_memory,
    }
}

/// The Mac's free memory in percent, as `memory_pressure` prints it (« System-wide memory free percentage: 57% »), the reading
/// the older app's start gate and bin/orchestrator-job take; `None` when it cannot be read, which holds nothing back.
pub(crate) fn free_memory() -> Option<u8> {
    let out = std::process::Command::new("memory_pressure")
        .output()
        .ok()?;
    free_percent_in(&String::from_utf8_lossy(&out.stdout))
}

fn free_percent_in(text: &str) -> Option<u8> {
    let (_, rest) = text.split_once("free percentage: ")?;
    rest.split('%').next()?.trim().parse().ok()
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

/// Rust's list, when it was last synced (Unix milliseconds), and why the read or sync since failed when it kept that list.
type QueueAt = (Result<Queue, String>, i64, Option<String>);

/// What a read leaves: a list synced now, else the last list kept with its time and why the read failed, so the note says
/// how old it is (rust/CLAUDE.md: a list is never empty while its source refills). With no list before, the failure.
fn next_queue(
    read: Result<(Queue, Result<usize, String>), String>,
    last: Option<QueueAt>,
    now_ms: i64,
) -> QueueAt {
    match (read, last) {
        (Ok((queue, Ok(_))), _) => (Ok(queue), now_ms, None),
        (Ok((queue, Err(e))), Some((Ok(_), at, _))) => {
            (Ok(queue), at, Some(format!("its sync failed: {e}")))
        }
        // a whole read that failed (the store locked, its 60 s limit) keeps the last list too, never blanks the page
        (Err(e), Some((Ok(queue), at, _))) => {
            (Ok(queue), at, Some(format!("its read failed: {e}")))
        }
        // Its reason shown where it is read: the picker keeps the stored list, and the orch line says so too.
        (Ok((_, Err(e))), _) => (Err(format!("its sync failed: {e}")), now_ms, None),
        (Err(e), _) => (Err(e), now_ms, None),
    }
}

/// What the issue pages' menus show and offer, read off the older app's list with the queue (task rust-issue-sidebar-menus): each
/// task's sidebar, and the projects a task may join.
#[derive(Clone, Default)]
struct Sides {
    tasks: BTreeMap<String, dash_store::older::Sidebar>,
    epics: Vec<String>,
}

#[derive(Default)]
struct QueueRead {
    last: Option<QueueAt>,
    /// What a task's page shows that this store does not hold, read with it.
    issues: Option<Result<Vec<Issue>, String>>,
    /// The sessions, each kind's spend and which queued tasks are Codex's, read with it: what a pick reads beside the queue.
    seats: Option<Result<Seats, String>>,
    /// The issue pages' sidebars, read with it; a read that failed keeps the last. Shared, so a look copies no row.
    sides: Option<Arc<Sides>>,
    /// When each of his picks was read into `sides` (`Board::noted_side`): a read started before it keeps it.
    noted: HashMap<String, i64>,
    reading: bool,
    /// When the last read ended, failed or not: the next starts a minute later, never at each look after a failure.
    tried_ms: i64,
}

impl QueueRead {
    fn stale(&self, now_ms: i64) -> bool {
        self.last.is_none() || now_ms - self.tried_ms >= ORCH_POLL.as_millis() as i64
    }
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

/// The sessions open, each kind's limits, the Codex tasks and the free memory: what the page's plan reads beside the queue.
type Seats = (usize, Vec<Limit>, HashSet<String>, Option<u8>);

/// What the page at / reads: this store's runs, and what the older app says Rust may pick next.
pub struct Board {
    tasks: SqliteTasks,
    /// Section 6's views, fed by this store's op log: /tasks' columns are the board view's. Started again if their thread ends
    /// (`restarted`); shared with each `patches` thread, which follows the new ones.
    views: Arc<Mutex<Views>>,
    queue: Arc<Mutex<QueueRead>>,
    every: Arc<Mutex<Polled<EveryRead>>>,
    data: PathBuf,
}

impl Board {
    pub fn open(dir: &Path) -> Result<Board, String> {
        let tasks = open_tasks(dir)?;
        let views = Arc::new(Mutex::new(board_views(open_tasks(dir)?)?));
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
            restarted(&mut views, &self.data, &why)?;
            views.rows("board")
        })
    }

    /// The views started again, as `board_view` does on a failed read: what a `patches` follower meets (its test).
    #[cfg(test)]
    pub(crate) fn restart_views(&self) -> Result<(), String> {
        let mut views = self.views.lock().unwrap_or_else(|e| e.into_inner());
        restarted(&mut views, &self.data, "a test's")
    }

    /// The ids of the elements each diff of the board view changes (`patched`), from the next diff on, on a thread of its
    /// own: what a page that follows the board patches, the web's `?view=patches` and the window alike (task rust-view-patches).
    /// The views started again (`restarted`, on any failed read) end every subscription they had: the thread follows the new
    /// ones and names the board whole first, for the diffs it missed, so the window's one stream never ends silently (/code-review
    /// of PR 1322). It ends at the first diff after the receiver is dropped. ponytail: while the views cannot start, it tries
    /// again every FOLLOW, a receiver gone unseen until a diff; the store is down then, and every page with it.
    pub fn patches(&self) -> Result<std::sync::mpsc::Receiver<Vec<String>>, String> {
        let (views, data) = (Arc::clone(&self.views), self.data.clone());
        let follow = move || -> Result<std::sync::mpsc::Receiver<dash_views::Diff>, String> {
            let mut views = views.lock().unwrap_or_else(|e| e.into_inner());
            views.subscribe("board").or_else(|why| {
                restarted(&mut views, &data, &why)?;
                views.subscribe("board")
            })
        };
        let mut diffs = follow()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("board-patches".into())
            .spawn(move || {
                let mut first = true;
                loop {
                    // the rows at once: the page drew them; after a restart, the board whole, for the diffs it missed
                    match diffs.recv() {
                        Ok(_) if first => first = false,
                        Ok(_) => {
                            if tx.send(vec![BOARD_ELEMENT.into()]).is_err() {
                                return;
                            }
                        }
                        // ended before its rows: views dying as they start, followed again after a pause
                        Err(_) => std::thread::sleep(dash_views::FOLLOW),
                    }
                    for diff in &diffs {
                        let ids = patched(&diff);
                        if !ids.is_empty() && tx.send(ids).is_err() {
                            return;
                        }
                    }
                    diffs = loop {
                        match follow() {
                            Ok(diffs) => break diffs,
                            Err(why) => {
                                eprintln!("dash: the board's patches: {why}");
                                std::thread::sleep(dash_views::FOLLOW);
                            }
                        }
                    };
                }
            })
            .map_err(|e| format!("the board's patches: {e}"))?;
        Ok(rx)
    }

    fn look(&self) -> Result<Look, String> {
        let rows = self.tasks.rows()?;
        let columns = columns(self.board_view()?);
        let standing = dash_core::tasks::TaskStore::standing(&self.tasks)?;
        let now_ms = now_ms();
        let (queue, issues, seats, sides) = {
            let mut queue = self.queue.lock().expect("the queue");
            if queue.stale(now_ms) && !queue.reading {
                let claims = dash_core::tasks::TaskStore::claims(&self.tasks)?;
                queue.reading = self.read_queue(claims);
            }
            (
                queue.last.clone(),
                queue.issues.clone(),
                queue.seats.clone(),
                queue.sides.clone(),
            )
        };
        let plan = match (&queue, seats) {
            (Some((Ok(queue), _, _)), Some(Ok((sessions, limits, codex, free)))) => {
                Some(Ok(plan_at(
                    &ByKind {
                        is_codex: |id: &str| codex.contains(id),
                    },
                    queue.clone(),
                    &standing,
                    sessions,
                    limits,
                    free,
                    (now_ms / 1000) as u64,
                )))
            }
            (Some((Ok(_), _, _)), Some(Err(e))) => Some(Err(e)),
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
            sides,
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
    pub fn graph(&self, shown: &Shown) -> Result<Drawing, String> {
        Ok(graph_drawing(&self.look()?, shown))
    }

    /// The page at /tasks/<id>; `None`: no open task of that id.
    pub fn task(&self, id: &str) -> Result<Option<Drawing>, String> {
        let mut look = self.look()?;
        // the board's every project, so its place and steps follow the board he came from (/code-review of PR 1395)
        look.every = self.read_every();
        let page = |tree| Drawing::Tree {
            title: format!("Rust task: {id}"),
            tree,
        };
        // a history not read leaves the rest of the page standing, and says so (/code-review of rust-task-page-linear)
        let history = self.tasks.history(id);
        Ok(task_tree(&look, id, &history).map(page))
    }

    /// His pick in a task page's menu, kept in the cached queue at once, so the page drawn after it shows it before the next read
    /// (/code-review of PR 1395); the next read then takes over. `priority`: 1 to 4, none to clear; `deadline` a day.
    pub fn noted(&self, id: &str, priority: Option<Option<u8>>, deadline: Option<Option<String>>) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((Ok(listed), _, _)) = &mut queue.last else {
            return;
        };
        for task in listed.tasks.iter_mut().filter(|t| one_line(&t.id) == id) {
            if let Some(p) = priority {
                task.rank.priority = p;
            }
            if let Some(d) = &deadline {
                task.rank.deadline = d.clone();
            }
        }
        queue.tried_ms = 0;
    }

    /// His pick in a sidebar menu, its task's sidebar read again into the cache at once, so the page drawn after it shows it
    /// (task rust-issue-sidebar-menus); a read that fails leaves the next minute's read to show it.
    pub fn noted_side(&self, id: &str) {
        let Ok(Some(side)) = dash_store::older::sidebar(&older_db(), id) else {
            return;
        };
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(sides) = &mut queue.sides {
            Arc::make_mut(sides).tasks.insert(id.to_string(), side);
            queue.noted.insert(id.to_string(), now_ms());
        }
    }

    /// His drag of `id`'s card to `column` on /tasks (task rust-board-linear-order): kept in the store, so the board view puts
    /// it there until a run claims it. Only a card the board view places moves, and only to a column the board has: the inner
    /// `Err` is that refusal, the outer one a store or views failure.
    pub fn move_to(&mut self, id: &str, column: &str) -> Result<Result<(), String>, String> {
        if !dash_views::rules::BOARD.contains(&column) {
            return Ok(Err(format!("{column}: no such column")));
        }
        if !columns(self.board_view()?).contains_key(id) {
            return Ok(Err(format!("{id}: not a card of Rust's board")));
        }
        self.tasks.move_to(id, column).map(Ok)
    }

    /// Whether `id` is a task this board lists, from what has answered: only such a task's old link opens its page here. Before
    /// the older app's list answers, an id it alone would list is not one yet (/code-review of rust-task-page-linear).
    pub fn holds(&self, id: &str) -> Result<bool, String> {
        Ok(listed(&self.look()?)
            .iter()
            .any(|t| t.id == id && t.other.is_none()))
    }

    /// Only while someone reads the page, and once at boot (`start_reads`), so a closed page costs the older app nothing more. On its own thread: a read can take its
    /// 60 s limit, and the store's loop is serial. A page's skeleton fills once it ends; true when the thread started.
    fn read_queue(&self, claims: Vec<Claim>) -> bool {
        let shared = Arc::clone(&self.queue);
        let data = self.data.clone();
        let started = std::thread::Builder::new().spawn(move || {
            // As the picker's pass: this store's list, synced first. A sync that fails keeps the last list and the time it was
            // synced, so the page's note ages and says the sync stopped.
            let synced = crate::reporting::read("queue", || {
                open_tasks(&data).and_then(|mut tasks| {
                    let sync = sync_list(&mut tasks, &older_db());
                    TaskStore::queue(&tasks).map(|queue| (queue, sync))
                })
            });
            let last = shared.lock().expect("the queue").last.clone();
            // a whole read that failed asks nothing more, though its last list is kept: its wait would only double
            let store_answered = synced.is_ok();
            let (read, at, stopped) = next_queue(synced, last, now_ms());
            let list_known = read.is_ok();
            let answered = store_answered && list_known;
            let ids: Vec<String> = read
                .iter()
                .flat_map(|q| &q.tasks)
                .map(|t| t.id.clone())
                .collect();
            // Shown at once: a slow terminal never holds back the older app's line.
            shared.lock().expect("the queue").last = Some((read, at, stopped));
            // the issue pages' menus, from the same file: a read that fails keeps the last (task rust-issue-sidebar-menus)
            let started = now_ms();
            if let Ok(sides) = crate::reporting::read("sidebars", read_sides) {
                let mut queue = shared.lock().expect("the queue");
                queue.sides = Some(Arc::new(kept_picks(sides, &mut queue, started)));
            }
            // No queue, no plan: nothing else is asked of the terminal.
            let seats = answered.then(|| {
                crate::reporting::read("seats", || {
                    let sessions = seats_taken(&*dash_terminal::from_env(), &claims)?;
                    // No HOME is no spend known. `home()` would exit, and from this thread that ends the whole server.
                    let home = std::env::var("HOME");
                    // The picker's own agent, so the page names the start the picker makes. ponytail: a Codex read and a kind read
                    // per queued task at each of the page's looks, a minute apart; the picker's cache if those show.
                    let limits = home.map(|home| {
                        let agents = Agents::new(claude_code(&home), codex(&home), &data);
                        agents.limits().unwrap_or_default()
                    });
                    let codex = ids.into_iter().filter(|id| is_codex(id)).collect();
                    Ok((sessions, limits.unwrap_or_default(), codex, free_memory()))
                })
            });
            // a kept list keeps its last seats too, so its Next line stays drawn while the store is locked
            if seats.is_some() || !list_known {
                shared.lock().expect("the queue").seats = seats;
            }
            // Last, so a slow answer holds back no Next line; not after a queue that failed, whose wait would only double.
            let issues =
                answered.then(|| crate::reporting::read("task issues", || older_app().issues()));
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
            queue.tried_ms = now_ms();
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
                let read = crate::reporting::read("task ls", || older_app().every_task());
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

/// Each page read, a new one started on its own thread once its poll has passed, its span named by its command.
fn handoff() -> Option<Words> {
    read_words(&READS.handoff, ORCH_POLL, "handoff", || {
        older_app().handoff()
    })
}

fn progress() -> Option<Words> {
    read_words(&READS.progress, ORCH_POLL, "progress", || {
        older_app().progress()
    })
}

fn roadmap() -> Option<Words> {
    read_words(&READS.roadmap, ORCH_POLL, "focus roadmap", || {
        older_app().roadmap()
    })
}

fn feedback_count() -> Option<Words> {
    read_words(&READS.feedback, SLOW_READ, "feedback count", || {
        older_app().feedback_count()
    })
}

fn cost_fit() -> Option<Words> {
    read_words(&READS.cost, SLOW_READ, "cost fit", || {
        older_app().cost_fit()
    })
}

/// The reads a page waits on, started now: `dash serve` calls it at boot, so the first page after a deploy shows what was read,
/// never its loader for the length of a read (task rust-page-latency; @nha, 2026-11-10: « I have the loader state showing for
/// some time. 10+ seconds. »). One read each a deploy: the polls go on only while a page is read. /insights' two stay its own
/// to start, `feedback count` took 48 s (/code-review of rust-page-latency). Each read is on its own thread; a board not read
/// is said, never fatal.
pub fn start_reads(board: &Board) {
    let _ = (handoff(), progress(), roadmap());
    if let Err(e) = board.look() {
        eprintln!("dash: the board's reads at boot: {e}");
    }
    let _ = board.read_every();
}

/// /inbox: what waits on him, then today's daily (task rust-home-prompt: Home's first two parts).
/// Its search field sits in one place, under its title, before its parts; `found`, the words and their hits from /search,
/// draws its results right under it (task rust-inbox-search-place; @nha, 2026-11-09: « we will need to find a good place for
/// the search when we get to rebuild that too »).
pub fn inbox_page(found: Option<(&str, Result<Vec<dash_core::registry::Hit>, String>)>) -> Drawing {
    let words = found.as_ref().map_or("", |(w, _)| w);
    let mut top = Map::new();
    top.insert(
        "search".into(),
        json!({"type": "search-bar", "props": {"placeholder": "Search the store", "value": words,
            "hidden": [["in", INBOX_SEARCH]]}}),
    );
    let mut children = vec!["search".to_string()];
    // no words, no results: the plain inbox (/code-review: an empty search said nothing was found for « »)
    if let Some((words, hits)) = found.filter(|(w, _)| !w.trim().is_empty()) {
        let text = |t: String| json!({"type": "text", "props": {"text": t}});
        let inside = match hits {
            Err(e) => {
                let e: String = one_line(&e).chars().take(200).collect();
                top.insert("found-none".into(), text(format!("The search failed: {e}")));
                "found-none"
            }
            Ok(hits) if hits.is_empty() => {
                top.insert(
                    "found-none".into(),
                    text(format!("Nothing found for « {words} ».")),
                );
                "found-none"
            }
            Ok(hits) => {
                let mut rows = vec![];
                for (i, hit) in hits.iter().enumerate() {
                    let line = if hit.body.is_empty() {
                        format!("fact {}", hit.id)
                    } else {
                        one_line(&hit.body)
                    };
                    let card = json!({"type": "card", "props": {"title": one_line(&hit.title), "line": line}});
                    top.insert(format!("found-{i}"), card);
                    rows.push(format!("found-{i}"));
                }
                top.insert(
                    "found-list".into(),
                    json!({"type": "list", "children": rows}),
                );
                "found-list"
            }
        };
        let section =
            json!({"type": "section", "props": {"title": "Results"}, "children": [inside]});
        top.insert("found".into(), section);
        children.push("found".into());
    }
    let (handoff, progress) = (handoff(), progress());
    let waits = |s: &str| crate::home::cards(s, crate::home::handoff_link);
    let daily = |s: &str| crate::home::cards(s, crate::home::progress_link);
    let parts: [Part; 2] = [
        ("waits", "Waiting on you", handoff.as_ref(), &waits),
        ("daily", "Today's daily", progress.as_ref(), &daily),
    ];
    read_page("Inbox", (top, children), &parts)
}

/// /plan: his focuses by horizon, Urgent, Now, Next, Later (task rust-home-prompt).
pub fn plan_page() -> Drawing {
    let roadmap = roadmap();
    let parts: [Part; 1] = [(
        "roadmap",
        "The roadmap",
        roadmap.as_ref(),
        &crate::home::roadmap,
    )];
    read_page("Plan", Default::default(), &parts)
}

/// /insights: his feedback against the work done, and what a task costs per estimate point (task rust-home-prompt).
pub fn insights_page() -> Drawing {
    let (feedback, cost) = (feedback_count(), cost_fit());
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
    read_page("Insights", Default::default(), &parts)
}
/// The `in=` /inbox's search field sends: /search draws its results on /inbox, where any other /search stays the older app's
/// (@nha's orchestrator, 2026-11-10: « FindLive kept for a browser's /search for now », its port task rust-search-port).
pub const INBOX_SEARCH: &str = "inbox";
/// /inbox's path, which /search's answer to its field names too; its registration below spells it, so dash-gate reads it.
pub const INBOX_PAGE: &str = "/inbox";
/// The board's path, which its patch stream and the window's first page name too; its registration below spells it, so
/// dash-gate reads it.
pub const BOARD_PAGE: &str = "/tasks";

// Home and its parts read nothing (task rust-home-prompt); the board's pages lock it. Each registered here, beside its drawing
// (pages.rs, task rust-registry-no-hubs).
inventory::submit! {
    Page { path: "/", step: "route_page", reads: pages::Reads::Nothing, redraw: false, serve: |a| a.form(Some(Ok(home_page()))) }
}
inventory::submit! {
    Page { path: "/inbox", step: "route_inbox", reads: pages::Reads::Nothing, redraw: true, serve: |a| a.form(Some(Ok(inbox_page(None)))) }
}
inventory::submit! {
    Page { path: "/plan", step: "route_plan", reads: pages::Reads::Nothing, redraw: true, serve: |a| a.form(Some(Ok(plan_page()))) }
}
inventory::submit! {
    Page { path: "/insights", step: "route_insights", reads: pages::Reads::Nothing, redraw: true, serve: |a| a.form(Some(Ok(insights_page()))) }
}
// /graph is the board's dependency view at its own URL (task rust-nav-board: « /graph /deps => are these not the same ? »)
inventory::submit! {
    Page { path: "/graph", step: "route_graph", reads: pages::Reads::Board, redraw: true, serve: |a| a.form(a.board::<Board>().map(|b| b.graph(&Shown::of(a)))) }
}
inventory::submit! {
    Page { path: "/tasks", step: "route_tasks", reads: pages::Reads::Board, redraw: true, serve: board_page }
}
inventory::submit! {
    Page { path: "/tasks/*", step: "route_task", reads: pages::Reads::Board, redraw: true, serve: task_page }
}

fn board_page(a: &Asked) -> crate::pages::Response {
    match TaskView::of(a.param("layout").as_deref()) {
        TaskView::Graph => a.form(a.board::<Board>().map(|b| b.graph(&Shown::of(a)))),
        view => {
            let project = a.param("project").filter(|p| !p.is_empty());
            a.form(
                a.board::<Board>()
                    .map(|b| b.tasks(view, project.as_deref())),
            )
        }
    }
}

fn task_page(a: &Asked) -> crate::pages::Response {
    match a.board::<Board>().map(|board| board.task(&a.id())) {
        Some(Ok(None)) => (
            "404 Not Found",
            crate::pages::TEXT,
            "no open task of that id".into(),
        ),
        page => a.form(page.and_then(Result::transpose)),
    }
}

/// `POST /tasks/<id>/set`: one of a task page's property menus (task rust-board-buttons-new-issue): the task, decoded. Never
/// an id `dash task update` would read as a flag.
pub(crate) fn set_posted(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    let path = words.next()?.split('?').next()?;
    let id = path.strip_prefix("/tasks/")?.strip_suffix("/set")?;
    // a path's segment: a '+' stays a plus (/code-review of PR 1395)
    let id = crate::pages::segment_decoded(id);
    (!id.is_empty() && !id.contains('/') && !id.starts_with('-')).then_some(id)
}

/// One read's last words, a new read started on its own thread once `every` has passed while its page is read. A read that
/// failed, or panicked, keeps the last words read: one slow answer never blanks a page, and a panic never stops the polling.
fn read_words(
    state: &Arc<Mutex<Polled<Words>>>,
    every: Duration,
    command: &'static str,
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
            let new = crate::reporting::read(command, || std::panic::catch_unwind(read))
                .unwrap_or_else(|_| Err("its read panicked".into()));
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
    /// `None` until the older app's list was read once.
    sides: Option<Arc<Sides>>,
}

/// The views started again on a failure (`why`), said once: a page's read (`Board::board_view`) and a `patches` thread alike,
/// so one failure never takes /tasks down until a restart.
fn restarted(views: &mut Views, data: &Path, why: &str) -> Result<(), String> {
    eprintln!("dash serve: the views failed ({why}), started again");
    *views = board_views(open_tasks(data)?)?;
    Ok(())
}

/// The views on their own thread, fed by this store's op log (`tasks` its own connection: the views read while a page writes).
fn board_views(tasks: SqliteTasks) -> Result<Views, String> {
    Views::start(Box::new(move |after| tasks.ops_after(after, 10_000)))
}

/// The board view's rows as /tasks' columns, whose words are the surface's statuses too: In Review among them, where his drag
/// put a card.
fn columns(board: Vec<dash_views::rules::Out>) -> HashMap<String, &'static str> {
    let column = |word: &str| dash_views::rules::BOARD.into_iter().find(|c| *c == word);
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
        Pick::Start { id, .. } | Pick::Adopt(id) => format!("Next: {}", one_line(id)),
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

/// Each open task and what it waits on, the plan's own words on each. A finished task is named only as what an open one waits on,
/// with no edge of its own; the graph leaves out a merged one (surface's `drawn`). A task's waits are its row's and the list's,
/// merged as `plan` merges them (`all_waits`): the list's alone drew no edge, one column of cards (task rust-graph-queue-edges).
/// A task held only by queue order says « queued » (`Held::queued`): no wait, and no edge.
fn graph_tasks(look: &Look) -> Vec<Value> {
    let plan = look.plan.as_ref().and_then(|p| p.as_ref().ok());
    let queue = look.queue.as_ref().and_then(|(q, _, _)| q.as_ref().ok());
    // By id on one line, the drawn name: two raw ids that read the same are one task, the first row standing for both, since a
    // graph refuses a name twice.
    let mut rows = HashMap::new();
    for row in &look.standing {
        rows.entry(one_line(&row.id)).or_insert(row);
    }
    let mut listed = HashMap::new();
    for task in queue.iter().flat_map(|q| &q.tasks) {
        listed.entry(one_line(&task.id)).or_insert(task);
    }
    let mut whys = HashMap::new();
    for held in plan.iter().flat_map(|p| &p.waiting) {
        whys.entry(one_line(&held.id)).or_insert(held.why.as_str());
    }
    // Finished as the store says: a reopen is the store's to judge (`reopen_held`, with the open-task cap), and the next
    // `dash orch` pass resets the row; a copy of that rule here disagreed with it (second /simplify of #1197).
    let finished = |id: &str| rows.get(id).is_some_and(|r| r.status == "finished");
    let next = plan.and_then(|p| match &p.next {
        Pick::Start { id, .. } | Pick::Adopt(id) => Some(id.as_str()),
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
    let waits_of = |id: &str| -> Vec<String> {
        if finished(id) {
            return vec![];
        }
        let waits = all_waits(rows.get(id).copied(), listed.get(id).copied());
        waits.iter().map(|w| one_line(w)).collect()
    };
    // Grows as it goes: what a task names may name more. Each task's waits read once, here.
    let mut waits = HashMap::new();
    let mut at = 0;
    while let Some(id) = ids.get(at).cloned() {
        let named = waits_of(&id);
        for wait in &named {
            if seen.insert(wait.clone()) {
                ids.push(wait.clone());
            }
        }
        waits.insert(id, named);
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
        let waits = &waits[id];
        json!({"id": id, "state": state, "says": one_line(&words), "waits": waits})
    };
    ids.iter().map(task).collect()
}

fn queue_failed(look: &Look) -> bool {
    matches!(look.queue, Some((Err(_), _, _)))
}

/// /graph's page, `shown` its selectors.
fn graph_drawing(look: &Look, shown: &Shown) -> Drawing {
    Drawing::Tree {
        title: "Rust tasks: the graph".into(),
        tree: graph_tree(look, shown),
    }
}

/// /tasks' board or list, `project`'s tasks alone when named.
fn list_drawing(look: &Look, view: TaskView, project: Option<&str>, runs: &str) -> Drawing {
    Drawing::Tree {
        title: "Tasks".into(),
        tree: list_tree(look, view, project, runs),
    }
}

/// The graph's selectors, from its URL: only the tasks of this epic (or roadmap item) and this label are ranked into its rows.
#[derive(Default)]
pub struct Shown {
    pub epic: Option<String>,
    pub label: Option<String>,
}

impl Shown {
    /// `?epic=` and `?label=` of /tasks?layout=graph, an empty one none.
    pub fn of(a: &Asked) -> Shown {
        let given = |name| a.param(name).filter(|v| !v.is_empty());
        Shown {
            epic: given("epic"),
            label: given("label"),
        }
    }

    /// The graph's URL with `epic` and `label` as given, each none left out: what a selector's link and an epic's link open.
    fn href(epic: Option<&str>, label: Option<&str>) -> String {
        let mut href = "/tasks?layout=graph".to_string();
        for (name, value) in [("epic", epic), ("label", label)] {
            if let Some(v) = value {
                href += &format!("&{name}={}", surface::segment(v));
            }
        }
        href
    }
}

/// How many of a row's ranked tasks its graph draws, what runs and the next pick added past them (task rust-graph-why-these;
/// @nha, 2026-11-10: « top N tasks, and their dependencies »).
const TOP: usize = 8;

/// The graph's rows: the roadmap's horizons first to last, each titled by its word, then the tasks on no roadmap item, or on a
/// horizon `HORIZONS` does not name, drawn only when there is one (/code-review of rust-graph-why-these: a copy of the four
/// words would drop a fifth's tasks unseen).
fn rows() -> Vec<(&'static str, String)> {
    let title = |h: &str| h[..1].to_uppercase() + &h[1..];
    let horizons = dash_core::tasks::HORIZONS.iter().map(|h| (*h, title(h)));
    horizons
        .chain([("", "On no roadmap item".to_string())])
        .collect()
}

/// The screen at /tasks?layout=graph as a UI tree (task rust-graph-why-these; @nha, 2026-11-10: « visually show why these 3
/// (linking back to the top epic now/then/later). Perhaps this is even 3 sections with graphs »): what runs next, how old that
/// is, the epic and label selectors, then a row a horizon, Urgent to Later, each its top `TOP` tasks in rank order (`dash orch
/// next`'s), grouped by roadmap item, each item a link to the graph of its tasks alone and a graph of its tasks and what they
/// wait on, left to right by wait depth. What runs and the next pick are marked, each with its rank reason. Then each waiting
/// task's whole why, which a node may clip. With nothing open it says why in their place, and the rest stands.
fn graph_tree(look: &Look, shown: &Shown) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let tasks = graph_tasks(look);
    let today = today_of(look.now_ms);
    let queue = look.queue.as_ref().and_then(|(q, _, _)| q.as_ref().ok());
    let mut listed = HashMap::new();
    for task in queue.iter().flat_map(|q| &q.tasks) {
        listed.entry(one_line(&task.id)).or_insert(task);
    }
    // the plan's own rank and reasons (`dash orch next`'s), over every task offered: the plan's order leaves a claimed one out,
    // and what runs says its reason too
    let mut offered: Vec<Queued> = queue.iter().flat_map(|q| q.tasks.clone()).collect();
    let order = dash_core::tasks::rank(&mut offered, &look.standing);
    // each id's place in that order and its reason, its first place where two ids read the same
    let mut reasons: HashMap<String, (usize, String)> = HashMap::new();
    for (at, (id, why)) in order.into_iter().enumerate() {
        reasons.entry(one_line(&id)).or_insert((at, why));
    }
    let by_id: HashMap<String, &Value> = (tasks.iter())
        .map(|t| (t["id"].as_str().unwrap_or_default().to_string(), t))
        .collect();
    let state = |id: &str| {
        by_id
            .get(id)
            .and_then(|t| t["state"].as_str())
            .unwrap_or_default()
    };
    // its own tasks: what the list offers and what its store holds open, never a task only named as a wait
    let own: HashSet<String> = (listed.keys().cloned())
        .chain(look.standing.iter().map(|r| one_line(&r.id)))
        .collect();
    let horizon = |id: &str| match listed.get(id) {
        Some(q) if q.rank.deadline.is_some() => "urgent",
        Some(q) => (q.rank.horizon.as_deref())
            .filter(|h| dash_core::tasks::HORIZONS.contains(h))
            .unwrap_or_default(),
        None => "",
    };
    let item = |id: &str| listed.get(id).and_then(|q| q.rank.item.clone());
    let passes = |id: &str| {
        let q = listed.get(id);
        let epic = shown.epic.as_deref().is_none_or(|e| {
            q.is_some_and(|q| q.epic.as_deref() == Some(e) || q.rank.item.as_deref() == Some(e))
        });
        let label = shown
            .label
            .as_deref()
            .is_none_or(|l| q.is_some_and(|q| q.labels.iter().any(|x| x.eq_ignore_ascii_case(l))));
        epic && label
    };
    let marked = |id: &str| matches!(state(id), "running" | "next");
    // in rank order, the unranked (a run the list no longer offers) after, as graph_tasks lists them
    let mut candidates: Vec<(usize, usize, String)> = (tasks.iter().enumerate())
        .filter_map(|(at, t)| {
            let id = t["id"].as_str()?.to_string();
            let open = !matches!(state(&id), "finished" | "ended");
            (open && own.contains(&id) && passes(&id)).then(|| {
                let rank = reasons.get(&id).map_or(usize::MAX, |r| r.0);
                (rank, at, id)
            })
        })
        .collect();
    candidates.sort();
    let mut elements = Map::new();
    let heading = json!({"type": "heading", "props": {"text": "Rust tasks"}});
    elements.insert("title".into(), heading);
    elements.insert("views".into(), views(TaskView::Graph, None));
    elements.insert("next".into(), next_words(look));
    elements.insert("note".into(), note(look));
    let mut children: Vec<String> = ["title", "views", "next", "note"].map(String::from).into();
    // the selectors: every epic and label of the list, the one shown marked, each link keeping the other
    let mut epics = BTreeSet::new();
    let mut labels = BTreeSet::new();
    for q in queue.iter().flat_map(|q| &q.tasks) {
        epics.extend(q.epic.iter().chain(&q.rank.item).cloned());
        // a label any case is one filter, its first spelling shown
        for l in &q.labels {
            if !labels.iter().any(|x: &String| x.eq_ignore_ascii_case(l)) {
                labels.insert(l.clone());
            }
        }
    }
    let selector = |label: &str,
                    all: BTreeSet<String>,
                    current: Option<&str>,
                    href: &dyn Fn(Option<&str>) -> String| {
        let mut links =
            vec![json!({"name": "All", "href": href(None), "current": current.is_none()})];
        links.extend(all.iter().map(
            |v| json!({"name": v, "href": href(Some(v)), "current": current.is_some_and(|c| c.eq_ignore_ascii_case(v))}),
        ));
        json!({"type": "links", "props": {"label": label, "links": links}})
    };
    let (epic, label) = (shown.epic.as_deref(), shown.label.as_deref());
    elements.insert(
        "epics".into(),
        selector("Epic", epics, epic, &|e| Shown::href(e, label)),
    );
    elements.insert(
        "labels".into(),
        selector("Label", labels, label, &|l| Shown::href(epic, l)),
    );
    children.extend(["epics".into(), "labels".into()]);
    if candidates.is_empty() {
        let none = match (epic, label) {
            (None, None) => none_open(look).to_string(),
            _ => "No open Rust task of this epic and label.".to_string(),
        };
        elements.insert("graph".into(), text(none));
        children.push("graph".into());
    }
    for (row, title) in rows().iter().filter(|_| !candidates.is_empty()) {
        let members: Vec<&String> = (candidates.iter().map(|c| &c.2))
            .filter(|id| horizon(id) == *row)
            .collect();
        if row.is_empty() && members.is_empty() {
            continue;
        }
        let key = format!("row-{}", if row.is_empty() { "none" } else { row });
        let drawn: Vec<&String> = (members.iter().enumerate())
            .filter(|(at, id)| *at < TOP || marked(id))
            .map(|(_, id)| *id)
            .collect();
        let mut inside = vec![];
        if drawn.is_empty() {
            let none = format!("Nothing open in {title}.");
            elements.insert(format!("{key}-none"), text(none));
            inside.push(format!("{key}-none"));
        }
        // a roadmap item a group, in the order of its first task
        let mut items: Vec<(Option<String>, Vec<&String>)> = vec![];
        for id in &drawn {
            let its = item(id);
            match items.iter_mut().find(|(i, _)| *i == its) {
                Some((_, ids)) => ids.push(id),
                None => items.push((its, vec![id])),
            }
        }
        for (at, (its, ids)) in items.iter().enumerate() {
            if let Some(its) = its {
                let link = json!({"name": its, "href": Shown::href(Some(its), label), "current": epic == Some(its.as_str())});
                let epic_key = format!("{key}-epic-{at}");
                elements.insert(
                    epic_key.clone(),
                    json!({"type": "links", "props": {"label": "Epic", "links": [link]}}),
                );
                inside.push(epic_key);
            }
            // its tasks, then what they wait on, through the chain
            let mut graph: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
            let mut seen: HashSet<String> = graph.iter().cloned().collect();
            let mut next = 0;
            while let Some(id) = graph.get(next).cloned() {
                for wait in by_id
                    .get(&id)
                    .and_then(|t| t["waits"].as_array())
                    .into_iter()
                    .flatten()
                {
                    let wait = wait.as_str().unwrap_or_default().to_string();
                    if by_id.contains_key(&wait) && seen.insert(wait.clone()) {
                        graph.push(wait);
                    }
                }
                next += 1;
            }
            let nodes: Vec<Value> = (graph.iter())
                .map(|id| {
                    let mut node = by_id[id].clone();
                    // its face as the board's card draws it: its column there; merged, done; ended unmerged, still to do
                    let status = match state(id) {
                        "finished" => "done",
                        "ended" => "backlog",
                        _ => look.columns.get(id).copied().unwrap_or("backlog"),
                    };
                    let (labels, assignee) = own_face(look, id, today);
                    node["title"] = title_of(look, id).into();
                    node["status"] = status.into();
                    node["labels"] = labels.into();
                    node["assignee"] = assignee.into();
                    // what runs and the next pick say why the pick ranks them where it does
                    if let (true, Some((_, why))) = (marked(id), reasons.get(id)) {
                        let says = node["says"].as_str().unwrap_or_default();
                        node["says"] = format!("{says}: {}", one_line(why)).into();
                    }
                    node
                })
                .collect();
            let count = nodes.iter().filter(|t| t["state"] != "finished").count();
            let graph = match count {
                n if n > surface::GRAPH_LIMIT => text(format!(
                    "{n} tasks: past the {} a graph draws. /tasks?layout=list shows them.",
                    surface::GRAPH_LIMIT
                )),
                _ => json!({"type": "task-graph", "props": {"tasks": nodes}}),
            };
            elements.insert(format!("{key}-graph-{at}"), graph);
            inside.push(format!("{key}-graph-{at}"));
        }
        let more = members.len() - drawn.len();
        if more > 0 {
            let words = format!(
                "{more} more in {title}, ranked below these: /tasks?layout=list lists every open task."
            );
            elements.insert(format!("{key}-more"), text(words));
            inside.push(format!("{key}-more"));
        }
        let section = json!({"type": "section", "props": {"title": title}, "children": inside});
        elements.insert(key.clone(), section);
        children.push(key);
    }
    let held = look.plan.iter().flatten().flat_map(|p| &p.waiting);
    for (i, h) in held.filter(|h| !h.queued).enumerate() {
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
        Some((Ok(_), _, _)) => "No Rust task open.",
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
    /// Its column is the board view's, so his drag moves it (`Board::move_to`).
    placed: bool,
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
    let queue = look.queue.as_ref().and_then(|(q, _, _)| q.as_ref().ok());
    let queued = queue.and_then(|q| q.tasks.iter().find(|t| one_line(&t.id) == id));
    let title = issue_of(look, id).map(|i| i.title.as_str());
    let title = title.or(queued.map(|t| t.title.as_str()));
    one_line(title.filter(|t| !t.is_empty()).unwrap_or(id))
}

fn row_of<'a>(look: &'a Look, id: &str) -> Option<&'a Row> {
    look.rows.iter().find(|r| one_line(&r.id) == id)
}

/// Each open task, in pick order: the plan's own, as the graph reads it (next, then what waits), its status beside it, its words
/// the plan's; with no plan known, the queue ranked as the plan ranks it (`rank`). After them what the older app holds open that no
/// plan names: held there, or its run over here and not yet merged.
fn listed(look: &Look) -> Vec<Listed> {
    let queue = look.queue.as_ref().and_then(|(q, _, _)| q.as_ref().ok());
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
        let placed = look.columns.contains_key(&id);
        all.push(Listed {
            id,
            status,
            says,
            other: None,
            placed,
        });
    }
    if !matches!(look.plan, Some(Ok(_))) {
        let mut tasks: Vec<Queued> = queue.iter().flat_map(|q| q.tasks.clone()).collect();
        let ranked = dash_core::tasks::rank(&mut tasks, &look.standing);
        let order: HashMap<String, usize> = ranked
            .into_iter()
            .enumerate()
            .map(|(at, (id, _))| (one_line(&id), at))
            .collect();
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
            placed: false,
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
        // a graal row refiled --owner rust is Rust's own too (task graal-moves-to-rust): listed once, from Rust's list
        let ours = matches!(open.project.as_str(), "site_perso" | "graal");
        let mine = ours && all.iter().any(|t| t.other.is_none() && t.id == id);
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
        // graal starts nothing (task graal-moves-to-rust): a row of its never reads as at work, unless Rust holds its run
        let status = match column_of(&open.status) {
            "in-progress" if open.project == "graal" && !look.columns.contains_key(&id) => {
                "backlog"
            }
            column => column,
        };
        all.push(Listed {
            id,
            status,
            says,
            other,
            placed: false,
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
    // Linear's « + New issue » at the head's right end (task rust-board-buttons-new-issue)
    elements.insert(
        "new".into(),
        json!({"type": "new-issue", "props": {"project": project}}),
    );
    elements.insert(
        "head".into(),
        json!({"type": "bar", "children": ["title", "new"]}),
    );
    elements.insert("views".into(), views(view, project));
    let mut children = vec!["head".to_string(), "views".into()];
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
    // the day a card's reminder is still ahead of, his own as the page's menus read it
    let today = today_of(look.now_ms);
    let mut groups = vec![];
    // the board's columns run left to right as Linear's, Backlog first; the list stacks the furthest along first, as
    // Linear's list does (task rust-board-linear-order; @nha, 2026-11-09: « wrong order »)
    let mut order = dash_views::rules::BOARD;
    if !board {
        order.reverse();
    }
    for status in order {
        let rows: Vec<(usize, &Listed)> = (tasks.iter().enumerate())
            .filter(|(_, t)| t.status == status)
            .collect();
        if rows.is_empty() && !board {
            continue;
        }
        // a board column draws its first cards and links the list for the rest; the list draws them all. Rust's own are always
        // drawn: his drag and the view's patches name them, and a card dragged past the first would vanish (/code-review)
        let total = rows.len();
        let mut drawn = 0;
        let rows: Vec<(usize, &Listed)> = (rows.into_iter())
            .filter(|(_, t)| {
                let keep = !board || t.other.is_none() || drawn < BOARD_CARDS;
                drawn += usize::from(keep);
                keep
            })
            .collect();
        let more = (rows.len() < total).then(|| {
            json!({"count": total - rows.len(),
                "href": format!("{}#{status}", TaskView::List.href(project))})
        });
        let mut names = vec![];
        for (i, task) in rows {
            let (project, title) = match &task.other {
                Some((project, title)) => (Some(project.as_str()), title.clone()),
                None => (None, title_of(look, &task.id)),
            };
            // one lookup for its comments and its why (/code-review)
            let issue = project
                .is_none()
                .then(|| issue_of(look, &task.id))
                .flatten();
            let comments = issue.map_or(0, |i| i.comments.len());
            // a card's label is its project and its assignee who runs it: Rust's own run here, through `dash orch`; another
            // project's task is no one's here (task rust-board-linear). Cards only: the list's rows show neither, and its text
            // says what its HTML shows (/code-review). A site_perso card adds what its page's menus set, read with the sidebars:
            // its owner, labels, project, cycle and reminder, so a pick on its page shows here (task rust-issue-sidebar-menus).
            let (labels, assignee) = match (board, project) {
                (false, _) => (vec![], None),
                (true, Some(project)) => (vec![project.to_string()], None),
                (true, None) => {
                    let (labels, assignee) = own_face(look, &task.id, today);
                    (labels, Some(assignee))
                }
            };
            // a card his drag moves: one the board view places, Rust's own (`move_to`); another project's status is its own
            let movable = board && task.placed;
            // a card's hover card shows its why; the list's rows have none (task rust-ui-controls)
            let why = issue.filter(|_| board).map_or("", |i| i.why.trim());
            let row = json!({"type": "issue-row", "props": {
                "id": task.id, "title": title, "status": status, "says": task.says, "comments": comments, "project": project,
                "labels": labels, "assignee": assignee, "movable": movable, "why": why,
            }});
            // by its task, never its place, so a view's diff names the same card in every drawing (task rust-view-patches)
            let name = card_name(&task.id, project);
            let name = match elements.contains_key(&name) {
                true => format!("{name}-{i}"),
                false => name,
            };
            elements.insert(name.clone(), row);
            names.push(name);
        }
        let group = json!({"type": "issue-group", "props": {"status": status, "more": more}, "children": names});
        elements.insert(status.into(), group);
        groups.push(status.to_string());
    }
    if board {
        // its cards follow the board view: the page takes each diff as a patch (task rust-view-patches)
        let board = json!({"type": "board", "props": {"view": "board"}, "children": groups});
        elements.insert(BOARD_ELEMENT.into(), board);
        children.push(BOARD_ELEMENT.into());
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
        let section = json!({"type": "section", "props": {"title": "What runs", "agents": true}, "children": ["runs-text"]});
        elements.insert("runs".into(), section);
        children.push("runs".into());
    }
    let page = json!({"type": "stack", "children": children});
    elements.insert("page".into(), page);
    json!({"root": "page", "elements": elements}).to_string()
}

/// The cards a board column draws before its link to the rest: Todo held 1205 on 2026-11-10, 1.4 MB of HTML whose render
/// took 1.6 s under load (task rust-page-latency, trace 1794314321744293823). Linear's board shows a column's first ones too.
const BOARD_CARDS: usize = 50;

/// The board's element on /tasks, the one a patch names whole.
const BOARD_ELEMENT: &str = "board";

/// A card's element on /tasks: `task-<id>`, `task-<project>/<id>` for another project's, so two projects' ids never meet.
fn card_name(id: &str, project: Option<&str>) -> String {
    match project {
        Some(project) => format!("task-{project}/{id}"),
        None => format!("task-{id}"),
    }
}

/// The elements of /tasks' board one diff of the board view changes (task rust-view-patches): a card whose words changed in
/// its column; else each column a card left or entered, its count with it, never a card inside a column drawn whole.
pub fn patched(diff: &dash_views::Diff) -> Vec<String> {
    let mut moves: BTreeMap<&str, [Option<&str>; 2]> = BTreeMap::new();
    for ((view, id, column, _), weight) in diff {
        if view == "board" {
            moves.entry(id).or_default()[usize::from(*weight > 0)] = Some(column);
        }
    }
    let (mut columns, mut cards) = (BTreeSet::new(), vec![]);
    for (id, [left, entered]) in moves {
        match (left, entered) {
            (Some(was), Some(is)) if was == is => cards.push((id, is)),
            _ => columns.extend(left.into_iter().chain(entered)),
        }
    }
    let cards = cards.into_iter().filter(|(_, at)| !columns.contains(at));
    let cards: Vec<String> = cards.map(|(id, _)| card_name(id, None)).collect();
    columns
        .into_iter()
        .map(str::to_string)
        .chain(cards)
        .collect()
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
                json!({"type": "list", "children": cards}),
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
        "suggestions": {"type": "list", "children": ["inbox", "plan", "insights"]},
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
/// `top`: elements drawn under its title before its parts, and their ids in order (/inbox's search field).
fn read_page(title: &str, top: (Map<String, Value>, Vec<String>), parts: &[Part]) -> Drawing {
    let (mut elements, first) = top;
    let mut children = vec!["title".to_string()];
    children.extend(first);
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
        placed: false,
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
    // Linear's sidebar (task rust-board-buttons-new-issue): its priority and due day from its queue row, read already; its
    // assignee, labels, project, cycle and reminder from the older app's list, read with it (task rust-issue-sidebar-menus).
    let queue = look.queue.as_ref().and_then(|(q, _, _)| q.as_ref().ok());
    let queued = queue.and_then(|q| q.tasks.iter().find(|t| one_line(&t.id) == id));
    let rank = queued.map(|q| &q.rank);
    let today = today_of(look.now_ms);
    let sides = look.sides.as_ref();
    let side = sides.and_then(|s| s.tasks.get(id));
    let sidebar = side.map(|side| {
        json!({
            "owner": side.owner, "labels": side.labels, "epic": side.epic, "cycle": side.cycle, "remind": side.remind,
            "owners": OWNERS.map(|o| (o, owner_name(o))), "label_set": settable_labels(),
            "epics": sides.map(|s| s.epics.clone()).unwrap_or_default(),
            "cycles": cycles(today, side.cycle.as_deref()), "remind_days": remind_days(today),
        })
    });
    // The task brief's template lays it out (task rust-doc-types): the page gives its fields alone.
    let brief = json!({"type": "document", "props": {
        "kind": "task-brief", "id": task.id, "title": title_of(look, id), "status": task.status, "says": task.says,
        "why": why, "checks": checks, "no_checks": no_checks,
        "events": events, "unread": unread, "comments": comments, "run": run, "pr": pr, "said": said,
        "priority": rank.and_then(|r| r.priority), "deadline": rank.and_then(|r| r.deadline.clone()),
        "sidebar": sidebar,
        "position": position(&board_order(look), id), "movable": task.placed,
        "due_days": due_days(today),
    }});
    Some(json!({"root": "issue", "elements": {"issue": brief}}).to_string())
}

/// The board's cards in its order: column by column as it draws them, each column in pick order, every project's (/code-review
/// of PR 1395: the pick order alone skipped across columns and left out the other projects' cards).
fn board_order(look: &Look) -> Vec<Listed> {
    let mut all = every_listed(look, None);
    let column = |t: &Listed| dash_views::rules::BOARD.iter().position(|c| *c == t.status);
    all.sort_by_key(|t| column(t).unwrap_or(usize::MAX));
    all
}

/// Its place among the board's cards, its previous and next a step away, each its page's link: Linear's « 42 / 2193 » (task
/// rust-board-buttons-new-issue). None when the board does not list it.
fn position(tasks: &[Listed], id: &str) -> Option<Value> {
    let at = tasks.iter().position(|t| t.id == id && t.other.is_none())?;
    let href_at = |i: Option<usize>| {
        let t = i.and_then(|i| tasks.get(i))?;
        Some(surface::task_href(
            &t.id,
            t.other.as_ref().map(|(p, _)| p.as_str()),
        ))
    };
    Some(json!({
        "at": at + 1, "of": tasks.len(),
        "previous": href_at(at.checked_sub(1)), "next": href_at(Some(at + 1)),
    }))
}

/// Today, his zone's, else the UTC day of `now_ms`: one day for what a page offers and what its post takes.
pub(crate) fn today_of(now_ms: i64) -> i64 {
    local_today().unwrap_or(now_ms.div_euclid(86_400_000))
}

/// Today in his zone, the Mac's (/code-review of PR 1395: a UTC day offered yesterday's presets past midnight in Paris).
pub(crate) fn local_today() -> Option<i64> {
    dash_core::days(&crate::pages::local_now())
}

/// The sidebar fields an issue page's menus post (task rust-issue-sidebar-menus), each to the older app's list.
pub(crate) const SIDE_FIELDS: [&str; 5] = ["assignee", "labels", "project", "cycle", "remind"];

/// His pick in a sidebar menu, as the older app writes it: `dash task update`'s flags, or `dash store set`'s one field where
/// `task update` has no flag (`:cycle`, `:remind`).
#[derive(Debug, PartialEq)]
pub(crate) enum SideWrite {
    Update(Vec<(&'static str, String)>),
    Set(&'static str, String),
}

impl SideWrite {
    pub(crate) fn apply(&self, dash: &Dash, id: &str) -> Result<String, String> {
        match self {
            SideWrite::Update(flags) => {
                let flags: Vec<(&str, &str)> =
                    flags.iter().map(|(f, v)| (*f, v.as_str())).collect();
                dash.update(id, &flags)
            }
            SideWrite::Set(field, value) => dash.set(id, field, value),
        }
    }
}

/// The write of `value` posted for sidebar `field` on a task whose sidebar is `side`, as `Write.task_property/3` checks it:
/// an owner of the list's, a label of the set turned on or off (the others kept), one of the list's epics, this week's or
/// the next cycle or the one it holds, a day; a label `on` or off as asked; « none » clears all but the assignee and the labels. Err: why it is refused.
pub(crate) fn side_write(
    field: &str,
    value: &str,
    on: Option<bool>,
    side: &dash_store::older::Sidebar,
    epics: &[String],
    today: i64,
) -> Result<SideWrite, String> {
    let none = value == "none";
    let one = |flag: &'static str, v: &str| Ok(SideWrite::Update(vec![(flag, v.to_string())]));
    match field {
        "assignee" if OWNERS.contains(&value) => one("--owner", value),
        "assignee" => Err(format!(
            "{value}: an assignee is one of {}",
            OWNERS.join(", ")
        )),
        // the state he asked for (`on`), never a flip of what is stored: the list may hold it already (/code-review of PR 1399)
        "labels" if settable_labels().contains(&value) => {
            let Some(on) = on else {
                return Err(format!("{value}: on=1 turns a label on, on=0 off"));
            };
            let mut labels: Vec<String> = side
                .labels
                .iter()
                .filter(|l| *l != value)
                .cloned()
                .collect();
            if on {
                labels.push(value.to_string());
            }
            match labels.is_empty() {
                true => one("--labels", "nil"),
                false => Ok(SideWrite::Update(
                    labels.into_iter().map(|l| ("--labels", l)).collect(),
                )),
            }
        }
        "labels" => Err(format!(
            "{value}: a label is one of {}",
            settable_labels().join(", ")
        )),
        "project" if none => one("--epic", "nil"),
        "project" if epics.iter().any(|e| e == value) => one("--epic", value),
        "project" => Err(format!("{value}: no such project")),
        "cycle" | "remind" if none => Ok(SideWrite::Set(
            if field == "cycle" { "cycle" } else { "remind" },
            "nil".into(),
        )),
        "cycle"
            if cycles(today, side.cycle.as_deref())
                .iter()
                .any(|(c, _)| c == value) =>
        {
            Ok(SideWrite::Set("cycle", value.into()))
        }
        "cycle" => Err(format!("{value}: a cycle is this week's or the next")),
        "remind" if value.len() == 10 && dash_core::days(value).is_some() => {
            Ok(SideWrite::Set("remind", value.into()))
        }
        "remind" => Err(format!(
            "{value}: a reminder is a day, written 2026-11-15, or none"
        )),
        _ => Err(format!("{field}: not a property of the issue page")),
    }
}

/// An assignee's name as Linear's menus print it, the older app's `assignee_label/1`.
fn owner_name(owner: &str) -> &str {
    match owner {
        "claude" => "Claude",
        "nicolas" => "Nicolas Ha",
        "vibe" => "Vibe",
        "rust" => "Rust",
        other => other,
    }
}

/// A site_perso card's labels and who runs it, the board's and the graph's (task graph-node-is-card): its project, then what its
/// page's menus set (`card_tags`); its owner there, else Rust, whose run it is here.
fn own_face<'a>(look: &'a Look, id: &str, today: i64) -> (Vec<String>, &'a str) {
    let side = look.sides.as_ref().and_then(|s| s.tasks.get(id));
    let mut labels = vec!["site_perso".to_string()];
    labels.extend(side.map(|s| card_tags(s, today)).unwrap_or_default());
    (labels, side.map_or("Rust", |s| owner_name(&s.owner)))
}

/// A board card's tags from its page's menus, after its project's: its labels, its project (epic), « Cycle Nov 9 » and
/// « Remind Nov 12 », each shown once set, so the board shows each pick of its page (task rust-issue-sidebar-menus, done-when 4).
/// A reminder before `today` has fired and is left off; a day that is no date is never printed raw (/code-review).
fn card_tags(side: &dash_store::older::Sidebar, today: i64) -> Vec<String> {
    let day = |prefix: &str, d: &Option<String>, from: i64| {
        let d = dash_core::days(d.as_deref()?).filter(|d| *d >= from)?;
        Some(format!("{prefix} {}", short_day(d)))
    };
    (side.labels.iter().cloned())
        .chain(side.epic.clone())
        .chain(day("Cycle", &side.cycle, i64::MIN))
        .chain(day("Remind", &side.remind, today))
        .collect()
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A day number's place in its week, Monday 0 to Sunday 6: the epoch's first day was a Thursday. Every menu's day reads it
/// (/code-review of PR 1399: four copies of it).
pub(crate) fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// A day number's year, month and day.
fn ymd(day: i64) -> (i64, usize, i64) {
    let date = dash_core::date(day);
    let part = |r: std::ops::Range<usize>| date.get(r).and_then(|p| p.parse().ok()).unwrap_or(1);
    (part(0..4), part(5..7) as usize, part(8..10))
}

/// « Nov 9 », as the older app's cycle labels print a day.
fn short_day(day: i64) -> String {
    let (_, m, d) = ymd(day);
    format!("{} {d}", MONTHS[m.clamp(1, 12) - 1])
}

/// « Fri 13 Nov 2026 », as the older app's menus print a preset's day.
fn full_day(day: i64) -> String {
    const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let (y, m, d) = ymd(day);
    let weekday = WEEKDAYS[weekday(day) as usize];
    format!("{weekday} {d} {} {y}", MONTHS[m.clamp(1, 12) - 1])
}

/// The cycles his menu offers from `today`, the older app's `Issues.cycles/0`: this week's and the next, each its Monday and
/// its name, « Cycle Nov 9 - Nov 15 · Current »; a past one the task `holds` is offered first, as `Write` keeps it.
fn cycles(today: i64, holds: Option<&str>) -> Vec<(String, String)> {
    let monday = today - weekday(today);
    let named = |start: i64| format!("Cycle {} - {}", short_day(start), short_day(start + 6));
    let mut offered: Vec<(String, String)> = [(0, "Current"), (1, "Upcoming")]
        .into_iter()
        .map(|(n, when)| {
            let start = monday + 7 * n;
            (dash_core::date(start), format!("{} · {when}", named(start)))
        })
        .collect();
    if let Some(held) = holds.filter(|h| !offered.iter().any(|(c, _)| c == h)) {
        let name = dash_core::days(held).map_or(held.to_string(), named);
        offered.insert(0, (held.to_string(), name));
    }
    offered
}

/// The days his « Remind me » menu offers from `today`, the older app's `remind_menu/1`: tomorrow, in two days, next week
/// (next Monday), in a month (its last day where the month is shorter); a day two share is offered once, each named with
/// its date.
fn remind_days(today: i64) -> Vec<(String, String)> {
    let (y, m, d) = ymd(today);
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let first = |y: i64, m: usize| dash_core::days(&format!("{y:04}-{m:02}-01")).unwrap_or(today);
    let (ay, am) = if nm == 12 { (ny + 1, 1) } else { (ny, nm + 1) };
    let month_long = first(ay, am) - first(ny, nm);
    let in_a_month = first(ny, nm) + d.min(month_long) - 1;
    let presets = [
        ("Tomorrow", today + 1),
        ("In two days", today + 2),
        ("Next week", today + 7 - weekday(today)),
        ("In a month", in_a_month),
    ];
    let mut days: Vec<(String, String)> = vec![];
    for (name, day) in presets {
        let date = dash_core::date(day);
        if !days.iter().any(|(d, _)| *d == date) {
            days.push((date, format!("{name} · {}", full_day(day))));
        }
    }
    days
}

/// The due days his menu offers from `today` (a day number), as Linear's: tomorrow, the end of this week (Friday, the next one
/// from a weekend), in one week, in two; a day two presets share is offered once.
fn due_days(today: i64) -> Vec<(String, String)> {
    // Monday is 1
    let weekday = weekday(today) + 1;
    let presets = [
        ("Tomorrow", 1),
        ("End of this week", (5 - weekday + 7) % 7),
        ("In one week", 7),
        ("In two weeks", 14),
    ];
    let mut days: Vec<(String, String)> = vec![];
    for (name, ahead) in presets {
        let day = dash_core::date(today + ahead);
        if !days.iter().any(|(_, d)| *d == day) {
            days.push((name.to_string(), day));
        }
    }
    days
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
        // what the orchestrator moved itself, said here rather than flagged (`TaskStore::note`)
        "note" => event.outcome.as_deref().map_or("Note".into(), one_line),
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

/// A list synced in the last 5 minutes is not news on a local-first page (@nha, 2026-11-09: « seems like not so good UI for a
/// local-first app »): only an older or failed sync says how old the list is and why.
const QUEUE_FRESH_MS: i64 = 5 * ORCH_POLL.as_millis() as i64;

fn queue_note(queue: &QueueAt, now_ms: i64) -> String {
    match queue {
        (Ok(_), at, None) if now_ms - at < QUEUE_FRESH_MS => String::new(),
        // past 5 minutes with no failure seen, no page read it: a sync runs only while one is read (`look`, `read_queue`)
        (Ok(_), at, None) => format!(
            "Rust's list, synced {}: it syncs only while a page is open.",
            age(now_ms - at)
        ),
        (Ok(_), at, Some(e)) => {
            format!("Rust's list, synced {}: {}", age(now_ms - at), one_line(e))
        }
        (Err(e), at, _) => format!(
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
    if let Some((Ok(queue), _, _)) = queue {
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

/// An open task given up on, out from under the cap. A task a run holds: that run ended and its pane closed, refused while its
/// agent works or its worktree holds unpushed work (`dash_orch::close`; rust-task-close-rust-runs).
pub fn close(dir: &Path, id: &str, why: &str, mut out: File) {
    if !holds_run(dir, id) {
        return written(dir, id, "closed", out, |tasks| tasks.close(id, why));
    }
    let report = reporting(dir, "close");
    let step = |name: &str, took: Duration| report.step_took(name, took);
    let tools = tools(dir);
    let (agent, launcher) = or_exit(agent_of(&tools, id));
    let mut tasks = or_exit(open_tasks(dir));
    let ports = &mut ports(&mut tasks, &tools, agent, launcher, &step);
    match dash_orch::close(id, why, ports, PACE) {
        Ok(said) => writeln!(out, "{said}").expect("stdout"),
        Err(why) => {
            writeln!(out, "refused: {}", why.replace('\n', " ")).expect("stdout");
            std::process::exit(1);
        }
    }
}

/// Whether a run of `dir`'s store holds `id`: its close is Rust's, never the older app's « no pane: nothing runs it ».
pub fn holds_run(dir: &Path, id: &str) -> bool {
    let status = open_tasks(dir).and_then(|tasks| TaskStore::status(&tasks, id));
    status.is_ok_and(|s| s.as_deref() == Some("claimed"))
}

/// Refuses a graal task, naming the Rust task that replaces it: its `:duplicate-of`, else none, rust-port-map moving graal's
/// modules (@nha, 2026-11-10: « All graal features should move to rust »). A list not read refuses too, as the run's brief,
/// read from the same file, would.
fn graal_refused(db: &Path, id: &str) -> Result<(), String> {
    let refusal = |by: Option<String>| match by {
        Some(by) => {
            format!("{id}: a graal task, and graal starts nothing now: Rust task {by} replaces it")
        }
        None => format!(
            "{id}: a graal task, and graal starts nothing now: no Rust task replaces it, rust-port-map moves graal's modules to Rust"
        ),
    };
    match dash_store::older::graal_held(db, id) {
        Ok(None) => Ok(()),
        Ok(Some(by)) => Err(refusal(by)),
        Err(e) => Err(format!("{id}: whether graal holds it is not known: {e}")),
    }
}

/// The agent and launcher of `id`'s kind. A kind not read, or none Rust runs, is refused.
fn agent_of<'a>(tools: &'a Tools, id: &str) -> Result<(&'a dyn Agent, &'a dyn Launcher), String> {
    Ok(match tools.list.0.agent(id)?.as_str() {
        "claude" => (&tools.agents.claude, &tools.launcher),
        "codex" => (&tools.agents.codex, &tools.codex),
        other => return Err(format!("{id}: Rust runs claude and codex, not {other}")),
    })
}

pub fn show(dir: &Path, id: &str, mut out: File) {
    let row = or_exit(or_exit(open_tasks(dir)).show(id));
    let row = or_exit(row.ok_or(format!("{id}: no such task")));
    writeln!(out, "{row}").expect("stdout");
}

/// Exit 0 on a task merged; any other outcome prints the same, with exit 1. A stop at the memory cap prints no outcome: exit
/// `STOP_CODE`, which `orchestrate` flags by name. `adopt`: a crashed run's session watched on (`adopt_task`), its login its own.
pub fn run(
    dir: &Path,
    id: &str,
    (account, model): (Option<&str>, Option<&str>),
    adopt: bool,
    mut out: File,
) {
    let report = reporting(dir, "run");
    let step = |name: &str, took: Duration| report.step_took(name, took);
    let tools = tools(dir);
    // Before its claim: a graal task, a kind not read, or none Rust runs, starts nothing. A run already started is watched on.
    if !adopt {
        or_exit(graal_refused(&older_db(), id));
    }
    let (agent, launcher) = or_exit(agent_of(&tools, id));
    let mut tasks = or_exit(open_tasks(dir));
    let ports = &mut ports(&mut tasks, &tools, agent, launcher, &step);
    let run = match adopt {
        true => adopt_task(id, ports, PACE),
        false => run_task_on(id, account, model, ports, PACE),
    };
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
    let terminal = dash_terminal::from_env();
    let pane = or_exit(terminal.open(&folder.display().to_string(), "rust claude"));
    let dash = repo().join("private/live/rust/dash");
    let line = mxc::pane_line(&dash.display().to_string(), &dir.display().to_string());
    or_exit(terminal.run(&pane, &line));
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

/// `dash orch next` (task rust-orch-pick-dynamic; @nha, 2026-11-09: « It MUST be clear algorithmically at any point in time what
/// is the next best task to pick »): the list synced as a pass syncs it, then `plan_now`, the function each pass picks with, so
/// what it prints is what a pass would start now, and why each other task waits. Nothing starts. Exit 1 when a read fails.
pub fn orch_next(dir: &Path, mut out: File) {
    let tools = tools(dir);
    let mut tasks = or_exit(open_tasks(dir));
    let synced = sync_list(&mut tasks, &older_db());
    let queue = or_exit(TaskStore::queue(&tasks));
    let rows = or_exit(TaskStore::standing(&tasks));
    // the one reading the plan took, printed beside it
    let read = std::cell::Cell::new(None);
    let memory = || {
        read.set(free_memory());
        read.get()
    };
    let step = |_: &str, _: Duration| {};
    // the picker alone asks Claude for Fable's allowance, once past `MODELS_FRESH` (RFC 20h): a run's process never does
    tools
        .agents
        .claude
        .freshen(dash_agent::usage, dash_agent::MODELS_FRESH);
    let mut ports = ports(&mut tasks, &tools, &tools.agents, &tools.launcher, &step);
    ports.free_memory = &memory;
    let plan = or_exit(plan_now(&ports, queue.clone(), &rows));
    // no free run yet: rust-free-runner starts them, and hands this its runs
    let by_free = free::start(ports.policy, &plan, &queue, &rows, &[], read.get());
    let mut text = next_text(&plan, read.get(), by_free.as_ref());
    // each queued task whose route names a pool, seats aside (RFC 20h): a hard task's Fable shows while every seat is taken
    // Claude's logins only: a Codex row is no Claude pool (ByKind splits them so for the pick)
    let limits = ports.agent.limits().unwrap_or_default();
    let limits: Vec<Limit> = (limits.into_iter())
        .filter(|l| l.account != dash_agent::codex::ACCOUNT)
        .collect();
    let pooled = |t: &&Queued| !is_codex(&t.id);
    let pooled_tasks: Vec<&Queued> = (plan.order.iter())
        .filter_map(|(id, _)| queue.tasks.iter().find(|t| &t.id == id))
        .filter(|t| !route_of(t).is_empty())
        .filter(pooled)
        .collect();
    text += &pools_text(&pooled_tasks, &limits, (now_ms() / 1000) as u64);
    text += &held_text(&tasks);
    text += &live_text(tools.forge.serving());
    if let Err(e) = synced {
        text += &format!("(the list's sync failed, its last copy stands: {e})\n");
    }
    write!(out, "{text}").expect("stdout");
}

/// Each of `tasks` with the pool it would start on and why, or why it would wait, seats aside (RFC 20h rule 1).
fn pools_text(tasks: &[&Queued], limits: &[Limit], now: u64) -> String {
    let line = |t: &&Queued| {
        let said = match dash_policy::spend(t, limits, now) {
            Ok((account, Some(p))) => {
                let on = account.map_or(String::new(), |a| format!(" on {}", one_line(&a)));
                format!("{}{on}: {}", p.pool, one_line(&p.why))
            }
            Ok((_, None)) => "no pool, the login's own model".into(),
            Err(why) => format!("waits: {}", one_line(&why)),
        };
        format!("{}\t{said}\n", one_line(&t.id))
    };
    match tasks.is_empty() {
        true => String::new(),
        false => format!("Pools:\n{}", tasks.iter().map(line).collect::<String>()),
    }
}

/// `dash orch route [<demand> <pool>[,<pool>]...|none]`: each demand's route, the pools it may spend best first (RFC 20h rule 4:
/// a new model or credit is a row, not code); with a demand, that route set first. `none`: no pool, the login's own model.
pub fn orch_route(dir: &Path, args: &[&str], mut out: File) {
    let mut tasks = or_exit(open_tasks(dir));
    let set = match args {
        [] => Ok(()),
        [demand, pools] if ["hard", "normal"].contains(demand) => {
            let pools: Vec<String> = match *pools {
                "none" => vec![],
                pools => pools.split(',').map(|p| p.trim().to_string()).collect(),
            };
            tasks.route(Demand::of(Some(demand)), &pools)
        }
        _ => Err("usage: dash orch route [hard|normal <pool>[,<pool>]...|none]".into()),
    };
    or_exit(set);
    let routes = or_exit(tasks.routes());
    let text: String = [Demand::Hard, Demand::Normal]
        .iter()
        .map(|d| {
            let set = routes.iter().find(|(name, _)| name == d.name());
            let (pools, whence) = match set {
                Some((_, pools)) => (pools.clone(), "set"),
                None => (default_route(*d), "default"),
            };
            let pools = if pools.is_empty() {
                "none".into()
            } else {
                pools.join(", ")
            };
            format!("{}: {pools} ({whence})\n", d.name())
        })
        .collect();
    write!(out, "{text}").expect("stdout");
}

/// Each held PR and its holder, as `dash orch next` ends (task rust-pr-one-owner, done-when 2): a hand-back skips a PR another
/// holds. A failed read says so.
fn held_text(tasks: &dyn TaskStore) -> String {
    let holds = match tasks.holds() {
        Ok(holds) if holds.is_empty() => return "Held PRs: none\n".into(),
        Ok(holds) => holds,
        Err(e) => return format!("Held PRs: unread: {e}\n"),
    };
    let line = |h: &dash_core::tasks::Hold| {
        let at = dash_core::stamp(h.touched_ms);
        format!(
            "#{}\t{}, since {at}: {}\n",
            h.pr,
            one_line(&h.holder),
            one_line(&h.why)
        )
    };
    format!("Held PRs:\n{}", holds.iter().map(line).collect::<String>())
}

/// `dash orch next`'s « Live: » line: whether the page serves main's rust/ (task rust-deploy-fetch-retry).
fn live_text(serving: Result<Result<String, String>, String>) -> String {
    match serving {
        Ok(Ok(said)) => format!("Live: {}\n", one_line(&said)),
        Ok(Err(behind)) => format!("Live: not main's rust/: {}\n", one_line(&behind)),
        Err(e) => format!("Live: unread: {}\n", one_line(&e)),
    }
}

/// Whether a look's pass starts a `--deploy` child: never under --once, whose one pick is all it was asked for, nor while its
/// last child runs; only with a failed deploy on a row.
fn starts_retry(once: bool, running: bool, pending: impl FnOnce() -> bool) -> bool {
    !once && !running && pending()
}

/// `dash orch --data <dir> --deploy`: the look's retry of each merge whose deploy failed (`dash_orch::redeploy`), what serves
/// printed. Exit 1 when it fails again.
pub fn redeploy_once(dir: &Path, mut out: File) {
    // Held to the end: a deploy that restarts the look loses its handle on this child, and the next look may start another
    // before these rows are written. That one leaves at once, never queued behind it for a second build straight after a
    // failure; the next look's pass retries (/code-review of 965bb0ab0).
    let lock = dir.join("redeploy.lock");
    let held = or_exit(File::create(&lock).map_err(|e| format!("{}: {e}", lock.display())));
    match held.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            writeln!(out, "a deploy retry already runs").expect("stdout");
            return;
        }
        Err(e) => or_exit(Err(format!("{}: {e}", lock.display()))),
    }
    let tools = tools(dir);
    let mut tasks = or_exit(open_tasks(dir));
    let step = |_: &str, _: Duration| {};
    let (agent, launcher) = (&tools.agents, &tools.launcher);
    let said = or_exit(redeploy(&mut ports(
        &mut tasks, &tools, agent, launcher, &step,
    )));
    let said = said.unwrap_or_else(|| "no failed deploy to retry".into());
    writeln!(out, "{said}").expect("stdout");
}

/// A PR's number from `dash land`'s or `dash pr`'s word: « 1390 », « #1390 » or its URL.
fn pr_number(pr: &str) -> Result<u64, String> {
    let n = match pr.split_once("/pull/") {
        Some((_, rest)) => rest.split('/').next().unwrap_or(rest),
        None => pr.trim_start_matches('#'),
    };
    n.parse().map_err(|_| format!("{pr}: no PR number"))
}

/// `dash pr hold <n> <why>`: the orchestrator holds the PR, so no hand-back starts a run on it (task rust-pr-one-owner; @nha,
/// 2026-11-10: « How will you fix systematically the wasted effort time and tokens of duplicate work ? »). Refused while a run at
/// work holds it: `dash task close` ends that run first. Held again, its hold is touched. Exit 1 on a refusal.
pub fn pr_hold(dir: &Path, pr: &str, why: &str, mut out: File) {
    let n = or_exit(pr_number(pr));
    let mut tasks = or_exit(open_tasks(dir));
    let took = or_exit(TaskStore::hold_pr(&mut tasks, n, ORCHESTRATOR, why));
    let from = took.map_or(String::new(), |h| format!(", taken from {}", h.holder));
    writeln!(out, "#{n} held by {ORCHESTRATOR}{from}: {why}. It ends on merge, dash pr release {n}, or 30 minutes untouched")
        .expect("stdout");
}

/// `dash pr release <n>`: its hold ended, whoever held it.
pub fn pr_release(dir: &Path, pr: &str, mut out: File) {
    let n = or_exit(pr_number(pr));
    let mut tasks = or_exit(open_tasks(dir));
    or_exit(TaskStore::unhold(&mut tasks, n));
    writeln!(out, "#{n} released").expect("stdout");
}

/// `dash land`'s hold around its land (done-when 1): the orchestrator's, refused while a run at work holds it; ended by the merge,
/// kept when it does not land, as the orchestrator still works on it.
/// Only another's live hold refuses it: a store that does not open, a word with no number (a branch name gh reads) or a write that
/// fails lands with no hold, said on stderr, as an urgent land is never blocked by its bookkeeping (/code-review of a76563791;
/// @nha, 2026-11-10: « urgent tasks may require that »).
fn land_held(
    tasks: Result<&mut dyn TaskStore, String>,
    pr: &str,
    parity: impl FnOnce() -> Result<(), String>,
    land: impl FnOnce() -> Result<(), (i32, String)>,
) -> Result<(), (i32, String)> {
    // before the hold and the wait: a refused PR stays free for the run that ports its gaps, and needs no green run (task
    // rust-port-parity, /code-review)
    parity().map_err(|e| (1, format!("land: refused: {e}")))?;
    let held = tasks.and_then(|tasks| Ok((tasks, pr_number(pr)?)));
    let held = match held {
        Ok((tasks, n)) => {
            let refused = |e: &String| e.starts_with(&format!("#{n} is held by "));
            match tasks.hold_pr(n, ORCHESTRATOR, "dash land") {
                Err(e) if refused(&e) => return Err((1, format!("land: refused, {e}"))),
                Err(e) => Err(e),
                Ok(_) => Ok((tasks, n)),
            }
        }
        Err(e) => Err(e),
    };
    if let Err(e) = &held {
        eprintln!("land: no hold taken: {e}");
    }
    land()?;
    // merged: a failed write leaves a hold that ends in 30 minutes
    if let Ok((tasks, n)) = held {
        let _ = tasks.unhold(n);
    }
    Ok(())
}

/// The next task and the account it starts on, the memory the pick read, the free pick, each task that waits and why, then the
/// order every task ranks in and why (task rust-orch-pick-dynamic, done-when 5).
fn next_text(plan: &Plan, free: Option<u8>, by_free: Option<&free::Start>) -> String {
    let on = match &plan.next {
        Pick::Start {
            account: Some(account),
            ..
        } => format!(", on {}", one_line(account)),
        _ => String::new(),
    };
    // the pool it spends and why (RFC 20h rule 1)
    let pool = match &plan.next {
        Pick::Start { pool: Some(p), .. } => format!("\nPool: {}, {}", p.pool, one_line(&p.why)),
        Pick::Start { pool: None, .. } => "\nPool: none on its route, the login's own model".into(),
        _ => String::new(),
    };
    let memory = free.map_or("unread, nothing held for it".into(), |f| {
        format!("{f}% free, a build needs {BUILD_FREE_PERCENT}%")
    });
    let waiting = waiting_text(plan);
    let by_free = by_free.map_or("none fits RFC 20e's table".into(), |s| {
        format!(
            "{} fits {} on {}, once rust-free-runner starts it",
            one_line(&s.id),
            s.model,
            s.tool
        )
    });
    let order: String = (plan.order.iter().enumerate())
        .map(|(at, (id, why))| format!("{}. {}\t{why}\n", at + 1, one_line(id)))
        .collect();
    format!(
        "{}{on}{pool}\nMemory: {memory}\nFree model: {by_free}\n{waiting}Order:\n{order}",
        next_line(plan)
    )
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
    // A look serve started ends between passes once that server is gone (task rust-one-launchd): never mid-pass, so a deploy it
    // runs is recorded; the next server's look takes over. One launchd started keeps pid 1 for a parent, so never ends here.
    let parent = unsafe { libc::getppid() };
    // A merge whose deploy failed, retried at each look by a child of its own until main serves (`dash_orch::redeploy`): one at
    // a time, and never in the look, which the deploy restarts.
    let mut deploying: Option<Child> = None;
    loop {
        // the picker alone asks Claude for Fable's allowance, once past `MODELS_FRESH` (RFC 20h): a run's process never does
        tools
            .agents
            .claude
            .freshen(dash_agent::usage, dash_agent::MODELS_FRESH);
        if !once && unsafe { libc::getppid() } != parent {
            writeln!(
                out,
                "its server is gone: this look ends, the next one's takes over"
            )
            .expect("stdout");
            std::process::exit(0);
        }
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
        let retrying = deploying
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if starts_retry(once, retrying, || {
            TaskStore::standing(&tasks).is_ok_and(|rows| deploy_pending(&rows))
        }) {
            let mut command =
                Command::new(std::env::current_exe().unwrap_or_else(|_| "dash".into()));
            command.args(["orch", "--data"]).arg(dir).arg("--deploy");
            match detached(&mut command).spawn() {
                Ok(child) => deploying = Some(child),
                Err(e) => writeln!(out, "no deploy retry started: {e}").expect("stdout"),
            }
        }
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
            Ok(Pick::Start { id, account, pool }) => {
                let on = account.as_deref().unwrap_or("its default account");
                let line = match &pool {
                    Some(p) => format!("started {id} on {on}, on {}: {}", p.pool, p.why),
                    None => format!("started {id} on {on}"),
                };
                let at = (account.as_deref(), pool.as_ref().map(|p| p.pool.as_str()));
                let skipped = launch(dir, &id, at, false, once, &report, &mut running);
                skipped.unwrap_or(line)
            }
            Ok(Pick::Adopt(id)) => {
                let line = format!("adopted {id}: its crashed run's session goes on in its pane");
                let skipped = launch(dir, &id, (None, None), true, once, &report, &mut running);
                skipped.unwrap_or(line)
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

/// A run's process started, waited for when `once`. A child of this look still alive for the same task starts none: the
/// next look may pick it again before that child has its claim, and a second would only fail on its lock. Some: why none
/// started, the line said instead.
fn launch(
    dir: &Path,
    id: &str,
    account: (Option<&str>, Option<&str>),
    adopt: bool,
    once: bool,
    report: &dash_statsd::Statsd,
    running: &mut Vec<(String, Child)>,
) -> Option<String> {
    if running.iter().any(|(r, _)| r == id) {
        return Some(format!("{id}: its run is still starting"));
    }
    let child = start(dir, id, account, adopt);
    if once {
        let status = or_exit(child).wait().map_err(|e| e.to_string());
        say_memory(report);
        std::process::exit(or_exit(status).code().unwrap_or(1));
    }
    match child {
        Ok(child) => running.push((id.to_string(), child)),
        Err(e) => eprintln!("{id}: {e}"),
    }
    None
}

fn start(
    dir: &Path,
    id: &str,
    (account, model): (Option<&str>, Option<&str>),
    adopt: bool,
) -> Result<Child, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(me);
    command.args(["task", "run", "--data"]).arg(dir).arg(id);
    if let Some(account) = account {
        command.args(["--account", account]);
    }
    if let Some(model) = model {
        command.args(["--model", model]);
    }
    if adopt {
        command.arg("--adopt");
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
    use dash_core::tasks::{Run, plan};
    use dash_meter::memory::TestBudget;
    use dash_policy::Defaults;

    /// A look starts no deploy retry under --once, none while its child runs, none with nothing failed.
    #[test]
    fn a_look_starts_a_deploy_retry_only_when_free_and_a_deploy_failed() {
        let _budget = TestBudget::new();
        assert!(starts_retry(false, false, || true));
        assert!(!starts_retry(true, false, || true), "--once");
        assert!(!starts_retry(false, true, || true), "its child runs");
        assert!(!starts_retry(false, false, || false), "nothing failed");
    }

    /// A second retry child, started while the first holds redeploy.lock, leaves at once: no build queued behind a failure.
    #[test]
    fn a_deploy_retry_leaves_while_another_holds_its_lock() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("redeploy-lock");
        let held = File::create(dir.0.join("redeploy.lock")).expect("lock file");
        held.lock().expect("the first child's lock");
        let said = dir.0.join("out");
        redeploy_once(&dir.0, File::create(&said).expect("out"));
        let said = std::fs::read_to_string(&said).expect("out");
        assert_eq!(said, "a deploy retry already runs\n");
    }

    /// What done-when 3 of rust-deploy-fetch-retry reads after each merge: `dash orch next` says whether main's rust/ serves.
    #[test]
    fn orch_next_says_whether_main_s_rust_serves() {
        let _budget = TestBudget::new();
        let live = "abc serves, no build: it holds the rust/ of main's def";
        assert_eq!(live_text(Ok(Ok(live.into()))), format!("Live: {live}\n"));
        let behind = live_text(Ok(Err("abc serves, not the rust/ of main's def".into())));
        assert_eq!(
            behind,
            "Live: not main's rust/: abc serves, not the rust/ of main's def\n"
        );
        assert_eq!(
            live_text(Err("git\nfetch".into())),
            "Live: unread: git fetch\n"
        );
    }

    /// Done-when 1 of brief-tests-prepush-skip: a run's brief is Rust's, written from the older app's database and the RFC pages
    /// its task cites, with no call to the older app (its `List` port has no CLI to ask), rule 2 asking each part's test.
    #[test]
    fn a_run_s_brief_is_written_from_the_database_and_its_rfc_rules() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("task-brief");
        // its own process (rust/CLAUDE.md rule 1): the scratch repo, never the live checkout
        unsafe { std::env::set_var("DASH_REPO", &dir.0) };
        let docs = dir.0.join("private/docs");
        std::fs::create_dir_all(&docs).expect("docs");
        // put together: the dash crate holds no tag literal (the gate's html_in_strings)
        let page = format!(
            "<{ol}><{li} id=\"rule-1\">One <{b}>rule</{b}>.</{li}></{ol}>",
            ol = "ol",
            li = "li",
            b = "b"
        );
        std::fs::write(docs.join("rfc-held.html"), page).expect("a page");
        let db = dir.0.join("older.db");
        let tables = "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT,
              why TEXT, done_when TEXT, epic TEXT, plan_first INTEGER);
            CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, title TEXT, data TEXT NOT NULL);
            CREATE TABLE comments (pos INTEGER, id TEXT, stage TEXT, task TEXT, file TEXT, quote TEXT, text TEXT,
              data TEXT NOT NULL);
            INSERT INTO tasks VALUES ('tasks.edn', 0, 't', 'open', '{\"run\":{\"prompt\":\"Build it\"}}', 'T',
              'per rfc-held and rfc-gone', '1. It works', NULL, NULL);";
        let store = dash_store::Connection::open(&db).expect("db");
        store.execute_batch(tables).expect("tables");
        let text = brief(&db, &dir.0, "t", &[]).expect("a brief");
        assert!(
            text.starts_with("Complete task t: Build it. His decided rules"),
            "{text}"
        );
        assert!(
            text.contains("his word, quoted): rfc-held#rule-1: One rule. Rules: 1."),
            "{text}"
        );
        assert!(
            text.contains("2. Each done-when part has its one proving test"),
            "{text}"
        );
        assert!(
            text.contains("Done when: 1. It works. A part also holds"),
            "{text}"
        );
        assert!(
            brief(&db, &dir.0, "gone", &[])
                .unwrap_err()
                .contains("no task gone")
        );
        let bare = "INSERT INTO tasks VALUES ('tasks.edn', 1, 'bare', 'open', '{}', 'B', NULL, NULL, NULL, NULL)";
        store.execute(bare, []).expect("a bare row");
        let refused = brief(&db, &dir.0, "bare", &[]).unwrap_err();
        assert!(refused.contains("nothing to launch"), "{refused}");
    }

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
                held: None,
                id: "ne\nxt\x1b[2J".into(),
                title: "its\nfaked\trow\x1b[2J\r\u{202e}\u{2028}".into(),
                reopened_ms: None,
                waits: vec![],
                layer: None,
                builds_nothing: false,
                rank: Default::default(),
                free: None,
                demand: Default::default(),
                route: None,
                ..Default::default()
            }],
        });
        let text = page(&[], Some(&(queue, 0, None)), "", 0, None);
        assert!(text.contains("\nne xt [2J\tqueued\t"), "{text}");
        assert!(text.contains("\tits faked row [2J   \n"), "{text}");
        let down = (Err("down\nBuilt from x.".into()), 0, None);
        let failed = page(&[], Some(&down), "", 0, Some("y"));
        assert!(queue_note(&down, 0).ends_with("0 s ago: down Built from x."));
        // why a sync stopped is one line too, however the older app wrote it
        let kept = (
            Ok(Queue {
                seats: 1,
                tasks: vec![],
            }),
            0,
            Some(format!("its sync failed: {}", down.0.clone().unwrap_err())),
        );
        assert!(queue_note(&kept, 0).ends_with("its sync failed: down Built from x."));
        assert!(!failed.contains("\nBuilt from x.\n"), "{failed}");
        let forged = "\nBuilt from x.\n";
        let queue = Ok(Queue {
            seats: 1,
            tasks: vec![Queued {
                held: None,
                id: forged.into(),
                title: forged.into(),
                reopened_ms: None,
                waits: vec![],
                layer: None,
                builds_nothing: false,
                rank: Default::default(),
                free: None,
                demand: Default::default(),
                route: None,
                ..Default::default()
            }],
        });
        let quoted = page(&[], Some(&(queue, 0, None)), "", 0, Some("y"));
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
            held: None,
            id: id.into(),
            title: "its words".into(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
            builds_nothing: false,
            rank: Default::default(),
            free: None,
            demand: Default::default(),
            route: None,
            ..Default::default()
        };
        let queue = Ok(Queue {
            seats: 1,
            tasks: vec![queued("a-task"), queued("next")],
        });
        let mut at = (queue, 1_000, None);
        // task rust-tasks-no-sync-note: a list synced under 5 minutes is not news, an older one says its age and why
        assert_eq!(queue_note(&at, 121_000), "");
        assert_eq!(queue_note(&at, 300_999), "");
        assert_eq!(
            queue_note(&at, 301_000),
            "Rust's list, synced 5 min ago: it syncs only while a page is open."
        );
        let text = page(&[row("")], Some(&at), "", 3_601_000, None);
        // the note is the list tree's line, never the runs' too
        assert!(!text.contains("list, synced"), "{text}");
        // a failed sync says so at once, however fresh the list it kept
        at.2 = Some("its sync failed: the older app timed out".into());
        assert_eq!(
            queue_note(&at, 2_000),
            "Rust's list, synced 1 s ago: its sync failed: the older app timed out"
        );
        // a whole read that failed (the store locked) keeps the last list, its time and why, never blanks the page
        let synced = at.0.clone().expect("a list");
        let fresh = next_queue(Ok((synced.clone(), Ok(2))), None, 1_000);
        assert_eq!(fresh, (Ok(synced.clone()), 1_000, None));
        let locked = next_queue(Err("database is locked".into()), Some(fresh), 400_000);
        assert_eq!(
            locked,
            (
                Ok(synced.clone()),
                1_000,
                Some("its read failed: database is locked".into())
            )
        );
        assert_eq!(
            queue_note(&locked, 400_000),
            "Rust's list, synced 6 min ago: its read failed: database is locked"
        );
        let kept = next_queue(Ok((synced, Err("timed out".into()))), Some(locked), 460_000);
        assert_eq!(kept.1, 1_000, "the list's time stays its last sync");
        assert_eq!(kept.2.as_deref(), Some("its sync failed: timed out"));
        // the next read waits a minute after the last one ended, failed or not, never the list's own age
        let mut reads = QueueRead {
            last: Some(kept),
            tried_ms: 460_000,
            ..Default::default()
        };
        assert!(!reads.stale(519_999));
        assert!(reads.stale(520_000));
        reads.last = None;
        assert!(reads.stale(460_000), "nothing read yet reads at once");
        // with no list before, the failure stands
        let none = next_queue(Err("database is locked".into()), None, 5);
        assert_eq!(none, (Err("database is locked".into()), 5, None));
        assert_eq!(text.matches("a-task\t").count(), 1, "{text}");
        assert!(
            text.contains("next\tqueued\t0\tnever\t\t\tits words\n"),
            "{text}"
        );
        let down = (Err("down now".into()), 0, None);
        let failed = page(&[], Some(&down), "", 0, None);
        assert!(queue_note(&down, 0).contains("is not known, 0 s ago: down now"));
        assert!(failed.contains("No Rust task yet."), "{failed}");
    }

    /// Task rust-orch-pick-dynamic, done-when 3: `dash orch next` prints the pick `plan_now` makes, its account, the memory it
    /// read and why each other task waits; orch's path test holds that `plan_now` is the pass's own pick.
    #[test]
    fn orch_next_prints_the_next_task_its_account_the_memory_read_and_why_others_wait() {
        let _budget = TestBudget::new();
        let queued = |id: &str, light: bool| Queued {
            id: id.into(),
            builds_nothing: light,
            ..Queued::default()
        };
        let queue = Queue {
            seats: 5,
            tasks: vec![queued("build", false), queued("review", true)],
        };
        let limits = vec![Limit {
            account: "second".into(),
            five_hours: 0.0,
            five_hours_resets: 0,
            week: 10.0,
            week_resets: 1,
            key: false,
            models: vec![],
        }];
        let low = plan(&Defaults, queue.clone(), &[], 0, limits.clone(), Some(12));
        let start = free::Start {
            id: "review".into(),
            tool: "opencode",
            model: "big-pickle",
        };
        assert_eq!(
            next_text(&low, Some(12), Some(&start)),
            "Next: review, on second\nPool: none on its route, the login's own model\nMemory: 12% free, a build needs 25%\nFree model: review fits big-pickle on opencode, \
             once rust-free-runner starts it\nbuild\tit builds, and 12% of memory is free, under the 25% a build needs: \
             only work that builds nothing starts (labelled Docs or Research)\n\nOrder:\n1. build\ton no roadmap item, \
             priority none, 0 wait on it, estimate none\n2. review\ton no roadmap item, priority none, 0 wait on it, \
             estimate none\n"
        );
        let unread = plan(&Defaults, queue, &[], 0, limits.clone(), None);
        assert_eq!(
            next_text(&unread, None, None),
            "Next: build, on second\nPool: none on its route, the login's own model\nMemory: unread, nothing held for it\n\
             Free model: none fits RFC 20e's table\n\
             review\tqueued\n\nOrder:\n1. build\ton no roadmap item, priority none, 0 wait on it, estimate none\n\
             2. review\ton no roadmap item, priority none, 0 wait on it, estimate none\n"
        );
        // task rust-pools-fable, done-when 3: a start names its pool and why, here a hard task on Fable while it has room
        let hard = Queue {
            seats: 5,
            tasks: vec![Queued {
                id: "hard".into(),
                demand: Demand::Hard,
                route: None,
                ..Queued::default()
            }],
        };
        let fable = vec![Limit {
            models: vec![dash_core::tasks::Scoped {
                model: "fable".into(),
                used: 2.0,
                resets: u64::MAX,
            }],
            ..limits[0].clone()
        }];
        let on_fable = plan(&Defaults, hard, &[], 0, fable, None);
        assert!(
            next_text(&on_fable, None, None).starts_with(
                "Next: hard, on second\nPool: fable, hard task: fable is first on its route and has room on second, 2% of \
                 its own allowance used\nMemory:"
            ),
            "{}",
            next_text(&on_fable, None, None)
        );
        // seats aside, each task whose route names a pool says its pool, or why it would wait
        let task = |id: &str, demand| Queued {
            id: id.into(),
            demand,
            ..Queued::default()
        };
        let (hard, other) = (task("hard", Demand::Hard), task("other", Demand::Hard));
        let fable = vec![Limit {
            models: vec![dash_core::tasks::Scoped {
                model: "fable".into(),
                used: 2.0,
                resets: 9_000,
            }],
            ..limits[0].clone()
        }];
        assert_eq!(
            pools_text(&[&hard], &fable, 1_000),
            "Pools:\nhard\tfable on second: hard task: fable is first on its route and has room on second, 2% of its own \
             allowance used\n"
        );
        let spent = vec![Limit {
            models: vec![dash_core::tasks::Scoped {
                model: "fable".into(),
                used: 100.0,
                resets: 2_000,
            }],
            ..limits[0].clone()
        }];
        assert_eq!(
            pools_text(&[&hard, &other], &spent, 1_000),
            "Pools:\nhard\twaits: fable is at its limit and refills in 17 min: a hard task waits for it rather than spend \
             the next pool\nother\twaits: fable is at its limit and refills in 17 min: a hard task waits for it rather than \
             spend the next pool\n"
        );
        assert_eq!(pools_text(&[], &spent, 1_000), "");
        let said = "Mach Virtual Memory Statistics: ...\nSystem-wide memory free percentage: 57%\n";
        assert_eq!(free_percent_in(said), Some(57));
        assert_eq!(free_percent_in("no reading"), None);
    }

    /// Task rust-pick-handbacks-first, done-when 1, PR 1363's case on the real store: its run ended with its PR open, the hand-back
    /// pass wrote the head and reopened it, and the sync started its row over. Behind 21 new tasks of higher tier and priority
    /// it ranked 22nd; now `dash orch next` names it next and prints why it ranks first.
    #[test]
    fn orch_next_names_a_task_handed_back_on_its_open_pr_first_and_says_why() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("tasks-hand-back-first");
        let mut tasks = store::open(&dir.0, &dir.0).expect("open");
        tasks.claim("pr-1363").expect("claim");
        let ended = Outcome::Ended("met, its session ended while CI ran, then: a conflict".into());
        let run = Run {
            pane: None,
            session: None,
            outcome: ended,
        };
        TaskStore::record(&mut tasks, "pr-1363", &run).expect("record");
        TaskStore::handed(&mut tasks, "pr-1363", "abc 1").expect("handed");
        let finished_ms = tasks.rows().expect("rows")[0].last_ms;
        let reopened = TaskStore::reopen_held(&mut tasks, "pr-1363", finished_ms + 1);
        assert_eq!(
            reopened,
            Ok(false),
            "the list's reopen, as a sync brings it"
        );
        let new = (1..=21).map(|n| Queued {
            id: format!("new-{n:02}"),
            rank: dash_core::tasks::Rank {
                horizon: Some("now".into()),
                item: Some("rust-poc".into()),
                priority: Some(1),
                ..Default::default()
            },
            ..Queued::default()
        });
        let handed = Queued {
            id: "pr-1363".into(),
            rank: dash_core::tasks::Rank {
                priority: Some(3),
                ..Default::default()
            },
            ..Queued::default()
        };
        let queue = Queue {
            seats: 5,
            tasks: new.chain([handed]).collect(),
        };
        let rows = TaskStore::standing(&tasks).expect("rows");
        let got = next_text(&plan(&Defaults, queue, &rows, 0, vec![], None), None, None);
        assert!(got.starts_with("Next: pr-1363\n"), "{got}");
        let first = "Order:\n1. pr-1363\thanded back on its open PR at abc: a fix of work already made starts first; \
                     on no roadmap item, priority 3, 0 wait on it, estimate none\n2. new-01\tnow (rust-poc), priority 1";
        assert!(got.contains(first), "{got}");
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
            held: None,
            id: id.into(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
            builds_nothing: false,
            rank: Default::default(),
            free: None,
            demand: Default::default(),
            route: None,
            ..Default::default()
        };
        let queue = Queue {
            seats: 5,
            tasks: vec![queued("after"), queued("first"), queued("also")],
        };
        let rows = dash_core::tasks::TaskStore::standing(&tasks).expect("rows");
        let plan = plan_text(&plan(&Defaults, queue, &rows, 0, vec![], None));
        let text = page(&[], None, &plan, 0, None);
        assert!(
            text.contains("Next: first\nalso\tqueued\nafter\twaits on first (open)\n\nTask\t"),
            "{text}"
        );
        let full = Plan {
            next: Pick::Wait("a waits: every seat is taken, 5 sessions open of 5 allowed".into()),
            waiting: vec![],
            order: vec![],
        };
        assert!(plan_text(&full).starts_with(
            "Next: nothing, a waits: every seat is taken, 5 sessions open of 5 allowed"
        ));
    }

    fn queued(ids: &[&str], seats: usize) -> Queue {
        let queued = |id: &&str| Queued {
            held: None,
            id: id.to_string(),
            title: String::new(),
            reopened_ms: None,
            waits: vec![],
            layer: None,
            builds_nothing: false,
            rank: Default::default(),
            free: None,
            demand: Default::default(),
            route: None,
            ..Default::default()
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
        let plan = plan(&Defaults, queue.clone(), &standing, sessions, limits, None);
        Look {
            rows: vec![],
            columns: viewed(&standing, Some(&queue)),
            standing,
            now_ms: 0,
            queue: Some((Ok(queue), 0, None)),
            issues: None,
            plan: Some(Ok(plan)),
            every: None,
            sides: None,
        }
    }

    fn drawn(look: &Look) -> String {
        surface::markup(
            &surface::screen::page("t", &graph_tree(look, &Shown::default())).expect("a page"),
        )
    }

    /// The node of `id`, from its opening to the end of its words: its card holds `data-task`, the node around it its class.
    fn node<'a>(page: &'a str, id: &str) -> &'a str {
        let start = page
            .find(&format!(r#"data-task="{id}""#))
            .unwrap_or_else(|| panic!("no node {id}: {page}"));
        let class = page[..start].rfind(r#"class="node "#).expect("its node");
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
        let (_, kind, html) = crate::pages::drawn(
            true,
            false,
            Some(Ok(graph_drawing(&look, &Shown::default()))),
        );
        assert_eq!(kind, crate::pages::HTML);
        assert!(html.contains(r#"data-from="base" data-to="top""#), "{html}");
        let (_, kind, text) = crate::pages::drawn(
            false,
            false,
            Some(Ok(graph_drawing(&look, &Shown::default()))),
        );
        assert_eq!(kind, crate::pages::TEXT);
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
            let (_, kind, html) =
                crate::pages::drawn(true, false, Some(Ok(list_drawing(&look, view, None, ""))));
            assert_eq!(kind, crate::pages::HTML);
            assert!(html.contains(r#"class="toggle""#), "{html}");
            let (_, kind, text) =
                crate::pages::drawn(false, false, Some(Ok(list_drawing(&look, view, None, ""))));
            assert_eq!(kind, crate::pages::TEXT);
            assert_eq!(
                text,
                surface::text(&list_tree(&look, view, None, "")).expect("a text")
            );
            assert!(
                text.starts_with("Tasks\nNew issue: POST /tasks/new title, description\nViews: ")
                    && !text.contains('<'),
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
        // task graph-node-is-card: each node is the board's card, its link and its labels the board's
        for part in [
            r#"href="/tasks/base""#,
            r#"li-tag">site_perso"#,
            r#"title="Rust""#,
        ] {
            assert!(node(&page, "base").contains(part), "{part}: {page}");
        }
        assert!(page.contains("Next: base"), "{page}");
        assert!(
            node(&page, "busy").contains(r#"class="node running""#),
            "{page}"
        );
        assert!(node(&page, "also").ends_with(">queued"), "{page}");
        assert!(
            !page.contains("behind base"),
            "queue order is no wait: {page}"
        );
    }

    /// Done-when 1 and 4 of rust-graph-queue-edges: a wait the list alone names (the older app's blocked-by, no run row yet)
    /// draws its edge, so the graph has a second column; a task held only by queue order says queued, in the graph and in the
    /// List view, and names no task it is behind.
    #[test]
    fn a_wait_the_list_names_draws_its_edge_and_queue_order_names_no_task() {
        let _budget = TestBudget::new();
        let mut queue = queued(&["head", "photo", "media", "other"], 5);
        queue.tasks[1].waits = vec!["media".into()];
        let look = look(vec![], queue, 0, vec![]);
        let page = drawn(&look);
        assert!(
            page.contains(r#"data-from="media" data-to="photo""#),
            "{page}"
        );
        assert!(node(&page, "photo").contains("waits on media"), "{page}");
        assert!(node(&page, "other").ends_with(">queued"), "{page}");
        assert!(!page.contains("behind head"), "{page}");
        let list = list_page(&look);
        assert!(list.contains(">queued<"), "{list}");
        assert!(!list.contains("behind head"), "the List view: {list}");
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
        let says = "a waits: every seat is taken, 2 sessions open of 2 allowed";
        assert!(node(&page, "a").contains(says), "{page}");
        assert!(node(&page, "b").ends_with(">queued"), "{page}");
        assert!(page.contains(&format!("Next: nothing, {says}")), "{page}");
        assert!(!page.contains(r#"class="node next""#), "{page}");

        let spent = Limit {
            account: "one".into(),
            five_hours: 100.0,
            five_hours_resets: 1,
            week: 10.0,
            week_resets: 1,
            key: false,
            models: vec![],
        };
        let on_load = look(vec![open("a", &[])], queued(&["a"], 5), 0, vec![spent]);
        let page = drawn(&on_load);
        let says = "a waits: every account is at its limit";
        assert!(node(&page, "a").contains(says), "{page}");
        assert!(page.contains(&format!("Next: nothing, {says}")), "{page}");
    }

    #[test]
    fn a_merged_wait_is_left_out_an_ended_one_drawn_and_the_past_stays_out() {
        let _budget = TestBudget::new();
        let standing = vec![
            stood("old", "finished", Some("merged"), &[]),
            stood("failed", "finished", Some("ended"), &["old"]),
            stood("gone-by", "finished", Some("merged"), &[]),
            stood("now", "open", None, &["old", "failed"]),
        ];
        let page = drawn(&look(standing, queued(&["now"], 5), 0, vec![]));
        assert!(!page.contains(r#"data-task="old""#), "{page}");
        assert!(
            node(&page, "failed").contains(r#"class="node ended""#),
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
            sides: None,
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
        assert!(
            page.contains("list, synced 9 h ago: it syncs only while a page is open."),
            "{page}"
        );
        // task rust-tasks-no-sync-note: the drawn page too, silent on a current list, at once on a failed sync
        known.now_ms = 60_000;
        assert!(!drawn(&known).contains("Rust's list"), "{}", drawn(&known));
        if let Some(queue) = &mut known.queue {
            queue.2 = Some("its sync failed: the older app timed out".into());
        }
        let page = drawn(&known);
        assert!(
            page.contains("synced 1 min ago: its sync failed: the older app timed out"),
            "{page}"
        );
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
            sides: None,
        };
        let down = || Some((Err("down".to_string()), 0, None));
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
        // a row draws its top tasks, past the graph's limit too, and counts the rest
        assert_eq!(many.matches(r#"class="node "#).count(), TOP, "{many}");
        assert!(many.contains("493 more in On no roadmap item"), "the words");
        assert!(many.contains("Next: t0"), "the next line");
        assert!(!many.contains("behind t0"), "no why of queue order: {many}");
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

    /// A task the list offers on roadmap item `item` of `horizon` (« » for none), its epic the item, waiting on `waits`.
    fn placed(id: &str, horizon: &str, item: &str, waits: &[&str]) -> Queued {
        let named = |w: &str| (!w.is_empty()).then(|| w.to_string());
        Queued {
            id: id.into(),
            waits: waits.iter().map(|w| w.to_string()).collect(),
            rank: dash_core::tasks::Rank {
                horizon: named(horizon),
                item: named(item),
                priority: Some(2),
                ..Default::default()
            },
            epic: named(item),
            ..Default::default()
        }
    }

    fn sectioned(look: &Look, shown: &Shown) -> (String, String) {
        let tree = graph_tree(look, shown);
        let page = surface::markup(&surface::screen::page("t", &tree).expect("a page"));
        (page, surface::text(&tree).expect("a text"))
    }

    /// The HTML of row `row`'s section (its horizon, « none » for no item), up to the next section.
    fn section<'a>(page: &'a str, row: &str) -> &'a str {
        let start = page
            .find(&format!(r#"class="issue-part" id="row-{row}""#))
            .unwrap_or_else(|| panic!("no row {row}: {page}"));
        let end = page[start + 1..]
            .find(r#"class="issue-part""#)
            .map_or(page.len(), |e| start + 1 + e);
        &page[start..end]
    }

    /// Done-when 1 of rust-graph-why-these: Now, Next and Later each draw their top tasks and what those wait on, each section
    /// linking its epic, the rest counted.
    #[test]
    fn the_graph_draws_now_next_and_later_each_its_top_tasks_their_waits_and_its_epic() {
        let _budget = TestBudget::new();
        let mut tasks: Vec<Queued> = (0..TOP + 2)
            .map(|i| placed(&format!("now-{i}"), "now", "rust-poc", &[]))
            .collect();
        // the first waits on a Later task: drawn in Now's graph as its wait
        tasks[0].waits = vec!["later-base".into()];
        tasks.push(placed("next-a", "next", "klar-plus-beta", &[]));
        tasks.push(placed("later-base", "later", "jobs", &[]));
        // a horizon the roadmap's four do not name is drawn on no item, never lost (/code-review)
        tasks.push(placed("odd", "someday", "jobs", &[]));
        let look = look(vec![], Queue { seats: 5, tasks }, 0, vec![]);
        let (page, text) = sectioned(&look, &Shown::default());
        for title in ["Urgent", "Now", "Next", "Later"] {
            assert!(text.contains(&format!("\n{title}\n")), "{title}: {text}");
            section(&page, &title.to_lowercase());
        }
        assert!(
            section(&page, "none").contains(r#"data-task="odd""#),
            "{page}"
        );
        let now = section(&page, "now");
        assert_eq!(now.matches(r#"data-task="now-"#).count(), TOP, "{now}");
        assert!(
            now.contains(r#"data-from="later-base" data-to="now-0""#),
            "{now}"
        );
        assert!(now.contains("2 more in Now"), "{now}");
        assert!(
            now.contains(r#"href="/tasks?layout=graph&#38;epic=rust-poc""#),
            "{now}"
        );
        let next = section(&page, "next");
        assert!(
            next.contains(r#"data-task="next-a""#) && !next.contains("now-"),
            "{next}"
        );
        assert!(next.contains("epic=klar-plus-beta"), "{next}");
        assert!(section(&page, "later").contains("epic=jobs"), "{page}");
        assert!(
            section(&page, "urgent").contains("Nothing open in Urgent."),
            "{page}"
        );
        assert!(
            text.contains("Epic: rust-poc /tasks?layout=graph&epic=rust-poc"),
            "{text}"
        );
    }

    /// Done-when 2 of rust-graph-why-these: what runs and the next pick are marked, each saying its rank reason, the one `dash
    /// orch next` prints.
    #[test]
    fn the_running_task_and_the_next_pick_are_marked_with_their_rank_reason() {
        let _budget = TestBudget::new();
        let tasks = vec![
            placed("busy", "now", "rust-poc", &[]),
            placed("first", "now", "rust-poc", &[]),
            placed("other", "next", "jobs", &[]),
        ];
        let standing = vec![stood("busy", "claimed", None, &[])];
        let queue = Queue { seats: 5, tasks };
        let look = look(standing, queue, 1, vec![]);
        // `dash orch next`'s reasons: the rank of every task offered, the claimed one too
        let mut offered = look
            .queue
            .as_ref()
            .unwrap()
            .0
            .as_ref()
            .unwrap()
            .tasks
            .clone();
        let order = dash_core::tasks::rank(&mut offered, &look.standing);
        let why = |id: &str| one_line(&order.iter().find(|o| o.0 == id).expect("ranked").1);
        let plan = &look.plan.as_ref().unwrap().as_ref().unwrap().order;
        assert_eq!(
            plan[0],
            order.iter().find(|o| o.0 == "first").unwrap().clone(),
            "the plan's own words"
        );
        let (page, text) = sectioned(&look, &Shown::default());
        let first = node(&page, "first");
        assert!(first.contains(r#"class="node next""#), "{first}");
        assert!(
            first.contains(&format!("the next pick: {}", why("first"))),
            "{first}"
        );
        assert!(
            why("first").contains("now (rust-poc), priority 2"),
            "{}",
            why("first")
        );
        let busy = node(&page, "busy");
        assert!(busy.contains(r#"class="node running""#), "{busy}");
        assert!(busy.contains(&why("busy")), "{busy}");
        assert!(
            text.contains(&format!("first\tnext\tthe next pick: {}", why("first"))),
            "{text}"
        );
        assert!(
            !node(&page, "other").contains("priority"),
            "only the marked say it: {page}"
        );
    }

    /// Done-when 3 of rust-graph-why-these: the epic and label selectors filter the graph, each kept in its URL and said in
    /// the text form, the one shown marked.
    #[test]
    fn epic_and_label_selectors_filter_the_graph_kept_in_the_url_and_the_text() {
        let _budget = TestBudget::new();
        let mut docs = placed("poc-docs", "now", "rust-poc", &[]);
        docs.labels = vec!["Docs".into()];
        let tasks = vec![
            placed("poc-build", "now", "rust-poc", &[]),
            docs,
            placed("beta", "next", "klar-plus-beta", &[]),
        ];
        let look = look(vec![], Queue { seats: 5, tasks }, 0, vec![]);
        let shown = Shown {
            epic: Some("rust-poc".into()),
            label: None,
        };
        let (page, text) = sectioned(&look, &shown);
        assert!(
            page.contains(r#"data-task="poc-build""#) && !page.contains(r#"data-task="beta""#),
            "{page}"
        );
        assert!(
            text.contains("Epic: All /tasks?layout=graph, klar-plus-beta /tasks?layout=graph&epic=klar-plus-beta, rust-poc /tasks?layout=graph&epic=rust-poc (shown)"),
            "{text}"
        );
        assert!(text.contains("Label: All /tasks?layout=graph&epic=rust-poc (shown), Docs /tasks?layout=graph&epic=rust-poc&label=Docs"), "{text}");
        let both = Shown {
            epic: Some("rust-poc".into()),
            label: Some("docs".into()),
        };
        let (page, text) = sectioned(&look, &both);
        assert!(page.contains(r#"data-task="poc-docs""#), "{page}");
        // a label any case is the one filter, marked shown (/code-review)
        assert!(text.contains("label=Docs (shown)"), "{text}");
        assert!(!page.contains(r#"data-task="poc-build""#), "{page}");
        let none = Shown {
            epic: Some("nothing".into()),
            label: None,
        };
        let (page, _) = sectioned(&look, &none);
        assert!(
            page.contains("No open Rust task of this epic and label."),
            "{page}"
        );
        // the URL's words reach the selectors: /tasks?layout=graph&epic=…&label=…
        let query = "layout=graph&epic=rust-poc&label=Docs";
        assert_eq!(
            crate::pages::param(query, "epic").as_deref(),
            Some("rust-poc")
        );
        assert_eq!(
            Shown::href(Some("a b"), Some("Docs")),
            "/tasks?layout=graph&epic=a%20b&label=Docs"
        );
    }

    /// Done-when 5 of rust-graph-why-these: rows run Urgent, Now, Next, Later down the page, each roadmap item a group inside
    /// its row, its tasks left to right by wait depth, each wait an arrow.
    #[test]
    fn rows_run_urgent_to_later_epics_inside_tasks_left_to_right_by_wait_depth() {
        let _budget = TestBudget::new();
        let mut due = placed("due", "later", "jobs", &[]);
        due.rank.deadline = Some("2026-11-15".into());
        let tasks = vec![
            due,
            placed("base", "now", "rust-poc", &[]),
            placed("middle", "now", "rust-poc", &["base"]),
            placed("top", "now", "rust-poc", &["middle"]),
            placed("beta", "now", "klar-plus-beta", &[]),
            placed("soon", "next", "jobs", &[]),
            placed("someday", "later", "jobs", &[]),
        ];
        let look = look(vec![], Queue { seats: 5, tasks }, 0, vec![]);
        let (page, _) = sectioned(&look, &Shown::default());
        let at = |row: &str| {
            page.find(&format!(r#"class="issue-part" id="row-{row}""#))
                .expect(row)
        };
        assert!(
            at("urgent") < at("now") && at("now") < at("next") && at("next") < at("later"),
            "{page}"
        );
        assert!(
            section(&page, "urgent").contains(r#"data-task="due""#),
            "a deadline is urgent: {page}"
        );
        let now = section(&page, "now");
        // two epics inside Now, each its link and its graph
        let (poc, beta) = (
            now.find("epic=rust-poc").expect("poc"),
            now.find("epic=klar-plus-beta").expect("beta"),
        );
        assert!(
            poc < beta && now.matches(r#"class="task-graph""#).count() == 2,
            "{now}"
        );
        let left = |id: &str| {
            let n = node(now, id);
            let x = &n[n.find("left:").expect("its left") + 5..];
            x[..x.find("px").expect("px")]
                .parse::<usize>()
                .expect("a number")
        };
        assert!(
            left("base") < left("middle") && left("middle") < left("top"),
            "{now}"
        );
        // clear arrows: one edge a wait, from the wait's right side to its task's left side, left to right, each ending in
        // its graph's own arrowhead, which that graph defines
        for (from, to) in [("base", "middle"), ("middle", "top")] {
            let edge = format!(r#"data-from="{from}" data-to="{to}" d="M"#);
            let at = now.find(&edge).unwrap_or_else(|| panic!("{edge}: {now}"));
            let path = &now[at + edge.len()..];
            let path = &path[..path.find('"').expect("its end")];
            let start: usize = path.split(' ').next().unwrap().parse().expect("x1");
            let end = path.rsplit(' ').nth(1).expect("its last x");
            let end: usize = end
                .rsplit(['C', 'H', ' '])
                .next()
                .unwrap()
                .parse()
                .expect("x2");
            assert_eq!((start, end), (left(from) + 300, left(to)), "{path}");
            let tail = &now[at..];
            let tag = &tail[..tail.find("/>").expect("the edge's end")];
            assert!(
                tag.ends_with(r#"marker-end="url(#row-now-graph-0-arrow)""#),
                "{tag}"
            );
        }
        assert_eq!(
            now.matches(r#"class="edge""#).count(),
            2,
            "an edge a wait, none else: {now}"
        );
        assert!(
            now.contains(r#"id="row-now-graph-0-arrow""#) && now.contains(r#"class="arrow""#),
            "{now}"
        );
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
    fn list_raw(look: &Look) -> String {
        surface::screen::page("t", &list_tree(look, TaskView::List, None, "")).expect("a page")
    }

    /// Its markup, what a test asserts a class or an attribute on (task rust-test-markup-only).
    fn list_page(look: &Look) -> String {
        surface::markup(&list_raw(look))
    }

    /// The words dash ui's window shows of the same page: its HTML, drawn as the window draws it.
    fn list_window(look: &Look) -> String {
        let props = dash_ui::PageProps {
            html: list_raw(look),
        };
        dash_ui::shown(dioxus::prelude::VirtualDom::new_with_props(
            dash_ui::Page,
            props,
        ))
    }

    /// A task's page as served, its style and scripts in.
    fn task_raw(look: &Look, id: &str) -> String {
        let tree = task_tree(look, id, &Ok(vec![])).unwrap_or_else(|| panic!("no task {id}"));
        surface::screen::page("t", &tree).expect("a page")
    }

    /// Its markup, what a test asserts a class or an attribute on (task rust-test-markup-only).
    fn task_page(look: &Look, id: &str) -> String {
        surface::markup(&task_raw(look, id))
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
        assert!(list_raw(&look).contains("--color-accent:"), "its theme");
        let page = list_page(&look);
        assert!(page.contains("Next: first"), "held waits on it: {page}");
        assert_eq!(rows_of(&page, "in-review"), ["reviewed"], "{page}");
        assert_eq!(rows_of(&page, "in-progress"), ["busy", "gone"], "{page}");
        assert_eq!(
            rows_of(&page, "todo"),
            ["first", "second", "third"],
            "next, then what is behind it, ranked: {page}"
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
            "reviewed", "busy", "gone", "first", "second", "third", "held", "unlisted",
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
    fn with_no_plan_known_the_list_ranks_the_queue_as_the_plan_does() {
        let _budget = TestBudget::new();
        let open = |id: &str| stood(id, "open", None, &[]);
        let mut look = look(
            vec![open("a"), open("z")],
            queued(&["z", "a"], 5),
            0,
            vec![],
        );
        // equal ranks: by id, never the list's stored order
        assert_eq!(rows_of(&list_page(&look), "todo"), ["a", "z"]);
        look.plan = None;
        assert_eq!(rows_of(&list_page(&look), "todo"), ["a", "z"]);
        look.plan = Some(Err("the sessions are down".into()));
        assert_eq!(rows_of(&list_page(&look), "todo"), ["a", "z"]);
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
            // what the orchestrator moved itself: its words alone (task rust-orch-flags-only-stuck)
            event(
                240_000,
                "note",
                None,
                Some("handed back to a new run: a conflict"),
            ),
        ];
        let tree = task_tree(&look, "a-task", &Ok(history.to_vec())).expect("its page");
        let said = surface::text(&tree).expect("its text");
        let told = "Activity\nStarted, try 1\t1970-01-01 00:00 UTC\nFinished: failed\t1970-01-01 00:01 UTC\nOpened: failed\t1970-01-01 00:02 UTC\nOpened\t1970-01-01 00:03 UTC\nhanded back to a new run: a conflict\t1970-01-01 00:04 UTC\n";
        assert!(said.contains(told), "{said}");
        // its pull request among its properties, the same tree drawn
        let page = surface::markup(&surface::screen::page("t", &tree).expect("a page"));
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
        if let Some((Ok(queue), _, _)) = &mut look.queue {
            queue.tasks[0].title = "From the queue".into();
        }
        let page = task_page(&look, "a");
        assert!(
            page.contains(r#"class="li-issue-title">From the queue"#),
            "{page}"
        );
        if let Some((Ok(queue), _, _)) = &mut look.queue {
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
        let unread_raw = task_raw(&look, "a");
        let unread = surface::markup(&unread_raw);
        let skeleton = r#"id="issue-why" class="skeleton" aria-busy="true" role="status" aria-label="Its words: being read""#;
        assert!(unread.contains(skeleton), "{unread}");
        // its two skeletons (its words, its checks), one refill loop
        let refills = unread_raw.matches("window.dashLive=1").count();
        assert_eq!(refills, 1, "{unread_raw}");
        let text =
            surface::text(&task_tree(&look, "a", &Ok(vec![])).expect("a page")).expect("a text");
        assert!(text.contains("Its words: being read."), "{text}");
        look.issues = Some(Ok(vec![issue("a", "t", &[])]));
        let read_raw = task_raw(&look, "a");
        let read = surface::markup(&read_raw);
        assert!(
            read.split(r#"id="issue-why">"#)
                .nth(1)
                .and_then(|p| p.split(r#"id="issue-checks""#).next())
                .is_some_and(|why| why.contains(r#"class="doc-text">why of a"#)),
            "{read}"
        );
        assert!(
            !read.contains("aria-busy") && !read_raw.contains("window.dashLive"),
            "{read_raw}"
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
        assert!(read.contains(r#"class="doc-text">None written."#), "{read}");
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
        let html = surface::markup(&surface::screen::page("Home", &tree).expect("a page"));
        let body = &html[html.find(r#"class="app""#).expect("the app")..];
        assert!(
            body.contains(r#"class="form" id="ask" method="post" action="/compass""#)
                && body.contains(r#"name="page" type="hidden" value="/""#)
                && body.contains(r#"name="text""#),
            "{body}"
        );
        assert_eq!(
            body.matches(r#"class="item issue-row""#).count(),
            3,
            "{body}"
        );
        for href in ["/inbox", "/plan", "/insights"] {
            assert!(
                body.contains(&format!(r#"class="item issue-row" href="{href}""#)),
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
            let Drawing::Tree { tree, .. } = read_page("Inbox", Default::default(), &parts) else {
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
        let html = surface::markup(&surface::screen::page("Inbox", &tree).expect("a page"));
        let card = r#"id="waits-0-0" class="item issue-row" href="/drafts/acme-intro">"#;
        assert!(
            html.contains(card) && html.contains(r#"class="title">Ann (Acme)"#),
            "{html}"
        );
        assert!(!html.contains("## ") && !html.contains("- acme"), "{html}");
        assert!(
            surface::text(&tree)
                .expect("a text")
                .contains("\nDrafts to review (1)\nAnn (Acme)\tacme-intro\t/drafts/acme-intro\n")
        );
    }

    /// Task rust-page-latency, done-when 3: /tasks with Todo's 1205 cards of 2026-11-10 drew 1.4 MB, its render 1.6 s under
    /// load (trace 1794314321744293823). A column draws its first BOARD_CARDS and links the list for the rest, its count whole,
    /// in the HTML and the text; the whole page, tree and render, takes under 300 ms; the list still draws every row.
    #[test]
    fn a_long_column_draws_its_first_cards_and_the_board_renders_under_300_ms() {
        let _budget = TestBudget::new();
        let mut look = look_empty();
        let open = |i: usize| Open {
            project: "klar".into(),
            id: format!("k-{i}"),
            status: "open".into(),
            title: format!("A klar task, number {i}, with a title as long as most of his"),
        };
        look.every = Some(Ok((0..1521).map(open).collect()));
        let at = std::time::Instant::now();
        let tree = list_tree(&look, TaskView::Board, None, "");
        let served = surface::screen::page("Tasks", &tree).expect("a page");
        let took = at.elapsed();
        assert!(took < Duration::from_millis(300), "/tasks took {took:?}");
        // the budget is what the browser downloads, its style and scripts in
        assert!(served.len() < 200_000, "{} bytes", served.len());
        // the count is the column's whole, its cards the first BOARD_CARDS, the rest one link after them (the surface's own
        // test checks its HTML: rust/crates/surface/src/screen.rs)
        let text = surface::text(&tree).expect("a text");
        assert!(text.contains("\nTodo (1521)\nk-0\t"), "{text}");
        assert_eq!(text.matches("\t#klar\n").count(), BOARD_CARDS, "{text}");
        assert!(
            text.contains("\nk-49\t") && !text.contains("\nk-50\t"),
            "{text}"
        );
        assert!(
            text.contains("\t#klar\n1471 more: /tasks?layout=list#todo\n"),
            "{text}"
        );
        // a project's board links its own list
        let klar =
            surface::text(&list_tree(&look, TaskView::Board, Some("klar"), "")).expect("a text");
        assert!(
            klar.contains("\n1471 more: /tasks?layout=list&project=klar#todo\n"),
            "{klar}"
        );
        // the list is where they all are: every row, no link
        let list = surface::text(&list_tree(&look, TaskView::List, None, "")).expect("a text");
        assert!(
            list.contains("\nk-1520\t") && !list.contains(" more: "),
            "the list's rows"
        );
    }

    /// /code-review of rust-page-latency: Rust's own cards are all drawn, past BOARD_CARDS too, since his drag and the view's
    /// patches name them; only the other projects' are cut.
    #[test]
    fn a_column_draws_every_rust_card_and_cuts_the_others() {
        let _budget = TestBudget::new();
        let ids: Vec<String> = (0..60).map(|i| format!("r-{i}")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let standing = ids.iter().map(|id| stood(id, "open", None, &[])).collect();
        let mut look = look(standing, queued(&ids, 5), 0, vec![]);
        let open = |i: usize| Open {
            project: "klar".into(),
            id: format!("k-{i}"),
            status: "open".into(),
            title: format!("k-{i} title"),
        };
        look.every = Some(Ok((0..100).map(open).collect()));
        let text = surface::text(&list_tree(&look, TaskView::Board, None, "")).expect("a text");
        for id in &ids {
            assert!(text.contains(&format!("\n{id}\t")), "{id}: {text}");
        }
        assert!(text.contains("\nTodo (160)\n"), "{text}");
        assert!(
            !text.contains("\nk-0\t") && text.contains("\n100 more: /tasks?layout=list#todo\n"),
            "{text}"
        );
    }

    /// Task rust-page-latency: an older app that answers every read at once, its database as much of it as the sync reads, in
    /// `dir`'s repo. Its handoff, daily and roadmap are words; `task issues` an empty list; `task ls` one klar task.
    fn older_app_answering(dir: &Path) {
        let older = dir.join("older-dash");
        let script = "#!/bin/sh\ncase \"$*\" in\n'task issues') echo '[]' ;;\n'task ls') printf 'klar\\tk-1\\topen\\tA klar task\\n' ;;\n*) echo \"- [ ] **$* read**\" ;;\nesac\n";
        std::fs::write(&older, script).expect("the older app");
        std::fs::set_permissions(&older, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        let db = dir.join("private/dashboard");
        std::fs::create_dir_all(&db).expect("its folder");
        let db = dash_store::Connection::open(db.join("dashboard_dev.db")).expect("its db");
        db.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT,
               blocked_by TEXT, owner TEXT, priority INTEGER, epic TEXT);
             CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, data TEXT NOT NULL);
             CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
             CREATE TABLE project_settings (project TEXT, field TEXT, value TEXT NOT NULL);",
        )
        .expect("its tables");
        // nextest gives each test its own process (rust/CLAUDE.md rule 1), so nothing else reads these variables
        unsafe {
            std::env::set_var("DASH_CLI", &older);
            std::env::set_var("DASH_REPO", dir);
            std::env::set_var("DASH_WORKTREES", dir);
        }
    }

    /// Every read `start_reads` starts, ended: none still reading, each page's and the board's.
    fn settled(board: &Board) -> bool {
        let reads = [&READS.handoff, &READS.progress, &READS.roadmap];
        let pages = reads.iter().all(|r| {
            let r = r.lock().expect("a read");
            r.last.is_some() && !r.reading
        });
        let queue = board.queue.lock().expect("the queue");
        let every = board.every.lock().expect("every");
        pages && queue.issues.is_some() && !queue.reading && every.last.is_some() && !every.reading
    }

    /// Task rust-page-latency, done-when 1: each background read is a root span named by its command, the older app's command
    /// as `dash <command>` runs it, the queue's sync as « queue ».
    #[test]
    fn each_background_read_is_a_span_named_by_its_command() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("read-spans");
        older_app_answering(&dir.0);
        let agent = std::net::TcpListener::bind("127.0.0.1:0").expect("the trace agent");
        // the switch on: any address is on, the statsd lines go nowhere that listens
        std::fs::write(dir.0.join("telemetry"), "127.0.0.1:9").expect("the switch");
        crate::reporting::arm_to(&dir.0, agent.local_addr().expect("its address"));
        let board = Board::open(&dir.0).expect("the tasks");
        start_reads(&board);
        agent.set_nonblocking(false).expect("blocking");
        let (seen, want) = (
            Arc::new(Mutex::new(BTreeSet::new())),
            [
                "cost fit",
                "feedback count",
                "focus roadmap",
                "handoff",
                "progress",
                "queue",
                "task issues",
                "task ls",
            ],
        );
        let found = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in agent.incoming() {
                // its head, then as many bytes as it says, then the agent's answer: the sender waits on it
                let mut stream = std::io::BufReader::new(stream.expect("a trace"));
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    std::io::BufRead::read_line(&mut stream, &mut line).expect("its head");
                    if let Some(n) = line.strip_prefix("Content-Length: ") {
                        length = n.trim().parse().expect("a length");
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                std::io::Read::read_exact(&mut stream, &mut body).expect("its body");
                let _ = stream
                    .get_mut()
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                let traces: Vec<Vec<Value>> = serde_json::from_slice(&body).expect("a v0.3 trace");
                // each trace's root, judged on the test's own thread: a panic here would only end this one
                for span in traces.iter().flatten().filter(|s| s["parent_id"] == 0) {
                    let field = |f: &str| span[f].as_str().unwrap_or_default().to_string();
                    found
                        .lock()
                        .expect("seen")
                        .insert((field("resource"), field("name")));
                }
            }
        });
        // /insights' reads start with its page, never at boot
        let _ = insights_page();
        let resources = |seen: &BTreeSet<(String, String)>| -> BTreeSet<String> {
            seen.iter().map(|(r, _)| r.clone()).collect()
        };
        for _ in 0..200 {
            if want
                .iter()
                .all(|w| resources(&seen.lock().expect("seen")).contains(*w))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let seen = seen.lock().expect("seen").clone();
        for w in want {
            assert!(resources(&seen).contains(w), "no span « {w} » in {seen:?}");
        }
        for (resource, name) in &seen {
            assert_eq!(
                name,
                crate::reporting::READ_SPAN,
                "the root of « {resource} »"
            );
        }
    }

    /// Task rust-page-latency, done-when 2: `dash serve` starts every read at boot (`start_reads`), so the first page drawn
    /// after a deploy shows what was read, not its loader, and is drawn in under 300 ms. Before, the first page asked for a
    /// read and drew « being read » for its whole length, 10 s and more (@nha, 2026-11-10).
    #[test]
    fn the_first_page_after_boot_shows_data_not_its_loader() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("boot-reads");
        older_app_answering(&dir.0);
        let board = Board::open(&dir.0).expect("the tasks");
        start_reads(&board);
        for _ in 0..200 {
            if settled(&board) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(settled(&board), "the reads started at boot ended");
        // /insights' reads start with its page, never at boot (`start_reads`)
        type Draw = fn(&Board) -> Drawing;
        let pages: [(&str, Draw); 3] = [
            ("/inbox", |_| inbox_page(None)),
            ("/plan", |_| plan_page()),
            ("/tasks", |b| {
                b.tasks(TaskView::Board, None).expect("the board")
            }),
        ];
        for (path, draw) in pages {
            let at = std::time::Instant::now();
            let Drawing::Tree { tree, .. } = draw(&board) else {
                panic!("{path}: a tree")
            };
            let served = surface::screen::page(path, &tree).expect("a page");
            let took = at.elapsed();
            let html = surface::markup(&served);
            let text = surface::text(&tree).expect("a text");
            assert!(
                !text.contains("being read"),
                "{path}, its first drawing: {text}"
            );
            assert!(took < Duration::from_millis(300), "{path} took {took:?}");
            assert!(!html.contains("being read"), "{path}: its loader is drawn");
            if path == "/inbox" {
                assert!(
                    text.contains("handoff read"),
                    "/inbox shows the handoff read: {text}"
                );
            }
        }
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
        read_words(&state, SLOW_READ, "up", || Ok("up".into()));
        assert_eq!(settled(&state), Some(Ok("up".into())));
        state.lock().expect("a read").at = now_ms() - 61_000;
        read_words(&state, SLOW_READ, "again", || Ok("again".into()));
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

    /// Done-when 1 of graal-moves-to-rust: dash task run refuses a graal task, a row of project graal Rust does not own or an
    /// issue of graal's board, naming the Rust task that replaces it; a graal task refiled --owner rust and any other run.
    #[test]
    fn a_graal_task_is_refused_naming_the_rust_task_that_replaces_it() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("graal-refused");
        let db = dir.0.join("older.db");
        let store = dash_store::Connection::open(&db).expect("db");
        let rows = "CREATE TABLE tasks (collection TEXT, id TEXT, project TEXT, owner TEXT, data TEXT NOT NULL);
            CREATE TABLE records (collection TEXT, id TEXT, data TEXT NOT NULL);
            INSERT INTO tasks VALUES ('tasks.edn', 'g-moved', 'graal', NULL, '{\"duplicate-of\":\"rust-port\"}'),
              ('tasks.edn', 'g-open', 'graal', ':claude', '{}'), ('tasks.edn', 'g-refiled', 'graal', ':rust', '{}'),
              ('tasks.edn', 'mine', 'site_perso', 'rust', '{}');
            INSERT INTO records VALUES ('issues-graal.edn', 'graal-230', '{}'), ('issues-klar.edn', 'klar-1', '{}');";
        store.execute_batch(rows).expect("rows");
        let refused = |id: &str| graal_refused(&db, id).unwrap_err();
        assert_eq!(
            refused("g-moved"),
            "g-moved: a graal task, and graal starts nothing now: Rust task rust-port replaces it"
        );
        for id in ["g-open", "graal-230"] {
            let refusal = refused(id);
            assert!(
                refusal.ends_with(
                    "no Rust task replaces it, rust-port-map moves graal's modules to Rust"
                ),
                "{refusal}"
            );
        }
        for id in ["g-refiled", "mine", "klar-1", "gone"] {
            assert_eq!(graal_refused(&db, id), Ok(()), "{id} runs");
        }
        let unread = graal_refused(&dir.0.join("none.db"), "mine").unwrap_err();
        assert!(
            unread.starts_with("mine: whether graal holds it is not known"),
            "{unread}"
        );
    }

    /// Done-when 3 of graal-moves-to-rust: /tasks In Progress lists no graal task, whatever its board says (Active, Ongoing),
    /// since graal starts nothing; another project's work in progress stays there.
    #[test]
    fn in_progress_lists_no_graal_task() {
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
            open("graal", "230", "Active"),
            open("graal", "68", "Ongoing"),
            open("graal", "graal-review-light", "in progress"),
            open("klar", "12", "In Progress"),
        ]));
        for project in [None, Some("graal")] {
            let text =
                surface::text(&list_tree(&look, TaskView::Board, project, "")).expect("a text");
            let progress = text.split("\nIn Progress (").nth(1).expect("the column");
            let progress = progress.split("\n\n").next().unwrap_or_default();
            assert!(!progress.contains("graal"), "{progress}\n\n{text}");
            assert!(text.contains("\n230\t230 title\t"), "listed still: {text}");
        }
        let all = surface::text(&list_tree(&look, TaskView::Board, None, "")).expect("a text");
        assert!(
            all.contains("\nIn Progress (1)\n12\t12 title\tklar\t#klar\n"),
            "{all}"
        );
        // a graal row refiled --owner rust whose run Rust holds stays at work; one Rust lists is listed once
        look.columns.insert("graal-ported".into(), "in-progress");
        let rows = look
            .every
            .as_mut()
            .and_then(|e| e.as_mut().ok())
            .expect("every");
        rows.extend([
            open("graal", "graal-ported", "claimed"),
            open("graal", "a-task", "open"),
        ]);
        let graal =
            surface::text(&list_tree(&look, TaskView::Board, Some("graal"), "")).expect("a text");
        assert!(
            graal.contains("\nIn Progress (1)\ngraal-ported\t"),
            "{graal}"
        );
        let all = surface::text(&list_tree(&look, TaskView::Board, None, "")).expect("a text");
        assert_eq!(
            all.matches("\na-task\t").count(),
            1,
            "Rust's own once: {all}"
        );
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
            all.starts_with("Tasks\nNew issue: POST /tasks/new title, description\nViews: Board /tasks (shown), "),
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
        assert!(klar.starts_with("Tasks: klar\nNew issue: POST /tasks/new title, description, project=klar\nViews: Board /tasks?project=klar (shown), List /tasks?layout=list&project=klar, "), "{klar}");
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
        // task rust-issue-sidebar-menus, done-when 4: what its page's five menus set shows on its card, once the sidebars are read
        look.sides = Some(Arc::new(Sides {
            tasks: BTreeMap::from([(
                "a-task".to_string(),
                dash_store::older::Sidebar {
                    owner: "nicolas".into(),
                    labels: vec!["Bug".into(), "Docs".into()],
                    epic: Some("rust-poc".into()),
                    cycle: Some("2026-11-09".into()),
                    // ahead of any day this test runs on: a reminder past has fired (card_tags_leave_a_fired_reminder_off)
                    remind: Some("2099-11-12".into()),
                },
            )]),
            epics: vec![],
        }));
        let card = surface::text(&board(&look, None)).expect("a text");
        look.sides = None;
        let card = card
            .lines()
            .find(|l| l.starts_with("a-task\t"))
            .expect("its card");
        assert!(
            card.ends_with(
                "\t#site_perso\t#Bug\t#Docs\t#rust-poc\t#Cycle Nov 9\t#Remind Nov 12\t@Nicolas Ha"
            ),
            "{card}"
        );
        // the list's rows show no label nor assignee, so neither does its text
        let list = surface::text(&list_tree(&look, TaskView::List, None, "")).expect("a text");
        assert!(
            !list.contains("#site_perso") && !list.contains("@Rust"),
            "{list}"
        );
        let html = surface::markup(
            &surface::screen::page("t", &list_tree(&look, TaskView::Board, None, ""))
                .expect("a page"),
        );
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

    /// /code-review of PR 1395: his pick in a task page's menu is in the cached queue at once, so the page drawn after it shows
    /// it, and the next look reads the list again; none clears it.
    #[test]
    fn a_property_he_set_shows_before_the_next_read() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("tasks-board-noted");
        let locks = dir.0.join("worktrees");
        let tasks = store::open(&dir.0, &locks).expect("open");
        let views = board_views(store::open(&dir.0, &locks).expect("a reader")).expect("views");
        let board = Board {
            tasks,
            views: Arc::new(Mutex::new(views)),
            queue: Arc::default(),
            every: Arc::default(),
            data: dir.0.clone(),
        };
        let rank = |board: &Board| {
            let queue = board.queue.lock().expect("the queue");
            match &queue.last {
                Some((Ok(q), _, _)) => q.tasks[0].rank.clone(),
                _ => panic!("a queue"),
            }
        };
        // no queue read yet: nothing to fold into
        board.noted("a", Some(Some(1)), None);
        board.queue.lock().expect("the queue").last = Some((Ok(queued(&["a"], 2)), now_ms(), None));
        board.queue.lock().expect("the queue").tried_ms = now_ms();
        board.noted("a", Some(Some(1)), Some(Some("2026-11-15".into())));
        assert_eq!(rank(&board).priority, Some(1));
        assert_eq!(rank(&board).deadline.as_deref(), Some("2026-11-15"));
        assert!(
            board.queue.lock().expect("the queue").stale(now_ms()),
            "read again next"
        );
        board.noted("a", Some(None), Some(None));
        assert_eq!((rank(&board).priority, rank(&board).deadline), (None, None));
    }

    /// Done-when 2 of rust-board-linear-order: his drag, `Board::move_to` as `POST /tasks/move` calls it, puts the card in that
    /// column of the drawn board, In Review included; a column the board has not, or a card it does not place, is refused.
    #[test]
    fn a_card_dragged_to_a_column_takes_that_status() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("tasks-board-move");
        let locks = dir.0.join("worktrees");
        let mut tasks = store::open(&dir.0, &locks).expect("open");
        tasks.sync(&queued(&["a"], 2)).expect("synced");
        let views = board_views(store::open(&dir.0, &locks).expect("a reader")).expect("views");
        let mut board = Board {
            tasks,
            views: Arc::new(Mutex::new(views)),
            queue: Arc::default(),
            every: Arc::default(),
            data: dir.0.clone(),
        };
        let at = |board: &Board| {
            columns(board.board_view().expect("the board"))
                .get("a")
                .copied()
        };
        assert_eq!(at(&board), Some("todo"));
        board
            .move_to("a", "in-review")
            .expect("read")
            .expect("moved");
        assert_eq!(at(&board), Some("in-review"));
        let mut look = look(
            vec![stood("a", "open", None, &[])],
            queued(&["a"], 2),
            0,
            vec![],
        );
        look.columns = columns(board.board_view().expect("the board"));
        let raw = surface::screen::page("t", &list_tree(&look, TaskView::Board, None, ""))
            .expect("a page");
        let page = surface::markup(&raw);
        assert_eq!(rows_of(&page, "in-review"), ["a"], "{page}");
        assert!(page.contains(r#"data-task="a" draggable="true""#), "{page}");
        assert!(raw.contains(r#"fetch("/tasks/move""#), "its script: {raw}");
        board
            .move_to("a", "backlog")
            .expect("read")
            .expect("moved back");
        assert_eq!(at(&board), Some("backlog"));
        // the picker obeys it: Backlog and In Review hold the task, Todo lets it start
        let next = |board: &Board| {
            let queue = TaskStore::queue(&board.tasks).expect("the queue");
            plan(&Defaults, queue, &[], 0, vec![], None).next
        };
        assert_eq!(next(&board), Pick::Wait("a: he moved it to Backlog".into()));
        board
            .move_to("a", "in-review")
            .expect("read")
            .expect("moved");
        assert_eq!(
            next(&board),
            Pick::Wait("a: he moved it to In Review".into())
        );
        board.move_to("a", "todo").expect("read").expect("moved");
        assert!(matches!(next(&board), Pick::Start { id, .. } if id == "a"));
        board.move_to("a", "backlog").expect("read").expect("moved");
        assert_eq!(
            board.move_to("a", "done"),
            Ok(Err("done: no such column".into()))
        );
        assert_eq!(
            board.move_to("z", "todo"),
            Ok(Err("z: not a card of Rust's board".into()))
        );
        assert_eq!(at(&board), Some("backlog"), "a refusal moves nothing");
        // a run claims it, then crashes: freed, the card shows its own column, never the drag made before the run
        let mut run = store::open(&dir.0, &locks).expect("a run");
        TaskStore::claim(&mut run, "a").expect("claimed");
        assert_eq!(at(&board), Some("in-progress"));
        drop(run);
        assert!(TaskStore::free(&mut board.tasks, "a").expect("freed"));
        assert_eq!(at(&board), Some("todo"), "the drag was spent by the claim");
        assert!(
            matches!(next(&board), Pick::Start { id, .. } if id == "a"),
            "and the picker forgets it too"
        );
    }

    /// Done-when 1 of rust-board-linear-order: the board's columns run Backlog, Todo, In Progress, In Review left to right, each
    /// there when empty; the list keeps its order, furthest along first (`the_list_shows_open_tasks_by_status_in_pick_order`).
    #[test]
    fn the_board_runs_backlog_to_in_review_left_to_right() {
        let _budget = TestBudget::new();
        let look = look_empty();
        let page = surface::markup(
            &surface::screen::page("t", &list_tree(&look, TaskView::Board, None, ""))
                .expect("a page"),
        );
        let at = |s: &str| page.find(&format!(r#"data-status="{s}""#)).expect(s);
        assert!(at("backlog") < at("todo"), "{page}");
        assert!(at("todo") < at("in-progress"), "{page}");
        assert!(at("in-progress") < at("in-review"), "{page}");
        let text = surface::text(&list_tree(&look, TaskView::Board, None, "")).expect("a text");
        let heads: Vec<&str> = text.lines().filter(|l| l.ends_with(" (0)")).collect();
        assert_eq!(
            heads,
            [
                "Backlog (0)",
                "Todo (0)",
                "In Progress (0)",
                "In Review (0)"
            ],
            "{text}"
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
        let said = r#"class="doc-text">fix &#60;this&#62;"#;
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

    /// Task rust-task-brief-linear, done-when 3: /tasks/<id> is the Task brief template, Linear's issue page, in both forms:
    /// its description and his comments drawn by the one DocView, editable in place, its Activity ending on a comment box
    /// that posts a comment on this page to the compass.
    #[test]
    fn a_task_page_is_the_task_brief_in_its_html_and_its_text() {
        let _budget = TestBudget::new();
        let mut look = look(
            vec![stood("busy", "claimed", None, &[])],
            queued(&[], 5),
            1,
            vec![],
        );
        look.issues = Some(Ok(vec![issue("busy", "Busy", &["fix <this>"])]));
        let tree = task_tree(&look, "busy", &Ok(vec![])).expect("its page");
        let html = surface::markup(&surface::screen::page("t", &tree).expect("a page"));
        // the template's markup is surface's to assert (a_task_brief_is_the_way_back_its_state_why_checks_run_and_comments);
        // here, that this page is drawn through it: no tag literal in dash (dash-gate's the_dash_crate_holds_no_html)
        let edit = r#"class="dv notion" contenteditable="plaintext-only""#;
        for part in [
            r#"id="issue" class="doc doc-task-brief""#,
            r#"class="li-issue-title">Busy"#,
            r#"id="issue-why">"#,
            r#"class="doc-text">why of busy"#,
            r#"class="doc-text">fix &#60;this&#62;"#,
            r#"id="issue-comment" method="post" action="/compass""#,
            r#"name="page" type="hidden" value="/tasks/busy""#,
            r#"class="li-issue-side""#,
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        // the description, the comment and the comment box's field, each the one DocView, editable in place
        assert_eq!(html.matches(edit).count(), 3, "{html}");
        let text = surface::text(&tree).expect("its text");
        let told = "\nBusy\nwhy of busy\nDone when\nNone written.\nLast run\n";
        assert!(text.contains(told), "{text}");
        // Linear's sidebar (task rust-issue-sidebar-menus): no assignee, labels or project menu until the list was read
        assert!(
            !text.contains("Assign to") && !text.contains("\nLabels\n"),
            "{text}"
        );
        look.queue = Some((Ok(queued(&["busy"], 5)), now_ms(), None));
        let side = dash_store::older::Sidebar {
            owner: "rust".into(),
            labels: vec!["Docs".into()],
            epic: Some("rust-poc".into()),
            cycle: None,
            remind: Some("2026-11-20".into()),
        };
        look.sides = Some(Arc::new(Sides {
            tasks: BTreeMap::from([("busy".to_string(), side)]),
            epics: vec!["crm".into(), "rust-poc".into()],
        }));
        let ours = surface::text(&task_tree(&look, "busy", &Ok(vec![])).expect("its page"))
            .expect("its text");
        // done-when 2: each of the five a design-system menu on the issue page, its value shown, his choices offered
        for part in [
            "\nRust\nAssign to...: POST /tasks/busy/set assignee=\nclaude: Claude\nnicolas: Nicolas Ha\nvibe: Vibe\nrust: Rust (set)\n",
            "\nAdd to cycle\nAdd to cycle...: POST /tasks/busy/set cycle=\nnone: No cycle (set)\n",
            "\nRemind 2026-11-20\nRemind me...: POST /tasks/busy/set remind=\nnone: No reminder\n",
            // the day set that no preset is comes after them, so each preset keeps its digit
            "\n2026-11-20: Remind 2026-11-20 (set)\nLabels\n",
            "\nLabels\nDocs\nChange or add labels...: POST /tasks/busy/set labels= with on=1 to turn one on, on=0 off\nBug: Bug\n",
            "\nDocs: Docs (set)\n",
            "\nProject\nrust-poc\nAdd to project...: POST /tasks/busy/set project=\nnone: No project\ncrm: crm\nrust-poc: rust-poc (set)\n",
        ] {
            assert!(ours.contains(part), "{part} in {ours}");
        }
        // its place on the board and his menus (task rust-board-buttons-new-issue): status only where the board view places
        // it, priority and due day always, the due days from today
        assert!(ours.starts_with("Tasks\t› Busy busy 1 / 1\n"), "{ours}");
        // a claimed run is placed by the board view (In Progress), so its status is his to move
        assert!(
            ours.contains("Change status: POST /tasks/busy/set status=\n"),
            "placed by the board view, moved from its page: {ours}"
        );
        assert!(
            ours.contains("Set priority: POST /tasks/busy/set priority=\n1: Urgent\n"),
            "{ours}"
        );
        assert!(
            ours.contains("Set due date: POST /tasks/busy/set due=\n"),
            "{ours}"
        );
        assert!(
            text.contains(
                "Activity\nNicolas\nfix <this>\nUndo\tRedo\nPreview\nDiscard\tComment\nProperties\n"
            ),
            "{text}"
        );
    }

    /// /code-review of rust-issue-sidebar-menus' card tags: a reminder before today has fired and leaves the card, one today
    /// stays, a past cycle stays (Linear's), and a day that is no date is never printed raw.
    #[test]
    fn card_tags_leave_a_fired_reminder_off() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-11").expect("a day");
        let side = |cycle: &str, remind: &str| dash_store::older::Sidebar {
            labels: vec!["Bug".into()],
            cycle: Some(cycle.into()),
            remind: Some(remind.into()),
            ..Default::default()
        };
        assert_eq!(
            card_tags(&side("2026-10-26", "2026-11-11"), today),
            ["Bug", "Cycle Oct 26", "Remind Nov 11"]
        );
        assert_eq!(
            card_tags(&side("2026-10-26", "2026-11-10"), today),
            ["Bug", "Cycle Oct 26"]
        );
        assert_eq!(card_tags(&side(":cycle\t46", "soon"), today), ["Bug"]);
    }

    /// /code-review of PR 1399: a minute's read that started before his pick never puts the old value back; one started after
    /// it wins, and the pick is forgotten.
    #[test]
    fn a_read_started_before_his_pick_keeps_his_pick() {
        let _budget = TestBudget::new();
        let side = |epic: &str| dash_store::older::Sidebar {
            epic: Some(epic.into()),
            ..Default::default()
        };
        let read = |epic: &str| Sides {
            tasks: BTreeMap::from([("a".to_string(), side(epic))]),
            epics: vec![],
        };
        let mut queue = QueueRead {
            sides: Some(Arc::new(read("crm"))),
            ..Default::default()
        };
        queue.noted.insert("a".into(), 100);
        // started at 50, before his pick at 100: his pick stays, still noted
        let kept = kept_picks(read("old"), &mut queue, 50);
        assert_eq!(kept.tasks["a"].epic.as_deref(), Some("crm"));
        assert!(queue.noted.contains_key("a"));
        // started at 150, after it: the read holds it, and it is forgotten
        let after = kept_picks(read("rust-poc"), &mut queue, 150);
        assert_eq!(after.tasks["a"].epic.as_deref(), Some("rust-poc"));
        assert!(queue.noted.is_empty());
    }

    /// Task rust-issue-sidebar-menus, done-when 1: the cycle list is this week's and the next, each its Monday and its dates as
    /// the older app names them, a past cycle the task holds offered first; and « Remind me »'s days, as the older app's.
    #[test]
    fn the_cycle_list_is_this_week_s_and_the_next_and_remind_me_offers_its_days() {
        let _budget = TestBudget::new();
        let at = |day: &str| dash_core::days(day).expect("a day");
        // a Wednesday, a Sunday: the week runs Monday to Sunday
        for today in ["2026-11-11", "2026-11-15"] {
            assert_eq!(
                cycles(at(today), None),
                [
                    (
                        "2026-11-09".to_string(),
                        "Cycle Nov 9 - Nov 15 · Current".to_string()
                    ),
                    (
                        "2026-11-16".into(),
                        "Cycle Nov 16 - Nov 22 · Upcoming".into()
                    )
                ],
                "{today}"
            );
        }
        // across a month's end, and a past cycle it holds offered first
        let held = cycles(at("2026-11-30"), Some("2026-11-02"));
        assert_eq!(held[0], ("2026-11-02".into(), "Cycle Nov 2 - Nov 8".into()));
        assert_eq!(held[1].1, "Cycle Nov 30 - Dec 6 · Current");
        assert_eq!(
            cycles(at("2026-11-11"), Some("2026-11-16")).len(),
            2,
            "a cycle offered is not offered twice"
        );
        // a Wednesday: tomorrow, in two days, next Monday, in a month
        assert_eq!(
            remind_days(at("2026-11-11")),
            [
                (
                    "2026-11-12".to_string(),
                    "Tomorrow · Thu 12 Nov 2026".to_string()
                ),
                ("2026-11-13".into(), "In two days · Fri 13 Nov 2026".into()),
                ("2026-11-16".into(), "Next week · Mon 16 Nov 2026".into()),
                ("2026-12-11".into(), "In a month · Fri 11 Dec 2026".into())
            ]
        );
        // a Sunday: next week is tomorrow, offered once; January's 31st in a month is February's last day
        let sunday = remind_days(at("2026-11-15"));
        assert_eq!(sunday.len(), 3, "{sunday:?}");
        let in_a_month = |day: &str| remind_days(at(day)).last().expect("a month").0.clone();
        assert_eq!(in_a_month("2027-01-31"), "2027-02-28");
        assert_eq!(in_a_month("2026-12-15"), "2027-01-15");
    }

    /// Task rust-board-buttons-new-issue: Linear's due presets from today, a day two share offered once; Friday's « end of
    /// this week » is today, a weekend's the next Friday. And a task's place among the board's: Linear's « 42 / 2193 ».
    #[test]
    fn the_due_menu_offers_linear_s_days_and_a_task_its_place() {
        let _budget = TestBudget::new();
        let at = |day: &str| dash_core::days(day).expect("a day");
        let names = |day: &str| -> Vec<String> {
            due_days(at(day))
                .into_iter()
                .map(|(n, d)| format!("{n} {d}"))
                .collect()
        };
        // a Tuesday
        assert_eq!(
            names("2026-11-10"),
            [
                "Tomorrow 2026-11-11",
                "End of this week 2026-11-13",
                "In one week 2026-11-17",
                "In two weeks 2026-11-24"
            ]
        );
        // a Thursday: tomorrow is the end of the week, offered once
        assert_eq!(
            names("2026-11-12"),
            [
                "Tomorrow 2026-11-13",
                "In one week 2026-11-19",
                "In two weeks 2026-11-26"
            ]
        );
        // a Saturday: the next Friday
        assert_eq!(names("2026-11-14")[1], "End of this week 2026-11-20");
        let listed = |id: &str| Listed {
            id: id.into(),
            status: "todo",
            says: String::new(),
            other: None,
            placed: false,
        };
        let tasks = [listed("a"), listed("b"), listed("c")];
        assert_eq!(
            position(&tasks, "b"),
            Some(json!({"at": 2, "of": 3, "previous": "/tasks/a", "next": "/tasks/c"}))
        );
        assert_eq!(
            position(&tasks, "a").expect("first")["previous"],
            Value::Null
        );
        assert_eq!(position(&tasks, "z"), None);
    }

    /// Task rust-tasks-runs-text-off-html: what runs, the agents' tables, is the text form's alone; his page holds none of it.
    #[test]
    fn what_runs_is_for_agents_and_his_page_holds_none_of_it() {
        let _budget = TestBudget::new();
        let look = look_empty();
        let runs = "Tasks: /tasks\nCode map: /deps\n\nTask\tState\nBuilt from abc123.\n";
        let tree = list_tree(&look, TaskView::List, None, runs);
        let text = surface::text(&tree).expect("a text");
        assert!(
            text.contains("\nWhat runs\n") && text.ends_with("Built from abc123.\n"),
            "{text}"
        );
        let html = surface::markup(&surface::screen::page("t", &tree).expect("a page"));
        assert!(
            !html.contains("Code map: /deps")
                && !html.contains("Task\tState")
                && !html.contains("What runs"),
            "{html}"
        );
    }
}

/// `dash land <pr>`: wait for its checks, merge at the checked head, close its task. The steps are taken as closures so a
/// test runs them with stand-ins.
#[allow(clippy::too_many_arguments)]
pub fn land<W, M, C, D, E, S>(
    pr: &str,
    title: &str,
    mut wait: W,
    mut merge: M,
    mut close: C,
    mut changed: D,
    mut deploy: E,
    mut say: S,
) -> Result<(), (i32, String)>
where
    W: FnMut(&str) -> (i32, String, String),
    M: FnMut(&str) -> Result<(), String>,
    C: FnMut(&str, &str, &str) -> Result<String, String>,
    D: FnMut(&str) -> bool,
    E: FnMut() -> Result<String, String>,
    S: FnMut(&str),
{
    say(&format!("land: waiting for checks on {pr}"));
    let (code, stdout, stderr) = wait(pr);
    if code == 1 {
        return Err((1, format!("land: checks red: {}", stderr.trim())));
    }
    if code != 0 {
        let why = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err((code, format!("land: no verdict: {why}")));
    }
    say(&format!("land: checks green on {pr}"));
    merge(pr).map_err(|e| (1, format!("land: merge failed: {e}")))?;
    say(&format!("land: merged {pr}"));
    match task_id_from_title(title) {
        Some(id) => {
            // the merge is done: a close that fails (a title naming no task in the store) is said, never the deploy's end
            match close(&id, pr, title) {
                Ok(words) => say(&format!("land: closed {id}{words}")),
                Err(e) => say(&format!("land: {id} not closed: {}", e.trim())),
            }
        }
        None => say("land: no task id in title, not closing"),
    }
    if changed(pr) {
        // the forge's deploy lock: a deploy already running is waited for, and said so
        say("land: deploying (rust/ changed), after any deploy already running");
        let serves = deploy().map_err(|e| (1, format!("land: deploy failed: {e}")))?;
        say(&format!("land: deployed: {serves}"));
    } else {
        say("land: no rust/ change, not deploying");
    }
    Ok(())
}

/// `dash_orch::landed` on the live store: the pane a run of Rust's opened for `id` stopped, its build output gone.
fn close_landed(dir: &Path, id: &str) -> Result<String, String> {
    let report = reporting(dir, "land");
    let step = |name: &str, took: Duration| report.step_took(name, took);
    let tools = tools(dir);
    let mut tasks = open_tasks(dir)?;
    // its kind's stop words; a kind not read is told Claude's
    let (agent, launcher) = agent_of(&tools, id).unwrap_or((&tools.agents.claude, &tools.launcher));
    dash_orch::landed(
        id,
        &mut ports(&mut tasks, &tools, agent, launcher, &step),
        PACE,
    )
}

fn task_id_from_title(title: &str) -> Option<String> {
    let after_first = title.split_once(']')?.1;
    let open = after_first.find('[')?;
    let close = after_first[open..].find(']')? + open;
    let id = &after_first[open + 1..close];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

pub fn land_pr(pr: &str, mut out: std::fs::File) {
    use std::io::Write;
    let title = dash_process::run(
        "gh",
        &["pr", "view", pr, "--json", "title", "-q", ".title"],
        None,
        std::time::Duration::from_secs(60),
    )
    .unwrap_or_default();
    let head = dash_process::run(
        "gh",
        &[
            "pr",
            "view",
            pr,
            "--json",
            "headRefOid",
            "-q",
            ".headRefOid",
        ],
        None,
        std::time::Duration::from_secs(60),
    )
    .unwrap_or_default();
    let head = head.trim().to_string();
    let wait = |target: &str| {
        let hook = repo().join(".claude/hooks/merge-claim.py");
        if !hook.is_file() {
            return (
                3,
                String::new(),
                format!("ci wait: no merge hook at {}", hook.display()),
            );
        }
        let every = std::env::var("DASH_CI_POLL_MS")
            .ok()
            .and_then(|ms| ms.parse().ok());
        let every = std::time::Duration::from_millis(every.unwrap_or(60_000));
        let read = || dash_forge::hook_rows(&hook, &repo(), target);
        let listens = || {
            std::thread::sleep(every);
            true
        };
        dash_forge::ci_wait(
            read,
            listens,
            std::time::Instant::now() + dash_forge::CI_DEADLINE,
        )
    };
    let merge = move |target: &str| {
        let args = [
            "pr",
            "merge",
            target,
            "--squash",
            "--match-head-commit",
            head.as_str(),
        ];
        dash_process::run("gh", &args, None, std::time::Duration::from_secs(60)).map(|_| ())
    };
    let close = |task_id: &str, pr: &str, title: &str| {
        let bin = older_app().bin.clone();
        let args = ["task", "done", task_id, pr, "merged", title];
        dash_process::run(&bin, &args, None, std::time::Duration::from_secs(60))?;
        // once the list closed it, as a run closes its own on a merge (task rust-clean-closed-builds): a stop or clean that fails
        // leaves it closed, said so (/code-review of ed8d13f30)
        Ok(match close_landed(&live_data(), task_id) {
            Ok(words) => format!(": {words}"),
            Err(e) => format!(", but {e}"),
        })
    };
    let changed = |target: &str| {
        let files = dash_process::run(
            "gh",
            &[
                "pr",
                "view",
                target,
                "--json",
                "files",
                "-q",
                ".files[].path",
            ],
            None,
            std::time::Duration::from_secs(60),
        )
        .unwrap_or_default();
        files.lines().any(|path| path.starts_with("rust/"))
    };
    // the forge's deploy, as the orchestrator's merges: it restarts the look on the new binary, which make rust/deploy alone never does
    let deploy = || github().deploy();
    let say = |line: &str| {
        let _ = writeln!(out, "{line}");
    };
    let mut tasks = open_tasks(&live_data());
    let tasks = tasks
        .as_mut()
        .map(|t| t as &mut dyn TaskStore)
        .map_err(|e| e.clone());
    let landed = land_held(
        tasks,
        pr,
        || crate::parity::land_check(&repo(), pr_number(pr)?),
        || land(pr, title.trim(), wait, merge, close, changed, deploy, say),
    );
    if let Err((code, msg)) = landed {
        eprintln!("{msg}");
        std::process::exit(code);
    }
}

#[cfg(test)]
mod land_tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    #[test]
    fn green_then_merge_then_close() {
        let _budget = TestBudget::new();
        let mut merged = false;
        let mut closed = false;
        let mut said = Vec::new();
        let mut deployed = false;
        land(
            "7",
            "[feat][task-1] add land",
            |_| (0, String::new(), String::new()),
            |_| {
                merged = true;
                Ok(())
            },
            |id, _, _| {
                assert_eq!(id, "task-1");
                closed = true;
                Ok(": no agent of its run left in pane p1, its build output gone".into())
            },
            |_| true,
            || {
                deployed = true;
                Ok("abc serves".into())
            },
            |line| said.push(line.to_string()),
        )
        .expect("green");
        assert!(merged && closed);
        assert!(deployed);
        assert!(said.iter().any(|l| l.contains("merged")));
        assert!(
            said.iter()
                .any(|l| l
                    == "land: closed task-1: no agent of its run left in pane p1, its build output gone"),
            "{said:?}"
        );
        // what serves is said, so a look that kept its old binary (the forge's error) is never a quiet « deployed »
        assert!(
            said.iter().any(|l| l == "land: deployed: abc serves"),
            "{said:?}"
        );
    }

    /// Task rust-port-parity, done-when 2: a PR deleting an Elixir route with an open gap (parity::tests holds each case) is
    /// refused before any hold or wait, so the run porting its gap can still take it; nothing is landed.
    #[test]
    fn a_parity_gap_refuses_the_land_before_its_hold() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("land-parity");
        let mut tasks = store::open(&dir.0, &dir.0).expect("open");
        let refused = land_held(
            Ok(&mut tasks),
            "9",
            || Err("/mail: control\tArchive".into()),
            || panic!("no land with a gap open"),
        );
        assert_eq!(
            refused,
            Err((1, "land: refused: /mail: control\tArchive".into()))
        );
        assert!(TaskStore::holds(&tasks).expect("read").is_empty());
    }

    /// PR 1334, 2026-11-10: its title named dash-land, no task in the store; the close failed and the land stopped before the
    /// deploy. A close that fails is said, and the deploy still runs.
    #[test]
    fn a_close_that_fails_is_said_and_the_deploy_still_runs() {
        let _budget = TestBudget::new();
        let (mut said, mut deployed) = (Vec::new(), false);
        land(
            "8",
            "[feat][no-such-task] x",
            |_| (0, String::new(), String::new()),
            |_| Ok(()),
            |_, _, _| Err("task done: no task no-such-task".into()),
            |_| true,
            || {
                deployed = true;
                Ok("abc serves".into())
            },
            |line| said.push(line.to_string()),
        )
        .expect("landed");
        assert!(deployed, "{said:?}");
        assert!(
            said.iter()
                .any(|l| l.starts_with("land: no-such-task not closed: ")),
            "{said:?}"
        );
    }

    /// Task rust-pr-one-owner, done-when 1: `dash land` takes the PR for the orchestrator, refused while a run at work holds
    /// it; a merge ends the hold, a red land keeps it.
    #[test]
    fn dash_land_takes_the_pr_and_its_merge_ends_the_hold() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("land-holds");
        let mut tasks = store::open(&dir.0, &dir.0).expect("open");
        let held = std::cell::RefCell::new(vec![]);
        let landed = land_held(
            Ok(&mut tasks),
            "https://github.com/o/r/pull/7/files",
            || Ok(()),
            || {
                held.borrow_mut().push("landing");
                Ok(())
            },
        );
        assert_eq!(landed, Ok(()));
        assert_eq!(held.borrow().len(), 1);
        assert!(
            TaskStore::holds(&tasks).expect("read").is_empty(),
            "the merge ended it"
        );
        let red = land_held(
            Ok(&mut tasks),
            "8",
            || Ok(()),
            || Err((1, "land: checks red".into())),
        );
        assert_eq!(red, Err((1, "land: checks red".into())));
        let holds = TaskStore::holds(&tasks).expect("read");
        assert_eq!((holds[0].pr, holds[0].holder.as_str()), (8, ORCHESTRATOR));
        // a run at work on #9 keeps it
        tasks.claim("t1").expect("claim");
        TaskStore::hold_pr(&mut tasks, 9, "run t1", "its run works on it").expect("taken");
        let refused = land_held(
            Ok(&mut tasks),
            "#9",
            || Ok(()),
            || panic!("no land beside its run"),
        );
        assert_eq!(
            refused,
            Err((
                1,
                "land: refused, #9 is held by run t1: its run works on it".into()
            ))
        );
        assert_eq!(held_text(&tasks).lines().next(), Some("Held PRs:"));
        let text = held_text(&tasks);
        assert!(
            text.contains("#8\torchestrator, since ") && text.contains(": dash land\n"),
            "{text}"
        );
        assert!(text.contains("#9\trun t1, since "), "{text}");
        // never blocked by its bookkeeping: a store that does not open, or a branch name, lands with no hold
        let landed = land_held(Err("locked".into()), "7", || Ok(()), || Ok(()));
        assert_eq!(landed, Ok(()));
        assert_eq!(
            land_held(Ok(&mut tasks), "task/foo", || Ok(()), || Ok(())),
            Ok(())
        );
    }

    #[test]
    fn red_then_nothing_merged() {
        let _budget = TestBudget::new();
        let mut merged = false;
        let mut said = Vec::new();
        let err = land(
            "7",
            "[feat][task-1] add land",
            |_| (1, String::new(), "boom".into()),
            |_| {
                merged = true;
                Ok(())
            },
            |_, _, _| Ok(String::new()),
            |_| false,
            || Ok(String::new()),
            |line| said.push(line.to_string()),
        )
        .expect_err("red");
        assert_eq!(err.0, 1);
        assert!(!merged);
        assert!(said.iter().any(|l| l.contains("waiting")));
    }

    #[test]
    fn no_task_id_then_merge_but_no_close() {
        let _budget = TestBudget::new();
        let mut merged = false;
        let mut closed = false;
        let mut said = Vec::new();
        land(
            "7",
            "[feat] add land",
            |_| (0, String::new(), String::new()),
            |_| {
                merged = true;
                Ok(())
            },
            |_, _, _| {
                closed = true;
                Ok(String::new())
            },
            |_| false,
            || Ok(String::new()),
            |line| said.push(line.to_string()),
        )
        .expect("green");
        assert!(merged && !closed);
        assert!(said.iter().any(|l| l.contains("no task id")));
    }

    #[test]
    fn deploy_runs_only_when_rust_changed() {
        let _budget = TestBudget::new();
        for changed in [false, true] {
            let mut deployed = false;
            land(
                "7",
                "[feat][task-1] add land",
                |_| (0, String::new(), String::new()),
                |_| Ok(()),
                |_, _, _| Ok(String::new()),
                |_| changed,
                || {
                    deployed = true;
                    Ok("abc serves".into())
                },
                |_| {},
            )
            .expect("green");
            assert_eq!(deployed, changed);
        }
    }
}
