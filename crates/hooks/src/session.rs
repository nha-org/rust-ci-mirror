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

/// `dash hook session-state start|busy|tool|idle`, Claude Code's JSON on stdin: tells bin/claude-supervise, which runs this
/// session on the host, what it cannot see from outside. SessionStart calls start, UserPromptSubmit busy, PostToolUse tool and
/// Stop idle; it writes .claude/run/<supervisor pid>.{session,state,agents,jobs,notified} and reads <pid>.wake
/// (bin/comment-wake) when no wake-wait took it first. Without CLAUDE_SUPERVISOR (a session not started by make claude) it
/// does nothing.
///   busy  tells the session of an approval it does not run yet, at each of his messages until it asks to restart, and once
///         of a change he refused (.claude/run/refused, written by bin/sandbox-apply).
///   tool  the same approval notice, once per approval: a session mid-turn hears of it at its next tool call.
///   idle  writes the running background tasks of the Stop payload to <pid>.agents (graal's and klar's fix, 2026-10-05: the
///         session's own list, not a guess from transcript times).
pub fn session_state(what: &str, mut out: fs::File) {
    let sup = env("CLAUDE_SUPERVISOR");
    // A helper `claude -p` inside the session inherits CLAUDE_SUPERVISOR: its id would be resumed in place of the session's
    // and its Stop would say idle mid-turn. A supervised session is interactive (« cli »), measured 2026-11-07.
    if sup.is_empty() || env("CLAUDE_CODE_ENTRYPOINT").starts_with("sdk-") {
        return;
    }
    let project = crate::project_dir().unwrap_or_else(|| ".".into());
    let run = Path::new(&project).join(".claude/run");
    let file = |ext: &str| run.join(format!("{sup}.{ext}"));
    let input = stdin();
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
    if what == "start" && !id.is_empty() && !source_is(&input, "compact") {
        thread_start(&threads, &id, &input, parsed.as_ref());
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

/// `find run -maxdepth 1 -name 'session-state*' -mtime +7 -delete`: what is `older` than that goes.
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
        if old && e.file_name().to_string_lossy().starts_with("session-state") {
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
/// The thread: the task (a reviewer's is review-<id>), the orchestrator in the main checkout, else the pane.
fn thread_start(threads: &Path, id: &str, input: &str, parsed: Option<&Value>) {
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
        let body: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("\"{k}\":{}", Value::from(*v)))
            .collect();
        append(
            &threads.with_extension("jsonl"),
            &format!("{{{}}}\n", body.join(",")),
        );
    }
    // a resume's first message is its resume note, never its prompt: an empty file keeps it out, so a restart falls back to
    // the thread's own words
    let prompt = threads.join(format!("{id}.prompt"));
    if source_is(input, "resume") && !prompt.exists() {
        write(&prompt, "");
    }
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
        record(&dir, &input);
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

/// The figures of one status line input, kept in `<dir>/<session id>.json` when its windows changed; the profile it ran
/// under first (`config_dir`): Dashboard.ClaudeAccounts names the account by it. ponytail: the path goes in unescaped; a
/// profile path holds no quote or backslash (dash claude-accounts add's names).
pub(crate) fn record(dir: &Path, input: &str) {
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
    if limits(input) == limits(&kept) {
        return;
    }
    let body = match input.strip_prefix('{') {
        Some(rest) => format!("{{\"config_dir\":\"{}\",{rest}", env("CLAUDE_CONFIG_DIR")),
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
        record(&dir, input);
        let kept = fs::read_to_string(dir.join("s-1.json")).expect("written");
        assert!(kept.starts_with("{\"config_dir\":\""), "{kept}");
        assert!(kept.ends_with(r#""session_id":"s-1","rate_limits":{"seven_day":{"used_percentage":40,"resets_at":1}}}"#));
        let older = fs::metadata(dir.join("s-1.json"))
            .and_then(|m| m.modified())
            .expect("mtime");
        std::thread::sleep(Duration::from_millis(20));
        record(
            &dir,
            &input.replace("\"rate_limits\"", "\"cost\":1,\"rate_limits\""),
        );
        let same = fs::metadata(dir.join("s-1.json"))
            .and_then(|m| m.modified())
            .expect("mtime");
        assert_eq!(older, same, "the same figures rewrite nothing");
        record(&dir, &input.replace("40", "41"));
        assert!(
            fs::read_to_string(dir.join("s-1.json"))
                .expect("read")
                .contains("41")
        );
        record(&dir, r#"{"five_hour":{"used_percentage":1}}"#);
        assert!(dir.join("unknown.json").is_file(), "no session id: unknown");
        let _ = fs::remove_dir_all(&dir);
    }
}
