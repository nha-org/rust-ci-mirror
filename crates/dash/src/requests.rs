//! Host request jobs (task rust-host-requests; @nha, 2026-11-11: « I think that one is on you »): every `<name> host request
//! <command>` line of sandbox/jobs.conf, served by `dash launchd run` itself, where bin/tailscale-bridge, denials-bridge and
//! slack-bridge each ran as a launchd job of their own. A session cannot run these inside its sandbox (tailscale fails on its
//! preferences, `log` refuses, `open --args` drops the arguments, `brew install` gets « sandbox-exec: Operation not permitted »),
//! so it writes `.claude/run/<name>.request` in its checkout: line 1 the value, line 2 `id=<token>`. The supervisor runs the
//! line's command, its one `{a|b}` or `{lo-hi}` slot filled with that value and nothing else, and answers in `<name>.out`: the
//! output, `exit=<code>`, then `asked=<token>` when the request had one. The main checkout and each of its worktrees are served.
//! The command comes from the host copy's jobs.conf, which follows main: a new formula for brew is a commit to that line.
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One `<name> host request <command>` line.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub name: String,
    pub argv: Vec<String>,
}

/// jobs.conf's words, as bin/sandbox-apply's `words`: split at blanks, '…' and "…" kept whole without their quotes, a {…} slot
/// kept whole with its braces and blanks. `None` on an unclosed quote or brace.
fn words(line: &str) -> Option<Vec<String>> {
    let (mut out, mut w, mut quote, mut brace, mut has) =
        (vec![], String::new(), None, false, false);
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => w.push(c),
            None if brace => {
                w.push(c);
                brace = c != '}';
            }
            None if c == '{' => {
                w.push(c);
                brace = true;
            }
            None if c == '\'' || c == '"' => (quote, has) = (Some(c), true),
            None if c.is_whitespace() => {
                if has || !w.is_empty() {
                    out.push(std::mem::take(&mut w));
                }
                has = false;
            }
            None => w.push(c),
        }
    }
    if quote.is_some() || brace {
        return None;
    }
    if has || !w.is_empty() {
        out.push(w);
    }
    Some(out)
}

/// The request jobs of jobs.conf's text.
pub fn jobs(conf: &str) -> Vec<Job> {
    conf.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(words)
        .filter_map(|ws| match &ws[..] {
            [name, profile, trigger, argv @ ..]
                if profile == "host" && trigger == "request" && !argv.is_empty() =>
            {
                Some(Job {
                    name: name.clone(),
                    argv: argv.to_vec(),
                })
            }
            _ => None,
        })
        .collect()
}

/// A {…} word's parts: before, the slot, after.
fn slot(w: &str) -> Option<(&str, &str, &str)> {
    let (a, b) = (w.find('{')?, w.rfind('}')?);
    (a < b).then(|| (&w[..a], &w[a + 1..b], &w[b + 1..]))
}

