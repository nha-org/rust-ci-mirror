//! Automations (task rust-automations, rfc-orchestration 8.2; @nha, 2026-11-10: « review the claude prompts and see if some
//! of it can be automated. Maybe a periodic task », « the mechanism to add a task, but not making them pile ? »): a recurring
//! prompt of his run by `dash orch` on a schedule, never by a Claude session's cron, which dies with its session (the 11-09
//! handover lost the morning run until 10:50). An automation is a row of `ALL`: a key, a schedule, and a `dash` command or a
//! brief. A brief files one task whose id is its key, reopened by each run; while that task is open a run is skipped, so they
//! never pile. A failed command reopens its key's task the same way. The task's report line « PUSH: what to tell him » reaches his
//! phone through `dash push`, once a report. Each run's state is a row of `watchers.sqlite`; /watchers shows it.
use crate::tasks::{now_ms, older_app, older_db, repo};
use crate::watchers;
use dash_core::stamp;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// When an automation runs: each day from a local `HH:MM` (a day the Mac slept through runs at its next pass), or every n minutes.
// ponytail: no automation runs every n minutes or a command yet; the first joins `ALL`, the tests run both kinds meanwhile.
#[cfg_attr(not(test), allow(dead_code))]
pub enum Every {
    Daily(&'static str),
    Minutes(i64),
}

/// What a run does: a `dash` command's words, or a brief its task runs.
#[cfg_attr(not(test), allow(dead_code))]
pub enum Does {
    Command(&'static [&'static str]),
    Brief(&'static str),
}

pub struct Automation {
    /// Its key, and the id of its one task.
    pub key: &'static str,
    pub title: &'static str,
    pub why: &'static str,
    pub done_when: &'static str,
    pub epic: &'static str,
    pub every: Every,
    pub does: Does,
    /// The .mcp.json servers its task's session needs (`:run :mcp`).
    pub mcp: &'static [&'static str],
    /// A pause file in `.claude/run` it also obeys, `YYYY-MM-DDTHH:MM` compared as text: the job-search pause
    /// (Dashboard.JobPause) for the morning run.
    pub pause_file: Option<&'static str>,
}

/// The morning run, which `.claude/hooks/morning-run.sh` armed as a CronCreate job in his session until 2026-11-10.
pub const MORNING_RUN: Automation = Automation {
    key: "morning-run",
    title: "Morning run: the daily sweep, applications prepared, one push",
    why: "(@nha, 2026-10-07: « Every tiny step I need to pull for every application, can we make this a lot more proactive? », \
          then « Build now please ») Filed and reopened each day by the automation morning-run (rfc-orchestration 8.2).",
    // three checks of at most 160 characters each, as dash task add's style check holds: one line of 207 refused the task
    done_when: "1. Today's /nicolas-daily ran, people answered first \
                2. Each lead at fit 4 or 5 has its CV package and a draft tagged :handover with today's date \
                3. The report's push field holds one line « PUSH: what to tell him »",
    epic: "clojure-applications",
    every: Every::Daily("08:07"),
    does: Does::Brief(
        "Morning run: /nicolas-daily, answering people first; applications are prepared in full again (@nha, 2026-11-04: \
         « prepare applications please. originator didn't contact us yet which makes me feel like it won't work »): each lead \
         at fit 4 or 5 gets its CV package and its draft tagged :handover with today's date; once tagged, \
         private/dashboard/bin/dash check must show no « not ready to hand over » for it (drop the lead if still flagged). \
         Your report record (dash task report) carries a field \"push\" holding one line « PUSH: what to tell him »: \
         http://100.72.158.115:7788/#maintenant and the « À confirmer » questions of those leads that no file and no \
         repository answers, then the night's held pushes, the output of private/dashboard/bin/dash morning-report. The \
         automation sends that line to his phone; send no PushNotification yourself.",
    ),
    mcp: &["mcp-server-linkedin", "whatsapp"],
    pause_file: Some("morning-run.paused-until"),
};

/// Every automation: each next one joins here.
pub const ALL: [&Automation; 1] = [&MORNING_RUN];

/// A pass a minute: a daily run starts within a minute of its time.
const PASS: Duration = Duration::from_secs(60);
/// A command's limit, and a call to the older app's CLI.
const CALL: Duration = Duration::from_secs(120);

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS automation (key TEXT PRIMARY KEY, at_ms INTEGER, day TEXT, said TEXT, \
                      ok INTEGER, pushed INTEGER) WITHOUT ROWID;";

