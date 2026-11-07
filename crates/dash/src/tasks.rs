//! The binary's wiring of section 20b's orchestrator: the store, the older app's list and judge, herdr, the approved
//! `make claude`, Claude Code, git and GitHub, on orch. One process per run: a run that crashes takes its claim's lock with it
//! and nothing else, and `dash orch` frees it on its next look.
use crate::reporting::{reporting, say_memory};
use dash_agent::ClaudeCode;
use dash_bridge::Dash;
use dash_core::tasks::{Agent, Claim, Limit, Outcome, Pick, Plan, Queue, Standing, TaskList, plan};
use dash_forge::GitHub;
use dash_launcher::ApprovedMake;
use dash_meter::cap::STOP_CODE;
use dash_orch::{Pace, Ports, next, run_task, seats_taken};
use dash_policy::Defaults;
use dash_store::tasks::{self as store, Row, SqliteTasks};
use dash_surface as surface;
use dash_terminal::Herdr;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
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
    agent: ClaudeCode,
    forge: GitHub,
    policy: Defaults,
}

fn home() -> String {
    or_exit(std::env::var("HOME").map_err(|e| format!("HOME: {e}")))
}

fn path(var: &str, default: &str) -> PathBuf {
    std::env::var(var).map_or_else(|_| Path::new(&home()).join(default), PathBuf::from)
}

fn repo() -> PathBuf {
    path("DASH_REPO", "repo/site_perso")
}

/// The folder the task worktrees sit in, and with them each claim's lock.
fn worktrees() -> PathBuf {
    path("DASH_WORKTREES", ".herdr/worktrees/site_perso")
}

fn open_tasks(dir: &Path) -> Result<SqliteTasks, String> {
    store::open(dir, &worktrees())
}

/// The data folder of the running tool, outside git: what `dash serve --port <n>` reads, and `make rust/serve` names.
pub fn live_data() -> PathBuf {
    repo().join("private/live/rust/data")
}

fn older_app() -> Dash {
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

fn tools() -> Tools {
    let home = home();
    let repo = repo();
    Tools {
        list: older_app(),
        terminal: Herdr::from_env(),
        launcher: ApprovedMake { home: home.clone() },
        agent: claude_code(&home),
        forge: GitHub {
            repo,
            root: worktrees(),
            base: "main".into(),
        },
        policy: Defaults,
    }
}

fn ports<'a>(
    tasks: &'a mut SqliteTasks,
    tools: &'a Tools,
    step: &'a dyn Fn(&str, Duration),
) -> Ports<'a> {
    Ports {
        tasks,
        list: &tools.list,
        terminal: &tools.terminal,
        launcher: &tools.launcher,
        agent: &tools.agent,
        forge: &tools.forge,
        policy: &tools.policy,
        step,
    }
}

/// The older app's queue, and when it was read (Unix milliseconds).
type QueueAt = (Result<Queue, String>, i64);

#[derive(Default)]
struct QueueRead {
    last: Option<QueueAt>,
    /// The sessions and the spend read with it, what a pick reads beside the queue.
    seats: Option<Result<(usize, Vec<Limit>), String>>,
    reading: bool,
}

/// What the page at / reads: this store's runs, and what the older app says Rust may pick next.
pub struct Board {
    tasks: SqliteTasks,
    queue: Arc<Mutex<QueueRead>>,
}

impl Board {
    pub fn open(dir: &Path) -> Result<Board, String> {
        let tasks = open_tasks(dir)?;
        let queue = Arc::default();
        Ok(Board { tasks, queue })
    }