/// The command, its one slot filled with `value`: one of the slot's words exactly (a word of two, `serve status`, is two words
/// of the command), or an integer in its range. Anything else is refused (bin/sandbox-apply's host_job, rfc-approve-policy U5).
pub fn fill(argv: &[String], value: &str) -> Result<Vec<String>, String> {
    let mut out = vec![];
    let mut filled = false;
    for w in argv {
        let Some((pre, s, post)) = slot(w) else {
            out.push(w.clone());
            continue;
        };
        if filled {
            return Err("two slots in its line".into());
        }
        filled = true;
        let range = s
            .split_once('-')
            .and_then(|(l, h)| Some((l.parse::<u64>().ok()?, h.parse::<u64>().ok()?)));
        if let Some((lo, hi)) = range {
            let v = Some(value)
                .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| (lo..=hi).contains(v))
                .ok_or(format!("refused « {value} »: only {lo} to {hi}"))?;
            out.push(format!("{pre}{v}{post}"));
        } else if s.split('|').any(|o| o == value) {
            out.extend(
                format!("{pre}{value}{post}")
                    .split_whitespace()
                    .map(str::to_string),
            );
        } else {
            return Err(format!(
                "refused « {value} »: only {}",
                s.split('|').collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if !filled && !value.is_empty() {
        return Err(format!("refused « {value} »: it takes no value"));
    }
    Ok(out)
}

fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// An answer: the body, `exit=`, then `asked=` when the request named one (an empty one too: a refused request with no id).
fn reply(body: &str, code: i32, id: Option<&str>) -> String {
    let mut out = body.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out += &format!("exit={code}\n");
    if let Some(id) = id {
        out += &format!("asked={id}\n");
    }
    out
}

/// A job's output, in a private file removed once dropped: read whole (a short answer) or a line at a time (a week of the
/// log), never all held in the supervisor's memory. `.1`: what went wrong before the job ran, said first.
struct Output(PathBuf, String);

impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl Output {
    fn text(&self) -> String {
        self.1.clone() + &String::from_utf8_lossy(&fs::read(&self.0).unwrap_or_default())
    }

    /// Its lines; a read that fails midway says so as a last line, so a cut answer is never taken for a whole one.
    fn lines(&self) -> impl Iterator<Item = String> {
        let mut file = File::open(&self.0).map(BufReader::new);
        let mut done = false;
        let read = std::iter::from_fn(move || {
            let r = file.as_mut().ok().filter(|_| !done)?;
            let mut b = vec![];
            match r.read_until(b'\n', &mut b) {
                Ok(0) => None,
                Ok(_) => {
                    if b.last() == Some(&b'\n') {
                        b.pop();
                    }
                    Some(String::from_utf8_lossy(&b).into_owned())
                }
                Err(e) => {
                    done = true;
                    Some(format!("read error, the lines after are missing: {e}"))
                }
            }
        });
        self.1
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
            .into_iter()
            .chain(read)
    }
}

/// A command, run in a process group of its own with stdout and stderr in one private file: past `limit` its group is killed,
/// exit 124, and the file says so. 127 when it does not start.
fn exec(argv: &[String], limit: Duration) -> (Output, i32) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let mut out = Output(
        std::env::temp_dir().join(format!("dash-request-{}-{n}", std::process::id())),
        String::new(),
    );
    let _ = fs::remove_file(&out.0);
    let mut file = match OpenOptions::new()
        .read(true)
        .append(true)
        .create_new(true)
        .mode(0o600)
        .open(&out.0)
    {
        Ok(f) => f,
        Err(e) => {
            out.1 = format!("{}: not run, no file for its output: {e}\n", argv[0]);
            return (out, 127);
        }
    };
    let spawned = file.try_clone().and_then(|o| {
        Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(o)
            .stderr(file.try_clone()?)
            .process_group(0)
            .spawn()
    });
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(file, "{}: {e}", argv[0]);
            return (out, 127);
        }
    };
    let until = Instant::now() + limit;
    let code = loop {
        if let Ok(Some(s)) = child.try_wait() {
            use std::os::unix::process::ExitStatusExt;
            break s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0));
        }
        if Instant::now() >= until {
            unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
            let _ = child.wait();
            // on a line of its own: a newline first only when the output does not end with one
            let mut last = *b"\n";
            let len = file.metadata().map_or(0, |m| m.len());
            if len > 0 {
                use std::os::unix::fs::FileExt;
                let _ = file.read_exact_at(&mut last, len - 1);
            }
            let nl = if last[0] == b'\n' { "" } else { "\n" };
            let _ = writeln!(file, "{nl}timed out after {} s", limit.as_secs());
            break 124;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    (out, code)
}

/// How long a job may run: tailscale 20 s (a command that waits for a click must not hold the next request), slack 90 s (its
/// quit, open and start), any other 30 minutes (a week of the log, a brew install).
fn limit(name: &str) -> Duration {
    match name {
        "tailscale" => Duration::from_secs(20),
        "slack" => Duration::from_secs(90),
        _ => Duration::from_secs(1800),
    }
}

