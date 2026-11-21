//! Claude Code's session hooks, `dash hook session-state|statusline|gh-meter-path` (task rust-hooks-rest-port; @nha,
//! 2026-11-10: « all these python scripts are also very brittle. we want to move all logic to rust and have it well
//! tested. »). They were .claude/hooks/session-state.sh, stop.sh, statusline-usage.sh and gh-meter-path.sh, with python3
//! for their JSON; the files they write are the same, byte for byte where a reader parses them (bin/claude-supervise,
//! bin/supervise-test, Dashboard.Threads, Dashboard.ClaudeAccounts). Each history is in `git log -- .claude/hooks`.
use crate::send::truthy;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn stdin() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    s
}

/// A file's first line, as `read -r` takes it: its ends' blanks trimmed; "" when it cannot be read.
fn first_line(p: &Path) -> String {
    fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .trim_matches([' ', '\t'])
        .to_string()
}

/// A file's text as `$(cat f)` gives it: its trailing newlines dropped.
fn cat(p: &Path) -> String {
    fs::read_to_string(p)
        .unwrap_or_default()
        .trim_end_matches('\n')
        .to_string()
}

fn write(p: &Path, text: &str) {
    let _ = fs::write(p, text);
}

/// `$(paste -sd<sep> - < f)`: the file's lines joined by `sep`.
fn paste(p: &Path, sep: &str) -> String {
    let text = fs::read_to_string(p).unwrap_or_default();
    text.strip_suffix('\n')
        .unwrap_or(&text)
        .split('\n')
        .collect::<Vec<_>>()
        .join(sep)
}

fn chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn local_now() -> String {
    (crate::host().local_at)((crate::host().now_ms)() / 1000)
}

/// The hook's answer to Claude Code: context for the session.
fn say(out: &mut impl Write, event: &str, context: &str) {
    let said =
        json!({"hookSpecificOutput": {"hookEventName": event, "additionalContext": context}});
    let _ = writeln!(out, "{said}");
}

/// The Stop of an orchestrator being handed over (dash's handover.rs, `account-handover.json` in `run`), once the new one is
/// ready: by its supervisor `sup` (the lock's pid, or `from_sup`, the one make started under it) or its Claude session
/// `session`, as the handover names the old one. Only Rust's handover (`by`): the older app writes the same file and wakes
/// no old one whose /goal this ended when its handover fails (/code-review of PR 1816). Its turn's end is kept
/// in `<sup>.handed` (the handover's start), which the minute's pass reads to close its pane, and its answer ends the turn
/// with no next one, /goal's included: between two goal turns the session is idle for seconds, which a pass a minute apart
/// misses (task handover-ends-old; on 2026-11-19 the old one, /goal on, kept acting past the handover). Only until the
/// handover's `handed_until` (seconds, `now` being the hook's): with `dash orch` stopped once the new one was ready, no pass
/// takes over, and every Stop of the old one would end its /goal for the hour the older app takes to fail the state
/// (/code-review of PR 1816).
pub fn handed(run: &Path, sup: &str, session: &str, now: u64) -> Option<Value> {
    let text = fs::read_to_string(run.join("account-handover.json")).ok()?;
    let h: Value = serde_json::from_str(&text).ok()?;
    let old = h["from"] == sup
        || h["from_sup"] == sup
        || !session.is_empty() && h["from_session"] == session;
    let live = h["handed_until"].as_u64().is_some_and(|until| now < until);
    (h["by"] == "rust" && h["ready"] == true && live && old).then(|| {
        write(&run.join(format!("{sup}.handed")), &h["started"].to_string());
        let to = h["to"].as_str().unwrap_or("the next login");
        json!({"continue": false, "stopReason": format!("Handed over to {to}: this session ends, its pane closes within the minute.")})
    })
}

/// Whether supervisor `pid` is between turns for the handover started at `started`: idle now, or a turn ended since its
/// successor was ready ([`handed`]).
pub fn ended(run: &Path, pid: &str, started: u64) -> bool {
    first_line(&run.join(format!("{pid}.state"))) == "idle"
        || first_line(&run.join(format!("{pid}.handed"))) == started.to_string()
}

/// A turn starts under supervisor `sup`, his message's or a wake's (its first tool call): a turn's end kept by [`handed`]
/// no longer stands, so the pass waits for this turn's own end and never closes the pane in the middle of it, a `dash ship`
/// or a merge cut halfway (/code-review of PR 1816). A /goal's next turn is not started: the Stop's `continue: false` stops
/// it, and should it not, the pass takes over once `OLD_WAIT_S` passed.
pub fn resumed(run: &Path, sup: &str) {
    let _ = fs::remove_file(run.join(format!("{sup}.handed")));
}

/// The supervisors whose files are in `run`, by pid: bin/claude-supervise writes `<pid>.supervisor` at its start, and this
/// hook `<pid>.state` under one from before that mark. Make, execed under a restarted supervisor's pid, has neither.
pub fn supervisors(run: &Path) -> Vec<String> {
    let entries = fs::read_dir(run).into_iter().flatten().flatten();
    let names = entries.filter_map(|e| e.file_name().into_string().ok());
    let mut pids: Vec<String> = names
        .filter_map(|n| {
            let pid = n.strip_suffix(".supervisor").or(n.strip_suffix(".state"));
            pid.map(String::from)
        })
        .collect();
    pids.sort();
    pids.dedup();
    pids
}

