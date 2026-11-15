//! `dash restart`: the session restarts itself in place, nothing lost (task rust-self-restart; @nha, 2026-11-14: « well you
//! can restart yourself afaik. is there not a command for this ? If not build it. »). It ports the claude-restart script and
//! adds the passover; the restart itself stays bin/claude-supervise's.
//!
//! 1. **Ask:** `dash restart "<open work>"` writes a passover note (the open work in its words, the watchers and agents of the
//!    last turn's end, wake-wait, the MCP servers that failed at this start) and `.claude/run/<supervisor>.restart`.
//! 2. **The restart is the supervisor's:** at the end of the turn, once no background agent runs, it starts Claude again in the
//!    same pane on the same login, resumed on the same conversation (@nha, 2026-11-14, in rust-self-restart's dialog: « Resume,
//!    same chat »), its first message the request's words: re-arm wake-wait, read the note, run `dash restart check`.
//! 3. **Check:** `dash restart check <note>` says whether wake-wait runs, then reads the MCP servers from the transcript's
//!    tools deltas written since the last restart was asked (Claude Code writes one at each start, a resume included, and
//!    another as each pending server settles: LinkedIn's took two minutes on 2026-11-14). A server of `.mcp.json` still
//!    pending is waited on; one that failed restarts the session once more (the only retry Claude Code has for a server
//!    that never connected); one that fails after its retry is named, never retried again.
use dash_core::{Command, Host};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "dash restart \"<open work>\" | dash restart check <passover> (restart this session at the end of its turn, \
                     same pane, login and conversation, with a passover note; check: wake-wait and the MCP servers after it)";
const WAKE_WAIT: &str = "private/dashboard/priv/commands/wake-wait";
/// The note's own marks, each a line of its own, worded so no open work reads as one.
const ASKED: &str = "- dash restart asked at ";
const RETRIED: &str = "- dash restart check retried: ";

/// Where a restart reads and writes: the checkout's `.claude/run`, the supervisor's pid, the folders holding the transcripts.
pub struct Here {
    pub run: PathBuf,
    pub sup: String,
    /// Claude's config folders, `CLAUDE_CONFIG_DIR` first, then `~/.claude` and each profile under its `accounts/`.
    pub claude: Vec<PathBuf>,
}

impl Here {
    fn file(&self, ext: &str) -> PathBuf {
        self.run.join(format!("{}.{ext}", self.sup))
    }
    /// A run file's lines, none when missing or empty.
    fn lines(&self, ext: &str) -> Vec<String> {
        let text = std::fs::read_to_string(self.file(ext)).unwrap_or_default();
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()
    }
    fn repo(&self) -> Option<&Path> {
        self.run.parent()?.parent()
    }
}