fn is_link(p: &Path) -> bool {
    p.symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// A link at the run folder, its parent or one of a job's files there: a run folder is a session's to write and the supervisor
/// runs unsandboxed, so none is ever followed.
/// ponytail: an honest-oops guard, as the bridges' (SANDBOX.md); a session swapping a folder between check and open is out of scope.
fn planted(run: &Path, name: &str) -> bool {
    let files =
        ["request", "request.serving", "out", "out.tmp"].map(|f| run.join(format!("{name}.{f}")));
    let folders = [Some(run), run.parent()];
    folders.iter().flatten().any(|p| is_link(p))
        || files.iter().any(|p| is_link(p))
        || run.parent().is_none()
}

/// A request file read through no link, 4 KB at most.
fn read_own(own: &Path) -> Option<String> {
    let mut text = String::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(own)
        .and_then(|f| {
            if !f.metadata()?.is_file() {
                return Err(std::io::ErrorKind::InvalidInput.into());
            }
            f.take(4096).read_to_string(&mut text)
        })
        .ok()
        .map(|_| text)
}

/// A request claimed by rename to `<name>.request.serving`, so one written meanwhile is the next, never deleted unread. The
/// claimed file stays until its answer is written: a supervisor stopped meanwhile leaves it for the next to answer (`sweep`).
fn claim(run: &Path, name: &str) -> Option<String> {
    let req = run.join(format!("{name}.request"));
    let own = run.join(format!("{name}.request.serving"));
    if planted(run, name) || req.metadata().map_or(true, |m| m.len() == 0) {
        return None;
    }
    fs::rename(&req, &own).ok()?;
    let text = read_own(&own);
    if text.is_none() {
        let _ = fs::remove_file(&own);
    }
    text
}

/// A request an earlier supervisor claimed and never answered (stopped for a new build or a restart): answered now, so its
/// caller is told at once instead of waiting out the job's limit. Not run again: the job may have finished.
fn sweep(run: &Path, name: &str) {
    let own = run.join(format!("{name}.request.serving"));
    if planted(run, name) || !own.exists() {
        return;
    }
    let text = read_own(&own).unwrap_or_default();
    let _ = fs::remove_file(&own);
    let id = text
        .lines()
        .nth(1)
        .and_then(|l| l.trim().strip_prefix("id="))
        .filter(|i| is_id(i));
    // answered already, stopped only before its .serving went: that answer stands
    let out = fs::read_to_string(run.join(format!("{name}.out"))).unwrap_or_default();
    if id.is_some_and(|i| out.lines().any(|l| l == format!("asked={i}"))) {
        return;
    }
    // its job may still run in its own group: re-asking at once could start a second one (a second brew on brew's lock)
    let why = "interrupted: the host's dash launchd stopped while it ran (a new build or a restart); the job may still be running \
               or have finished: check its effect before asking again";
    answer(run, name, &reply(why, 75, id));
}

/// The answer, written whole: a new file (never through a link planted there), then renamed over `<name>.out`.
fn answer(run: &Path, name: &str, text: &str) {
    let tmp = run.join(format!("{name}.out.tmp"));
    let _ = fs::remove_file(&tmp);
    let wrote = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()));
    if wrote.is_ok() {
        let _ = fs::rename(&tmp, run.join(format!("{name}.out")));
    }
}

/// One request: its text read, its job run, the answer it gets. `main`: the main checkout's run folder.
pub fn serve(job: &Job, main: &Path, text: &str) -> String {
    let mut lines = text.lines();
    let value = lines.next().unwrap_or("").trim().to_string();
    let id = lines
        .next()
        .and_then(|l| l.trim().strip_prefix("id="))
        .filter(|i| is_id(i));
    match job.name.as_str() {
        "denials" => denials(job, &value, id),
        "slack" => slack(job, &value, id),
        _ => plain(job, main, &value, id),
    }
}

/// Any other job: its line filled with the value and run. Tailscale's `down`, once it took, leaves tailscale.down in the main
/// checkout's run folder, whichever checkout asked (his intent: bin/comment-wake's watchdog reads it there and brings no
/// Tailscale up while it exists), and `up` removes it.
fn plain(job: &Job, main: &Path, value: &str, id: Option<&str>) -> String {
    let argv = match fill(&job.argv, value) {
        Ok(a) => a,
        Err(e) => return reply(&format!("refused: {} {e}", job.name), 2, id),
    };
    let down = main.join("tailscale.down");
    let tailscale = job.name == "tailscale";
    if tailscale && value == "up" {
        let _ = fs::remove_file(&down);
    }
    let (out, code) = exec(&argv, limit(&job.name));
    let mut body = out.text();
    if tailscale && value == "down" && code == 0 {
        // the main checkout's folder, guarded as an asking one's; a marker not written is said, the watchdog would undo his down
        let made = match planted(main, "tailscale") || is_link(&down) {
            true => Err(std::io::Error::other("a link is planted there")),
            false => fs::create_dir_all(main).and_then(|()| File::create(&down).map(drop)),
        };
        if let Err(e) = made {
            body += &format!("tailscale.down not written in {}: {e}\n", main.display());
        }
    }
    reply(&body, code, id)
}