/// The old orchestrator of a handover whose lock names `from`: the supervisor make started under it, when `from` has no
/// `.session`, and the old one's Claude session. A restart execs make under the supervisor's pid, alive with none of its
/// files, the lock still on it (2026-11-19): the old one is then a supervisor among `from`'s descendants (make, its shell,
/// the supervisor: a few levels). Never the thread's last session: any plain session of the main checkout is on that
/// thread, and the handover would end the wrong one (/code-review of PR 1816).
pub fn old_one(
    run: &Path,
    from: &str,
    alive: &dyn Fn(&str) -> bool,
    parent: &dyn Fn(&str) -> Option<String>,
) -> (Option<String>, Option<String>) {
    let session =
        |pid: &str| Some(first_line(&run.join(format!("{pid}.session")))).filter(|s| !s.is_empty());
    if let Some(s) = session(from) {
        return (None, Some(s));
    }
    let under = |pid: &String| {
        let up = std::iter::successors(parent(pid), |p| parent(p));
        up.take(8).any(|p| p == from)
    };
    let sup = supervisors(run)
        .into_iter()
        .find(|pid| alive(pid) && under(pid));
    let s = sup.as_deref().and_then(session);
    (sup, s)
}

/// The old orchestrators of handover `h`, alive, `new` left out: a supervisor that is its `from`, its `from_sup`, or runs its
/// `from_session` (a restart moved the session under another). Only a supervisor: the lock's pid after a restart is make's,
/// which never turns idle nor holds the pane. With none found and the lock's pid alive, that pid stands for the old one,
/// never between turns: the pass waits its time out, and never reads a live old one it cannot match as dead (/code-review
/// of PR 1816).
pub fn olds(run: &Path, h: &Value, new: Option<&str>, alive: &dyn Fn(&str) -> bool) -> Vec<String> {
    let from = h["from"].as_str();
    let session = h["from_session"].as_str();
    let mut pids = supervisors(run);
    pids.retain(|pid| {
        let pid = pid.as_str();
        let named = Some(pid) == from || Some(pid) == h["from_sup"].as_str();
        let runs = || session.is_some_and(|s| first_line(&run.join(format!("{pid}.session"))) == s);
        Some(pid) != new && (named || runs()) && alive(pid)
    });
    if let Some(from) = from.filter(|f| pids.is_empty() && Some(*f) != new && alive(f)) {
        pids.push(from.into());
    }
    pids
}

/// A handover started at `started` failed: each old one whose Stop [`handed`] ended its /goal is woken to carry on, its
/// `.handed` gone, so it is never left idle with the lock given back (/code-review of handover-ends-old).
pub fn unhand(run: &Path, started: u64) {
    for pid in supervisors(run) {
        let handed = run.join(format!("{pid}.handed"));
        if first_line(&handed) == started.to_string() && fs::remove_file(&handed).is_ok() {
            append(
                &run.join(format!("{pid}.wake")),
                "The account handover failed: you keep the orchestrator lock. Carry on as /orchestrator, your /goal set \
                 again if one ran.\n",
            );
        }
    }
}

/// `"<key>" *: *"<value>"` somewhere in the raw input, as the shell's `=~` read it
fn source_is(input: &str, value: &str) -> bool {
    regex::Regex::new(&format!(r#""source" *: *"{value}""#)).is_ok_and(|r| r.is_match(input))
}

/// An approval this session does not run yet: `a` the newest (.claude/run/approved, bin/sandbox-apply), `l` the one its
/// supervisor loaded (<pid>.supervisor), `m` the notice, empty when there is none. Not for the session that asked
/// (private/dashboard/bin/dash approval): the supervisor moves it by itself; not once it asked (<pid>.restart). At once, at the
/// end of the turn, never on his next message (@nha, 2026-10-09, in klar: « it restarted when I send the message in the
/// session instead, this is wrong »). The main session only (task dev-guards-2; @nha, 2026-10-22: « YES. ALWAYS. ALWAYS. »):
/// a task or review session's DASH_SESSION is its task id, and a restart mid-work costs its turn.
struct Notice {
    a: String,
    l: String,
    m: String,
}

fn notice(run: &Path, p: &str) -> Notice {
    let none = Notice {
        a: String::new(),
        l: String::new(),
        m: String::new(),
    };
    let session = env("DASH_SESSION");
    if !session.is_empty() && session != "session" && session != env("HERDR_PANE_ID") {
        return none;
    }
    let a = first_line(&run.join("approved"));
    let l = first_line(&run.join(format!("{p}.supervisor")));
    let asked =
        run.join(format!("{p}.asked")).exists() || run.join(format!("{p}.restart")).exists();
    if a.is_empty() || l.is_empty() || a == l || asked {
        return Notice { a, l, ..none };
    }
    let m = format!(
        "Nicolas approved {}, a sandbox or host change this session does not run yet. Restart into it: run private/dashboard/bin/dash restart \"<what to resume>\" as the last step of this turn (after answering his message, if he sent one). The restart waits for your own background agents and jobs; a watcher that never ends (wake-wait, the mail watch, a monitor) does not count. Stop any job you no longer need.",
        chars(&a, 12)
    );
    Notice { a, l, m }
}

/// The OSC 7501 report a session-state event makes, as herdr-core reads a program's own status (its status.rs: `state=` then
/// `:`-separated fields, ended by ST): his message or a tool call is work, an AskUserQuestion a dialog, a permission prompt a
/// permission, the turn's end done (task rust-watch-osc7501; @nha, 2026-11-13: « we should probably use that directly for our
/// watcher then »). A dialog answered is work again at its PostToolUse.
/// ponytail: wired in site_perso's .claude/settings.json only, beside the supervisor's busy, tool and idle; other repos' tasks
/// get their status from herdr itself today. Their sessions need these hooks in ~/.claude/settings.json once hc runs them
/// (herdr-core-live-test). A parallel tool's PostToolUse can say working over an open prompt: tasks run with permissions
/// skipped and AskUserQuestion comes alone; key the prompt by tool_use_id if a stop is missed for it.
pub fn osc7501(what: &str) -> Option<&'static str> {
    Some(match what {
        "busy" | "tool" => "\x1b]7501;state=working:app=claude\x1b\\",
        "asking" => "\x1b]7501;state=blocked:kind=question:app=claude\x1b\\",
        "permission" => "\x1b]7501;state=blocked:kind=permission:app=claude\x1b\\",
        "idle" => "\x1b]7501;state=done:app=claude\x1b\\",
        _ => return None,
    })
}

/// The hook's output to its session's terminal: `what`'s report whole, or nothing. A full pty drops it; one cut midway is
/// finished within 200 ms, since an OSC left open would swallow the pane's next frame (/code-review of PR 1570).
pub fn report(what: &str, tty: &mut impl Write) {
    use std::io::ErrorKind::{Interrupted, WouldBlock};
    let Some(bytes) = osc7501(what) else { return };
    let mut rest = bytes.as_bytes();
    for _ in 0..200 {
        match tty.write(rest) {
            Ok(n) if n == rest.len() => return,
            Ok(0) => return,
            Ok(n) => rest = &rest[n..],
            Err(e) if e.kind() == Interrupted => {}
            Err(e) if e.kind() == WouldBlock && rest.len() < bytes.len() => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(_) => return,
        }
    }
}

/// The session's terminal, opened to write: the hook's own, else the nearest ancestor's (Claude Code starts a hook with no
/// controlling terminal, as it does its Bash tool). None outside one.
fn terminal() -> Option<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // never blocks: a pty nobody drains drops the report rather than hold the hook (/code-review)
    let flags = libc::O_NONBLOCK | libc::O_NOCTTY;
    let open = |p: &Path| {
        fs::OpenOptions::new()
            .write(true)
            .custom_flags(flags)
            .open(p)
            .ok()
    };
    open(Path::new("/dev/tty")).or_else(|| ancestor_tty().as_deref().and_then(open))
}