    fn look(&self) -> Result<Look, String> {
        let rows = self.tasks.rows()?;
        let standing = dash_core::tasks::TaskStore::standing(&self.tasks)?;
        let now_ms = now_ms();
        let (queue, seats) = {
            let mut queue = self.queue.lock().expect("the queue");
            let stale = queue
                .last
                .as_ref()
                .is_none_or(|(_, at)| now_ms - at >= ORCH_POLL.as_millis() as i64);
            if stale && !queue.reading {
                let claims = dash_core::tasks::TaskStore::claims(&self.tasks)?;
                queue.reading = self.read_queue(claims);
            }
            (queue.last.clone(), queue.seats.clone())
        };
        let plan = match (&queue, seats) {
            (Some((Ok(queue), _)), Some(Ok((sessions, limits)))) => Some(Ok(plan(
                &Defaults,
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
            plan,
        })
    }

    pub fn page(&self) -> Result<String, String> {
        let look = self.look()?;
        let plan = match &look.plan {
            Some(Ok(plan)) => plan_text(plan),
            Some(Err(e)) => format!("{}\n\n", not_known(e)),
            None => String::new(),
        };
        Ok(page(
            &look.rows,
            look.queue.as_ref(),
            &plan,
            look.now_ms,
            option_env!("DASH_COMMIT"),
        ))
    }

    /// The page at /graph.
    pub fn graph(&self) -> Result<String, String> {
        surface::page("Rust tasks: the graph", &graph_tree(&self.look()?))
    }

    /// Only while someone reads the page, so a closed page costs the older app nothing. On its own thread: a read can take its
    /// 60 s limit, and the store's loop is serial. The next reload shows it; true when the thread started.
    fn read_queue(&self, claims: Vec<Claim>) -> bool {
        let shared = Arc::clone(&self.queue);
        let started = std::thread::Builder::new().spawn(move || {
            let read = older_app().queue();
            let answered = read.is_ok();
            // Shown at once: a slow terminal never holds back the older app's line.
            shared.lock().expect("the queue").last = Some((read, now_ms()));
            // No queue, no plan: nothing else is asked of the terminal.
            let seats = answered.then(|| {
                let sessions = seats_taken(&Herdr::from_env(), &claims)?;
                // No HOME is no spend known. `home()` would exit, and from this thread that ends the whole server.
                let home = std::env::var("HOME");
                let limits = home.map(|home| claude_code(&home).limits().unwrap_or_default());
                Ok((sessions, limits.unwrap_or_default()))
            });
            let mut queue = shared.lock().expect("the queue");
            queue.seats = seats;
            queue.reading = false;
        });
        started.is_ok()
    }
}

/// What both pages read, in one look. `plan`: `None` until the queue has answered, an error when the sessions did not.
struct Look {
    rows: Vec<Row>,
    standing: Vec<Standing>,
    now_ms: i64,
    queue: Option<QueueAt>,
    plan: Option<Result<Plan, String>>,
}

fn now_ms() -> i64 {
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

fn row_line(row: &Row, now_ms: i64) -> String {
    // claim() keeps the last attempt's outcome and words: a running task shows neither. A claim whose lock is free runs nowhere,
    // whatever its row says: a crash, or a reboot with no `dash orch` to free it.
    let running = row.status == "claimed";
    let said = if running {
        ""
    } else {
        row.said.as_deref().unwrap_or_default()
    };
    let first: String = said
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect();
    let outcome = if row.run_gone {
        claim_word(true)
    } else if running {
        waits_on(row.outcome.as_deref()).unwrap_or(claim_word(false))
    } else {
        row.outcome.as_deref().unwrap_or("none yet")
    };
    let pr = pr_in(said).unwrap_or_default();
    // No try, no run: what `add` wrote, or what a reopen forgot.
    let last = if row.attempts == 0 {
        "never".to_string()
    } else {
        age(now_ms - row.last_ms)
    };
    line(
        &row.id,
        &row.status,
        row.attempts,
        &last,
        outcome,
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

/// What runs next and why the others wait, as `plan` says it: the words the picker acts on.
fn plan_text(plan: &Plan) -> String {
    let next = next_line(plan);
    let waiting: String = plan
        .waiting
        .iter()
        .map(|held| format!("{}\t{}\n", one_line(&held.id), one_line(&held.why)))
        .collect();
    format!("{next}\n{waiting}\n")
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
    let next = match &look.plan {
        Some(Ok(plan)) => next_line(plan),
        Some(Err(e)) => not_known(e),
        None if queue_failed(look) => {
            "Next: not known, the older app's queue did not answer.".into()
        }
        None => "Next: not known yet: the next reload shows it.".into(),
    };
    let note = queue_note(look.queue.as_ref(), look.now_ms);
    let tasks = graph_tasks(look);
    let graph = match tasks.len() {
        0 if matches!(look.queue, Some((Ok(_), _))) => text("No Rust task open.".into()),
        0 => {
            text("This store has no task open, and the older app's queue has not answered.".into())
        }
        n if n > surface::GRAPH_LIMIT => text(format!(
            "{n} tasks: past the {} a graph draws. /, the list, shows them.",
            surface::GRAPH_LIMIT
        )),
        _ => json!({"type": "task-graph", "props": {"tasks": tasks}}),
    };
    let mut elements = Map::new();
    let heading = json!({"type": "heading", "props": {"text": "Rust tasks"}});
    elements.insert("title".into(), heading);
    elements.insert("next".into(), text(next));
    elements.insert("note".into(), text(note));
    elements.insert("graph".into(), graph);
    let mut children: Vec<String> = ["title", "next", "note", "graph"].map(String::from).into();
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

/// Plain text: the drawn screen is `graph_tree`'s. `make rust/deploy` matches the whole « Built from » line.
fn page(
    rows: &[Row],
    queue: Option<&QueueAt>,
    plan: &str,
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
    let note = queue_note(queue, now_ms);
    if table.is_empty() {
        table = "No Rust task yet.\n".into();
    }
    let commit = commit.unwrap_or("a local build");
    format!(
        "Rust tasks\nGraph: /graph\nDependencies: /deps\n{note}\n\n{plan}Task\tState\tTries\tLast run\tOutcome\tPR\tWords\n{table}\nBuilt from {commit}.\n"
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
    let tools = tools();
    let mut tasks = or_exit(open_tasks(dir));
    let run = run_task(id, account, &mut ports(&mut tasks, &tools, &step), PACE);
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

/// Picks and starts, each run a process of its own. `once`: the first pick's run, its exit this one's, nothing else started.
pub fn orchestrate(dir: &Path, once: bool, began: Instant, mut out: File) {
    let report = reporting(dir, "orch");
    let step = |name: &str, took: Duration| report.step_took(name, took);
    let tools = tools();
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
        let look = report.step("pass", || next(&mut ports(&mut tasks, &tools, &step)));
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
    use dash_core::tasks::Queued;
    use dash_meter::memory::TestBudget;

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
        assert!(text.contains("Dependencies: /deps\n"), "{text}");
        assert!(text.ends_with("\nBuilt from abc123.\n"), "{text}");
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
        let failed = page(
            &[],
            Some(&(Err("down\nBuilt from x.".into()), 0)),
            "",
            0,
            Some("y"),
        );
        assert!(failed.contains("0 s ago: down Built from x.\n"), "{failed}");
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
        let text = page(&[row("")], Some(&(queue, 1_000)), "", 121_000, None);
        assert!(text.contains("queue, read 2 min ago."), "{text}");
        assert_eq!(text.matches("a-task\t").count(), 1, "{text}");
        assert!(
            text.contains("next\tqueued\t0\tnever\t\t\tits words\n"),
            "{text}"
        );
        let failed = page(&[], Some(&(Err("down now".into()), 0)), "", 0, None);
        assert!(
            failed.contains("did not answer, 0 s ago: down now"),
            "{failed}"
        );
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
            plan: Some(Ok(plan)),
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
            week: 10.0,
            week_resets: 1,
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
            plan: None,
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
            plan: None,
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
