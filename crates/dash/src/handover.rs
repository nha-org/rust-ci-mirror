//! The orchestrator handed to the next login before its own runs out, make before break (task rust-account-handover-port,
//! rfc-rust-unified#scheduling; ports `Dashboard.Orch.AccountHandover` and the orchestrator's rule of `Dashboard.ClaudeAccounts`;
//! @nha, 2026-11-01: « No orchestration can stop even in case of a session restart failure so there must be a handover. »).
//!
//! 1. **When:** the orchestrator's login is forecast to run out within `MARGIN_S` at its burn (dash_core::forecast, the sooner
//!    of its 5 hours and its week), while his switch `claude-auto` is on, and another login has room.
//! 2. **Start:** the lock (`.claude/run/owner`) stays the running one's; a passover file and a fresh orchestrator on the next
//!    login in its own herdr tab (`herdr/session.sh resume --fresh --orchestrator --account`). The old one keeps working.
//! 3. **Take over:** once the new one's wake-wait runs and the old one is between turns (or `OLD_WAIT_S` passed), the lock
//!    moves, the old one ends (`<pid>.close`, its pane closed), the new one is woken with the passover, the move logged in
//!    `~/.claude/accounts/orchestrator.jsonl` with `"by":"rust"`, which `dash limits` prints.
//! 4. **A failed start** closes the new tab while the old one lives, gives it the lock back, and waits `BACKOFF_S`.
//!
//! The state is `.claude/run/account-handover.json`, the older app's own file, so one handover runs at a time across both:
//! made by an exclusive create, and written as a start still launching dated a day ahead, which the older app's pass reads as
//! « starting » and leaves alone (its `advance/2`), while its `orchestrator_panes/0` keeps its limit resumes off both panes.
//! ponytail: the older app's other triggers (its pane on the limit screen, no turn 10 minutes after a wake) and its hand
//! command stay its own until it is removed.
use dash_agent::Forecast;
use dash_core::tasks::Agent as _;
use dash_core::tasks::Limit;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How far ahead a forecast hands over: a new session takes a minute or two to start, its wake-wait a few more. Wider than the
/// older app's 30 minutes, so Rust's pass goes first while both run.
pub const MARGIN_S: u64 = 45 * 60;
const COOLDOWN_S: u64 = 300;
/// The new one's wake-wait, at most this long after its start.
const WAIT_S: u64 = 600;
/// The old one's turn past this once the new one is ready: stuck on its limit screen, its login out by now.
const OLD_WAIT_S: u64 = 1800;
const BACKOFF_S: u64 = 900;
/// How far ahead the state is dated for the older app, which then reads it as a start still launching. Past every Rust
/// handover's end (`WAIT_S` + `OLD_WAIT_S`, 40 minutes), short of a day: with `dash orch` stopped mid-handover, the older
/// app's `advance/2` fails it (its lock back to the old one) some 70 minutes on, never 24 hours (/code-review of #1513).
const PARK_S: u64 = 3600;
const PROJECT: &str = "site_perso";

/// Where a handover reads and writes: `claude` is `~/.claude`, `run` the main checkout's `.claude/run`, `repo` that checkout.
pub struct Paths {
    pub claude: PathBuf,
    pub run: PathBuf,
    pub repo: PathBuf,
}

impl Paths {
    fn state(&self) -> PathBuf {
        self.run.join("account-handover.json")
    }
    fn failed(&self) -> PathBuf {
        self.run.join("account-handover-failed.json")
    }
    /// The orchestrator's moves, the older app's log too.
    pub fn log(&self) -> PathBuf {
        self.claude.join("accounts/orchestrator.jsonl")
    }
    fn threads(&self) -> PathBuf {
        self.claude.join("run/threads.jsonl")
    }
    /// A run-dir file's first line, trimmed; none when missing or empty.
    fn read(&self, name: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.run.join(name)).ok()?;
        let line = text.trim().to_string();
        (!line.is_empty()).then_some(line)
    }
}

/// What a handover does outside its files: herdr's launcher and panes, a process's life, his phone. Stand-ins in a test.
pub trait World {
    /// `herdr/session.sh` with `args`: its exit code and output.
    fn launch(&self, args: &[String]) -> (i32, String);
    fn close(&self, pane: &str) -> Result<(), String>;
    fn alive(&self, pid: &str) -> bool;
    fn tell(&self, words: &str);
}

/// A login as the handover weighs it: its name, who is signed in there (`accountUuid|organization`, else the email; none for a
/// profile with no login) and its email, its figures with their forecast (none when nothing was read), and his rules for it:
/// `claude-disabled` and its `claude-thresholds` line (100 unless he set one), as `ClaudeAccounts.plan/2` read them.
pub struct Login {
    pub name: String,
    pub who: Option<String>,
    pub email: Option<String>,
    pub forecast: Option<Forecast>,
    pub disabled: bool,
    pub line: f64,
}

/// His rules for the logins: the older app's settings `claude-disabled` (« a,b ») and `claude-thresholds` (« a=85,b=80 »).
#[derive(Default)]
pub struct Rules {
    pub disabled: Vec<String>,
    pub lines: HashMap<String, f64>,
}

impl Rules {
    pub fn parse(disabled: Option<&str>, thresholds: Option<&str>) -> Rules {
        let names = |s: Option<&str>| -> Vec<String> {
            let parts = s.unwrap_or("").split(',').map(str::trim);
            parts.filter(|n| !n.is_empty()).map(String::from).collect()
        };
        let pair = |p: &String| {
            let (n, v) = p.split_once('=')?;
            Some((n.to_string(), v.parse::<f64>().ok()?))
        };
        Rules {
            disabled: names(disabled),
            lines: names(thresholds).iter().filter_map(pair).collect(),
        }
    }

    /// Where `name` stops taking the orchestrator: his line, 100 at most.
    fn line(&self, name: &str) -> f64 {
        self.lines.get(name).copied().unwrap_or(100.0).min(100.0)
    }
}

fn clock(at: u64) -> String {
    dash_core::stamp(at as i64 * 1000)
}

/// One pass at `now`: a handover under way moves on, else one starts once the orchestrator's login runs out within the margin.
/// What it did, in a line; an error when a start or take-over failed.
pub fn pass(
    p: &Paths,
    w: &dyn World,
    logins: &[Login],
    auto: bool,
    now: u64,
) -> Result<String, String> {
    match state(p) {
        Some(s) if s["by"] == "rust" => advance(p, w, s, now),
        Some(s) => Ok(format!(
            "the older app's handover to {} runs: it moves it on",
            s["to"].as_str().unwrap_or("?")
        )),
        None if !auto => {
            Ok("stay: the automatic switch is off (dash claude-accounts auto on)".into())
        }
        None => due(p, w, logins, now),
    }
}