/// The first ancestor's controlling terminal: a pty's slave, major 16, is /dev/ttys<minor>, checked by one stat, never a scan
/// of /dev on each tool call (/code-review).
#[cfg(target_os = "macos")]
fn ancestor_tty() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let mut pid = std::process::id() as i32;
    for _ in 0..16 {
        let info = bsdinfo(pid)?;
        // NODEV, (dev_t)-1, is no terminal
        if info.e_tdev != u32::MAX && info.e_tdev != 0 {
            let dev = u64::from(info.e_tdev);
            let tty = PathBuf::from(format!("/dev/ttys{:03}", dev & 0xff_ffff));
            return (dev >> 24 == 16 && fs::metadata(&tty).is_ok_and(|m| m.rdev() == dev))
                .then_some(tty);
        }
        if info.pbi_ppid <= 1 {
            return None;
        }
        pid = info.pbi_ppid as i32;
    }
    None
}

#[cfg(target_os = "macos")]
fn bsdinfo(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let got =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (got == size).then_some(info)
}

/// The parent of process `pid`, none for one gone or launchd's own child: by it dash's handover finds the supervisor make
/// started under a restarted one's pid.
#[cfg(target_os = "macos")]
pub fn parent(pid: &str) -> Option<String> {
    let ppid = bsdinfo(pid.parse().ok()?)?.pbi_ppid;
    (ppid > 1).then(|| ppid.to_string())
}

/// ponytail: no parent read off macOS, where no orchestrator runs; /proc/<pid>/stat's ppid if one does.
#[cfg(not(target_os = "macos"))]
pub fn parent(_pid: &str) -> Option<String> {
    None
}

/// ponytail: Linux hooks rely on /dev/tty alone; walk /proc/<pid>/stat's tty_nr if one runs detached there.
#[cfg(not(target_os = "macos"))]
fn ancestor_tty() -> Option<PathBuf> {
    None
}