/// A window of `<n>m`, `<n>h` or `<n>d` (a bare number is minutes), in minutes; `08` is eight.
fn minutes(s: &str) -> Option<u64> {
    let (n, per) = match s.chars().last()? {
        'h' => (&s[..s.len() - 1], 60),
        'd' => (&s[..s.len() - 1], 1440),
        'm' => (&s[..s.len() - 1], 1),
        _ => (s, 1),
    };
    if n.is_empty() || n.len() > 5 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(n.parse::<u64>().ok()? * per)
}

/// A line's `msgID=<digits and dots>`, the first one.
fn msg_id(l: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(i) = l[from..].find("msgID=") {
        let at = from + i;
        let rest = &l[at + 6..];
        let n = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if n > 0 {
            return Some(&l[at..at + 6 + n]);
        }
        from = at + 6;
    }
    None
}

/// The msgIDs of tccd's lines that name the process.
fn tcc_ids(lines: impl Iterator<Item = String>, p: &str) -> HashSet<String> {
    lines
        .filter(|l| l.contains(p))
        .filter_map(|l| msg_id(&l).map(str::to_string))
        .collect()
}

/// A tccd line kept for one process: it names the process, or shares a msgID with one that does (the service, logged before
/// it, and the result after).
fn tcc_keep(l: &str, p: &str, ids: &HashSet<String>) -> bool {
    l.contains(p) || msg_id(l).is_some_and(|m| ids.contains(m))
}

/// The Sandbox denials of a window, optionally for one process: line 1 `[tcc] <window> [process]`. The window fills the
/// line's `{lo-hi}` slot in minutes; a process adds `AND eventMessage CONTAINS "<process>"` to its predicate. With tcc, tccd's
/// AUTHREQ lines instead, an hour at most, kept for that process (task erlexec-tcc-prompt). The last 500 lines (2000 for tcc),
/// a cut said first.
fn denials(job: &Job, value: &str, id: Option<&str>) -> String {
    let Some(id) = id else {
        return reply(
            "refused: no id line (private/dashboard/bin/dash denials older than the supervisor? pull main)",
            2,
            Some(""),
        );
    };
    let (tcc, rest) = match value.split_once(' ') {
        Some(("tcc", r)) => (true, r),
        _ if value == "tcc" => (true, ""),
        _ => (false, value),
    };
    let rest = rest.trim();
    let (win, proc_) = rest.split_once(' ').unwrap_or((rest, ""));
    let proc_ = proc_.trim();
    let refused = || {
        reply(
            &format!(
                "refused: \"{}{win} {proc_}\" (a window of 1m to 7d, 1m to 60m for tcc: 90, 90m, 6h or 2d, then an optional process name)",
                if tcc { "tcc " } else { "" }
            ),
            2,
            Some(id),
        )
    };
    let Some(mins) = minutes(win).filter(|m| !tcc || *m <= 60) else {
        return refused();
    };
    if !(proc_.is_empty() || is_id(proc_)) {
        return refused();
    }
    let Ok(mut argv) = fill(&job.argv, &mins.to_string()) else {
        return refused();
    };
    let Some(at) = argv
        .iter()
        .position(|w| w == "--predicate")
        .map(|i| i + 1)
        .filter(|i| *i < argv.len())
    else {
        return reply(
            "refused: denials' line in sandbox/jobs.conf has no --predicate",
            2,
            Some(id),
        );
    };
    if tcc {
        // tccd logs its AUTHREQ lines at info and debug level: only those, an hour of all its debug is slow to read
        argv[at] = r#"subsystem == "com.apple.TCC" AND eventMessage CONTAINS "AUTHREQ""#.into();
        argv.splice(
            at - 1..at - 1,
            ["--info".to_string(), "--debug".to_string()],
        );
    } else if !proc_.is_empty() {
        argv[at] = format!("{} AND eventMessage CONTAINS \"{proc_}\"", argv[at]);
    }
    let (out, code) = exec(&argv, limit(&job.name));
    // a line at a time, the last `cap` kept: a week of the log is never held whole
    let ids = match tcc && !proc_.is_empty() {
        true => Some(tcc_ids(out.lines(), proc_)),
        false => None,
    };
    let cap = if tcc { 2000 } else { 500 };
    let (mut last, mut n) = (VecDeque::with_capacity(cap), 0usize);
    for l in out.lines() {
        if ids.as_ref().is_some_and(|ids| !tcc_keep(&l, proc_, ids)) {
            continue;
        }
        n += 1;
        if last.len() == cap {
            last.pop_front();
        }
        last.push_back(l);
    }
    let mut body = String::new();
    if n > cap {
        body += &format!("truncated: the last {cap} of {n} lines\n");
    }
    for l in last {
        body += &l;
        body.push('\n');
    }
    reply(&body, code, Some(id))
}