fn open(dir: &Path) -> Result<dash_store::Connection, String> {
    let db = watchers::open(dir)?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    Ok(db)
}

/// An automation's last run: when, its local day, what it said and whether it worked; paused by his switch (watchers'
/// `paused` table, one for watchers and automations); the last report pushed.
#[derive(Default, Debug, PartialEq)]
pub struct State {
    pub at_ms: Option<i64>,
    pub day: Option<String>,
    pub said: String,
    pub ok: bool,
    pub paused: bool,
    pub pushed: Option<i64>,
}

pub fn state(dir: &Path, key: &str) -> Result<State, String> {
    let sql = "SELECT at_ms, day, coalesce(said, ''), coalesce(ok, 1), pushed FROM automation WHERE key = ?1";
    let db = open(dir)?;
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let mut rows = q
        .query_map([key], |r| {
            Ok(State {
                at_ms: r.get(0)?,
                day: r.get(1)?,
                said: r.get(2)?,
                ok: r.get(3)?,
                paused: false,
                pushed: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let s = rows.next().transpose().map_err(|e| e.to_string())?;
    let s = s.unwrap_or(State {
        ok: true,
        ..State::default()
    });
    Ok(State {
        paused: watchers::is_paused(dir, key),
        ..s
    })
}

/// A run of `key` at `at_ms` on local `day`: what it said, and whether it worked. A run that failed keeps its day unset, so
/// the next pass tries again.
fn ran(
    dir: &Path,
    key: &str,
    at_ms: i64,
    day: Option<&str>,
    ok: bool,
    said: &str,
) -> Result<(), String> {
    let sql = "INSERT INTO automation (key, at_ms, day, said, ok) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(key) DO UPDATE \
               SET at_ms = ?2, day = coalesce(?3, day), said = ?4, ok = ?5";
    let db = open(dir)?;
    db.execute(sql, (key, at_ms, day, said, ok))
        .map(drop)
        .map_err(|e| e.to_string())
}

fn pushed(dir: &Path, key: &str, report: Option<i64>) -> Result<(), String> {
    let sql = "INSERT INTO automation (key, pushed) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET pushed = ?2";
    let db = open(dir)?;
    db.execute(sql, (key, report))
        .map(drop)
        .map_err(|e| e.to_string())
}

/// What a run reaches: the list of tasks, the older app's CLI and his phone. A trait so the test runs on a stand-in.
pub trait Board {
    /// Task `id`'s stage and run prompt, none when the list has no such task.
    fn stage(&self, id: &str) -> Result<Option<(String, Option<String>)>, String>;
    /// Files `a`'s task (`exists` false) or reopens it, its run prompt `prompt`.
    fn file(&self, a: &Automation, prompt: &str, exists: bool) -> Result<(), String>;
    /// Runs `dash <words>`: what it printed, or why it failed.
    fn command(&self, words: &[&str]) -> Result<String, String>;
    /// Task `id`'s newest report: its id and record.
    fn report(&self, id: &str) -> Result<Option<(i64, String)>, String>;
    fn push(&self, words: &str) -> Result<(), String>;
}

/// The line « PUSH: what to tell him » a report record holds in any of its texts: its text.
fn push_line(record: &str) -> Option<String> {
    fn find(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::String(s) => s
                .lines()
                .find_map(|l| l.trim().strip_prefix("PUSH: ").map(str::to_string)),
            serde_json::Value::Array(a) => a.iter().find_map(find),
            serde_json::Value::Object(o) => o.values().find_map(find),
            _ => None,
        }
    }
    find(&serde_json::from_str(record).ok()?).filter(|t| !t.trim().is_empty())
}

/// A daily run catches up at most this many hours after its time, then waits for the next day: the morning run, deployed at
/// 18:01 on 2026-11-10, ran that evening as a late morning run, though the old cron had run it at 08:07.
const LATE_HOURS: u32 = 4;
/// A failed run tries again after this, never at every minute's pass (it retried each minute on 2026-11-10).
const RETRY_MS: i64 = 15 * 60_000;

/// The last time of day a daily run at `at` (`HH:MM`) still catches up, as text compared like `now`'s time.
fn late_bound(at: &str) -> String {
    let (h, m) = at.split_once(':').unwrap_or((at, "00"));
    let h: u32 = h.parse().unwrap_or(0);
    format!("{:02}:{m}", (h + LATE_HOURS).min(24))
}

/// Whether a failed daily run still waits before its retry, at `now_ms`; an every-N-minutes one keeps its own interval.
fn retrying(every: &Every, s: &State, now_ms: i64) -> bool {
    matches!(every, Every::Daily(_)) && !s.ok && s.at_ms.is_some_and(|at| now_ms - at < RETRY_MS)
}

/// Whether `every` is due at local `now` (`YYYY-MM-DDTHH:MM`) and `now_ms`, after the last run `s`.
fn due(every: &Every, s: &State, now: &str, now_ms: i64) -> bool {
    let (today, time) = now.split_once('T').unwrap_or((now, ""));
    if retrying(every, s, now_ms) {
        return false;
    }
    match every {
        Every::Daily(at) => {
            time >= *at && time < late_bound(at).as_str() && s.day.as_deref() != Some(today)
        }
        Every::Minutes(n) => s.day.is_none() || s.at_ms.is_none_or(|at| now_ms - at >= n * 60_000),
    }
}

/// What holds `a` at local `now`, `run` being `.claude/run`: his switch, or its pause file while ahead.
fn held(a: &Automation, s: &State, run: &Path, now: &str) -> Option<String> {
    if s.paused {
        return Some("paused by his switch".into());
    }
    let until = a.pause_file.and_then(|f| paused_until(run, f, now))?;
    Some(format!("paused until {until}"))
}

/// When `a` runs next, in words.
pub fn next(a: &Automation, s: &State, run: &Path, now: &str, now_ms: i64) -> String {
    if let Some(held) = held(a, s, run, now) {
        return held;
    }
    if due(&a.every, s, now, now_ms) {
        return "due now: at the next pass, within a minute".into();
    }
    if let Some(at) = s.at_ms.filter(|_| retrying(&a.every, s, now_ms)) {
        return format!("tries again at {}", stamp(at + RETRY_MS));
    }
    match (&a.every, s.at_ms) {
        (Every::Daily(at), _) => {
            let (today, time) = now.split_once('T').unwrap_or((now, ""));
            match s.day.as_deref() == Some(today) || time >= late_bound(at).as_str() {
                true => format!("tomorrow at {at} local"),
                false => format!("today at {at} local"),
            }
        }
        (Every::Minutes(n), Some(at)) => stamp(at + n * 60_000),
        (Every::Minutes(_), None) => "due now".into(),
    }
}

/// The schedule in words.
pub fn schedule(a: &Automation) -> String {
    let does = match a.does {
        Does::Command(words) => format!("runs dash {}", words.join(" ")),
        Does::Brief(_) => format!("files or reopens task {}: {}", a.key, a.title),
    };
    match a.every {
        Every::Daily(at) => format!("each day at {at} local: {does}"),
        Every::Minutes(n) => format!("every {n} minutes: {does}"),
    }
}

/// The pause file's end while it is ahead of `now`, as morning-run.sh and Dashboard.JobPause read it.
fn paused_until(run: &Path, file: &str, now: &str) -> Option<String> {
    let until = std::fs::read_to_string(run.join(file)).ok()?;
    let until = until.trim();
    let shape = until.len() == 16
        && until.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            10 => b == b'T',
            13 => b == b':',
            _ => b.is_ascii_digit(),
        });
    (shape && now < until).then(|| until.to_string())
}

/// One pass of `a` at local `now` and `now_ms`: `run` is `.claude/run`. What it did, in a line.
pub fn pass(
    dir: &Path,
    run: &Path,
    board: &dyn Board,
    a: &Automation,
    now: &str,
    now_ms: i64,
) -> Result<String, String> {
    let s = state(dir, a.key)?;
    // a pause holds its report's push too: what it holds comes back in the first pass after, as Dashboard.JobPause's
    if let Some(held) = held(a, &s, run, now) {
        return Ok(held);
    }
    let (mut said, mut failed) = (vec![], vec![]);
    // the report claimed before its push, so a push that lands is never sent twice; one that fails puts the claim back for
    // the next pass, and never holds the run
    if let Some((id, record)) = board.report(a.key)?
        && s.pushed.is_none_or(|p| id > p)
    {
        pushed(dir, a.key, Some(id))?;
        if let Some(words) = push_line(&record) {
            match board.push(&words) {
                Ok(()) => said.push(format!("pushed report {id}")),
                Err(e) => {
                    pushed(dir, a.key, s.pushed)?;
                    failed.push(format!("the push of report {id} failed: {e}"));
                }
            }
        }
    }
    if due(&a.every, &s, now, now_ms) {
        let today = now.split_once('T').map_or(now, |(d, _)| d);
        let (line, ok) = run_once(board, a)?;
        ran(dir, a.key, now_ms, Some(today), ok, &line)?;
        match ok {
            true => said.push(line),
            false => failed.push(line),
        }
    } else {
        said.push(format!("next: {}", next(a, &s, run, now, now_ms)));
    }
    match failed.is_empty() {
        true => Ok(said.join("; ")),
        false => Err([said, failed].concat().join("; ")),
    }
}

/// A due run of `a`: what it did and whether that is healthy. Only an open task holds it, never a second task; any other
/// stage (done, dropped, new, backlog) or none is free. An open task without its brief (a filing whose update failed) gets it
/// again. A daily run held by a task still open from an earlier day is stuck, NOT OK on /watchers, so the next wake names it.
fn run_once(board: &dyn Board, a: &Automation) -> Result<(String, bool), String> {
    let task = board.stage(a.key)?;
    let open = task.as_ref().is_some_and(|(stage, _)| stage == "open");
    let briefed = match (&a.does, &task) {
        (Does::Brief(brief), Some((_, prompt))) => prompt.as_deref() == Some(*brief),
        _ => true,
    };
    if open && briefed {
        let stuck = matches!(a.every, Every::Daily(_));
        let why = if stuck {
            ", from an earlier run: see its pane"
        } else {
            ""
        };
        return Ok((
            format!("skipped: task {} is still open{why}", a.key),
            !stuck,
        ));
    }
    let (exists, verb) = match task {
        Some(_) => (true, "reopened"),
        None => (false, "filed"),
    };
    Ok(match a.does {
        Does::Brief(brief) => {
            board.file(a, brief, exists)?;
            (format!("{verb} task {}", a.key), true)
        }
        Does::Command(words) => match board.command(words) {
            Ok(out) => (
                format!("ran: {}", out.lines().last().unwrap_or("no output")),
                true,
            ),
            Err(e) => {
                let prompt = format!(
                    "The automation {} ran `dash {}` and it failed: {e}. Find the cause and fix it.",
                    a.key,
                    words.join(" ")
                );
                board.file(a, &prompt, exists)?;
                (format!("failed, task {} {verb}: {e}", a.key), false)
            }
        },
    })
}

/// What reads an automation's task's stage: the live list's, or a stand-in in a test.
type Stage = dyn Fn(&str) -> Result<Option<(String, Option<String>)>, String>;

/// Each automation's row on /watchers at local `now`: its schedule, last run, next run, its one task by `stage`, its switch.
fn rows_at(dir: &Path, run: &Path, now: &str, now_ms: i64, stage: &Stage) -> Vec<watchers::Row> {
    let row = |a: &Automation| {
        let (beat, next) = match state(dir, a.key) {
            Err(e) => (
                format!("its state did not read: {e}"),
                "unknown, its state did not read".into(),
            ),
            Ok(s) => {
                let beat = match s.at_ms {
                    None => "none yet".into(),
                    Some(at) if !s.ok => format!("NOT OK, {}: {}", stamp(at), s.said),
                    Some(at) => format!("ok, {}: {}", stamp(at), s.said),
                };
                (beat, next(a, &s, run, now, now_ms))
            }
        };
        let task = match stage(a.key) {
            Ok(Some((open, _))) if open == "open" => {
                format!("{} is open: a run waits for it", a.key)
            }
            Ok(Some((other, _))) => format!("none: {} is {other}, reopened at the next run", a.key),
            Ok(None) => format!("none yet: {} is filed at the first run", a.key),
            Err(e) => format!("its stage did not read: {e}"),
        };
        watchers::Row {
            name: a.key.into(),
            served: "Rust, an automation",
            watches: schedule(a),
            beat,
            woke: "nothing: it files its task".into(),
            fed: None,
            next,
            task,
            pause: Ok((a.key.into(), watchers::is_paused(dir, a.key))),
        }
    };
    ALL.iter().map(|a| row(a)).collect()
}

/// Each automation's row on /watchers now, its task's stage from the live list.
pub fn rows(dir: &Path) -> Vec<watchers::Row> {
    let stage = |id: &str| dash_store::older::stage(&older_db(), id);
    rows_at(
        dir,
        &repo().join(".claude/run"),
        &local_now(),
        now_ms(),
        &stage,
    )
}

/// The keys his switch on /watchers may name, beside Rust's watchers.
pub fn keys() -> Vec<&'static str> {
    ALL.iter().map(|a| a.key).collect()
}