/// `dash hook session-state start|busy|tool|idle|asking|permission`, Claude Code's JSON on stdin. Each of busy, tool, asking,
/// permission and idle first writes its OSC 7501 report to the session's terminal (`osc7501`), supervised or not. Then it
/// tells bin/claude-supervise, which runs this session on the host, what it cannot see from outside. SessionStart calls start,
/// UserPromptSubmit busy, PostToolUse tool, PreToolUse on AskUserQuestion asking, PermissionRequest permission and Stop idle; it writes .claude/run/<supervisor pid>.{session,state,agents,jobs,notified} and reads <pid>.wake
/// (bin/comment-wake) when no wake-wait took it first. Without CLAUDE_SUPERVISOR (a session not started by make claude) it
/// does nothing.
///   busy  tells the session of an approval it does not run yet, at each of his messages until it asks to restart, and once
///         of a change he refused (.claude/run/refused, written by bin/sandbox-apply).
///   tool  the same approval notice, once per approval: a session mid-turn hears of it at its next tool call.
///   idle  writes the running background tasks of the Stop payload to <pid>.agents (graal's and klar's fix, 2026-10-05: the
///         session's own list, not a guess from transcript times).
pub fn session_state(what: &str, mut out: fs::File) {
    // A helper `claude -p` inside the session inherits CLAUDE_SUPERVISOR and the terminal: its id would be resumed in place of
    // the session's, and its Stop would say idle, or done to the terminal, mid-turn. A supervised session is interactive
    // (« cli »), measured 2026-11-07.
    if env("CLAUDE_CODE_ENTRYPOINT").starts_with("sdk-") {
        return;
    }
    let input = stdin();
    // a subagent's tool call says nothing: its `working` would overwrite its session's open dialog (/code-review)
    let subagent = what == "tool" && input.contains("\"agent_id\"");
    if let Some(mut tty) = osc7501(what).filter(|_| !subagent).and_then(|_| terminal()) {
        report(what, &mut tty);
    }
    let sup = env("CLAUDE_SUPERVISOR");
    if sup.is_empty() || matches!(what, "asking" | "permission") {
        return;
    }
    let project = crate::project_dir().unwrap_or_else(|| ".".into());
    let run = Path::new(&project).join(".claude/run");
    let file = |ext: &str| run.join(format!("{sup}.{ext}"));
    // The hottest path: nothing parsed, nothing logged unless it speaks. A subagent's tool call (its payload carries
    // agent_id) never takes the notice: the restart is the main session's.
    if what == "tool" {
        if input.contains("\"agent_id\"") {
            return;
        }
        // a tool call of the main session means a turn runs, also one his message did not start. ponytail: a turn is seen
        // busy from its first tool call on, not before it.
        if first_line(&file("state")) == "idle" {
            write(&file("state"), "busy\n");
            resumed(&run, &sup);
        }
        let n = notice(&run, &sup);
        if n.m.is_empty() || first_line(&file("notified")) == n.a {
            return;
        }
        write(&file("notified"), &format!("{}\n", n.a));
        let now = local_now();
        append(
            &run.join(format!("session-state.{}.log", &now[..10])),
            &format!("{now} {sup} tool told={}\n", chars(&n.a, 12)),
        );
        say(&mut out, "PostToolUse", &n.m);
        return;
    }
    let _ = fs::create_dir_all(&run);
    let parsed: Option<Value> = serde_json::from_str(&input).ok();
    let id = parsed
        .as_ref()
        .and_then(|d| match &d["session_id"] {
            Value::String(s) => Some(s.clone()),
            Value::Null | Value::Bool(false) => None,
            v => Some(v.to_string()),
        })
        .unwrap_or_default();
    if !id.is_empty() {
        write(&file("session"), &format!("{id}\n"));
    }
    let home = PathBuf::from(env("HOME"));
    let threads = home.join(".claude/run/threads");
    let mut takeover = None;
    if what == "start" && !id.is_empty() && !source_is(&input, "compact") {
        takeover = thread_start(&threads, &id, &input, parsed.as_ref());
    }
    if what == "busy" && !id.is_empty() && !threads.join(format!("{id}.prompt")).exists() {
        let prompt = parsed
            .as_ref()
            .and_then(|d| d["prompt"].as_str())
            .unwrap_or("");
        if !prompt.trim().is_empty() {
            write(&threads.join(format!("{id}.prompt")), prompt);
        }
    }
    // A supervisor from before the approved-copy model never writes this mark, and never restarts on an approval.
    if what == "start" && !file("supervisor").exists() {
        let _ = writeln!(
            out,
            "This session runs under an old supervisor (pid {sup}): an approval will not restart it. Tell Nicolas, once, in this terminal: quit claude, then CLAUDE_RESUME_ID={id} make -C {} claude",
            env("CLAUDE_PROJECT_DIR")
        );
    }
    // One line per event, kept, so a restart that waited can be explained afterwards (@nha, 2026-10-06: « it seems the
    // session waited for me to say "approved sandbox" to THEN restart », and nothing was left to say why). One file per
    // day, appended only; only a start prunes files over a week old (graal's fix, broadcast 2026-10-07).
    if what == "start" {
        prune(&run, Duration::from_secs(8 * 86_400));
    }
    match what {
        // Only a launch is between turns: a /compact, a /clear or a resume leaves the state as it was (graal's review of its
        // PR #419, 2026-10-09).
        "start" => {
            if source_is(&input, "startup") {
                write(&file("state"), "idle\n");
            }
        }
        "busy" => {
            write(&file("state"), "busy\n");
            resumed(&run, &sup);
            let n = notice(&run, &sup);
            let mut ctx = n.m.clone();
            // told here: the turn's first tool call does not repeat it
            if !n.m.is_empty() {
                write(&file("notified"), &format!("{}\n", n.a));
            }
            // asked already but held by its own background tasks: say so
            let agents = fs::metadata(file("agents")).is_ok_and(|m| m.len() > 0);
            let asked = file("restart").exists()
                || (file("asked").exists() && !n.a.is_empty() && n.a != n.l);
            if n.m.is_empty() && agents && asked {
                ctx = format!(
                    "The restart this session asked for waits on its background tasks: {}. It happens at the end of the turn after they end; stop any you no longer need.",
                    paste(&file("agents"), ";")
                );
            }
            let r = cat(&run.join("refused"));
            if !r.is_empty() && r != cat(&run.join("refused.seen")) {
                write(&run.join("refused.seen"), &format!("{r}\n"));
                let why = r.split_once(' ').map_or(r.as_str(), |(_, w)| w);
                ctx = joined(
                    &ctx,
                    &format!(
                        "Nicolas refused the change at {}: {why}. Rework it (or drop it), then hand it over again.",
                        chars(&r, 12)
                    ),
                );
            }
            // a wake no wake-wait took (none running, or his message came first): handed over with his message
            if fs::rename(file("wake"), file("wake.told")).is_ok() {
                ctx = joined(&ctx, &paste(&file("wake.told"), " "));
                let _ = fs::remove_file(file("wake.told"));
            }
            if !ctx.is_empty() {
                say(&mut out, "UserPromptSubmit", &ctx);
            }
        }
        "idle" => {
            if let Some(d) = &parsed {
                let (held, jobs) = background(d);
                write(&file("agents"), &lines(&held));
                write(&file("jobs"), &lines(&jobs));
            }
            write(&file("state"), "idle\n");
            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);
            if let Some(stop) = handed(&run, &sup, &id, now.map_or(0, |d| d.as_secs())) {
                let _ = writeln!(out, "{stop}");
            }
        }
        _ => {}
    }
    let agents = fs::read_to_string(file("agents"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .count();
    let now = local_now();
    append(
        &run.join(format!("session-state.{}.log", &now[..10])),
        &format!(
            "{now} {sup} {what} state={} approved={} asked={} agents={agents}\n",
            cat(&file("state")),
            chars(&cat(&run.join("approved")), 12),
            if file("asked").exists() { "y" } else { "n" },
        ),
    );
    // a session new to an orchestrator thread takes it over: what the one before had in flight, before it acts. Last, so a
    // hook cut at its timeout has written every file above (/code-review).
    if let Some((thread, predecessor, starts)) = takeover {
        let repo = Path::new(&project);
        let live = &crate::takeover::live;
        let text = crate::takeover::text(&home, repo, &thread, Some(&predecessor), &starts, live);
        let _ = write!(out, "{}", crate::takeover::context(text));
    }
}

fn joined(ctx: &str, more: &str) -> String {
    if ctx.is_empty() {
        more.into()
    } else {
        format!("{ctx} {more}")
    }
}

fn lines(v: &[String]) -> String {
    v.iter().map(|x| format!("{x}\n")).collect()
}

fn append(p: &Path, line: &str) {
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// `find run -maxdepth 1 -name 'session-state*' -mtime +7 -delete`: what is `older` than that goes, dash restart's passover
/// notes too (read once, by the resumed session's check; /code-review high of PR 1631).
fn prune(run: &Path, older: Duration) {
    let Ok(entries) = fs::read_dir(run) else {
        return;
    };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age >= older);
        let name = e.file_name().to_string_lossy().into_owned();
        if old && (name.starts_with("session-state") || name.starts_with("passover-restart-")) {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Python's `str()` of a field, `default` when the key is missing: the line format bin/supervise-test case 2 reads.
fn py(v: &Value, key: &str, default: &str) -> String {
    match v.get(key) {
        None => default.into(),
        Some(Value::Null) => "None".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => (if *b { "True" } else { "False" }).into(),
        Some(x) => x.to_string(),
    }
}

/// What still runs after the turn dies with a restart: (held, jobs). Held for agents, workflows and the shells the session
/// started (klar: a dev server or a CI wait dies with the tree too). Not held for monitors and the watchers a session
/// relaunches forever (comment watch, mail watch, wake-wait), which would hold every restart: they go to jobs, and the
/// restart notice says to relaunch them. No background_tasks in the payload (an older Claude Code): held, saying why.
fn background(d: &Value) -> (Vec<String>, Vec<String>) {
    let (mut held, mut jobs) = (vec![], vec![]);
    let tasks = &d["background_tasks"];
    if tasks.is_null() {
        held.push("unknown: this Stop payload has no background_tasks".into());
    }
    let watcher = regex::Regex::new("comment-watch|watch comments|mail-watch|mail watch|wake-wait")
        .expect("the watchers' pattern");
    for t in tasks
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t.is_object())
    {
        if t["status"].as_str() != Some("running") || t["type"].as_str() == Some("monitor") {
            continue;
        }
        // the command first and whole: description first and a 120 cut left « priv/commands/wa » in the note (2026-10-21)
        let command = if truthy(&t["command"]) {
            py(t, "command", "")
        } else {
            String::new()
        };
        let described = if truthy(&t["description"]) {
            format!(" ({})", py(t, "description", ""))
        } else {
            String::new()
        };
        let what = format!("{command}{described}").trim().to_string();
        let line = format!(
            "{} {} {}",
            py(t, "type", "?"),
            py(t, "id", "?"),
            chars(&what, 400)
        );
        if t["type"].as_str() == Some("shell") && watcher.is_match(&what) {
            jobs.push(line);
        } else {
            held.push(line);
        }
    }
    (held, jobs)
}

/// The thread record (task session-threads; @nha, 2026-10-30: « persistent sessions have their prompt, session id (for
/// --resume) and tab/pane saved and can be either resumed (if tokens are not stale) or restarted (if context is too old) »):
/// a start line per launch, resume or /clear in ~/.claude/run/threads.jsonl, which Dashboard.Threads replays after a reboot
/// and bin/claude-account reads for a resume's account; the first message of each session id beside it, threads/<id>.prompt.
/// The thread: the task (a reviewer's is review-<id>), the orchestrator in the main checkout, else the pane. An orchestrator's
/// line keeps its predecessor, the Claude id before it on the thread (task orch-takeover-state), carried by a resume of the
/// same id; returned with the thread and the record's start lines only when this id is new to the thread, a takeover (a
/// /clear too: its session keeps nothing of the work in flight).
fn thread_start(
    threads: &Path,
    id: &str,
    input: &str,
    parsed: Option<&Value>,
) -> Option<(String, String, Vec<Value>)> {
    let (session, pane) = (env("DASH_SESSION"), env("HERDR_PANE_ID"));
    let project = crate::project_dir();
    let checkout = PathBuf::from(project.clone().unwrap_or_else(|| ".".into()));
    // a task's DASH_SESSION first; a delegate (HERDR_DELEGATE) in the main checkout is never its orchestrator
    let thread = if !session.is_empty() && session != pane && session != "session" {
        session
    } else if checkout.join(".git").is_dir() && env("HERDR_DELEGATE").is_empty() {
        let here = project.clone().unwrap_or_else(|| env("PWD"));
        let name = here
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        format!("orchestrator:{name}")
    } else {
        format!("pane:{}", if pane.is_empty() { "none" } else { &pane })
    };
    let config = env("CLAUDE_CONFIG_DIR");
    let accounts = format!("{}/.claude/accounts/", env("HOME"));
    let account = match config.strip_prefix(&accounts) {
        Some(rest) if !rest.is_empty() => config.rsplit('/').next().unwrap_or("").to_string(),
        _ => "default".into(),
    };
    let _ = fs::create_dir_all(threads);
    let record = threads.with_extension("jsonl");
    let starts = match thread.starts_with("orchestrator:") {
        true => crate::takeover::starts(&record),
        false => vec![],
    };
    let mine: Vec<&Value> = (starts.iter())
        .filter(|e| e["thread"] == thread.as_str())
        .collect();
    // an id the thread held before (a resume of an older session) is no takeover and keeps its own predecessor (/code-review)
    let seen = mine.iter().rev().find(|e| e["id"] == id);
    let (predecessor, new) = match (seen, mine.last()) {
        (Some(e), _) => (e["predecessor"].as_str().map(String::from), false),
        // an empty id names no session (/code-review of PR 1635)
        (None, Some(e)) => (
            e["id"].as_str().filter(|s| !s.is_empty()).map(String::from),
            true,
        ),
        (None, None) => (None, false),
    };
    // an input that is not JSON wrote no line, as python's json.load failed
    if let Some(d) = parsed {
        let source = if truthy(&d["source"]) {
            py(d, "source", "")
        } else {
            String::new()
        };
        let cwd = project.unwrap_or_else(|| env("PWD"));
        let at = utc_now();
        let fields = [
            ("ev", "start"),
            ("at", at.as_str()),
            ("source", source.as_str()),
            ("thread", thread.as_str()),
            ("id", id),
            ("account", account.as_str()),
            ("workspace", &env("HERDR_WORKSPACE_ID")),
            ("tab", &env("HERDR_TAB_ID")),
            ("pane", &env("HERDR_PANE_ID")),
            ("cwd", cwd.as_str()),
        ];
        let more = predecessor.as_deref().map(|p| ("predecessor", p));
        let body: Vec<String> = (fields.iter().copied().chain(more))
            .map(|(k, v)| format!("\"{k}\":{}", Value::from(v)))
            .collect();
        append(&record, &format!("{{{}}}\n", body.join(",")));
    }
    // a resume's first message is its resume note, never its prompt: an empty file keeps it out, so a restart falls back to
    // the thread's own words
    let prompt = threads.join(format!("{id}.prompt"));
    if source_is(input, "resume") && !prompt.exists() {
        write(&prompt, "");
    }
    let predecessor = predecessor.filter(|_| new && parsed.is_some())?;
    // read again, this session's own line in: its pane is the orchestrator's, never the task that pane held before
    // (/code-review high of PR 1635)
    Some((thread, predecessor, crate::takeover::starts(&record)))
}

/// `YYYY-MM-DDTHH:MM:SSZ`, now
fn utc_now() -> String {
    let s = (crate::host().now_ms)().div_euclid(1000);
    let (day, sec) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        dash_core::date(day),
        sec / 3600,
        sec % 3600 / 60,
        sec % 60
    )
}

/// `dash hook statusline`, the status line command: records Claude Code's own usage figures for the runner and /status
/// (tasks usage-budget, claude-accounts; Dashboard.Orch.Budget and Dashboard.ClaudeAccounts read them). Claude Code hands
/// the status line its `rate_limits` on stdin, and nothing else on this Mac keeps it. One file per Claude session (its
/// session_id, never a pid), whole and atomic; the reader takes the highest figure of the newest file's window and ignores
/// files older than a window, so nothing here deletes. Written only when a window's figure changed: a redraw of an idle pane
/// on the old account never makes its file the newest (/code-review of #367). It runs on each refresh, the app up or down.
pub fn statusline(mut out: fs::File) {
    let dir = std::env::var("CLAUDE_WEEKLY_LIMIT_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map_or_else(
            || PathBuf::from(env("HOME")).join(".claude/weekly-limit"),
            PathBuf::from,
        );
    let input = stdin();
    if input.contains("\"seven_day\"") || input.contains("\"five_hour\"") {
        record(&dir, &input, signed_in().as_deref());
    }
    // the user-level status line forwards to Orca when its hook env is set: this one replaces it here, so it forwards too
    let orca = PathBuf::from(env("HOME")).join(".orca/agent-hooks/claude-statusline.sh");
    if !env("ORCA_AGENT_HOOK_PORT").is_empty() && crate::is_exec(&orca) {
        let child = std::process::Command::new("/bin/sh")
            .arg(&orca)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut i) = child.stdin.take() {
                let _ = i.write_all(input.as_bytes());
            }
            if let Ok(o) = child.wait_with_output() {
                let _ = out.write_all(&o.stdout);
            }
        }
    }
}