/// A tool the supervisor calls besides a job's line: `DASH_<NAME>` names a stand-in for the tests, as DASH_LAUNCHCTL.
fn tool(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}

fn slack_pids(pgrep: &str) -> Vec<libc::pid_t> {
    Command::new(pgrep)
        .args(["-x", "Slack"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.trim().parse().ok())
                .filter(|p: &libc::pid_t| *p > 1)
                .collect()
        })
        .unwrap_or_default()
}

/// Slack.app relaunched, plainly or through replicator's capture proxy (task slack-desktop-capture): line 1 `plain`, the slot's
/// first word, or `proxy`, its second. A running Slack ignores an `open`'s arguments, so it is quit first (TERM, as its menu;
/// 15 s at most). The answer names Slack's launch line as the host sees it, and proxy holds only once each flag is on it.
fn slack(job: &Job, value: &str, id: Option<&str>) -> String {
    let Some(id) = id else {
        return reply("refused: no id line (dash slack writes one)", 2, Some(""));
    };
    let choices: Vec<&str> = job
        .argv
        .iter()
        .find_map(|w| slot(w))
        .map_or(vec![], |(_, s, _)| s.split('|').collect());
    let pick = match value {
        "plain" => choices.first(),
        "proxy" => choices.get(1),
        _ => None,
    };
    let Some(pick) = pick else {
        return reply(
            &format!("refused: \"{value}\" (proxy or plain)"),
            2,
            Some(id),
        );
    };
    let argv = match fill(&job.argv, pick) {
        Ok(a) => a,
        Err(e) => return reply(&format!("refused: slack {e}"), 2, Some(id)),
    };
    let pgrep = tool("DASH_PGREP", "/usr/bin/pgrep");
    for pid in slack_pids(&pgrep) {
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    let wait = |secs: u64, until_running: bool| {
        let end = Instant::now() + Duration::from_secs(secs);
        while slack_pids(&pgrep).is_empty() == until_running && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(500));
        }
    };
    wait(15, false);
    if !slack_pids(&pgrep).is_empty() {
        return reply("Slack did not quit in 15 s: nothing opened", 1, Some(id));
    }
    let (out, mut code) = exec(&argv, Duration::from_secs(60));
    let mut body = out.text();
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    wait(10, true);
    let mut line = Command::new(&pgrep)
        .args(["-lf", r"Slack\.app/Contents/MacOS/Slack"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    if code == 0 && line.is_empty() {
        (code, line) = (1, "no Slack process after 10 s".into());
    } else if code == 0 && value == "proxy" {
        // each flag on its own: Slack or macOS may reorder them or add one between
        let on = format!(" {line} ");
        if let Some(f) = pick
            .split_whitespace()
            .filter(|f| *f != "--args")
            .find(|f| !on.contains(&format!(" {f} ")))
        {
            code = 1;
            line = format!("{line}: no {f} on it");
        }
    }
    reply(&format!("{body}{line}"), code, Some(id))
}

/// The run folders served: the main checkout's and each worktree's, from the main checkout's own list.
fn runs(repo: &Path) -> Vec<PathBuf> {
    let mut out = vec![repo.join(".claude/run")];
    let listed = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "list", "--porcelain"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Ok(o) = listed {
        for wt in String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("worktree "))
        {
            // by its real path: the main checkout under another spelling (/private/var) would be swept again as new, its
            // request in progress answered as interrupted
            if fs::canonicalize(wt).ok() != fs::canonicalize(repo).ok() {
                out.push(Path::new(wt).join(".claude/run"));
            }
        }
    }
    out
}