fn state(p: &Paths) -> Option<Value> {
    let text = std::fs::read_to_string(p.state()).ok()?;
    serde_json::from_str(&text).ok().filter(Value::is_object)
}

fn put(p: &Paths, s: &Value) -> Result<(), String> {
    whole(&p.state(), &s.to_string())
}

/// `text` written beside `file`, then renamed over it: a reader never sees it empty or half written.
fn whole(file: &Path, text: &str) -> Result<(), String> {
    let fresh = file.with_extension(format!("{}.tmp", std::process::id()));
    let failed = |e: std::io::Error| format!("{}: {e}", file.display());
    std::fs::write(&fresh, text).map_err(failed)?;
    std::fs::rename(&fresh, file).map_err(failed)
}

fn append(file: &Path, text: &str) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("{}: {e}", file.display());
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(failed)?;
    f.write_all(text.as_bytes()).map_err(failed)
}

fn due(p: &Paths, w: &dyn World, logins: &[Login], now: u64) -> Result<String, String> {
    if let Some((at, why)) = last_failure(p).filter(|(at, _)| now.saturating_sub(*at) < BACKOFF_S) {
        return Ok(format!(
            "stay: the last handover failed {} s ago ({why}): it tries again after 15 minutes",
            now - at
        ));
    }
    let current = orchestrator_account(p, w);
    let Some(cur) = logins.iter().find(|l| l.name == current) else {
        return Ok(format!(
            "stay: the orchestrator's login {current} is no profile here (dash claude-accounts lists them)"
        ));
    };
    let cur = Some(cur);
    let why = match out_why(cur, now) {
        Some(why) => format!("{current} {why}"),
        None => {
            let lasts = cur
                .and_then(|l| l.forecast.as_ref())
                .and_then(Forecast::runs_out);
            return Ok(match lasts {
                Some((at, window, _)) => {
                    format!("stay: {current}'s {window} lasts to {}", clock(at))
                }
                None => format!("stay: {current} lasts to its resets"),
            });
        }
    };
    if let Some(at) = last_move(p).filter(|at| now.saturating_sub(*at) < COOLDOWN_S) {
        return Ok(format!(
            "stay: cooldown, the orchestrator moved {} s ago",
            now - at
        ));
    }
    match target(logins, cur, now) {
        Some(to) => start(
            p,
            w,
            &to.name,
            &current,
            &format!("auto: {why}, {} has room", to.name),
            now,
        ),
        None => Ok(format!(
            "stuck: {why}, and no other login has room for the orchestrator"
        )),
    }
}

/// Why login `cur` is out at `now`: signed in nowhere, or forecast to run out within the margin. None while it lasts.
fn out_why(cur: Option<&Login>, now: u64) -> Option<String> {
    // a name no profile folder holds (a renamed profile, a thread line's typo) is unknown, never out: a healthy orchestrator
    // would be moved every cooldown (/code-review); `due` says so
    let cur = cur?;
    if cur.who.is_none() {
        return Some("is signed in nowhere".into());
    }
    let (at, window, burn) = cur.forecast.as_ref()?.runs_out()?;
    (at <= now + MARGIN_S)
        .then(|| format!("runs out of its {window} at {} at {burn:.1}%/h", clock(at)))
}

/// Whether `a` and `b` hold one login: by name, by `who`, or by email where either names no id (`ClaudeAccounts.holds?/2`).
fn same_login(a: &Login, b: &Login) -> bool {
    let no_id = |l: &Login| l.who == l.email;
    a.name == b.name
        || (a.who.is_some() && a.who == b.who)
        || ((no_id(a) || no_id(b)) && a.email.is_some() && a.email == b.email)
}

/// The login a handover takes: the one whose week resets soonest with room past the margin and under his line, never one he
/// disabled, never the orchestrator's own login under another name (rust/CLAUDE.md: spend the sooner reset first; the
/// handover of 2026-11-02 09:14 started on a profile holding the same login and never finished; `ClaudeAccounts.orch_target`).
fn target<'a>(logins: &'a [Login], cur: Option<&Login>, now: u64) -> Option<&'a Login> {
    let same = |l: &Login| cur.is_some_and(|c| same_login(c, l));
    let under = |l: &Login| {
        l.forecast
            .as_ref()
            .is_some_and(|f| f.limit.room() && f.limit.week < l.line && f.limit.five_hours < l.line)
    };
    let resets = |l: &Login| {
        let r = l.forecast.as_ref().map_or(0, |f| f.limit.week_resets);
        if r > now { r } else { now + 7 * 86_400 }
    };
    let mut fit: Vec<&Login> = logins
        .iter()
        .filter(|l| l.who.is_some() && !l.disabled && !same(l))
        .filter(|l| under(l))
        .filter(|l| out_why(Some(l), now).is_none())
        .collect();
    fit.sort_by_key(|l| resets(l));
    fit.into_iter().next()
}

/// The login the orchestrator runs on: the lock holder's Claude session's start line, else its thread's last, else `default`
/// (`ClaudeAccounts.orchestrator_account/0`).
fn orchestrator_account(p: &Paths, w: &dyn World) -> String {
    let starts = starts(p);
    let owner = p.read("owner").filter(|o| w.alive(o));
    let session = owner.and_then(|o| p.read(&format!("{o}.session")));
    let by_session =
        session.and_then(|s| starts.iter().rev().find(|e| e["id"] == s.as_str()).cloned());
    let thread = format!("orchestrator:{PROJECT}");
    let by_thread = || {
        starts
            .iter()
            .rev()
            .find(|e| e["thread"] == thread.as_str())
            .cloned()
    };
    let line = by_session.or_else(by_thread);
    let account = line.as_ref().and_then(|e| e["account"].as_str());
    account
        .filter(|a| !a.is_empty())
        .unwrap_or("default")
        .to_string()
}

/// The thread log's start lines, oldest first.
fn starts(p: &Paths) -> Vec<Value> {
    let text = std::fs::read_to_string(p.threads()).unwrap_or_default();
    let lines = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok());
    lines.filter(|e| e["ev"] == "start").collect()
}

/// The last move's time, from the log's last line.
fn last_move(p: &Paths) -> Option<u64> {
    let text = std::fs::read_to_string(p.log()).ok()?;
    let last: Value = serde_json::from_str(text.lines().last()?).ok()?;
    dash_agent::unix_of(last["at"].as_str()?)
}