/// The list, the older app's CLI and his phone, for real.
struct Live;

/// `dash task add`'s overlap refusal names the tasks to set apart: « run it again with --distinct-from <ids> --because ».
fn distinct_from(refused: &str) -> Option<&str> {
    let after = refused.split("--distinct-from ").nth(1)?;
    after.split_whitespace().next()
}

impl Board for Live {
    fn stage(&self, id: &str) -> Result<Option<(String, Option<String>)>, String> {
        dash_store::older::stage(&older_db(), id)
    }

    fn file(&self, a: &Automation, prompt: &str, exists: bool) -> Result<(), String> {
        let dash = older_app().bin;
        let call = |words: &[&str]| dash_process::run(&dash, words, None, CALL);
        if !exists {
            let start = format!("the automation {} runs it on its schedule", a.key);
            let add = [
                "task",
                "add",
                a.key,
                "--title",
                a.title,
                "--why",
                a.why,
                "--done-when",
                a.done_when,
                "--epic",
                a.epic,
                "--priority",
                "2",
                "--start",
                &start,
            ];
            let filed = call(&add);
            // an automation's task is its own: set apart from the closest, named by the refusal, once
            if let Err(refused) = &filed
                && let Some(ids) = distinct_from(refused)
            {
                let because = format!(
                    "the automation {}'s one task, reopened by each run (rfc-orchestration 8.2)",
                    a.key
                );
                let apart = ["--distinct-from", ids, "--because", &because];
                call(&[&add[..], &apart[..]].concat())?;
            } else {
                filed?;
            }
        }
        let mut words = vec![
            "task",
            "update",
            a.key,
            "--status",
            "open",
            "--run.prompt",
            prompt,
        ];
        for m in a.mcp {
            words.extend(["--run.mcp", m]);
        }
        call(&words).map(drop)
    }