/// A busy flag cleared when its thread ends, a panic included: a job never stays busy for good.
struct Free(Arc<AtomicBool>);

impl Drop for Free {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The request jobs `dash launchd run` serves: each job on a thread of its own, so a week of the log or a brew install never
/// holds tailscale, nor the supervisor's other jobs. One request of a job at a time, the checkouts one after another. The
/// worktrees are listed on a thread too: a slow git never stalls the supervisor's loop.
/// ponytail: a request in progress when the supervisor stops is left to finish in its own group (its caller is told by the next
/// supervisor's sweep); end it there if one ever hangs.
#[derive(Default)]
pub struct Requests {
    jobs: Vec<Job>,
    main: PathBuf,
    runs: Arc<Mutex<Vec<PathBuf>>>,
    listing: Arc<AtomicBool>,
    seen: HashSet<PathBuf>,
    busy: HashMap<String, Arc<AtomicBool>>,
}

impl Requests {
    /// The host copy's jobs.conf read again, and the checkouts listed again in the background (each 5 s, with the
    /// supervisor's other jobs). A missing or unreadable file serves nothing.
    pub fn read(&mut self, host: &str, repo: &Path) {
        let conf = fs::read_to_string(Path::new(host).join("sandbox/jobs.conf"));
        self.jobs = conf.map(|t| jobs(&t)).unwrap_or_default();
        self.main = repo.join(".claude/run");
        if self.listing.swap(true, Ordering::SeqCst) {
            return;
        }
        let (runs, listing, repo) = (
            self.runs.clone(),
            Free(self.listing.clone()),
            repo.to_path_buf(),
        );
        std::thread::spawn(move || {
            let _listing = listing;
            let listed = self::runs(&repo);
            *runs.lock().unwrap_or_else(|e| e.into_inner()) = listed;
        });
    }

