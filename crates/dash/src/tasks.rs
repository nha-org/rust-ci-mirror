//! The binary's wiring of section 20b's orchestrator: the store, the older app's list and judge, herdr, the approved
//! `make claude`, Claude Code, git and GitHub, on orch. One process per run: a run that crashes takes its claim's lock with it
//! and nothing else, and `dash orch` frees it on its next look.
use dash_agent::ClaudeCode;
use dash_bridge::Dash;
use dash_core::tasks::{Outcome, Pick};
use dash_forge::GitHub;
use dash_launcher::ApprovedMake;
use dash_orch::{Pace, Ports, next, run_task};
use dash_policy::Defaults;
use dash_store::tasks::{self as store, SqliteTasks};
use dash_terminal::Herdr;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

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

fn tools() -> Tools {
    let home = or_exit(std::env::var("HOME").map_err(|e| format!("HOME: {e}")));
    let path = |var: &str, default: &str| {
        std::env::var(var).map_or_else(|_| Path::new(&home).join(default), PathBuf::from)
    };
    let repo = path("DASH_REPO", "repo/site_perso");
    let claude = Path::new(&home).join(".claude");
    let bin = std::env::var("DASH_CLI");
    let bin = bin.unwrap_or_else(|_| {
        repo.join("private/dashboard/bin/dash")
            .display()
            .to_string()
    });
    Tools {
        list: Dash { bin },
        terminal: Herdr::from_env(),
        launcher: ApprovedMake { home: home.clone() },
        agent: ClaudeCode {
            projects: claude.join("projects"),
            limits: claude.join("weekly-limit"),
        },
        forge: GitHub {
            repo,
            root: path("DASH_WORKTREES", ".herdr/worktrees/site_perso"),
            base: "main".into(),
        },
        policy: Defaults,
    }
}

fn ports<'a>(tasks: &'a mut SqliteTasks, tools: &'a Tools) -> Ports<'a> {
    Ports {
        tasks,
        list: &tools.list,
        terminal: &tools.terminal,
        launcher: &tools.launcher,
        agent: &tools.agent,
        forge: &tools.forge,
        policy: &tools.policy,
    }
}

pub fn show(dir: &Path, id: &str, mut out: File) {
    let row = or_exit(or_exit(store::open(dir)).show(id));
    let row = or_exit(row.ok_or(format!("{id}: no such task")));
    writeln!(out, "{row}").expect("stdout");
}

/// Exit 0 on a task merged; any other outcome prints the same, with exit 1.
pub fn run(dir: &Path, id: &str, account: Option<&str>, mut out: File) {
    let tools = tools();
    let mut tasks = or_exit(store::open(dir));
    let run = or_exit(run_task(id, account, &mut ports(&mut tasks, &tools), PACE));
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
pub fn orchestrate(dir: &Path, once: bool, mut out: File) {
    let tools = tools();
    let mut tasks = or_exit(store::open(dir));
    let mut running: Vec<Child> = vec![];
    let mut said = String::new();
    loop {
        running.retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_))));
        let line = match next(&mut ports(&mut tasks, &tools)) {
            Ok(Pick::Start { id, account }) => {
                let child = start(dir, &id, account.as_deref());
                if once {
                    let status = or_exit(child).wait().map_err(|e| e.to_string());
                    std::process::exit(or_exit(status).code().unwrap_or(1));
                }
                match child {
                    Ok(child) => running.push(child),
                    Err(e) => eprintln!("{id}: {e}"),
                }
                format!(
                    "started {id} on {}",
                    account.unwrap_or("its default account".into())
                )
            }
            Ok(Pick::Wait(why)) if once => {
                writeln!(out, "{why}").expect("stdout");
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

fn start(dir: &Path, id: &str, account: Option<&str>) -> Result<Child, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(me);
    command.args(["task", "run", "--data"]).arg(dir).arg(id);
    if let Some(account) = account {
        command.args(["--account", account]);
    }
    command.spawn().map_err(|e| e.to_string())
}