    fn command(&self, words: &[&str]) -> Result<String, String> {
        dash_process::run(&older_app().bin, words, None, CALL)
    }

    fn report(&self, id: &str) -> Result<Option<(i64, String)>, String> {
        dash_store::older::last_report(&older_db(), id)
    }

    fn push(&self, words: &str) -> Result<(), String> {
        crate::alerts::tell(words)
    }
}

/// A pass over every automation and its beat on /watchers, whatever each said.
fn beating(dir: &Path) -> String {
    let now = local_now();
    let run = repo().join(".claude/run");
    let mut lines = vec![];
    let mut ok = true;
    for a in ALL {
        match pass(dir, &run, &Live, a, &now, now_ms()) {
            Ok(line) => lines.push(format!("{}: {line}", a.key)),
            Err(e) => {
                ok = false;
                let line = format!("{}: {e}", a.key);
                if let Err(e) = ran(dir, a.key, now_ms(), None, false, &e) {
                    eprintln!("automations: the record: {e}");
                }
                lines.push(line);
            }
        }
    }
    let said = lines.join("; ");
    if let Err(e) = watchers::beat(dir, &watchers::AUTOMATIONS, now_ms(), ok, &said) {
        eprintln!("automations: the beat: {e}");
    }
    said
}

/// A pass every minute for as long as `dash orch` runs. His switch on the automations watcher holds every automation; his
/// switch on one automation holds that one.
pub fn watch(dir: PathBuf) {
    watchers::run(dir, &watchers::AUTOMATIONS, PASS, |dir| Ok(beating(dir)));
}

