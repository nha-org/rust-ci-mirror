//! The binary's wiring of section 20b's orchestrator: the store, the older app's list and judge, herdr, the approved
//! `make claude`, Claude Code, git and GitHub, on orch. One process per run: a run that crashes takes its claim's lock with it
//! and nothing else, and `dash orch` frees it on its next look.
use crate::reporting::{reporting, say_memory};
use dash_agent::ClaudeCode;
use dash_bridge::Dash;
use dash_core::tasks::{Outcome, Pick, Queue, TaskList};
use dash_forge::GitHub;
use dash_launcher::ApprovedMake;
use dash_meter::cap::STOP_CODE;
use dash_orch::{Pace, Ports, next, run_task};
use dash_policy::Defaults;
use dash_store::tasks::{self as store, Row, SqliteTasks};
use dash_terminal::Herdr;
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

fn tools() -> Tools {
    let home = home();
    let repo = repo();
    let claude = Path::new(&home).join(".claude");
    Tools {
        list: older_app(),
        terminal: Herdr::from_env(),
        launcher: ApprovedMake { home: home.clone() },
        agent: ClaudeCode {
            projects: claude.join("projects"),
            limits: claude.join("weekly-limit"),
        },
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

    pub fn page(&self) -> Result<String, String> {
        let rows = self.tasks.rows()?;
        let now_ms = now_ms();
        let queue = {
            let mut queue = self.queue.lock().expect("the queue");
            let stale = queue
                .last
                .as_ref()
                .is_none_or(|(_, at)| now_ms - at >= ORCH_POLL.as_millis() as i64);
            if stale && !queue.reading {
                queue.reading = self.read_queue();
            }
            queue.last.clone()
        };
        Ok(page(
            &rows,
            queue.as_ref(),
            now_ms,
            option_env!("DASH_COMMIT"),
        ))
    }

    /// Only while someone reads the page, so a closed page costs the older app nothing. On its own thread: a read can take its
    /// 60 s limit, and the store's loop is serial. The next reload shows it; true when the thread started.
    fn read_queue(&self) -> bool {
        let shared = Arc::clone(&self.queue);
        let started = std::thread::Builder::new().spawn(move || {
            let read = older_app().queue();
            let mut queue = shared.lock().expect("the queue");
            queue.last = Some((read, now_ms()));
            queue.reading = false;
        });
        started.is_ok()
    }
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
        "run gone"
    } else if running {
        "running"
    } else {
        row.outcome.as_deref().unwrap_or("none yet")
    };
    let pr = pr_in(said).unwrap_or_default();
    let last = age(now_ms - row.last_ms);
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

/// Plain text: no screen is drawn until rust-ui-tree settles the rendering. `make rust/deploy` reads the « Built from » line.
fn page(rows: &[Row], queue: Option<&QueueAt>, now_ms: i64, commit: Option<&str>) -> String {
    let mut table: String = rows.iter().map(|r| row_line(r, now_ms)).collect();
    let note = match queue {
        Some((Ok(queue), at)) => {
            let waiting = queue
                .tasks
                .iter()
                .filter(|t| !rows.iter().any(|r| r.id == t.id));
            for task in waiting {
                table += &line(&task.id, "queued", 0, "never", "", "", &task.title);
            }
            format!("The older app's queue, read {}.", age(now_ms - at))
        }
        Some((Err(e), at)) => format!(
            "The older app's queue did not answer, {}: {}",
            age(now_ms - at),
            one_line(e)
        ),
        None => "The older app's queue is being read: the next reload shows it.".into(),
    };
    if table.is_empty() {
        table = "No Rust task yet.\n".into();
    }
    let commit = commit.unwrap_or("a local build");
    format!(
        "Rust tasks\nDependencies: /deps\n{note}\n\nTask\tState\tTries\tLast run\tOutcome\tPR\tWords\n{table}\nBuilt from {commit}.\n"
    )
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
        let text = page(&[row(said)], None, 301_000, Some("abc123"));
        let line =
            "a-task\tfinished\t2\t5 min ago\tmerged\thttps://github.com/o/r/pull/12\tTASK_DONE\n";
        assert!(text.contains(line), "{text}");
        assert!(text.contains("Dependencies: /deps\n"), "{text}");
        assert!(text.contains("Built from abc123."), "{text}");
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
        let text = page(&[], Some(&(queue, 0)), 0, None);
        assert!(text.contains("\nne xt [2J\tqueued\t"), "{text}");
        assert!(text.contains("\tits faked row [2J   \n"), "{text}");
        let failed = page(
            &[],
            Some(&(Err("down\nBuilt from x.".into()), 0)),
            0,
            Some("y"),
        );
        assert!(failed.contains("0 s ago: down Built from x.\n"), "{failed}");
    }

    #[test]
    fn a_task_on_its_next_try_shows_nothing_of_the_last_one() {
        let _budget = TestBudget::new();
        let mut retried = row("failed at https://github.com/o/r/pull/12");
        retried.status = "claimed".into();
        retried.outcome = Some("failed".into());
        let text = page(&[retried], None, 1_000, None);
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
        let text = page(std::slice::from_ref(&gone), None, 1_000, None);
        assert!(text.contains("run gone"), "{text}");
        assert!(!text.contains("running"), "{text}");
        gone.run_gone = false;
        let text = page(&[gone], None, 1_000, None);
        assert!(text.contains("running"), "{text}");
        assert!(!text.contains("run gone"), "{text}");
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
        let text = page(&[row("")], Some(&(queue, 1_000)), 121_000, None);
        assert!(text.contains("queue, read 2 min ago."), "{text}");
        assert_eq!(text.matches("a-task\t").count(), 1, "{text}");
        assert!(
            text.contains("next\tqueued\t0\tnever\t\t\tits words\n"),
            "{text}"
        );
        let failed = page(&[], Some(&(Err("down now".into()), 0)), 0, None);
        assert!(
            failed.contains("did not answer, 0 s ago: down now"),
            "{failed}"
        );
        assert!(failed.contains("No Rust task yet."), "{failed}");
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
