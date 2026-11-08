//! The binary's wiring of section 20b's orchestrator: the store, the older app's list and judge, herdr, the approved
//! `make claude`, Claude Code, git and GitHub, on orch. One process per run: a run that crashes takes its claim's lock with it
//! and nothing else, and `dash orch` frees it on its next look.
use crate::kinds::{Agents, ByKind};
use crate::reporting::{reporting, say_memory};
use dash_agent::ClaudeCode;
use dash_agent::codex::Codex;
use dash_bridge::{Dash, Issue, Open};
use dash_core::tasks::{
    Agent, Claim, Launcher, Limit, Outcome, Pick, Plan, Queue, Standing, TaskList, Terminal, plan,
    waits_unmet,
};
use dash_forge::GitHub;
use dash_launcher::{ApprovedCodex, ApprovedMake, mxc};
use dash_meter::cap::STOP_CODE;
use dash_orch::{Pace, Ports, next, run_task, seats_taken};
use dash_store::tasks::{self as store, AgentRow, Row, SqliteTasks};
use dash_surface::{self as surface, TaskView};
use dash_terminal::Herdr;
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

fn open_tasks(dir: &Path) -> Result<SqliteTasks, String> {
    store::open(dir, &worktrees())
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

/// The older app's queue, and when it was read (Unix milliseconds).
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

/// What waits on him (`dash handoff`) and today's daily (`dash progress`), as the older app prints them.
type HomeRead = (Result<String, String>, Result<String, String>);

/// A read apart from the queue's, started only by the page that shows it: Home's two reads, the board's every task. A /tasks
/// tab or the window costs the older app no Home read, Home no `task ls`, and no read waits on another's (/code-review of
/// rust-nav-board and of rust-nav-polish).
struct Polled<T> {
    last: Option<T>,
    at: i64,
    reading: bool,
}

impl<T> Default for Polled<T> {
    fn default() -> Self {
        Polled { last: None, at: 0, reading: false }
    }
}

/// Every project's open tasks, as `dash task ls` lists them (task rust-nav-polish).
type EveryRead = Result<Vec<Open>, String>;

/// A read that failed keeps the last words read: one slow answer never blanks what waits on him.
fn kept(new: Result<String, String>, old: Option<&Result<String, String>>) -> Result<String, String> {
    match (new, old) {
        (Err(_), Some(Ok(said))) => Ok(said.clone()),
        (new, _) => new,
    }
}

type Seats = (usize, Vec<Limit>, HashSet<String>);

/// What the page at / reads: this store's runs, and what the older app says Rust may pick next.
pub struct Board {
    tasks: SqliteTasks,
    queue: Arc<Mutex<QueueRead>>,
    home: Arc<Mutex<Polled<HomeRead>>>,
    every: Arc<Mutex<Polled<EveryRead>>>,
    data: PathBuf,
}

impl Board {
    pub fn open(dir: &Path) -> Result<Board, String> {
        let tasks = open_tasks(dir)?;
        let (queue, home, every) = (Arc::default(), Arc::default(), Arc::default());
        let data = dir.to_path_buf();
        Ok(Board {
            tasks,
            queue,
            home,
            every,
            data,
        })
    }

    fn look(&self) -> Result<Look, String> {
        let rows = self.tasks.rows()?;
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
            standing,
            now_ms,
            queue,
            issues,
            plan,
            home: None,
            every: None,
        })
    }

    /// The list as its UI tree, the runs below it: what dash ui's window draws, at /tasks?view=tree. No views: the window
    /// follows no link.
    pub fn tree(&self) -> Result<String, String> {
        let look = self.look()?;
        Ok(list_tree(&look, None, None, Some(&self.runs(&look))))
    }

    /// /tasks as the board or the list (task rust-nav-board): the Dioxus page for a browser, else the same tree's text
    /// (rust/CLAUDE.md rule 4). Every project's open tasks, or `project`'s alone. The graph is `graph`'s.
    pub fn tasks(&self, view: TaskView, project: Option<&str>, html: bool) -> Result<String, String> {
        let mut look = self.look()?;
        look.every = self.read_every();
        let tree = list_tree(&look, Some(view), project, None);
        if html {
            surface::screen::page("Tasks", &tree)
        } else {
            surface::screen::text(&tree)
        }
    }

    /// The board's dependency view, at /tasks?layout=graph and /graph.
    pub fn graph(&self) -> Result<String, String> {
        surface::page("Rust tasks: the graph", &graph_tree(&self.look()?))
    }

    /// Home at /: what waits on him, today's daily, what runs; HTML for a browser, else the tree's text.
    pub fn home(&self, html: bool) -> Result<String, String> {
        let mut look = self.look()?;
        look.home = self.read_home();
        let tree = home_tree(&look, &self.runs(&look));
        if html {
            surface::screen::page("Home", &tree)
        } else {
            surface::screen::text(&tree)
        }
    }

    /// The page at /tasks/<id>; `None`: no open task of that id.
    pub fn task(&self, id: &str) -> Result<Option<String>, String> {
        let look = self.look()?;
        let page = |tree: String| surface::page(&format!("Rust task: {id}"), &tree);
        task_tree(&look, id).map(page).transpose()
    }

    /// Only while someone reads the page, so a closed page costs the older app nothing. On its own thread: a read can take its
    /// 60 s limit, and the store's loop is serial. The next reload shows it; true when the thread started.
    fn read_queue(&self, claims: Vec<Claim>) -> bool {
        let shared = Arc::clone(&self.queue);
        let data = self.data.clone();
        let started = std::thread::Builder::new().spawn(move || {
            let read = older_app().queue();
            let answered = read.is_ok();
            let ids: Vec<String> = read
                .iter()
                .flat_map(|q| &q.tasks)
                .map(|t| t.id.clone())
                .collect();
            // Shown at once: a slow terminal never holds back the older app's line.
            shared.lock().expect("the queue").last = Some((read, now_ms()));
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

    /// Home's last reads, a new pair started on its own thread once a minute while Home is read; the two at once.
    fn read_home(&self) -> Option<HomeRead> {
        let mut state = self.home.lock().expect("home");
        let stale = state.last.is_none() || now_ms() - state.at >= ORCH_POLL.as_millis() as i64;
        if stale && !state.reading {
            let shared = Arc::clone(&self.home);
            let started = std::thread::Builder::new().spawn(move || {
                let (waits, daily) = std::thread::scope(|s| {
                    let waits = s.spawn(|| older_app().handoff());
                    let daily = older_app().progress();
                    (waits.join().unwrap_or_else(|_| Err("its read panicked".into())), daily)
                });
                let mut state = shared.lock().expect("home");
                let old = state.last.take();
                let old = old.as_ref().map(|(w, d)| (w, d));
                state.last = Some((kept(waits, old.map(|o| o.0)), kept(daily, old.map(|o| o.1))));
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

/// What both pages read, in one look. `plan`: `None` until the queue has answered, an error when the sessions did not.
struct Look {
    rows: Vec<Row>,
    standing: Vec<Standing>,
    now_ms: i64,
    queue: Option<QueueAt>,
    /// `None` until the older app has answered.
    issues: Option<Result<Vec<Issue>, String>>,
    plan: Option<Result<Plan, String>>,
    /// `None` until the older app has answered Home's reads.
    home: Option<HomeRead>,
    /// `None` until the older app has listed every project's open tasks.
    every: Option<Result<Vec<Open>, String>>,
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

/// The screen at /graph as a UI tree: what runs next, how old that is, the graph, then each waiting task's whole why, which a node
/// may clip. With no graph to draw it says why in its place, and the rest stands.
fn graph_tree(look: &Look) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let next = next_words(look);
    let note = queue_note(look.queue.as_ref(), look.now_ms);
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
    elements.insert("next".into(), text(next));
    elements.insert("note".into(), text(note));
    elements.insert("graph".into(), graph);
    let mut children: Vec<String> = ["title", "views", "next", "note", "graph"].map(String::from).into();
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

fn next_words(look: &Look) -> String {
    match &look.plan {
        Some(Ok(plan)) => next_line(plan),
        Some(Err(e)) => not_known(e),
        None if queue_failed(look) => {
            "Next: not known, the older app's queue did not answer.".into()
        }
        None => "Next: not known yet: the next reload shows it.".into(),
    }
}

fn none_open(look: &Look) -> &'static str {
    match look.queue {
        Some((Ok(_), _)) => "No Rust task open.",
        _ => "This store has no task open, and the older app's queue has not answered.",
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
    let offered = |id: &str| {
        let mut tasks = queue.iter().flat_map(|q| &q.tasks);
        tasks.any(|t| one_line(&t.id) == id)
    };
    let standing = |id: &str| (look.standing.iter()).find(|r| one_line(&r.id) == id);
    let unmet = |id: &str| standing(id).and_then(|r| waits_unmet(&look.standing, &r.waits));
    let mut all = vec![];
    for task in graph_tasks(look) {
        let word = |field: &str| task[field].as_str().unwrap_or_default().to_string();
        let (id, state) = (word("id"), word("state"));
        let status = match state.as_str() {
            // Named by an open task, its own run over: the past stays out, unless the older app holds it open, below.
            "finished" | "ended" => continue,
            "running" => "in-progress",
            "next" => "todo",
            // A gone run is still In Progress until `dash orch` frees it: « run gone » says so, as the older board's « dead ».
            _ if standing(&id).is_some_and(|r| r.status == "claimed") => "in-progress",
            _ if offered(&id) && unmet(&id).is_none() => "todo",
            _ => "backlog",
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
            None => ("backlog", "the older app's queue does not offer it".into()),
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

/// Why a task's words are missing, when they are.
fn no_words(look: &Look) -> String {
    match &look.issues {
        None => "Not read yet: the next reload shows it.".into(),
        // A bad answer's error quotes all of it: a line here, the whole in the log of whoever runs the command.
        Some(Err(e)) => {
            let e: String = one_line(e).chars().take(200).collect();
            format!("The older app did not answer: {e}")
        }
        Some(Ok(_)) => "The older app holds no such task.".into(),
    }
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
/// beside the list (done-when 3 of rust-linear-view). `view`: `None` for the window's list of Rust's tasks, which names no
/// views; `project`: the one project /tasks is kept to; `runs`: the window's runs quote, last.
fn list_tree(
    look: &Look,
    view: Option<TaskView>,
    project: Option<&str>,
    runs: Option<&str>,
) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut elements = Map::new();
    let title = match (view, project) {
        (None, _) => "Rust tasks".to_string(),
        (Some(_), None) => "Tasks".into(),
        (Some(_), Some(p)) => format!("Tasks: {}", one_line(p)),
    };
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": title}}),
    );
    let mut children = vec!["title".to_string()];
    if let Some(view) = view {
        elements.insert("views".into(), views(view, project));
        children.push("views".into());
    }
    // what Rust runs next is site_perso's: another project's board has no such line
    if project.is_none_or(|p| p == "site_perso") {
        elements.insert("next".into(), text(next_words(look)));
        let note = queue_note(look.queue.as_ref(), look.now_ms);
        elements.insert("note".into(), text(note));
        children.extend(["next", "note"].map(String::from));
        if let Some(Err(_)) = &look.issues {
            elements.insert("words".into(), text(no_words(look)));
            children.push("words".into());
        }
    }
    let others = match (view, &look.every) {
        (None, _) | (_, Some(Ok(_))) => None,
        (Some(_), None) => Some("The other projects' tasks are being read: the next reload shows them.".to_string()),
        (Some(_), Some(Err(e))) => {
            let e: String = one_line(e).chars().take(200).collect();
            Some(format!("The other projects' tasks are not shown: the older app did not answer: {e}"))
        }
    };
    if let Some(others) = others {
        elements.insert("every".into(), text(others));
        children.push("every".into());
    }
    let tasks = match view {
        None => listed(look),
        Some(_) => every_listed(look, project),
    };
    let board = view == Some(TaskView::Board);
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
            let row = json!({"type": "issue-row", "props": {
                "id": task.id, "title": title, "status": status, "says": task.says, "comments": comments, "project": project,
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
    if let Some(runs) = runs {
        let quote = json!({"type": "quote", "props": {"text": runs.trim_end()}});
        elements.insert("runs".into(), quote);
        children.push("runs".into());
    }
    let page = json!({"type": "stack", "children": children});
    elements.insert("page".into(), page);
    json!({"root": "page", "elements": elements}).to_string()
}

/// One of Home's reads as cards under `part`, its parts' headings sections of their own; else why it is missing.
fn home_part(
    elements: &mut Map<String, Value>,
    part: &str,
    read: Option<&Result<String, String>>,
    link: fn(&str, Option<&str>) -> Option<String>,
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
            return vec![put(
                format!("{part}-text"),
                text("Being read: the next reload shows it.".into()),
            )];
        }
    };
    let read = crate::home::cards(said, link);
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
        let cards = put(
            format!("{part}-{g}"),
            json!({"type": "cards", "children": cards}),
        );
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

/// The screen at / as a UI tree (task rust-nav-board; @nha, 2026-11-08: « initial contents should be though about »): what
/// needs him first, then what runs. Each read is cards (title, one line, link), never its Markdown (task rust-nav-polish).
/// `runs`: the runs table and what stands above it, last, so its build line ends the page.
fn home_tree(look: &Look, runs: &str) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let (waits, daily) = match &look.home {
        Some((waits, daily)) => (Some(waits), Some(daily)),
        None => (None, None),
    };
    let mut elements = Map::new();
    let waits = home_part(&mut elements, "waits", waits, crate::home::handoff_link);
    let daily = home_part(&mut elements, "daily", daily, crate::home::progress_link);
    let more = json!({
        "page": {"type": "stack", "children": ["title", "waits", "daily", "runs"]},
        "title": {"type": "heading", "props": {"text": "Home"}},
        "waits": {"type": "section", "props": {"title": "Waiting on you"}, "children": waits},
        "daily": {"type": "section", "props": {"title": "Today's daily"}, "children": daily},
        "runs": {"type": "section", "props": {"title": "What runs"}, "children": ["next", "note", "runs-text"]},
        "next": text(next_words(look)),
        "note": text(queue_note(look.queue.as_ref(), look.now_ms)),
        "runs-text": {"type": "quote", "props": {"text": runs.trim_end()}},
    });
    elements.extend(more.as_object().expect("an object").clone());
    json!({"root": "page", "elements": elements}).to_string()
}

/// The screen at /tasks/<id> as a UI tree: its title, status, why, checks and last run, then his comments on it. `None`: the older
/// app's list has answered and no open task reads as `id`. Before it has, a task may still be one: the page says its words are not
/// known, never that it does not exist.
fn task_tree(look: &Look, id: &str) -> Option<String> {
    let tasks = listed(look);
    let unknown = Listed {
        id: id.to_string(),
        status: "backlog",
        says: no_words(look),
        other: None,
    };
    let task = match tasks.iter().find(|t| t.id == id) {
        Some(task) => task,
        None if matches!(look.issues, Some(Ok(_))) => return None,
        None => &unknown,
    };
    let issue = issue_of(look, id);
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let quote = |t: String| json!({"type": "quote", "props": {"text": t}});
    let mut elements = Map::new();
    let mut add = |key: String, element: Value| {
        elements.insert(key.clone(), element);
        key
    };
    let mut part = |key: &str, title: String, parts: Vec<(String, Value)>| {
        let names: Vec<String> = parts.into_iter().map(|(k, e)| add(k, e)).collect();
        add(
            key.into(),
            json!({"type": "section", "props": {"title": title}, "children": names}),
        )
    };

    let why = issue.map(|i| lines_kept(&i.why)).filter(|w| !w.is_empty());
    let why = part(
        "why",
        "Why".into(),
        vec![(
            "why-text".into(),
            text(why.unwrap_or_else(|| no_words(look))),
        )],
    );

    let parts = issue.iter().flat_map(|i| &i.checks);
    let judged = parts.clone().filter(|c| c.met.is_some()).count();
    let met = parts.clone().filter(|c| c.met == Some(true)).count();
    let total = parts.clone().count();
    let title = if judged > 0 {
        format!("Checks: {met} of {total} met")
    } else {
        "Checks".to_string()
    };
    let mut checks: Vec<(String, Value)> = parts
        .enumerate()
        .map(|(i, c)| {
            let props = json!({"text": one_line(&c.text), "met": c.met});
            let check = json!({"type": "check", "props": props});
            (format!("check-{i}"), check)
        })
        .collect();
    if checks.is_empty() {
        let none = issue.map_or_else(|| no_words(look), |_| "None written.".into());
        checks.push(("no-checks".into(), text(none)));
    }
    let checks = part("checks", title, checks);

    let mut run: Vec<(String, Value)> = vec![];
    match row_of(look, id).filter(|r| r.attempts > 0) {
        None => run.push(("run-line".into(), text("Never run.".into()))),
        Some(row) => {
            let last = last_run(row, look.now_ms);
            let line = format!("Try {}, {}: {}.", row.attempts, last.when, last.outcome);
            run.push(("run-line".into(), text(one_line(&line))));
            if let Some(pr) = pr_in(last.said) {
                let link = json!({"type": "pull-request", "props": {"url": pr}});
                run.push(("run-pr".into(), link));
            }
            if !last.said.is_empty() {
                run.push(("run-said".into(), quote(lines_kept(last.said))));
            }
        }
    }
    let run = part("run", "Last run".into(), run);

    let said = issue.iter().flat_map(|i| &i.comments).enumerate();
    let comments: Vec<(String, Value)> = said
        .map(|(i, c)| (format!("comment-{i}"), quote(lines_kept(c))))
        .collect();
    let comments =
        (!comments.is_empty()).then(|| part("comments", "Your comments".into(), comments));

    let bar = add(
        "bar".into(),
        json!({"type": "issue-bar", "props": {"id": task.id}}),
    );
    let heading = add(
        "title".into(),
        json!({"type": "heading", "props": {"text": title_of(look, id)}}),
    );
    let state = add(
        "state".into(),
        json!({"type": "issue-state", "props": {"status": task.status, "says": task.says}}),
    );
    let mut children = vec![bar, heading, state, why, checks, run];
    children.extend(comments);
    add(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    Some(json!({"root": "page", "elements": elements}).to_string())
}

fn queue_note(queue: Option<&QueueAt>, now_ms: i64) -> String {
    match queue {
        Some((Ok(_), at)) => format!("The older app's queue, read {}.", age(now_ms - at)),
        Some((Err(e), at)) => format!(
            "The older app's queue did not answer, {}: {}",
            age(now_ms - at),
            one_line(e)
        ),
        None => "The older app's queue is being read: the next reload shows it.".into(),
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

/// Plain text, Home's last part: the drawn screen is `graph_tree`'s. `make rust/deploy` matches the whole « Built from » line.
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

fn names_in(waits: &str) -> Vec<String> {
    let names = waits.split(',').filter(|w| !w.is_empty());
    names.map(String::from).collect()
}

/// A write, then its row. A refusal is its line on stderr, with exit 1.
fn written(
    dir: &Path,
    id: &str,
    mut out: File,
    write: impl FnOnce(&mut SqliteTasks) -> Result<(), String>,
) {
    let mut tasks = or_exit(open_tasks(dir));
    or_exit(write(&mut tasks));
    let row = or_exit(tasks.show(id));
    writeln!(out, "{}", row.unwrap_or_default()).expect("stdout");
}

/// A new open task.
pub fn add(dir: &Path, id: &str, waits: &str, out: File) {
    written(dir, id, out, |tasks| tasks.add(id, &names_in(waits)));
}

/// The list a task waits on replaced, emptied by an empty word.
pub fn waits(dir: &Path, id: &str, waits: &str, out: File) {
    written(dir, id, out, |tasks| tasks.waits(id, &names_in(waits)));
}

/// An open task given up on, out from under the cap.
pub fn close(dir: &Path, id: &str, why: &str, out: File) {
    written(dir, id, out, |tasks| tasks.close(id, why));
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

/// Picks and starts, each run a process of its own. `once`: the first pick's run, its exit this one's, nothing else started.
pub fn orchestrate(dir: &Path, once: bool, began: Instant, mut out: File) {
    let report = reporting(dir, "orch");
    let step = |name: &str, took: Duration| report.step_took(name, took);
    let tools = tools(dir);
    let mut tasks = or_exit(open_tasks(dir));
    let mut first = true;
    let mut running: Vec<(String, Child)> = vec![];
    let mut said = String::new();
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
    use dash_core::tasks::Queued;
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
            }],
        });
        let text = page(&[], Some(&(queue, 0)), "", 0, None);
        assert!(text.contains("\nne xt [2J\tqueued\t"), "{text}");
        assert!(text.contains("\tits faked row [2J   \n"), "{text}");
        let down = (Err("down\nBuilt from x.".into()), 0);
        let failed = page(&[], Some(&down), "", 0, Some("y"));
        assert!(queue_note(Some(&down), 0).ends_with("0 s ago: down Built from x."));
        assert!(!failed.contains("\nBuilt from x.\n"), "{failed}");
        let forged = "\nBuilt from x.\n";
        let queue = Ok(Queue {
            seats: 1,
            tasks: vec![Queued {
                id: forged.into(),
                title: forged.into(),
                reopened_ms: None,
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
        };
        let queue = Ok(Queue {
            seats: 1,
            tasks: vec![queued("a-task"), queued("next")],
        });
        let at = (queue, 1_000);
        let text = page(&[row("")], Some(&at), "", 121_000, None);
        // the note is the list tree's line, never the runs' too
        assert!(queue_note(Some(&at), 121_000).contains("queue, read 2 min ago."));
        assert!(!text.contains("queue, read"), "{text}");
        assert_eq!(text.matches("a-task\t").count(), 1, "{text}");
        assert!(
            text.contains("next\tqueued\t0\tnever\t\t\tits words\n"),
            "{text}"
        );
        let down = (Err("down now".into()), 0);
        let failed = page(&[], Some(&down), "", 0, None);
        assert!(queue_note(Some(&down), 0).contains("did not answer, 0 s ago: down now"));
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
        }
    }

    fn look(standing: Vec<Standing>, queue: Queue, sessions: usize, limits: Vec<Limit>) -> Look {
        let plan = plan(&Defaults, queue.clone(), &standing, sessions, limits);
        Look {
            rows: vec![],
            standing,
            now_ms: 0,
            queue: Some((Ok(queue), 0)),
            issues: None,
            plan: Some(Ok(plan)),
            home: None,
            every: None,
        }
    }

    fn drawn(look: &Look) -> String {
        surface::page("t", &graph_tree(look)).expect("a page")
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
        let mut no_plan = Look {
            rows: vec![gone],
            standing: vec![
                stood("a-task", "claimed", None, &[]),
                stood("b", "open", None, &[]),
            ],
            now_ms: 0,
            queue: None,
            issues: None,
            plan: None,
            home: None,
            every: None,
        };
        let page = drawn(&no_plan);
        assert!(page.contains("queue is being read"), "{page}");
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
        assert!(page.contains("queue, read 9 h ago."), "{page}");
        known.plan = None;
        let page = drawn(&known);
        assert!(
            page.contains("Next: not known yet: the next reload"),
            "{page}"
        );
        assert!(node(&page, "a").contains("not known yet"), "{page}");
    }

    #[test]
    fn a_queue_that_failed_promises_no_reload_and_never_says_nothing_is_open() {
        let _budget = TestBudget::new();
        let failed = |standing: Vec<Standing>, queue: Option<QueueAt>| Look {
            rows: vec![],
            standing,
            now_ms: 0,
            queue,
            issues: None,
            plan: None,
            home: None,
            every: None,
        };
        let down = || Some((Err("down".to_string()), 0));
        let page = drawn(&failed(vec![stood("b", "open", None, &[])], down()));
        assert!(page.contains("queue did not answer."), "{page}");
        assert!(!page.contains("next reload"), "{page}");
        assert!(node(&page, "b").contains("not known"), "{page}");
        assert!(!node(&page, "b").contains("not known yet"), "{page}");
        for queue in [down(), None] {
            let page = drawn(&failed(vec![], queue));
            assert!(!page.contains("No Rust task open."), "{page}");
            assert!(page.contains("has not answered"), "{page}");
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
        surface::screen::page("t", &list_tree(look, Some(TaskView::List), None, None))
            .expect("a page")
    }

    /// The words dash ui's window shows of the same tree, through the same component.
    fn list_window(look: &Look) -> String {
        let props = surface::screen::ScreenProps {
            tree: list_tree(look, None, None, None),
            theme: "app light",
        };
        dash_ui::shown(dioxus::prelude::VirtualDom::new_with_props(
            surface::screen::Screen,
            props,
        ))
    }

    fn task_page(look: &Look, id: &str) -> String {
        let tree = task_tree(look, id).unwrap_or_else(|| panic!("no task {id}"));
        surface::page("t", &tree).expect("a page")
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
        assert!(page.contains(r#"class="count">2"#), "{page}");
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
        assert!(
            window.starts_with("Rust tasks\nNext: third"),
            "the window names no views it cannot follow: {window}"
        );
        let tree = list_tree(&look, None, None, Some("runs\nBuilt from y."));
        let alone = surface::screen::text(&tree).expect("a text");
        assert!(alone.ends_with("\nruns\nBuilt from y.\n"), "the window keeps its runs: {alone}");
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
        assert!(page.contains(r#"id="title">bookmarked"#), "{page}");
        assert!(page.contains("Not read yet"), "{page}");
        look.issues = Some(Err("down".into()));
        assert!(task_page(&look, "bookmarked").contains("did not answer: down"));
        look.issues = Some(Ok(vec![]));
        assert!(task_tree(&look, "bookmarked").is_none());
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
            page.contains(r#"id="title">Its &lt;odd&gt; name"#),
            "{page}"
        );
        assert!(page.contains(r#"data-status="todo""#), "{page}");
        assert!(page.contains("the next pick"), "{page}");
        assert!(
            page.contains("(@nha: « because »)\nA second line"),
            "{page}"
        );
        assert!(page.contains("Checks: 1 of 3 met"), "{page}");
        assert!(page.contains(r#"class="check met""#), "{page}");
        assert!(page.contains(r#"class="check unmet""#), "{page}");
        assert!(page.contains("A third"), "{page}");
        assert!(page.contains("Last run"), "{page}");
        assert!(page.contains("Try 2, 5 min ago: blocked."), "{page}");
        assert!(page.contains(r#">#12"#), "{page}");
        assert!(
            page.contains("needs: a &lt;tap&gt;\n"),
            "its lines kept: {page}"
        );
        assert!(task_tree(&look, "no-such").is_none());
    }

    #[test]
    fn a_task_that_never_ran_or_whose_words_are_not_read_says_so() {
        let _budget = TestBudget::new();
        let mut look = look(vec![], queued(&["a"], 5), 0, vec![]);
        if let Some((Ok(queue), _)) = &mut look.queue {
            queue.tasks[0].title = "From the queue".into();
        }
        let page = task_page(&look, "a");
        assert!(page.contains(r#"id="title">From the queue"#), "{page}");
        if let Some((Ok(queue), _)) = &mut look.queue {
            queue.tasks[0].title = String::new();
        }
        let page = task_page(&look, "a");
        assert!(page.contains("Never run."), "{page}");
        assert!(page.contains("Not read yet: the next reload"), "{page}");
        assert!(page.contains(r#"id="title">a"#), "{page}");
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

    /// Done-when 4 of rust-nav-board: Home reads what waits on him first, then his daily, then what runs, the build line last.
    #[test]
    fn home_shows_his_waits_then_his_daily_then_what_runs() {
        let _budget = TestBudget::new();
        let mut look = look_empty();
        let shown = |look: &Look| surface::screen::text(&home_tree(look, "runs\nBuilt from y.\n")).expect("a text");
        let reading = shown(&look);
        assert_eq!(reading.matches("Being read: the next reload shows it.").count(), 2, "{reading}");
        look.home = Some((Ok("## His three steps\n- Connect\u{1b}[2J Datadog\n".into()), Err("down\nBuilt from x.".into())));
        let home = shown(&look);
        assert!(home.starts_with("Home\n\nWaiting on you\n\nHis three steps\n- Connect [2J Datadog\n\nToday's daily\n"), "{home}");
        assert!(
            home.contains(
                "\nThe older app did not answer: down Built from x.\n\nWhat runs\nNext: "
            ),
            "{home}"
        );
        assert!(home.ends_with("\nruns\nBuilt from y.\n"), "{home}");
        look.home = Some((Ok("  \n".into()), Ok("Forward progress, today: review.".into())));
        let quiet = shown(&look);
        assert!(
            quiet.contains(
                "Waiting on you\nNothing.\n\nToday's daily\nForward progress, today: review\n"
            ),
            "{quiet}"
        );
        let html = surface::screen::page("Home", &home_tree(&look, "")).expect("a page");
        assert!(html.contains(r#"class="issue-part" id="waits""#) && html.contains(">Waiting on you<"), "{html}");
        // done-when 1 of rust-nav-polish: a step is a card, its whole a link, never the Markdown it was read from
        look.home = Some((Ok("## Drafts to review (1)\n- acme-intro -> Ann (Acme)\n".into()), Ok(String::new())));
        let html = surface::screen::page("Home", &home_tree(&look, "")).expect("a page");
        let card = r#"class="card" id="waits-0-0" href="/d/acme-intro">"#;
        assert!(html.contains(card) && html.contains(r#"class="title">Ann (Acme)"#), "{html}");
        assert!(!html.contains("## ") && !html.contains("- acme"), "{html}");
        let text = shown(&look);
        assert!(text.contains("\nDrafts to review (1)\n- Ann (Acme): acme-intro /d/acme-intro\n"), "{text}");
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
        let board = |look: &Look, project: Option<&str>| {
            list_tree(look, Some(TaskView::Board), project, None)
        };
        let shown =
            |project: Option<&str>| surface::screen::text(&board(&look, project)).expect("a text");
        let all = shown(None);
        assert!(
            all.starts_with("Tasks\nViews: Board /tasks (shown), "),
            "{all}"
        );
        assert!(
            all.contains("\nIn Review (1)\nnow-note\tnow-note title\tsite_perso\n"),
            "{all}"
        );
        assert!(
            all.contains("\nIn Progress (1)\n12\t12 title\tklar\n"),
            "{all}"
        );
        assert!(all.contains("\nBacklog (1)\n3\t3 title\tgraal\n"), "{all}");
        assert_eq!(
            all.matches("\na-task\t").count(),
            1,
            "Rust's own once: {all}"
        );
        let klar = shown(Some("klar"));
        assert!(klar.starts_with("Tasks: klar\nViews: Board /tasks?project=klar (shown), List /tasks?layout=list&project=klar, "), "{klar}");
        assert!(
            klar.contains("\nIn Progress (1)\n12\t12 title\tIn Progress\n")
                && !klar.contains("a-task")
                && !klar.contains("Next:"),
            "{klar}"
        );
        assert!(
            shown(Some("site_perso")).contains("a-task\t"),
            "Rust's own are site_perso's"
        );
        let html = surface::screen::page("t", &list_tree(&look, Some(TaskView::Board), None, None))
            .expect("a page");
        assert!(
            html.contains(r#"data-task="12" href="/projects/klar?issue=12""#),
            "another project's task opens on its board: {html}"
        );
        look.every = Some(Err("down".into()));
        let down = surface::screen::text(&board(&look, None)).expect("a text");
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
        assert_eq!(kept(Ok("1 thing".into()), Some(&said)), Ok("1 thing".into()));
        assert_eq!(kept(Err("down".into()), None), Err("down".into()));
        assert_eq!(kept(Err("down".into()), Some(&Err("before".into()))), Err("down".into()));
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
        assert!(opened.contains("Your comments"), "{opened}");
        assert!(opened.contains("fix &lt;this&gt;"), "{opened}");
        assert!(!task_page(&look, "quiet").contains("Your comments"));
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