// POSIX's tzset, which the libc crate does not bind on every target.
unsafe extern "C" {
    fn tzset();
}

/// The local time now, as `YYYY-MM-DDTHH:MM`, the hook's `date +%Y-%m-%dT%H:%M`.
pub(crate) fn local_now() -> String {
    // SAFETY: localtime_r writes only into `tm`, which outlives the call. tzset first: localtime_r need not read TZ again,
    // and `dash orch` runs for days, across a change of hour.
    // tzset writes libc's zone globals: one caller at a time, the automations' pass and a /watchers view among them
    static TZ: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = TZ.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let tm = unsafe {
        tzset();
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use std::cell::RefCell;

    /// The list as a map of stages, and what each call did.
    #[derive(Default)]
    struct Stand {
        stage: RefCell<Option<(String, Option<String>)>>,
        report: RefCell<Option<(i64, String)>>,
        did: RefCell<Vec<String>>,
        fails: bool,
        push_fails: RefCell<bool>,
    }

    fn task(stage: &str, prompt: Option<&str>) -> Option<(String, Option<String>)> {
        Some((stage.into(), prompt.map(str::to_string)))
    }

    impl Board for Stand {
        fn stage(&self, _: &str) -> Result<Option<(String, Option<String>)>, String> {
            Ok(self.stage.borrow().clone())
        }
        fn file(&self, a: &Automation, prompt: &str, exists: bool) -> Result<(), String> {
            let verb = if exists { "reopen" } else { "file" };
            self.did.borrow_mut().push(format!(
                "{verb} {} mcp {}: {prompt}",
                a.key,
                a.mcp.join(",")
            ));
            *self.stage.borrow_mut() = task("open", Some(prompt));
            Ok(())
        }
        fn command(&self, words: &[&str]) -> Result<String, String> {
            self.did
                .borrow_mut()
                .push(format!("dash {}", words.join(" ")));
            if self.fails {
                Err("exit 1".into())
            } else {
                Ok("polled".into())
            }
        }
        fn report(&self, _: &str) -> Result<Option<(i64, String)>, String> {
            Ok(self.report.borrow().clone())
        }
        fn push(&self, words: &str) -> Result<(), String> {
            if *self.push_fails.borrow() {
                return Err("dash push: exit 1".into());
            }
            self.did.borrow_mut().push(format!("push {words}"));
            Ok(())
        }
    }

    #[test]
    fn a_brief_files_one_task_per_key_and_a_run_is_skipped_while_it_is_open() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("automations");
        let run = dir.0.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let board = Stand::default();
        let a = &MORNING_RUN;
        let at = 1_794_300_000_000;
        // before 08:07: nothing, and the next run is today's
        let said = pass(&dir.0, &run, &board, a, "2026-11-10T08:06", at).unwrap();
        assert_eq!(said, "next: today at 08:07 local");
        assert!(board.did.borrow().is_empty());
        // 08:07, no task yet: its one task filed, with the brief and the servers its session needs
        let said = pass(&dir.0, &run, &board, a, "2026-11-10T08:07", at).unwrap();
        assert_eq!(said, "filed task morning-run");
        let did = board.did.borrow().join("\n");
        assert!(
            did.starts_with(
                "file morning-run mcp mcp-server-linkedin,whatsapp: Morning run: /nicolas-daily"
            ),
            "{did}"
        );
        // the same day: ran, so nothing more
        let said = pass(&dir.0, &run, &board, a, "2026-11-10T09:00", at + 1).unwrap();
        assert_eq!(said, "next: tomorrow at 08:07 local");
        assert_eq!(board.did.borrow().len(), 1);
        // the next day with yesterday's task still open: skipped, never a second task, NOT OK (stuck, so the next wake names
        // it), and not tried again that day
        let said = pass(&dir.0, &run, &board, a, "2026-11-11T08:07", at + 2);
        assert_eq!(
            said,
            Err(
                "skipped: task morning-run is still open, from an earlier run: see its pane".into()
            )
        );
        assert!(!state(&dir.0, a.key).unwrap().ok);
        pass(&dir.0, &run, &board, a, "2026-11-11T08:08", at + 3).unwrap();
        assert_eq!(board.did.borrow().len(), 1);
        // its report's PUSH: line reaches his phone once; the task done, the Mac asleep at 08:07: the next pass reopens it
        *board.report.borrow_mut() = Some((
            7,
            r#"{"kind":"done","push":"PUSH: 3 applications ready"}"#.into(),
        ));
        *board.stage.borrow_mut() = task("done", None);
        let said = pass(
            &dir.0,
            &run,
            &board,
            a,
            "2026-11-12T10:30",
            at + 2 * 86_400_000,
        )
        .unwrap();
        assert_eq!(said, "pushed report 7; reopened task morning-run");
        pass(
            &dir.0,
            &run,
            &board,
            a,
            "2026-11-12T10:31",
            at + 2 * 86_400_000 + 60_000,
        )
        .unwrap();
        let did = board.did.borrow();
        assert_eq!(did[1], "push 3 applications ready");
        assert!(did[2].starts_with("reopen morning-run"), "{}", did[2]);
        assert_eq!(did.len(), 3);
        let s = state(&dir.0, a.key).unwrap();
        assert_eq!(
            (s.day.as_deref(), s.pushed, s.ok),
            (Some("2026-11-12"), Some(7), true)
        );
    }

    /// The morning run deployed at 18:01 ran that evening and retried each minute (2026-11-10): a daily run catches up only
    /// within LATE_HOURS of its time, and a failed run waits RETRY_MS before it tries again.
    #[test]
    fn a_daily_run_never_catches_up_in_the_evening_and_a_failure_waits_before_its_retry() {
        let _budget = TestBudget::new();
        let every = Every::Daily("08:07");
        let fresh = State {
            ok: true,
            ..State::default()
        };
        let at = 1_794_300_000_000;
        assert!(due(&every, &fresh, "2026-11-10T08:07", at));
        assert!(due(&every, &fresh, "2026-11-10T12:06", at));
        assert!(!due(&every, &fresh, "2026-11-10T12:07", at));
        assert!(!due(&every, &fresh, "2026-11-10T18:01", at));
        let failed = State {
            ok: false,
            at_ms: Some(at),
            ..State::default()
        };
        assert!(!due(&every, &failed, "2026-11-10T08:10", at + 3 * 60_000));
        assert!(due(&every, &failed, "2026-11-10T08:22", at + RETRY_MS));
        assert_eq!(late_bound("22:30"), "24:30");
    }

    #[test]
    fn a_failed_push_never_holds_the_run_and_is_tried_again_and_a_task_without_its_brief_gets_it() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("automations-push");
        let run = dir.0.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let a = &MORNING_RUN;
        // a filing whose update failed: the task open with no brief. The run gives it its brief, never skips on it
        let board = Stand {
            stage: RefCell::new(task("open", None)),
            ..Stand::default()
        };
        *board.report.borrow_mut() = Some((3, r#"{"push":"PUSH: hello"}"#.into()));
        *board.push_fails.borrow_mut() = true;
        let said = pass(&dir.0, &run, &board, a, "2026-11-10T08:07", 1);
        assert_eq!(
            said,
            Err("reopened task morning-run; the push of report 3 failed: dash push: exit 1".into())
        );
        assert!(
            board.did.borrow()[0]
                .starts_with("reopen morning-run mcp mcp-server-linkedin,whatsapp: Morning run:")
        );
        // the run is recorded, the push left for the next pass, which sends it once
        let s = state(&dir.0, a.key).unwrap();
        assert_eq!((s.day.as_deref(), s.pushed), (Some("2026-11-10"), None));
        *board.push_fails.borrow_mut() = false;
        assert_eq!(
            pass(&dir.0, &run, &board, a, "2026-11-10T08:08", 2).unwrap(),
            "pushed report 3; next: tomorrow at 08:07 local"
        );
        pass(&dir.0, &run, &board, a, "2026-11-10T08:09", 3).unwrap();
        assert_eq!(board.did.borrow().len(), 2, "{:?}", board.did.borrow());
        // a pause holds the push too: the job-search pause sends nothing, the report goes after it
        *board.report.borrow_mut() = Some((4, r#"{"push":"PUSH: later"}"#.into()));
        std::fs::write(run.join("morning-run.paused-until"), "2026-11-10T18:00").unwrap();
        assert_eq!(
            pass(&dir.0, &run, &board, a, "2026-11-10T09:00", 4).unwrap(),
            "paused until 2026-11-10T18:00"
        );
        assert_eq!(board.did.borrow().len(), 2);
        // a task put in the backlog holds nothing: the next run reopens it
        *board.stage.borrow_mut() = task("backlog", Some("Morning run"));
        let said = pass(&dir.0, &run, &board, a, "2026-11-11T08:07", 5).unwrap();
        assert_eq!(said, "pushed report 4; reopened task morning-run");
    }

    #[test]
    fn a_pause_holds_a_run_and_a_failed_command_reopens_its_one_task() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("automations-pause");
        let run = dir.0.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let board = Stand::default();
        let a = &MORNING_RUN;
        // his switch holds it, and resumes it
        assert!(watchers::toggle(&dir.0, a.key, &keys()).unwrap());
        let said = pass(&dir.0, &run, &board, a, "2026-11-10T08:07", 1).unwrap();
        assert_eq!(said, "paused by his switch");
        assert!(!watchers::toggle(&dir.0, a.key, &keys()).unwrap());
        assert!(watchers::toggle(&dir.0, "nope", &keys()).is_err());
        // the job-search pause file holds it too, only while ahead and only a date-time
        std::fs::write(run.join("morning-run.paused-until"), "2026-11-10T18:00").unwrap();
        let said = pass(&dir.0, &run, &board, a, "2026-11-10T08:07", 2).unwrap();
        assert_eq!(said, "paused until 2026-11-10T18:00");
        std::fs::write(run.join("morning-run.paused-until"), "monday").unwrap();
        assert_eq!(
            pass(&dir.0, &run, &board, a, "2026-11-10T08:07", 3).unwrap(),
            "filed task morning-run"
        );
        // a command: run every 5 minutes, its failure one task, then skipped while that task is open
        let failing = Stand {
            fails: true,
            ..Stand::default()
        };
        let poll = Automation {
            key: "mail-check",
            every: Every::Minutes(5),
            does: Does::Command(&["mail", "poll"]),
            mcp: &[],
            pause_file: None,
            ..MORNING_RUN
        };
        let said = pass(&dir.0, &run, &failing, &poll, "2026-11-10T08:00", 0);
        assert_eq!(said, Err("failed, task mail-check filed: exit 1".into()));
        assert!(
            !state(&dir.0, "mail-check").unwrap().ok,
            "a failed command reads NOT OK"
        );
        let said = pass(&dir.0, &run, &failing, &poll, "2026-11-10T08:03", 180_000).unwrap();
        assert!(said.starts_with("next: "), "{said}");
        let said = pass(&dir.0, &run, &failing, &poll, "2026-11-10T08:05", 300_000).unwrap();
        assert_eq!(said, "skipped: task mail-check is still open");
        assert_eq!(failing.did.borrow().len(), 2, "{:?}", failing.did.borrow());
    }

    #[test]
    fn its_row_on_watchers_shows_last_and_next_run_its_task_and_switch() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("automations-rows");
        let run = dir.0.join("run");
        let open = |_: &str| Ok(task("open", None));
        let rows = rows_at(&dir.0, &run, "2026-11-10T08:00", 0, &open);
        let text = dash_surface::text(&watchers::tree_of(&dir.0, "the feed", None, "", rows))
            .expect("text");
        let at = text
            .find("morning-run\n")
            .unwrap_or_else(|| panic!("no automation row: {text}"));
        let row = &text[at..];
        for line in [
            "Served by: Rust, an automation",
            "Watches: each day at 08:07 local: files or reopens task morning-run",
            "Last run: none yet",
            "Next run: today at 08:07 local",
            "Open task: morning-run is open: a run waits for it",
            "Pause: POST /watchers/pause?name=morning-run",
        ] {
            assert!(row.contains(line), "lacks {line}: {row}");
        }
        // his switch: Resume in the HTML, paused in the text; a run recorded shows its line and tomorrow's time
        assert!(watchers::toggle(&dir.0, "morning-run", &keys()).expect("pauses"));
        ran(
            &dir.0,
            "morning-run",
            1_794_300_000_000,
            Some("2026-11-10"),
            true,
            "filed task morning-run",
        )
        .unwrap();
        let done = |_: &str| Ok(task("done", None));
        let tree = watchers::tree_of(
            &dir.0,
            "the feed",
            None,
            "",
            rows_at(&dir.0, &run, "2026-11-10T09:00", 0, &done),
        );
        let html = dash_surface::screen::page("Watchers", &tree).expect("html");
        assert!(
            html.contains("/watchers/pause?name=morning-run") && html.contains("Resume"),
            "{html}"
        );
        let text = dash_surface::text(&tree).expect("text");
        assert!(text.contains("Next run: paused by his switch"), "{text}");
        assert!(text.contains(": filed task morning-run\n"), "{text}");
        assert!(
            text.contains("Open task: none: morning-run is done, reopened at the next run"),
            "{text}"
        );
    }

    #[test]
    fn a_push_line_is_found_in_any_text_of_the_record() {
        let _budget = TestBudget::new();
        assert_eq!(
            push_line(r#"{"next":"a\nPUSH: hello"}"#).as_deref(),
            Some("hello")
        );
        assert_eq!(
            push_line(r#"{"checks":[{"output":"PUSH: x"}]}"#).as_deref(),
            Some("x")
        );
        assert_eq!(push_line(r#"{"push":"PUSH: "}"#), None);
        assert_eq!(push_line("not json"), None);
        assert_eq!(
            distinct_from(
                "Update one instead, or run it again with --distinct-from a,b --because <why> to file it"
            ),
            Some("a,b")
        );
    }
}