/// The log's last line as `dash limits` prints it: when, from where to where, by which app, and why.
pub fn last_line(p: &Paths) -> Option<String> {
    let text = std::fs::read_to_string(p.log()).ok()?;
    let last: Value = serde_json::from_str(text.lines().last()?).ok()?;
    let s = |k: &str| last[k].as_str().unwrap_or("?").to_string();
    let by = match last["by"].as_str() {
        Some("rust") => "Rust",
        _ => "the older app",
    };
    let at = s("at");
    let at = dash_agent::unix_of(&at).map_or(at, clock);
    Some(format!(
        "orchestrator: last handover {at}, {} to {}, by {by}: {}",
        s("from"),
        s("to"),
        s("why")
    ))
}

/// `dash limits`' lines for Claude: each login's 5 hours and week, the week's burn and the hour it runs out at that burn
/// (dash_core::forecast), then the orchestrator's last handover.
pub fn limit_lines(p: &Paths, now: u64) -> Result<Vec<String>, String> {
    let agent = dash_agent::ClaudeCode {
        projects: p.claude.join("projects"),
        limits: p.claude.join("weekly-limit"),
    };
    let mut lines: Vec<String> = agent
        .forecasts_at(now, dash_agent::READINGS)?
        .iter()
        .map(|f| {
            let l = &f.limit;
            let week = match f.week {
                Some(o) => match o.at {
                    // out already: an hour that is always this minute reads as a forecast, never as out (live, 2026-11-12)
                    Some(at) if at <= now => {
                        format!("burns {:.1}%/h, out now, until its reset", o.burn)
                    }
                    Some(at) => format!("burns {:.1}%/h, runs out {}", o.burn, clock(at)),
                    None => format!("burns {:.1}%/h, lasts to its reset", o.burn),
                },
                None => "no week read".into(),
            };
            format!("{}; week {week}", line("claude", l))
        })
        .collect();
    lines.push(last_line(p).unwrap_or_else(|| "orchestrator: no handover logged".into()));
    Ok(lines)
}