/// The email of the login this session's profile holds now, from its `.claude.json` (`~/.claude.json` for `default`, which
/// sets no `CLAUDE_CONFIG_DIR`). None: signed out or unread.
fn signed_in() -> Option<String> {
    let config = env("CLAUDE_CONFIG_DIR");
    let home = PathBuf::from(env("HOME"));
    // a profile's own file only: one missing mid-login is no login, never default's (/code-review)
    let file = match config.is_empty() {
        true => home.join(".claude.json"),
        false => PathBuf::from(&config).join(".claude.json"),
    };
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(file).ok()?).ok()?;
    json["oauthAccount"]["emailAddress"]
        .as_str()
        .map(String::from)
}

/// The figures of one status line input, kept in `<dir>/<session id>.json` when its windows changed; the profile it ran
/// under first (`config_dir`), then the login it held (`login`, its email): dash_agent reads a figure by its login, never by
/// the profile, which may hold another login since (task account-ends-rest). ponytail: the path goes in unescaped; a
/// profile path holds no quote or backslash (dash claude-accounts add's names).
pub(crate) fn record(dir: &Path, input: &str, login: Option<&str>) {
    // sed's greedy `.*"session_id"`: the last one on the first line that holds one
    let sid = regex::Regex::new(r#""session_id" *: *"([A-Za-z0-9_-]*)""#).expect("sid");
    let id = input
        .lines()
        .find_map(|l| sid.captures_iter(l).last().map(|c| c[1].to_string()))
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "unknown".into());
    let re = regex::Regex::new(r#""(five_hour|seven_day)" *: *\{[^}]*\}"#).expect("limits");
    let limits = |text: &str| {
        re.find_iter(text)
            .map(|m| m.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let kept = fs::read_to_string(dir.join(format!("{id}.json"))).unwrap_or_default();
    // the same figures under the same login rewrite nothing; a login changed is written (/code-review)
    let stamp = login.map(|l| format!("\"login\":{},", serde_json::json!(l)));
    let same_login = stamp.as_deref().is_none_or(|s| kept.contains(s));
    if limits(input) == limits(&kept) && same_login {
        return;
    }
    let body = match input.strip_prefix('{') {
        Some(rest) => {
            let login = stamp.unwrap_or_default();
            format!(
                "{{\"config_dir\":\"{}\",{login}{rest}",
                env("CLAUDE_CONFIG_DIR")
            )
        }
        None => input.to_string(),
    };
    let tmp = dir.join(format!("{id}.tmp"));
    let _ = fs::create_dir_all(dir)
        .and_then(|()| fs::write(&tmp, body))
        .and_then(|()| fs::rename(&tmp, dir.join(format!("{id}.json"))));
}

/// `dash hook gh-meter-path`, SessionStart: the gh shim first on this session's PATH, so each gh call is counted, cached
/// and paced (task gh-meter; the shim's header says how). Claude Code sources CLAUDE_ENV_FILE before each Bash command; the
/// shim's own folder holds gh only.
pub fn gh_meter_path() {
    let project = crate::project_dir().map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        PathBuf::from,
    );
    let shim = project.join("private/dashboard/priv/gh-meter");
    let file = env("CLAUDE_ENV_FILE");
    if file.is_empty() || !crate::is_exec(&shim.join("gh")) {
        return;
    }
    let shim = shim.display().to_string();
    if !fs::read_to_string(&file)
        .unwrap_or_default()
        .contains(&shim)
    {
        append(Path::new(&file), &format!("export PATH=\"{shim}:$PATH\"\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A start prunes the week-old session-state files and dash restart's passover notes, nothing else of the run folder.
    #[dash_meter::dash_test]
    fn a_start_prunes_old_session_states_and_passover_notes() {
        let run = dash_meter::TestDir::new("prune");
        let old = SystemTime::now() - Duration::from_secs(9 * 86_400);
        for name in [
            "session-state-s1",
            "passover-restart-1.md",
            "4242.restart-note",
            "passover-restart-2.md",
        ] {
            let f = fs::File::create(run.0.join(name)).expect("file");
            if name != "passover-restart-2.md" {
                f.set_modified(old).expect("mtime");
            }
        }
        prune(&run.0, Duration::from_secs(8 * 86_400));
        let mut left: Vec<String> = (fs::read_dir(&run.0).expect("read").flatten())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["4242.restart-note", "passover-restart-2.md"]);
    }

    /// Done-when 1 of rust-watch-osc7501: the hook's output bytes for each event, as herdr-core's status.rs encodes a report
    /// (OSC, 7501, `;`, `state=` first, `:` between fields, ST): working, blocked on a dialog or a permission, done; any other
    /// event writes nothing.
    #[dash_meter::dash_test]
    fn each_session_event_writes_its_osc_7501_report() {
        let bytes = |what: &str| {
            let mut tty = vec![];
            report(what, &mut tty);
            tty
        };
        let want = [
            ("busy", "state=working:app=claude"),
            ("tool", "state=working:app=claude"),
            ("asking", "state=blocked:kind=question:app=claude"),
            ("permission", "state=blocked:kind=permission:app=claude"),
            ("idle", "state=done:app=claude"),
        ];
        for (what, fields) in want {
            let said = bytes(what);
            assert_eq!(
                said,
                format!("\x1b]7501;{fields}\x1b\\").into_bytes(),
                "{what}"
            );
            // read back as hc reads it: the fields between the OSC's `;` and ST
            let body = std::str::from_utf8(&said[7..said.len() - 2]).expect("text");
            let state = body.split(':').find_map(|f| f.strip_prefix("state="));
            eprintln!(
                "{what} -> {:?} state={state:?}",
                String::from_utf8_lossy(&said)
            );
        }
        assert!(bytes("start").is_empty() && bytes("statusline").is_empty());
        // a pty that takes `takes` bytes per write, refusing every other write: all or nothing, never an OSC left open
        struct Pty {
            got: Vec<u8>,
            takes: usize,
            refuse: bool,
        }
        impl Write for Pty {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.refuse = !self.refuse;
                if !self.refuse || self.takes == 0 {
                    return Err(std::io::ErrorKind::WouldBlock.into());
                }
                let n = b.len().min(self.takes);
                self.got.extend(&b[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let through = |takes| {
            let mut pty = Pty {
                got: vec![],
                takes,
                refuse: false,
            };
            report("idle", &mut pty);
            pty.got
        };
        assert_eq!(through(5), osc7501("idle").expect("done").as_bytes());
        assert!(through(0).is_empty(), "a full pty drops it whole");
    }

    /// The walk up to a terminal: under a session (a pane's claude above nextest) it names a tty device; in CI, none. Nothing
    /// is written to it.
    #[dash_meter::dash_test]
    fn the_ancestor_s_terminal_is_a_tty_device() {
        use std::os::unix::fs::FileTypeExt;
        let found = ancestor_tty();
        eprintln!("ancestor tty: {found:?}");
        if let Some(p) = found {
            assert!(p.to_string_lossy().starts_with("/dev/tty"), "{p:?}");
            assert!(
                fs::metadata(&p)
                    .expect("there")
                    .file_type()
                    .is_char_device()
            );
        }
    }

    /// Done-when 3 of rust-hooks-rest-port, the Stop half: the background tasks' lines bin/supervise-test case 2 reads,
    /// watchers to jobs, monitors and ended ones dropped, no list held with its reason.
    #[dash_meter::dash_test]
    fn a_stop_payload_splits_its_running_tasks_into_held_and_jobs() {
        let d = json!({"background_tasks": [
            {"type": "shell", "id": "b1", "status": "running", "command": "dash watch comments", "description": "watch"},
            {"type": "subagent", "id": "a9", "status": "running", "description": "review"},
            {"type": "monitor", "id": "m1", "status": "running", "command": "x"},
            {"type": "shell", "id": "b2", "status": "completed", "command": "y"},
            {"id": "b3", "status": "running", "command": "c".repeat(500)}]});
        let (held, jobs) = background(&d);
        assert_eq!(jobs, ["shell b1 dash watch comments (watch)"]);
        assert_eq!(held[0], "subagent a9 (review)");
        assert_eq!(held[1], format!("? b3 {}", "c".repeat(400)));
        assert_eq!(held.len(), 2);
        let (held, jobs) = background(&json!({"session_id": "s1"}));
        assert_eq!(held, ["unknown: this Stop payload has no background_tasks"]);
        assert!(jobs.is_empty());
    }

    /// statusline-usage.sh's rule: a file per session, rewritten only when a window's figure changed, the profile first.
    #[dash_meter::dash_test]
    fn the_status_line_keeps_a_session_s_figures_only_when_they_change() {
        let dir = std::env::temp_dir().join(format!("statusline-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let input = r#"{"session_id":"s-1","rate_limits":{"seven_day":{"used_percentage":40,"resets_at":1}}}"#;
        record(&dir, input, Some("a@x"));
        let kept = fs::read_to_string(dir.join("s-1.json")).expect("written");
        assert!(kept.starts_with("{\"config_dir\":\""), "{kept}");
        assert!(kept.contains(r#","login":"a@x","session_id""#), "{kept}");
        assert!(kept.ends_with(r#""session_id":"s-1","rate_limits":{"seven_day":{"used_percentage":40,"resets_at":1}}}"#));
        let older = fs::metadata(dir.join("s-1.json"))
            .and_then(|m| m.modified())
            .expect("mtime");
        std::thread::sleep(Duration::from_millis(20));
        record(
            &dir,
            &input.replace("\"rate_limits\"", "\"cost\":1,\"rate_limits\""),
            Some("a@x"),
        );
        let same = fs::metadata(dir.join("s-1.json"))
            .and_then(|m| m.modified())
            .expect("mtime");
        assert_eq!(older, same, "the same figures rewrite nothing");
        record(&dir, &input.replace("40", "41"), Some("a@x"));
        assert!(
            fs::read_to_string(dir.join("s-1.json"))
                .expect("read")
                .contains("41")
        );
        record(&dir, &input.replace("40", "41"), Some("b@x"));
        let moved = fs::read_to_string(dir.join("s-1.json")).expect("read");
        assert!(
            moved.contains(r#""login":"b@x""#),
            "the same figures under another login are written: {moved}"
        );
        record(&dir, r#"{"five_hour":{"used_percentage":1}}"#, None);
        assert!(dir.join("unknown.json").is_file(), "no session id: unknown");
        let _ = fs::remove_dir_all(&dir);
    }
}