/// wake-wait's line: its pid one of the running wake-waits (`pgrep`, as the supervisor asks: a stale pid reused by another
/// process is no wake-wait), or the step that re-arms it.
fn wake_wait(h: &Here) -> String {
    let running = std::process::Command::new("pgrep")
        .args(["-f", "priv/commands/[w]ake-wait"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let pid = h.lines("wake-wait").into_iter().next();
    match pid.filter(|p| running.lines().any(|l| l.trim() == p)) {
        Some(pid) => format!("wake-wait: runs (pid {pid})"),
        None => format!("wake-wait: not running: start {WAKE_WAIT} in the background now"),
    }
}

/// The request's words, the supervisor's step 1 of the restart notice.
fn request(note: &Path) -> String {
    let note = note.display();
    format!(
        "Passover of dash restart: start {WAKE_WAIT} in the background first, read {note}, then run \
         private/dashboard/bin/dash restart check {note} and do what it prints.\n"
    )
}

/// The restart asked: the note written, then the request; the note's path. `words` is the open work, in the session's words;
/// none still restarts (an approval's restart never waits on wording).
pub fn ask(h: &Here, words: &str, now: u64) -> Result<PathBuf, String> {
    let words = match words.trim() {
        "" => "none given: read the conversation's last turns",
        w => w,
    };
    let listed = |ext: &str| match h.lines(ext) {
        l if l.is_empty() => "- none".to_string(),
        l => l
            .iter()
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    let failed = match servers(h, None).failed {
        f if f.is_empty() => "none".to_string(),
        f => f.join(", "),
    };
    let note = h.run.join(format!("passover-restart-{now}.md"));
    let text = format!(
        "# Passover, restart in place at {stamp}\n\n\
         Same pane, same login, same conversation: bin/claude-supervise resumes it at the end of the turn.\n\n\
         ## Open work\n{words}\n\n\
         ## Watchers at the last turn's end (relaunch them)\n{jobs}\n\n\
         ## wake-wait\n{wake}\n\n\
         ## Agents at the last turn's end (the restart waits for them)\n{agents}\n\n\
         ## MCP servers failed at this session's start\n{failed}\n\n\
         ## Read again before acting\ndash task ls, dash prs, dash asks, dash session list\n\n\
         ## Marks\n{ASKED}{asked}\n",
        stamp = dash_core::stamp(now as i64 * 1000),
        jobs = listed("jobs"),
        wake = wake_wait(h),
        agents = listed("agents"),
        asked = dash_core::iso(now as i64),
    );
    std::fs::write(&note, text).map_err(|e| format!("{}: {e}", note.display()))?;
    let req = h.file("restart");
    std::fs::write(&req, request(&note))
        .map_err(|e| format!("{}: {e}: no restart will happen", req.display()))?;
    Ok(note)
}

/// After the restart: wake-wait's line, then the MCP servers' verdict from the deltas since the last ask. A failed server of
/// `.mcp.json` not retried yet is retried by one more restart, its name and the new ask kept in the note.
pub fn check(h: &Here, note: &Path, now: u64) -> Result<String, String> {
    let text = std::fs::read_to_string(note).map_err(|e| format!("{}: {e}", note.display()))?;
    // the last section, appended to: a line of the open work never reads as a mark
    let tail = text.rsplit_once("\n## Marks\n").map_or("", |(_, m)| m);
    let marks = |mark: &str| -> Vec<String> {
        let found = tail.lines().filter_map(|l| l.strip_prefix(mark));
        found
            .flat_map(|l| l.split(", "))
            .map(String::from)
            .collect()
    };
    let retried = marks(RETRIED);
    let since = marks(ASKED).pop();
    let ours = project_servers(h);
    let seen = servers(h, since.as_deref());
    let mut said = vec![wake_wait(h)];
    if !seen.read {
        said.push(
            "MCP: no tools list since the restart yet: run dash restart check again in a minute"
                .into(),
        );
        return Ok(said.join("\n"));
    }
    let ours_in =
        |l: &[String]| -> Vec<String> { l.iter().filter(|n| ours.contains(n)).cloned().collect() };
    let pending = ours_in(&seen.pending);
    let failed = ours_in(&seen.failed);
    let (again, fresh): (Vec<String>, Vec<String>) =
        failed.into_iter().partition(|n| retried.contains(n));
    // a server still connecting holds the verdict: a retry's restart now would cut its connect (/code-review of #1631)
    if !pending.is_empty() {
        said.push(format!(
            "MCP: {} still connecting: run dash restart check again in a minute",
            pending.join(", ")
        ));
        return Ok(said.join("\n"));
    }
    if !again.is_empty() {
        said.push(format!(
            "MCP: {} failed again after one retry: name it to Nicolas in this turn's report; no more restart for it",
            again.join(", ")
        ));
    }
    if !fresh.is_empty() {
        let names = fresh.join(", ");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(note)
            .map_err(|e| format!("{}: {e}", note.display()))?;
        let marks = format!("{RETRIED}{names}\n{ASKED}{}\n", dash_core::iso(now as i64));
        f.write_all(marks.as_bytes())
            .map_err(|e| format!("{}: {e}", note.display()))?;
        let req = h.file("restart");
        std::fs::write(&req, request(note)).map_err(|e| format!("{}: {e}", req.display()))?;
        said.push(format!(
            "MCP: {names} failed at this start: retrying once, this session restarts at the end of this turn; end it now"
        ));
    }
    if again.is_empty() && fresh.is_empty() {
        said.push(match retried.is_empty() {
            true => "MCP: every server of .mcp.json connected".into(),
            false => format!(
                "MCP: every server of .mcp.json connected, {} after one retry",
                retried.join(", ")
            ),
        });
    }
    let others: Vec<String> = (seen.failed.iter())
        .filter(|n| !ours.contains(n))
        .cloned()
        .collect();
    if !others.is_empty() {
        said.push(format!(
            "MCP outside .mcp.json, failed, not retried: {}",
            others.join(", ")
        ));
    }
    Ok(said.join("\n"))
}

/// The servers `.mcp.json` names.
fn project_servers(h: &Here) -> Vec<String> {
    let Some(file) = h.repo().map(|r| r.join(".mcp.json")) else {
        return vec![];
    };
    let json: Value = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    json["mcpServers"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// The MCP servers as the transcript's last tools delta left them.
#[derive(Default)]
struct Servers {
    /// Whether a delta was found at all.
    read: bool,
    failed: Vec<String>,
    pending: Vec<String>,
}

/// This session's servers, from its transcript's last tools delta written at or after `since` (an ISO time); none unread.
fn servers(h: &Here, since: Option<&str>) -> Servers {
    let session = h.lines("session").into_iter().next();
    let Some(file) = session.and_then(|s| transcript(&h.claude, &s)) else {
        return Servers::default();
    };
    std::fs::File::open(file)
        .map(|f| last_delta(std::io::BufReader::new(f), since))
        .unwrap_or_default()
}

/// The last `deferred_tools_delta` at or after `since`: Claude Code writes the whole failed and pending lists in each one.
/// Times compare as text, both ISO in UTC (« 2026-11-14T13:10:54.090Z » after « 2026-11-14T13:10:50Z »).
fn last_delta(lines: impl BufRead, since: Option<&str>) -> Servers {
    let last = lines
        .lines()
        .map_while(Result::ok)
        .filter(|l| l.contains("\"failedMcpServers\""))
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .filter(|v| v["attachment"]["type"] == "deferred_tools_delta")
        // to the second: « …:50.090Z » is at « …:50Z », though '.' sorts before 'Z' (/code-review of #1631)
        .filter(|v| {
            let second = |t: &str| t.get(..19).unwrap_or(t).to_string();
            since.is_none_or(|s| {
                v["timestamp"]
                    .as_str()
                    .is_some_and(|t| second(t) >= second(s))
            })
        })
        .last();
    let Some(a) = last.as_ref().map(|v| &v["attachment"]) else {
        return Servers::default();
    };
    let names = |k: &str| -> Vec<String> {
        let list = a[k].as_array().into_iter().flatten();
        let name = |s: &Value| s.as_str().or(s["name"].as_str()).map(String::from);
        list.filter_map(name).collect()
    };
    Servers {
        read: true,
        failed: names("failedMcpServers"),
        pending: names("pendingMcpServers"),
    }
}

/// The session's transcript, its newest copy under one of Claude's config folders' `projects/*/`.
fn transcript(claude: &[PathBuf], session: &str) -> Option<PathBuf> {
    if session.is_empty()
        || !session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let dirs = |p: PathBuf| -> Vec<PathBuf> {
        let entries = std::fs::read_dir(p).into_iter().flatten().flatten();
        entries.map(|e| e.path()).filter(|p| p.is_dir()).collect()
    };
    let files = claude
        .iter()
        .flat_map(|c| dirs(c.join("projects")))
        .map(|d| d.join(format!("{session}.jsonl")));
    let modified = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    files.filter(|p| p.is_file()).max_by_key(modified)
}

/// Claude's config folders: `CLAUDE_CONFIG_DIR`, `~/.claude`, then each profile of `~/.claude/accounts`.
fn config_dirs(home: &Path) -> Vec<PathBuf> {
    let set = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty());
    let claude = home.join(".claude");
    let entries = std::fs::read_dir(claude.join("accounts"))
        .into_iter()
        .flatten()
        .flatten();
    let profiles = entries.map(|e| e.path()).filter(|p| p.is_dir());
    let mut dirs: Vec<PathBuf> = set.map(PathBuf::from).into_iter().collect();
    dirs.push(claude);
    dirs.extend(profiles);
    dirs
}

/// The checkout the session runs in: Claude's project folder, else DASH_REPO, else the nearest folder up from here holding
/// `.git`.
fn checkout() -> Option<PathBuf> {
    let set = |k: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    // DASH_REPO before the nearest .git, as the old claude-restart read it: a nested worktree's cwd never names another
    // .claude/run than the supervisor's (/code-review of #1631)
    (crate::project_dir().map(PathBuf::from))
        .or_else(|| set("DASH_REPO"))
        .or_else(|| {
            let here = std::env::current_dir().ok()?;
            here.ancestors()
                .find(|d| d.join(".git").exists())
                .map(Path::to_path_buf)
        })
}

fn run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    let here = || -> Result<Here, String> {
        let sup = std::env::var("CLAUDE_SUPERVISOR").unwrap_or_default();
        if sup.is_empty() {
            return Err(
                "dash restart: not under bin/claude-supervise (start with make claude)".into(),
            );
        }
        let repo = checkout()
            .ok_or("dash restart: no checkout found (CLAUDE_PROJECT_DIR, .git, DASH_REPO)")?;
        Ok(Here {
            run: repo.join(".claude/run"),
            sup,
            claude: config_dirs(&host.home()?),
        })
    };
    let now = (host.now_ms() / 1000) as u64;
    let said = match args {
        // a passover note that exists: open work worded « check … » is a restart like any other (/code-review of #1631)
        ["check", note] if Path::new(note).is_file() => {
            here().and_then(|h| check(&h, Path::new(note), now))
        }
        // the script's words, unquoted ones too: a restart an approval asked for never fails on its wording
        words => here().and_then(|h| {
            let note = ask(&h, &words.join(" "), now)?;
            Ok(format!(
                "dash restart: restarting at the end of this turn; passover {}",
                note.display()
            ))
        }),
    };
    match said {
        Ok(s) => {
            let _ = writeln!(out, "{s}");
        }
        Err(e) => {
            let _ = writeln!(out, "{e}");
            std::process::exit(1)
        }
    }
}

inventory::submit! { Command { usage: USAGE, words: &["restart"], run: Some(run) } }

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    const ASKED_AT: u64 = 1_763_150_000;

    fn here(name: &str) -> Here {
        let root = std::env::temp_dir().join(format!("restart-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let run = root.join("repo/.claude/run");
        std::fs::create_dir_all(&run).unwrap();
        let mcp = r#"{"mcpServers":{"mcp-server-linkedin":{},"malt":{}}}"#;
        std::fs::write(root.join("repo/.mcp.json"), mcp).unwrap();
        let home = root.join("home/.claude");
        Here {
            run,
            sup: "4242".into(),
            claude: vec![home.clone(), home.join("accounts/second")],
        }
    }

    /// This session's transcript under a profile, with one tools delta per `(at, failed, pending)`, after an older one.
    fn deltas(h: &Here, list: &[(u64, &[&str], &[&str])]) {
        std::fs::write(h.file("session"), "s-1\n").unwrap();
        let dir = h.claude[1].join("projects/-repo");
        std::fs::create_dir_all(&dir).unwrap();
        let older = r#"{"type":"attachment","timestamp":"2025-01-01T00:00:00.000Z","attachment":{"type":"deferred_tools_delta","failedMcpServers":[{"name":"stale"}]}}"#;
        let mut text = format!("{older}\n{{\"type\":\"user\"}}\n");
        for (at, failed, pending) in list {
            let failed: Vec<Value> = failed
                .iter()
                .map(|n| serde_json::json!({"name": n, "error": "x"}))
                .collect();
            let at = format!("{}.090Z", dash_core::iso(*at as i64).trim_end_matches('Z'));
            let a = serde_json::json!({"type":"deferred_tools_delta","failedMcpServers":failed,"pendingMcpServers":pending});
            text.push_str(&format!(
                "{}\n",
                serde_json::json!({"type":"attachment","timestamp":at,"attachment":a})
            ));
        }
        std::fs::write(dir.join("s-1.jsonl"), text).unwrap();
    }

    /// A process pgrep finds as a wake-wait, killed when dropped.
    struct WakeWait(std::process::Child);
    impl Drop for WakeWait {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn wake_wait_runs(h: &Here) -> WakeWait {
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 10; :", "priv/commands/wake-wait"])
            // its sleep outlives the kill by seconds: never on the test's pipes
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::fs::write(h.file("wake-wait"), format!("{}\n", child.id())).unwrap();
        WakeWait(child)
    }

    /// Done-when 1: dash restart writes the passover (open work, watchers, agents, failed MCPs) and the supervisor's request,
    /// which bin/claude-supervise turns into a restart in the same pane and login at the turn's end (supervise-test 2, 2b).
    /// A delta in the second of the mark counts, though its fraction sorts it before a whole-second mark (/code-review of #1631).
    #[dash_test]
    fn a_delta_in_the_second_of_the_mark_is_read() {
        let line = r#"{"timestamp":"2026-11-14T13:10:50.090Z","attachment":{"type":"deferred_tools_delta","failedMcpServers":["malt"],"pendingMcpServers":[]}}"#;
        let seen = last_delta(std::io::Cursor::new(line), Some("2026-11-14T13:10:50Z"));
        assert!(
            seen.read && seen.failed == ["malt"],
            "read: {}, failed: {:?}",
            seen.read,
            seen.failed
        );
        let later = last_delta(std::io::Cursor::new(line), Some("2026-11-14T13:10:51Z"));
        assert!(
            !later.read,
            "a delta before the mark's second is the old session's"
        );
    }

    #[dash_test]
    fn restart_writes_the_passover_and_the_supervisor_s_request() {
        let h = here("ask");
        deltas(&h, &[(ASKED_AT - 60, &["mcp-server-linkedin"], &[])]);
        std::fs::write(
            h.file("jobs"),
            "shell w1 private/dashboard/priv/commands/wake-wait\n",
        )
        .unwrap();
        std::fs::write(h.file("agents"), "subagent a1 (review of PR 1619)\n").unwrap();
        // a stale pid (this test's own) is no wake-wait
        std::fs::write(h.file("wake-wait"), format!("{}\n", std::process::id())).unwrap();
        let note = ask(&h, "two approved LinkedIn sends wait on its MCP", ASKED_AT).unwrap();
        let text = std::fs::read_to_string(&note).unwrap();
        for part in [
            "## Open work\ntwo approved LinkedIn sends wait on its MCP",
            "- shell w1 private/dashboard/priv/commands/wake-wait",
            "## wake-wait\nwake-wait: not running",
            "- subagent a1 (review of PR 1619)",
            "## MCP servers failed at this session's start\nmcp-server-linkedin",
        ] {
            assert!(text.contains(part), "the note lacks {part:?}:\n{text}");
        }
        let req = std::fs::read_to_string(h.file("restart")).unwrap();
        let note = note.display().to_string();
        assert!(
            req.contains(&format!(
                "start {WAKE_WAIT} in the background first, read {note}"
            )),
            "{req}"
        );
        assert!(req.contains(&format!("dash restart check {note}")), "{req}");
        let bare = ask(&h, "  ", ASKED_AT + 1).unwrap();
        assert!(
            std::fs::read_to_string(bare)
                .unwrap()
                .contains("## Open work\nnone given"),
            "no words still restarts"
        );
    }

    /// Done-when 2: after the restart the check says whether wake-wait runs, waits on a pending server, retries a failed MCP
    /// of .mcp.json once (one more restart, kept in the note), then names it, never a third start; a server failing only on
    /// the retry gets its own retry; one outside .mcp.json is named, not retried; a delta older than the ask is never read.
    #[dash_test]
    fn check_rearms_wake_wait_retries_a_failed_mcp_once_then_names_it() {
        let h = here("check");
        deltas(&h, &[(ASKED_AT - 60, &["mcp-server-linkedin"], &[])]);
        let note = ask(
            &h,
            "open work\n- dash restart check retried: malt",
            ASKED_AT,
        )
        .unwrap();
        std::fs::remove_file(h.file("restart")).unwrap();
        let before = check(&h, &note, ASKED_AT + 5).unwrap();
        assert!(
            before.contains("no tools list since the restart yet"),
            "the old start's delta is not this one's: {before}"
        );
        let _ww = wake_wait_runs(&h);
        deltas(&h, &[(ASKED_AT + 10, &["pane"], &["mcp-server-linkedin"])]);
        let pending = check(&h, &note, ASKED_AT + 20).unwrap();
        assert!(pending.contains("wake-wait: runs (pid"), "{pending}");
        assert!(
            pending.contains("MCP: mcp-server-linkedin still connecting"),
            "{pending}"
        );
        assert!(
            !h.file("restart").exists(),
            "a pending server is waited on, never restarted for"
        );
        deltas(
            &h,
            &[
                (ASKED_AT + 10, &["pane"], &["mcp-server-linkedin"]),
                (ASKED_AT + 130, &["mcp-server-linkedin", "pane"], &[]),
            ],
        );
        let first = check(&h, &note, ASKED_AT + 140).unwrap();
        assert!(
            first.contains("MCP: mcp-server-linkedin failed at this start: retrying once"),
            "{first}"
        );
        assert!(first.contains("not retried: pane"), "{first}");
        assert!(h.file("restart").exists(), "the retry is one more restart");
        std::fs::remove_file(h.file("restart")).unwrap();
        let twice = check(&h, &note, ASKED_AT + 150).unwrap();
        assert!(
            twice.contains("no tools list since the restart yet"),
            "a second check before the retry's start: {twice}"
        );
        deltas(
            &h,
            &[(ASKED_AT + 200, &["mcp-server-linkedin", "malt"], &[])],
        );
        let second = check(&h, &note, ASKED_AT + 210).unwrap();
        assert!(
            second.contains("MCP: mcp-server-linkedin failed again after one retry: name it"),
            "{second}"
        );
        assert!(
            second.contains("MCP: malt failed at this start: retrying once"),
            "malt's own retry: {second}"
        );
        std::fs::remove_file(h.file("restart")).unwrap();
        deltas(
            &h,
            &[(ASKED_AT + 300, &["mcp-server-linkedin", "malt"], &[])],
        );
        let third = check(&h, &note, ASKED_AT + 310).unwrap();
        assert!(
            third.contains("mcp-server-linkedin, malt failed again after one retry"),
            "{third}"
        );
        assert!(!h.file("restart").exists(), "never a second retry");
        deltas(&h, &[(ASKED_AT + 400, &["pane"], &[])]);
        let healed = check(&h, &note, ASKED_AT + 410).unwrap();
        assert!(
            healed.contains(
                "every server of .mcp.json connected, mcp-server-linkedin, malt after one retry"
            ),
            "{healed}"
        );
    }
}