inventory::submit! { crate::usage::Command::usage("dash limits") }
/// `dash limits`: each kind's limits as the picker reads them; exit 1 when a kind's read failed, after printing the others.
pub fn limits(mut out: std::fs::File) {
    let home = crate::tasks::home();
    let keys = dash_store::keys::open(&crate::tasks::live_data()).and_then(|k| k.list());
    // Claude's lines carry each week's forecast and the last handover
    let claude = Paths {
        claude: Path::new(&home).join(".claude"),
        run: crate::tasks::repo().join(".claude/run"),
        repo: crate::tasks::repo(),
    };
    let now = (crate::tasks::now_ms() / 1000) as u64;
    let mut failed = false;
    match limit_lines(&claude, now) {
        Ok(lines) => lines
            .iter()
            .for_each(|l| writeln!(out, "{l}").expect("stdout")),
        Err(e) => {
            failed = true;
            writeln!(out, "claude: {e}").expect("stdout");
        }
    }
    let reads = [
        ("codex", crate::tasks::codex(&home).limits()),
        (
            "key",
            keys.map(|keys| keys.iter().map(|k| k.limit()).collect()),
        ),
    ];
    for (kind, read) in reads {
        match read {
            Ok(limits) => {
                for l in limits {
                    writeln!(out, "{}", line(kind, &l)).expect("stdout");
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

/// One login's line in `dash limits`, every kind's.
fn line(kind: &str, l: &dash_core::tasks::Limit) -> String {
    match l.key {
        true => format!("key {}: {}% of its cap spent", l.account, l.week),
        false => format!(
            "{kind} {}: 5 hours {}%, week {}%, week resets {}",
            l.account, l.five_hours, l.week, l.week_resets
        ),
    }
}

fn last_failure(p: &Paths) -> Option<(u64, String)> {
    let text = std::fs::read_to_string(p.failed()).ok()?;
    let f: Value = serde_json::from_str(&text).ok()?;
    Some((
        f["at"].as_u64()?,
        f["why"].as_str().unwrap_or("").to_string(),
    ))
}

/// A fresh orchestrator started on `to`, the running one kept until it takes over.
fn start(
    p: &Paths,
    w: &dyn World,
    to: &str,
    from_account: &str,
    why: &str,
    now: u64,
) -> Result<String, String> {
    let from = p
        .read("owner")
        .filter(|o| w.alive(o))
        .ok_or("no live orchestrator holds the lock (.claude/run/owner): nothing to hand over")?;
    let file = p.run.join(format!("passover-{now}.md"));
    let s = json!({
        "by": "rust", "starting": true, "at": now + PARK_S, "started": now,
        "from": from, "from_session": p.read(&format!("{from}.session")),
        "from_pane": p.read(&format!("{from}.pane")), "from_account": from_account,
        "to": to, "file": file.to_string_lossy(), "why": why,
    });
    // one handover at a time, the older app's included: written whole beside it, then linked in place, which fails when the
    // file is there; never a claim left empty or half written by a death between its create and its write (/code-review)
    let claim = p.run.join(format!(
        "account-handover.json.{}.claim",
        std::process::id()
    ));
    std::fs::write(&claim, s.to_string()).map_err(|e| format!("{}: {e}", claim.display()))?;
    let claimed = std::fs::hard_link(&claim, p.state());
    let _ = std::fs::remove_file(&claim);
    // only a file already there is a handover under way: a missing run dir or a denial is said as itself (/code-review)
    claimed.map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => {
            format!("a handover already runs ({}): {e}", p.state().display())
        }
        _ => format!("the handover's claim ({}): {e}", p.state().display()),
    })?;
    let passover = passover(from_account, to, why, now);
    if let Err(e) = std::fs::write(&file, passover) {
        return fail(p, w, s, &format!("the passover was not written: {e}"), now);
    }
    let args: Vec<String> = [
        "resume",
        PROJECT,
        "orchestrator",
        "--orchestrator",
        "--fresh",
        "--cwd",
        &p.repo.to_string_lossy(),
        "--account",
        to,
        "--goal",
        &format!("/orchestrator handover {}", file.display()),
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    let (code, out) = w.launch(&args);
    // only the pane the launcher says it made: a pane id scraped from an error may be the old orchestrator's (/code-review)
    let pane = launched_pane(&out);
    let mut s = s;
    s["pane"] = json!(pane);
    if code != 0 {
        let why = format!(
            "the new orchestrator on {to} did not start ({code}): {}",
            out.trim()
        );
        return fail(p, w, s, &why, now);
    }
    // a pane never named is a new one nobody could find or close: failed now, never read later as a launch still under way
    // (/code-review). Told it waits for « Handover done », it runs only wake-wait and never acts.
    if pane.is_none() {
        let why = format!(
            "the launcher named no pane for the new orchestrator on {to}; it waits idle in its tab: {}",
            out.trim()
        );
        return fail(p, w, s, &why, now);
    }
    // its pane unrecorded, the launched tab would be an orphan nobody closes: closed now (/code-review)
    if let Err(e) = put(p, &s) {
        return fail(
            p,
            w,
            s,
            &format!("the started pane was not recorded: {e}"),
            now,
        );
    }
    Ok(format!(
        "started a new orchestrator on {to} in {}: {why}",
        pane.unwrap_or_default()
    ))
}

/// The pane a launcher's output names: session.sh ends with « resume: <repo> (<pane>) ».
fn launched_pane(out: &str) -> Option<String> {
    out.lines().rev().find_map(|l| {
        let (_, rest) = l.trim_end().split_once(": ")?;
        let pane = rest.rsplit_once(" (")?.1.strip_suffix(')')?;
        pane.contains(':').then(|| pane.to_string())
    })
}

fn passover(from: &str, to: &str, why: &str, now: u64) -> String {
    format!(
        "# Orchestrator passover, {}\n\nFrom the orchestrator on {from} to the one on {to}: {why}.\n\n\
         Until a wake says « Handover done », run only private/dashboard/priv/commands/wake-wait: the other one still acts.\n\n\
         ## Read again before acting\ndash task ls, dash prs, dash asks, dash claude-accounts, dash limits, dash session list\n",
        clock(now)
    )
}

/// The supervisor of `pane`, alive: the new orchestrator.
fn new_pid(p: &Paths, w: &dyn World, pane: Option<&str>) -> Option<String> {
    let pane = pane?;
    supervisors(p)
        .into_iter()
        .find(|pid| p.read(&format!("{pid}.pane")).as_deref() == Some(pane) && w.alive(pid))
}

fn supervisors(p: &Paths) -> Vec<String> {
    let entries = std::fs::read_dir(&p.run).into_iter().flatten().flatten();
    let names = entries.filter_map(|e| e.file_name().into_string().ok());
    names
        .filter_map(|n| n.strip_suffix(".supervisor").map(String::from))
        .collect()
}

/// The old orchestrator, alive: by its supervisor pid, or by its Claude session after a restart moved it under another.
fn old_pids(p: &Paths, w: &dyn World, s: &Value, new: Option<&str>) -> Vec<String> {
    let from = s["from"].as_str();
    let session = s["from_session"].as_str();
    let mut pids: Vec<String> = from
        .map(String::from)
        .into_iter()
        .chain(supervisors(p))
        .collect();
    pids.retain(|pid| {
        Some(pid.as_str()) != new
            && (Some(pid.as_str()) == from
                || session
                    .is_some_and(|sid| p.read(&format!("{pid}.session")).as_deref() == Some(sid)))
            && w.alive(pid)
    });
    pids.sort();
    pids.dedup();
    pids
}

fn advance(p: &Paths, w: &dyn World, mut s: Value, now: u64) -> Result<String, String> {
    let to = s["to"].as_str().unwrap_or("?").to_string();
    let started = s["started"].as_u64().unwrap_or(0);
    // a start whose pass died before its launch answered: left to the clock
    if s["pane"].is_null() {
        return match now.saturating_sub(started) > WAIT_S {
            true => fail(
                p,
                w,
                s,
                &format!("the start on {to} never finished its launch"),
                now,
            ),
            false => Ok(format!("waiting: the new orchestrator on {to} is starting")),
        };
    }
    let new = new_pid(p, w, s["pane"].as_str());
    let olds = old_pids(p, w, &s, new.as_deref());
    // once its wake-wait was seen it is ready: wake-wait exits on each wake, so a later look may miss it
    let ready = s["ready"] == true
        || new.as_deref().is_some_and(|n| {
            p.read(&format!("{n}.wake-wait"))
                .is_some_and(|ww| w.alive(&ww))
        });
    if ready && s["ready"] != true {
        s["ready"] = json!(true);
        s["ready_at"] = json!(now);
        put(p, &s)?;
    }
    let between = |pid: &String| p.read(&format!("{pid}.state")).as_deref() == Some("idle");
    let overdue = now.saturating_sub(s["ready_at"].as_u64().unwrap_or(started)) > OLD_WAIT_S;
    match (new, ready) {
        (Some(new), true) if olds.iter().all(between) || overdue => {
            take_over(p, w, &s, &new, &olds, now)
        }
        // the old one died meanwhile: the new one is the only orchestrator left, never closed
        (Some(new), _) if olds.is_empty() => take_over(p, w, &s, &new, &olds, now),
        (None, true) => fail(
            p,
            w,
            s,
            &format!("the new orchestrator on {to} ended before it took over"),
            now,
        ),
        (Some(_), true) => Ok(format!(
            "waiting: the new orchestrator on {to} is ready; the old one finishes its turn"
        )),
        _ if now.saturating_sub(started) > WAIT_S => {
            let why = format!(
                "the new orchestrator on {to} ran no wake-wait within {} minutes",
                WAIT_S / 60
            );
            fail(p, w, s, &why, now)
        }
        _ => Ok(format!(
            "waiting: the new orchestrator on {to} has no wake-wait yet"
        )),
    }
}

/// The lock moved, every old one ended, the new one woken with the passover and every wake an old one left unhandled.
fn take_over(
    p: &Paths,
    w: &dyn World,
    s: &Value,
    new: &str,
    olds: &[String],
    now: u64,
) -> Result<String, String> {
    let to = s["to"].as_str().unwrap_or("?");
    // the lock and the wake before any old one ends: an error on either returns with every old one alive and its wakes
    // unread, so the next pass runs this whole again and loses none (/code-review of #1513)
    whole(&p.run.join("owner"), &format!("{new}\n"))?;
    let unhandled: Vec<String> = olds
        .iter()
        .flat_map(|pid| ["wake.read", "wake"].map(|ext| p.read(&format!("{pid}.{ext}"))))
        .flatten()
        .collect();
    let left = match unhandled.is_empty() {
        true => String::new(),
        false => format!(
            " The old one left these wakes unhandled: {}",
            unhandled.join(" ")
        ),
    };
    let wake = format!(
        "Handover done: you hold the orchestrator lock (.claude/run/owner, supervisor {new}) on {to}. Read {}, then carry on \
         as /orchestrator.{left}\n",
        s["file"].as_str().unwrap_or("the passover")
    );
    append(&p.run.join(format!("{new}.wake")), &wake)?;
    for pid in olds {
        let pane = match Some(pid.as_str()) == s["from"].as_str() {
            true => s["from_pane"].as_str().map(String::from),
            false => p.read(&format!("{pid}.pane")),
        };
        end_session(p, w, pid, &format!("handed over to {to}"), pane.as_deref());
    }
    let line = json!({"at": dash_core::iso(now as i64), "from": s["from_account"], "to": to, "why": s["why"], "by": "rust"});
    if let Some(dir) = p.log().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // logged, never returned: the wake is out, so a pass that ran this again would wake the new one twice (/code-review)
    if let Err(e) = append(&p.log(), &format!("{line}\n")) {
        eprintln!("handover: {e}");
    }
    let _ = std::fs::remove_file(p.failed());
    let _ = std::fs::remove_file(p.state());
    w.tell(&format!(
        "The orchestrator moved to {to}. Remote Control lists it only on that login: sign in to it, then open its pane."
    ));
    Ok(format!(
        "handed over to {to}: supervisor {new} holds the lock"
    ))
}

/// A session ended by the handover: marked first, so its supervisor reads a close asked for and resumes nothing.
fn end_session(p: &Paths, w: &dyn World, pid: &str, why: &str, pane: Option<&str>) {
    // logged, never an early return: a take-over half done would run again and wake the new one twice (/code-review)
    let mark = p.run.join(format!("{pid}.close"));
    if let Err(e) = std::fs::write(&mark, format!("{why}\n")) {
        eprintln!("handover: {}: {e}", mark.display());
    }
    if let Some(pane) = pane
        && let Err(e) = w.close(pane)
    {
        eprintln!("handover: pane {pane} did not close: {e}");
    }
}

/// The new tab closed, the lock and the thread back with the old one while it lives, the failure kept for the backoff. A new
/// one alive with the old one gone never fails here: `advance` hands it the lock.
fn fail(p: &Paths, w: &dyn World, s: Value, why: &str, now: u64) -> Result<String, String> {
    let new = new_pid(p, w, s["pane"].as_str());
    if let Some(pane) = s["pane"].as_str() {
        match &new {
            Some(new) => end_session(p, w, new, "handover failed", Some(pane)),
            None => {
                let _ = w.close(pane);
            }
        }
    }
    // a dead old one never gets the lock back: bin/orchestrator-job starts a fresh one on a dead owner (/code-review)
    // every step runs and each error is said: an early return left the state, so each pass closed the pane again with no
    // backoff (/code-review of #1513)
    let mut errors = vec![];
    if let Some(from) = s["from"].as_str().filter(|f| w.alive(f)) {
        errors.extend(whole(&p.run.join("owner"), &format!("{from}\n")).err());
    }
    repoint(p, &s, now);
    let kept = json!({"at": now, "why": why, "to": s["to"], "by": "rust"}).to_string();
    let kept = std::fs::write(p.failed(), kept);
    errors.extend(kept.err().map(|e| format!("{}: {e}", p.failed().display())));
    let _ = std::fs::remove_file(p.state());
    let also = match errors.is_empty() {
        true => String::new(),
        false => format!(" ({})", errors.join("; ")),
    };
    Err(format!(
        "{why}; the old orchestrator keeps working, the next try in 15 minutes{also}"
    ))
}

/// The thread's last start the old session again: the new one's start line named it.
fn repoint(p: &Paths, s: &Value, now: u64) {
    let Some(sid) = s["from_session"].as_str() else {
        return;
    };
    let Some(mut line) = starts(p).into_iter().rev().find(|e| e["id"] == sid) else {
        return;
    };
    line["at"] = json!(dash_core::iso(now as i64));
    line["source"] = json!("handover-failed");
    let log = std::fs::OpenOptions::new().append(true).open(p.threads());
    if let Err(e) = log.and_then(|mut f| writeln!(f, "{line}")) {
        eprintln!("handover: the thread log: {e}");
    }
}

/// Who is signed in on each login, from its `.claude.json` (`~/.claude.json` for `default`): `accountUuid|organization`, else
/// the email; and the email. Nothing else of the file is read.
pub fn who(claude: &Path, login: &str) -> (Option<String>, Option<String>) {
    let file = match login {
        "default" => claude.with_file_name(".claude.json"),
        name => claude.join("accounts").join(name).join(".claude.json"),
    };
    let read =
        || -> Option<Value> { serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok() };
    // Claude Code rewrites the file often: a read caught mid-write is read again, never taken for a login signed out, which
    // would move a healthy orchestrator (/code-review)
    let Some(json) = read().or_else(|| {
        std::thread::sleep(Duration::from_millis(200));
        read()
    }) else {
        return (None, None);
    };
    let a = &json["oauthAccount"];
    let email = a["emailAddress"].as_str().map(String::from);
    let who = match a["accountUuid"].as_str() {
        Some(id) => Some(format!(
            "{id}|{}",
            a["organizationName"].as_str().unwrap_or("")
        )),
        None => email.clone(),
    };
    (who, email)
}

/// The live world: herdr's launcher in the main checkout, its panes, `kill -0`, his phone through the older app.
pub struct Live {
    pub repo: PathBuf,
}

impl World for Live {
    fn launch(&self, args: &[String]) -> (i32, String) {
        let script = self
            .repo
            .join("herdr/session.sh")
            .to_string_lossy()
            .to_string();
        let mut words: Vec<&str> = vec![&script];
        words.extend(args.iter().map(String::as_str));
        // a delegate waits up to a minute for Claude's prompt and half a minute to submit: five minutes is a hang
        let env = [("DASH_MEMORY_GATE", "app"), ("DASH_NO_PRS", "1")];
        match dash_process::answer(
            "bash",
            &words,
            Some(&self.repo),
            &env,
            Duration::from_secs(300),
        ) {
            Ok((status, out, err)) => (status.code().unwrap_or(1), format!("{out}{err}")),
            Err(e) => (127, e.why),
        }
    }

    fn close(&self, pane: &str) -> Result<(), String> {
        dash_terminal::from_env().close(pane)
    }

    fn alive(&self, pid: &str) -> bool {
        pid.parse::<i32>()
            .is_ok_and(|p| p > 0 && unsafe { libc::kill(p, 0) } == 0)
    }

    fn tell(&self, words: &str) {
        if let Err(e) = crate::alerts::tell(words) {
            eprintln!("handover: the push: {e}");
        }
    }
}

/// The live world's status-line readings of each login, newest first (dash_agent::ClaudeCode).
fn agent(claude: &Path) -> dash_agent::ClaudeCode {
    dash_agent::ClaudeCode {
        projects: claude.join("projects"),
        limits: claude.join("weekly-limit"),
    }
}

/// `by_login`'s readings joined by `seen`, the watcher's history of each login's newest reading over the live burn's
/// look-back, which this pass's newest join. The status line keeps one file per session, rewritten in place, so a login one
/// session runs on (the orchestrator's) has one reading on disk and no live burn without it (/code-review of #1513).
pub fn with_history(
    mut by_login: HashMap<String, Vec<(u64, Limit)>>,
    now: u64,
    seen: &mut Vec<(u64, Limit)>,
) -> HashMap<String, Vec<(u64, Limit)>> {
    seen.extend(by_login.values().filter_map(|rs| rs.first().cloned()));
    seen.retain(|(at, _)| now.saturating_sub(*at) <= dash_core::forecast::LOOK_BACK_S);
    seen.sort_by(|a, b| (&a.1.account, b.0).cmp(&(&b.1.account, a.0)));
    seen.dedup_by(|a, b| a.0 == b.0 && a.1.account == b.1.account);
    for r in seen.iter() {
        by_login
            .entry(r.1.account.clone())
            .or_default()
            .push(r.clone());
    }
    for rs in by_login.values_mut() {
        rs.sort_by_key(|r| std::cmp::Reverse(r.0));
        rs.dedup_by_key(|r| r.0);
    }
    by_login
}

/// Each login's figures, forecast, who holds it and his rules for it, as the pass weighs them, from its readings.
pub fn logins(
    claude: &Path,
    by_login: HashMap<String, Vec<(u64, Limit)>>,
    now: u64,
    rules: &Rules,
) -> Vec<Login> {
    let mut read = agent(claude).forecasts_of(by_login, now);
    dash_agent::logins(claude)
        .into_iter()
        .map(|name| {
            let at = read.iter().position(|f| f.limit.account == name);
            let (who, email) = who(claude, &name);
            Login {
                who,
                email,
                forecast: at.map(|i| read.remove(i)),
                disabled: rules.disabled.contains(&name),
                line: rules.line(&name),
                name,
            }
        })
        .collect()
}

/// The watcher's history of each login's newest reading, over the live burn's look-back (`logins`). Lost when `dash orch`
/// restarts: the live burn is back within its 5-minute span once a session writes again.
static SEEN: std::sync::Mutex<Vec<(u64, Limit)>> = std::sync::Mutex::new(vec![]);

/// One pass on the live machine, from `dash orch`'s watcher: his switch and rules read from the older app's settings. Who
/// holds each login is read only when a handover may start: never while one runs or the switch is off (/code-review of #1513).
fn live_pass(_dir: &Path) -> Result<String, String> {
    let home = crate::tasks::home_var()?;
    let repo = crate::tasks::repo();
    let p = Paths {
        claude: Path::new(&home).join(".claude"),
        run: repo.join(".claude/run"),
        repo: repo.clone(),
    };
    let db = crate::tasks::older_db();
    let setting = |field| dash_store::older::setting(&db, field);
    let auto = setting("claude-auto")?.as_deref() == Some("on");
    let now = (crate::tasks::now_ms() / 1000) as u64;
    // the readings on every pass, so the history is whole when a handover is weighed; who holds each login only then
    let readings = agent(&p.claude).readings(dash_agent::READINGS)?;
    let readings = with_history(
        readings,
        now,
        &mut SEEN.lock().unwrap_or_else(|e| e.into_inner()),
    );
    let logins = match auto && state(&p).is_none() {
        true => {
            let rules = Rules::parse(
                setting("claude-disabled")?.as_deref(),
                setting("claude-thresholds")?.as_deref(),
            );
            logins(&p.claude, readings, now, &rules)
        }
        false => vec![],
    };
    pass(&p, &Live { repo }, &logins, auto, now)
}

/// A pass every minute for as long as `dash orch` runs, its beat on /watchers.
pub fn watch(dir: PathBuf) {
    let sleep = Duration::from_millis(crate::watchers::HANDOVER.every_ms as u64);
    // its beat after every pass, as the other watchers do: run beats only while paused (/code-review)
    crate::watchers::run(dir, &crate::watchers::HANDOVER, sleep, |dir| {
        crate::watchers::beaten(dir, &crate::watchers::HANDOVER, live_pass(dir))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::forecast::Out;
    use dash_core::tasks::Limit;
    use dash_meter::{TestDir, dash_test};
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    const NOW: u64 = 1_794_500_000;

    /// herdr, the processes and his phone, as files and lists.
    #[derive(Default)]
    struct StandIn {
        launched: RefCell<Vec<Vec<String>>>,
        answer: (i32, String),
        closed: RefCell<Vec<String>>,
        alive: RefCell<HashSet<String>>,
        told: RefCell<Vec<String>>,
    }

    impl World for StandIn {
        fn launch(&self, args: &[String]) -> (i32, String) {
            self.launched.borrow_mut().push(args.to_vec());
            self.answer.clone()
        }
        fn close(&self, pane: &str) -> Result<(), String> {
            self.closed.borrow_mut().push(pane.into());
            Ok(())
        }
        fn alive(&self, pid: &str) -> bool {
            self.alive.borrow().contains(pid)
        }
        fn tell(&self, words: &str) {
            self.told.borrow_mut().push(words.into());
        }
    }

    fn paths(dir: &TestDir) -> Paths {
        let p = Paths {
            claude: dir.0.join("home/.claude"),
            run: dir.0.join("repo/.claude/run"),
            repo: dir.0.join("repo"),
        };
        std::fs::create_dir_all(&p.run).expect("run");
        std::fs::create_dir_all(p.claude.join("run")).expect("threads");
        p
    }

    fn put_file(p: &Paths, name: &str, text: &str) {
        std::fs::write(p.run.join(name), text).expect("a run file");
    }

    fn login(name: &str, who: &str, week: f64, resets: u64, runs_out: Option<u64>) -> Login {
        let limit = Limit {
            account: name.into(),
            week,
            week_resets: resets,
            ..Limit::default()
        };
        Login {
            name: name.into(),
            who: Some(who.into()),
            forecast: Some(Forecast {
                limit,
                week: Some(Out {
                    burn: 5.0,
                    at: runs_out,
                }),
                five_hours: None,
            }),
            email: None,
            disabled: false,
            line: 100.0,
        }
    }

    /// The target is never one he disabled, nor one past his `claude-thresholds` line, nor the orchestrator's own login held
    /// by email where one profile names no id (/code-review of #1513; `ClaudeAccounts.orch_target`, `holds?/2`).
    #[dash_test]
    fn the_target_keeps_his_rules_and_matches_a_login_by_email() {
        let rules = Rules::parse(Some("third, "), Some("fourth=80,bad=x"));
        assert_eq!(rules.disabled, ["third"]);
        assert_eq!((rules.line("fourth"), rules.line("bad")), (80.0, 100.0));
        let cur = Login {
            email: Some("a@x".into()),
            ..login("default", "A|org", 97.0, NOW + 86_400, Some(NOW + 600))
        };
        let with = |l: Login| Login {
            disabled: rules.disabled.contains(&l.name),
            line: rules.line(&l.name),
            ..l
        };
        let logins = [
            // the same login, its file naming only the email
            Login {
                email: Some("a@x".into()),
                ..login("again", "a@x", 0.0, NOW + 3600, None)
            },
            with(login("third", "C|org", 0.0, NOW + 2 * 3600, None)),
            with(login("fourth", "D|org", 85.0, NOW + 3 * 3600, None)),
            with(login("fifth", "E|org", 10.0, NOW + 4 * 3600, None)),
        ];
        let to = target(&logins, Some(&cur), NOW).map(|l| l.name.as_str());
        assert_eq!(to, Some("fifth"));
    }

    /// One session's file, rewritten in place, still gives a live burn: the watcher's history joins each pass's newest
    /// (/code-review of #1513). A kept answer newer than readings too old for a burn replaces them, never a stale week read
    /// as empty.
    #[dash_test]
    fn one_session_s_file_burns_through_the_history_and_a_fresh_answer_beats_a_stale_week() {
        let dir = TestDir::new("handover-history");
        let p = paths(&dir);
        let reading = |used: f64| Limit {
            account: "default".into(),
            week: used,
            week_resets: NOW + 2 * 86_400,
            ..Limit::default()
        };
        let mut seen = vec![];
        let one = |at: u64, used: f64| {
            HashMap::from([("default".to_string(), vec![(at, reading(used))])])
        };
        with_history(one(NOW - 3600, 90.0), NOW - 3600, &mut seen);
        let joined = with_history(one(NOW, 95.0), NOW, &mut seen);
        assert_eq!(joined["default"].len(), 2, "{joined:?}");
        let f = &agent(&p.claude).forecasts_of(joined, NOW)[0];
        assert_eq!(
            f.week.map(|o| (o.burn, o.at)),
            Some((5.0, Some(NOW + 3600)))
        );

        // second: its session ran last week; get_usage asked now says 99%
        std::fs::create_dir_all(p.claude.join("accounts/second")).expect("a profile");
        let old = Limit {
            account: "second".into(),
            week: 40.0,
            week_resets: NOW - 86_400,
            ..Limit::default()
        };
        let asked = Limit {
            week: 99.0,
            week_resets: NOW + 86_400,
            ..old.clone()
        };
        dash_agent::remember(&p.claude.join("model-limits"), &asked).expect("kept");
        let by = HashMap::from([("second".to_string(), vec![(NOW - 8 * 86_400, old)])]);
        let f = agent(&p.claude).forecasts_of(by, NOW + 60);
        assert_eq!(f[0].limit.week, 99.0, "{f:?}");
    }

    /// Part 1 of the task: a recorded burn, default's week 90% to 95% over the last hour in the status line's files, reads in
    /// `dash limits` as 5%/h running out an hour on; a login read only by its kept `get_usage` answer lasts to its reset.
    #[dash_test]
    fn dash_limits_names_the_hour_each_week_runs_out_at_its_recorded_burn() {
        let dir = TestDir::new("handover-limits");
        let p = paths(&dir);
        let readings = p.claude.join("weekly-limit");
        std::fs::create_dir_all(&readings).expect("readings");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let resets = now + 2 * 86_400;
        for (minutes_ago, used) in [(60, 90.0), (40, 91.5), (20, 93.5), (0, 95.0)] {
            let file = readings.join(format!("s{minutes_ago}.json"));
            let reading = json!({"config_dir": "", "rate_limits": {
                "seven_day": {"used_percentage": used, "resets_at": resets}}});
            std::fs::write(&file, reading.to_string()).expect("a reading");
            let at = UNIX_EPOCH + Duration::from_secs(now - minutes_ago * 60);
            std::fs::File::options()
                .write(true)
                .open(&file)
                .and_then(|f| f.set_modified(at))
                .expect("its time");
        }
        let second = Limit {
            account: "second".into(),
            week: 10.0,
            week_resets: resets + 86_400,
            ..Limit::default()
        };
        // a profile no session ran on lately: its kept get_usage answer is its reading
        std::fs::create_dir_all(p.claude.join("accounts/second")).expect("a profile");
        dash_agent::remember(&p.claude.join("model-limits"), &second).expect("kept");
        let out = Limit {
            account: "third".into(),
            week: 100.0,
            week_resets: resets,
            ..Limit::default()
        };
        std::fs::create_dir_all(p.claude.join("accounts/third")).expect("a profile");
        dash_agent::remember(&p.claude.join("model-limits"), &out).expect("kept");
        let lines = limit_lines(&p, now).expect("lines");
        let third = lines.iter().find(|l| l.starts_with("claude third:"));
        assert!(
            third.is_some_and(|l| l.ends_with("out now, until its reset")),
            "a login at 100% is out, never forecast to run out this minute: {lines:?}"
        );
        let lines: Vec<_> = lines
            .into_iter()
            .filter(|l| !l.starts_with("claude third:"))
            .collect();
        assert_eq!(
            lines[0],
            format!(
                "claude default: 5 hours 0%, week 95%, week resets {resets}; week burns 5.0%/h, runs out {}",
                clock(now + 3600)
            )
        );
        assert!(
            lines[1].starts_with("claude second: 5 hours 0%, week 10%"),
            "{lines:?}"
        );
        assert!(lines[1].ends_with("lasts to its reset"), "{lines:?}");
        assert_eq!(lines[2], "orchestrator: no handover logged");
    }

    /// Part 2 of the task: forecast to run out, a new orchestrator starts on the next login with room (never default's own
    /// login under another name), and the old one ends only once it is between turns, the move logged as Rust's.
    #[dash_test]
    fn forecast_to_run_out_a_new_orchestrator_takes_over_after_the_old_one_s_turn() {
        let dir = TestDir::new("handover-flow");
        let p = paths(&dir);
        let w = StandIn {
            answer: (0, "resume: site_perso (w9:p2)\n".into()),
            ..StandIn::default()
        };
        w.alive.borrow_mut().insert("100".into());
        for (name, text) in [
            ("owner", "100"),
            ("100.state", "busy"),
            ("100.session", "S-old"),
            ("100.pane", "w1:p1"),
        ] {
            put_file(&p, name, text);
        }
        let start = json!({"ev": "start", "id": "S-old", "account": "default", "thread": "orchestrator:site_perso"});
        std::fs::write(p.threads(), format!("{start}\n")).expect("threads");
        let logins = [
            login("default", "A|org", 97.0, NOW + 86_400, Some(NOW + 36 * 60)),
            // default's own login under another name, resetting soonest: no target
            login("again", "A|org", 0.0, NOW + 3600, None),
            login("second", "B|org", 0.0, NOW + 3 * 86_400, None),
        ];
        // a login lasting past the margin stays
        let lasting = [
            login("default", "A|org", 50.0, NOW + 86_400, Some(NOW + 3 * 3600)),
            login("second", "B|org", 0.0, NOW, None),
        ];
        let stay = pass(&p, &w, &lasting, true, NOW).expect("a pass");
        assert!(stay.starts_with("stay: default's week lasts to"), "{stay}");
        assert_eq!(
            pass(&p, &w, &logins, false, NOW).expect("a pass"),
            "stay: the automatic switch is off (dash claude-accounts auto on)"
        );

        let said = pass(&p, &w, &logins, true, NOW).expect("a start");
        assert!(
            said.starts_with(
                "started a new orchestrator on second in w9:p2: auto: default runs out of its week"
            ),
            "{said}"
        );
        let args = w.launched.borrow()[0].join(" ");
        assert!(
            args.contains("--orchestrator --fresh") && args.contains("--account second"),
            "{args}"
        );
        let s = state(&p).expect("a state");
        assert_eq!(
            (s["by"].as_str(), s["starting"].as_bool()),
            (Some("rust"), Some(true))
        );
        assert!(
            s["at"].as_u64() > Some(NOW),
            "dated ahead: the older app reads a start still launching"
        );
        assert_eq!(
            p.read("owner").as_deref(),
            Some("100"),
            "the lock stays the old one's"
        );

        assert_eq!(
            pass(&p, &w, &logins, true, NOW + 60).expect("a pass"),
            "waiting: the new orchestrator on second has no wake-wait yet"
        );
        for (name, text) in [
            ("200.supervisor", ""),
            ("200.pane", "w9:p2"),
            ("200.wake-wait", "300"),
        ] {
            put_file(&p, name, text);
        }
        w.alive
            .borrow_mut()
            .extend(["200".to_string(), "300".to_string()]);
        let ready = pass(&p, &w, &logins, true, NOW + 120).expect("a pass");
        assert_eq!(
            ready,
            "waiting: the new orchestrator on second is ready; the old one finishes its turn"
        );
        assert!(w.closed.borrow().is_empty(), "the old one works on");

        put_file(&p, "100.state", "idle");
        put_file(&p, "100.wake", "his comment c-1");
        let handed = pass(&p, &w, &logins, true, NOW + 180).expect("a take-over");
        assert_eq!(
            handed,
            "handed over to second: supervisor 200 holds the lock"
        );
        assert_eq!(p.read("owner").as_deref(), Some("200"));
        assert_eq!(
            p.read("100.close").as_deref(),
            Some("handed over to second")
        );
        assert_eq!(*w.closed.borrow(), ["w1:p1"]);
        let wake = p.read("200.wake").expect("the new one woken");
        assert!(
            wake.starts_with("Handover done") && wake.contains("his comment c-1"),
            "{wake}"
        );
        assert!(state(&p).is_none(), "the state gone");
        assert_eq!(w.told.borrow().len(), 1, "his phone told");
        let line = last_line(&p).expect("the log");
        assert!(
            line.contains("default to second, by Rust: auto: default runs out of its week"),
            "{line}"
        );
        let again = pass(&p, &w, &logins, true, NOW + 200).expect("a pass");
        assert!(again.starts_with("stay: cooldown"), "{again}");
    }

    /// A start herdr refuses gives the old one its lock back and waits 15 minutes; an older app's handover is left to it.
    #[dash_test]
    fn a_failed_start_keeps_the_old_one_and_the_older_app_s_handover_is_left_alone() {
        let dir = TestDir::new("handover-fail");
        let p = paths(&dir);
        let w = StandIn {
            answer: (
                1,
                "resume: site_perso (w9:p2)\nherdr: the pane never took the prompt".into(),
            ),
            ..StandIn::default()
        };
        w.alive.borrow_mut().insert("100".into());
        put_file(&p, "owner", "100");
        let logins = [
            login("default", "A|org", 99.0, NOW + 86_400, Some(NOW + 600)),
            login("second", "B|org", 0.0, NOW + 3 * 86_400, None),
        ];
        // the orchestrator's login unknown here: it stays, never moved for want of a profile (/code-review)
        let start = json!({"ev": "start", "id": "S", "account": "renamed", "thread": "orchestrator:site_perso"});
        std::fs::write(p.threads(), format!("{start}\n")).expect("threads");
        let unknown = pass(&p, &w, &logins, true, NOW).expect("a pass");
        assert!(
            unknown.starts_with("stay: the orchestrator's login renamed is no profile here"),
            "{unknown}"
        );
        std::fs::write(p.threads(), "").expect("threads");
        let failed = pass(&p, &w, &logins, true, NOW).expect_err("a failed start");
        assert!(failed.contains("did not start (1)"), "{failed}");
        assert_eq!(*w.closed.borrow(), ["w9:p2"], "the half-made tab closed");
        assert_eq!(p.read("owner").as_deref(), Some("100"));
        assert!(state(&p).is_none());
        let wait = pass(&p, &w, &logins, true, NOW + 60).expect("a pass");
        assert!(
            wait.starts_with("stay: the last handover failed 60 s ago"),
            "{wait}"
        );
        put_file(&p, "account-handover.json", r#"{"to":"second","at":1}"#);
        let older = pass(&p, &w, &logins, true, NOW + 2000).expect("a pass");
        assert_eq!(
            older,
            "the older app's handover to second runs: it moves it on"
        );
        assert_eq!(w.launched.borrow().len(), 1, "no second start");
        // a launch that names no pane fails at once, never read later as one still launching (/code-review)
        std::fs::remove_file(p.state()).expect("the older state gone");
        std::fs::remove_file(p.failed()).expect("the backoff gone");
        let quiet = StandIn {
            answer: (0, "started\n".into()),
            ..StandIn::default()
        };
        quiet.alive.borrow_mut().insert("100".into());
        let unnamed = pass(&p, &quiet, &logins, true, NOW + 3000).expect_err("no pane named");
        assert!(
            unnamed.starts_with("the launcher named no pane"),
            "{unnamed}"
        );
        assert!(state(&p).is_none() && p.read("owner").as_deref() == Some("100"));

        // the lock not given back still ends the handover with its backoff, never a state failed again each pass; a
        // claim that cannot link says so, never « already runs » (/code-review of #1513)
        std::fs::remove_file(p.failed()).expect("the backoff gone");
        let s = json!({"by": "rust", "from": "100", "to": "second", "pane": "w9:p2"});
        put(&p, &s).expect("a state");
        // the rename's source a folder: the write fails
        let tmp = p.run.join(format!("owner.{}.tmp", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("a write that fails");
        let said = fail(&p, &w, s, "a test", NOW + 4000).expect_err("failed");
        assert!(
            said.contains("the next try in 15 minutes (") && said.contains("owner"),
            "{said}"
        );
        assert!(state(&p).is_none() && last_failure(&p).is_some());
    }
}