    /// Each job with a request waiting and no thread at work gets one. A run folder seen for the first time is swept first.
    pub fn tick(&mut self) {
        // no jobs read (a host copy mid-sync): nothing served, no folder taken as swept
        if self.jobs.is_empty() {
            return;
        }
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !runs.contains(&self.main) && !self.main.as_os_str().is_empty() {
            runs.insert(0, self.main.clone());
        }
        for run in &runs {
            if self.seen.insert(run.clone()) {
                self.jobs.iter().for_each(|j| sweep(run, &j.name));
            }
        }
        for job in &self.jobs {
            let busy = self.busy.entry(job.name.clone()).or_default();
            if busy.load(Ordering::SeqCst) {
                continue;
            }
            let file = format!("{}.request", job.name);
            let waiting: Vec<PathBuf> = runs
                .iter()
                .filter(|r| fs::metadata(r.join(&file)).is_ok_and(|m| m.len() > 0))
                .cloned()
                .collect();
            if waiting.is_empty() {
                continue;
            }
            busy.store(true, Ordering::SeqCst);
            let (free, job, main) = (Free(busy.clone()), job.clone(), self.main.clone());
            std::thread::spawn(move || {
                let _free = free;
                for run in waiting {
                    if let Some(text) = claim(&run, &job.name) {
                        eprintln!("dash launchd: request {} in {}", job.name, run.display());
                        answer(&run, &job.name, &serve(&job, &main, &text));
                        let _ = fs::remove_file(run.join(format!("{}.request.serving", job.name)));
                    }
                }
            });
        }
    }
}

inventory::submit! { crate::usage::Command::usage("dash host <job> <value>") }
/// `dash host <job> <value>`: a request from this checkout, its answer printed without its `asked=` line; exit 0 when the job
/// exited 0. Waits as long as the job may run, and a minute more for the supervisor's tick.
pub fn ask(name: &str, value: &str, mut out: impl Write) -> i32 {
    // the checkout's top: a worktree's own run folder, which the supervisor serves as the main checkout's
    let top = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let Some(dir) = top else {
        eprintln!("dash host: run it inside a site_perso checkout");
        return 2;
    };
    // the jobs the host serves: its copy's jobs.conf, which follows main (SITE_PERSO_CONF as bin/crm-phone's), else this
    // checkout's when the host copy cannot be read
    let conf_dir = std::env::var("SITE_PERSO_CONF").map_or_else(
        |_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/site_perso"),
        PathBuf::from,
    );
    let host = conf_dir.join("host/sandbox/jobs.conf");
    let (from, conf) = match fs::read_to_string(&host) {
        Ok(t) => (host, t),
        Err(_) => {
            let own = dir.join("sandbox/jobs.conf");
            let t = fs::read_to_string(&own).unwrap_or_default();
            (own, t)
        }
    };
    let known: Vec<String> = jobs(&conf).into_iter().map(|j| j.name).collect();
    if !known.iter().any(|k| k == name) {
        eprintln!(
            "dash host: no request job {name} in {}; its request jobs: {}",
            from.display(),
            known.join(" ")
        );
        return 2;
    }
    let run = dir.join(".claude/run");
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let id = format!("{secs}.{}", std::process::id());
    let answered = run.join(format!("{name}.out"));
    let tmp = run.join(format!("{name}.request.tmp"));
    let sent = fs::create_dir_all(&run)
        .and_then(|()| match fs::remove_file(&answered) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        })
        .and_then(|()| fs::write(&tmp, format!("{value}\nid={id}\n")))
        .and_then(|()| fs::rename(&tmp, run.join(format!("{name}.request"))));
    if let Err(e) = sent {
        eprintln!(
            "dash host: cannot write {}: {e}; no request sent",
            run.display()
        );
        return 1;
    }
    let until = Instant::now() + limit(name) + Duration::from_secs(60);
    while Instant::now() < until {
        // ponytail: one caller a job a checkout; a second one's request replaces the first's, whose wait then times out
        if let Ok(text) = fs::read_to_string(&answered)
            && text.lines().any(|l| l == format!("asked={id}"))
        {
            for l in text.lines().filter(|l| !l.starts_with("asked=")) {
                let _ = writeln!(out, "{l}");
            }
            // the supervisor's trailer, the last exit= line: the job's own output may hold one
            let exit = text.lines().rev().find(|l| l.starts_with("exit="));
            return i32::from(exit != Some("exit=0"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    eprintln!(
        "dash host: no answer from the host's dash launchd in {} s (is com.nicolasha.dash running? bin/crm-phone installs it)",
        (limit(name) + Duration::from_secs(60)).as_secs()
    );
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    const CONF: &str = "\
# a comment naming host request is no job
tailscale        host               request         /usr/local/bin/tailscale {status|up|down|serve status}
denials          host               request         /usr/bin/log show --style compact --last {1-10080}m --predicate 'sender == \"Sandbox\"'
claude           sandbox.sb         pane            claude --dangerously-skip-permissions
tailscale-serve  host               login           /usr/local/bin/tailscale serve --bg 7788
";

    fn conf_job(name: &str) -> Job {
        jobs(CONF)
            .into_iter()
            .find(|j| j.name == name)
            .expect("its line")
    }

    #[test]
    fn only_host_request_lines_are_request_jobs() {
        let _budget = TestBudget::new();
        let names: Vec<String> = jobs(CONF).into_iter().map(|j| j.name).collect();
        assert_eq!(names, ["tailscale", "denials"]);
        assert_eq!(
            conf_job("denials").argv.last().map(String::as_str),
            Some(r#"sender == "Sandbox""#)
        );
    }

    #[test]
    fn jobs_conf_requests_tailscale_denials_slack_and_brew_of_bash() {
        let _budget = TestBudget::new();
        // the mirror holds rust/ alone; in the monorepo a missing jobs.conf fails
        let Some(repo) = dash_meter::monorepo() else {
            return;
        };
        let conf = fs::read_to_string(repo.join("sandbox/jobs.conf")).expect("jobs.conf");
        let all = jobs(&conf);
        let names: Vec<&str> = all.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["tailscale", "denials", "slack", "brew"]);
        let brew = &all[3].argv;
        assert_eq!(
            fill(brew, "bash").expect("bash is listed"),
            ["/opt/homebrew/bin/brew", "install", "bash"]
        );
        assert!(fill(brew, "wget").is_err());
        assert!(
            all[1].argv.iter().any(|w| w == "{1-10080}m"),
            "{:?}",
            all[1].argv
        );
    }

    #[test]
    fn a_slot_takes_one_listed_word_or_a_number_in_range() {
        let _budget = TestBudget::new();
        let ts = conf_job("tailscale").argv;
        assert_eq!(
            fill(&ts, "serve status").expect("listed"),
            ["/usr/local/bin/tailscale", "serve", "status"]
        );
        for bad in ["up|down", "serve", "status; rm", "", "UP"] {
            assert!(fill(&ts, bad).is_err(), "{bad:?} was let through");
        }
        let d = conf_job("denials").argv;
        assert!(
            fill(&d, "360")
                .expect("in range")
                .contains(&"360m".to_string())
        );
        for bad in ["0", "10081", "+5", "6h", "-1"] {
            assert!(fill(&d, bad).is_err(), "{bad:?} was let through");
        }
        let none = vec!["/bin/echo".to_string()];
        assert!(fill(&none, "x").is_err());
        assert_eq!(fill(&none, "").expect("no value"), ["/bin/echo"]);
    }

    #[test]
    fn a_sweep_answers_an_unanswered_claim_and_keeps_a_written_answer() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("requests-sweep");
        let run = dir.0.join(".claude/run");
        fs::create_dir_all(&run).expect("run");
        // claimed, never answered: told at once, with its id
        fs::write(run.join("brew.request.serving"), "bash\nid=a1\n").expect("serving");
        sweep(&run, "brew");
        let out = fs::read_to_string(run.join("brew.out")).expect("answered");
        assert!(
            out.starts_with("interrupted: ") && out.ends_with("exit=75\nasked=a1\n"),
            "{out}"
        );
        assert!(!run.join("brew.request.serving").exists());
        // answered, stopped only before its .serving went: the answer stands
        fs::write(
            run.join("brew.out"),
            "==> Installing bash\nexit=0\nasked=a2\n",
        )
        .expect("out");
        fs::write(run.join("brew.request.serving"), "bash\nid=a2\n").expect("serving");
        sweep(&run, "brew");
        assert_eq!(
            fs::read_to_string(run.join("brew.out")).expect("out"),
            "==> Installing bash\nexit=0\nasked=a2\n"
        );
        assert!(!run.join("brew.request.serving").exists());
    }

    #[test]
    fn a_timed_out_job_says_so_on_a_line_of_its_own() {
        let _budget = TestBudget::new();
        for (script, want) in [
            ("echo waiting; sleep 5", "waiting\ntimed out after 1 s\n"),
            ("printf waiting; sleep 5", "waiting\ntimed out after 1 s\n"),
        ] {
            let argv = ["/bin/sh", "-c", script].map(String::from);
            let (out, code) = exec(&argv, Duration::from_secs(1));
            assert_eq!((out.text(), code), (want.to_string(), 124), "{script}");
        }
        let (out, code) = exec(&["/no/such/tool".to_string()], Duration::from_secs(1));
        assert_eq!(code, 127);
        assert!(out.text().starts_with("/no/such/tool: "), "{}", out.text());
    }

    #[test]
    fn a_window_reads_minutes_hours_or_days() {
        let _budget = TestBudget::new();
        for (w, m) in [("6h", 360), ("08", 8), ("90m", 90), ("2d", 2880)] {
            assert_eq!(minutes(w), Some(m), "{w}");
        }
        for w in ["", "m", "x6h", "6x", "123456"] {
            assert_eq!(minutes(w), None, "{w}");
        }
    }

    #[test]
    fn tcc_keeps_the_process_lines_and_their_msgid() {
        let _budget = TestBudget::new();
        let lines = [
            "AUTHREQ_CTX: msgID=401.7, service=kTCCServiceSystemPolicyAppData",
            "AUTHREQ_ATTRIBUTION: msgID=401.7, binary_path=/x/erlexec",
            "AUTHREQ_RESULT: msgID=401.8, OTHER-APP",
            "AUTHREQ noise msgID=, NOISE",
            "AUTHREQ no id, names erlexec",
        ];
        let ids = tcc_ids(lines.iter().map(|l| l.to_string()), "erlexec");
        let kept: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| tcc_keep(l, "erlexec", &ids))
            .collect();
        assert_eq!(kept, [lines[0], lines[1], lines[4]]);
    }
}
