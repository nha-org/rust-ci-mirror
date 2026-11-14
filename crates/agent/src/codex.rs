//! Core's Agent for Codex's TUI, section 20b's second agent kind: its start screen, a session's outcome from its rollout, and
//! its limit from `codex app-server`'s `account/rateLimits/read`. Recorded 2026-11-08 on codex-cli 0.153.4, his free ChatGPT
//! login: fixtures/codex.
use super::{connects, outcome_of};
use dash_core::tasks::{Agent, Limit, Outcome, Screen};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The account a Codex limit names: one login per Mac, so the kind is its name, and a pick's account says which kind it is for.
pub const ACCOUNT: &str = "codex";

/// Codex's first words, which its launch passes. herdr names a Codex session only once its first turn starts (its SessionStart
/// hook, recorded 2026-11-08), and orch briefs a session only once named: this turn names it, and the brief, typed while it runs,
/// joins it. A turn that ended on these words alone answered no brief, so it is no outcome.
pub const OPENER: &str = "The brief of your task comes in the next message: wait for it.";

pub struct Codex {
    /// `codex`, on PATH.
    pub bin: String,
    /// `~/.codex/sessions`: `YYYY/MM/DD/rollout-<time>-<session>.jsonl`.
    pub sessions: PathBuf,
}

/// Each start dialog's answer as its menu writes it: an update skipped (its first option runs npm), the folder trusted and its hooks
/// too, since herdr's hook is what names the session. Only a line holding nothing but the option counts, as for Claude's.
const ANSWERS: [&str; 3] = ["Skip", "Yes, continue", "Trust all and continue"];

/// The option on this line and whether the pointer is on it.
fn option(line: &str) -> Option<(&str, bool)> {
    let line = line.trim();
    let rest = line.strip_prefix('›').map(str::trim_start);
    let words = rest.unwrap_or(line);
    let words = words.trim_start_matches(|c: char| c.is_ascii_digit());
    let words = words.strip_prefix(". ")?;
    Some((words, rest.is_some()))
}

/// The windows of a reply's `codex` bucket: one up to 5 hours is the short one, the longest is the week (a free plan has one
/// window, 30 days). A window past its reset reads as empty, as Claude's do.
pub fn limit_in(result: &Value, now: u64) -> Option<Limit> {
    let bucket = &result["rateLimitsByLimitId"][ACCOUNT];
    let bucket = if bucket.is_object() {
        bucket
    } else {
        &result["rateLimits"]
    };
    let windows: Vec<&Value> = ["primary", "secondary"]
        .iter()
        .map(|w| &bucket[*w])
        .filter(|w| w["usedPercent"].is_number())
        .collect();
    let used = |w: &Value| {
        let resets = w["resetsAt"].as_u64().unwrap_or(0);
        let used = w["usedPercent"].as_f64().unwrap_or(0.0);
        if resets > now { used } else { 0.0 }
    };
    // A window of no stated length counts as the week: spent, it holds a start.
    let minutes = |w: &&&Value| w["windowDurationMins"].as_u64().unwrap_or(u64::MAX);
    let short = windows.iter().find(|w| minutes(w) <= 5 * 60);
    let week = windows
        .iter()
        .filter(|w| minutes(w) > 5 * 60)
        .max_by_key(minutes);
    (!windows.is_empty()).then(|| Limit {
        key: false,
        account: ACCOUNT.into(),
        five_hours: short.map_or(0.0, |w| used(w)),
        five_hours_resets: short.and_then(|w| w["resetsAt"].as_u64()).unwrap_or(0),
        week: week.map_or(0.0, |w| used(w)),
        week_resets: week.and_then(|w| w["resetsAt"].as_u64()).unwrap_or(0),
        models: vec![],
    })
}

/// A call's limit: the server starts in about a second (0.96 s measured, 2026-11-08).
const ASK: Duration = Duration::from_secs(15);

/// One request to `codex app-server` over stdio, after its handshake: its `result`. The server is killed after, answered or not.
fn ask(bin: &str, method: &str) -> Result<Value, String> {
    let what = format!("{bin} app-server {method}");
    let mut child = Command::new(bin)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{what}: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let out = child.stdout.take().expect("piped");
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let requests = [
        json!({"id": 0, "method": "initialize", "params": {"clientInfo": {"name": "dash", "version": "0"}}}),
        json!({"method": "initialized"}),
        json!({"id": 1, "method": method}),
    ];
    let mut stdin = child.stdin.take().expect("piped");
    let sent = requests
        .iter()
        .try_for_each(|r| writeln!(stdin, "{r}"))
        .map_err(|e| format!("{what}: {e}"));
    let deadline = std::time::Instant::now() + ASK;
    let answer = sent.and_then(|()| {
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = rx
                .recv_timeout(left)
                .map_err(|_| format!("{what}: no answer within {ASK:?}"))?;
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if reply["id"] != 1 {
                continue;
            }
            break match reply.get("result") {
                Some(result) => Ok(result.clone()),
                None => Err(format!("{what}: {}", reply["error"])),
            };
        }
    });
    // npm's `codex` is a node wrapper: a kill reaches it alone, and the native server it started exits on its stdin's end (tested
    // 2026-11-08). So the end first, then the kill of whatever is left.
    drop(stdin);
    let gone = std::time::Instant::now() + Duration::from_secs(2);
    while matches!(child.try_wait(), Ok(None)) && std::time::Instant::now() < gone {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    answer
}

/// The session's rollout, newest day first: its file names end with the session's id.
fn rollout(sessions: &Path, session: &str) -> Result<Option<PathBuf>, String> {
    let tail = format!("-{session}.jsonl");
    let listed = |dir: &Path| -> Result<Vec<PathBuf>, String> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("{}: {e}", dir.display())),
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort_by(|a, b| b.cmp(a));
        Ok(paths)
    };
    // ponytail: walks every day until the session's; its id is a UUIDv7, so its day could be read from it if this ever shows.
    for year in listed(sessions)? {
        for month in listed(&year)? {
            for day in listed(&month)? {
                let found = listed(&day)?.into_iter().find(|f| {
                    f.file_name()
                        .is_some_and(|n| n.to_string_lossy().ends_with(&tail))
                });
                if found.is_some() {
                    return Ok(found);
                }
            }
        }
    }
    Ok(None)
}

/// Codex's own words when its account is spent, or when it could not reach its server after its retries.
fn stopped(error: String) -> Outcome {
    let words = error.replace('’', "'").to_lowercase();
    if words.contains("hit your usage limit") || words.contains("usage limit reached") {
        Outcome::Limited(error)
    } else if error.contains("stream disconnected") || error.contains("error sending request") {
        Outcome::Offline(error)
    } else {
        Outcome::Ended(error)
    }
}

/// The last turn of a rollout: `None` while one runs, after an interrupt (he typed, as Claude's null stop), or with its last line
/// half written. A turn ends at `task_complete`, its message the outcome; an `error` with no message after it stopped the turn.
fn outcome_in(text: &str) -> Option<Outcome> {
    let (mut ended, mut error) = (None::<String>, None::<String>);
    // The turn's last prompt: injected context starts with a tag, and is none.
    let mut asked = String::new();
    let mut whole = true;
    for line in text.lines() {
        let parsed = serde_json::from_str::<Value>(line);
        whole = parsed.is_ok();
        let Ok(line) = parsed else { continue };
        let event = &line["payload"];
        if line["type"] == "response_item" && event["role"] == "user" {
            let texts = event["content"].as_array().into_iter().flatten();
            let mut texts = texts.filter_map(|c| c["text"].as_str());
            if let Some(text) = texts.rfind(|t| !t.starts_with('<')) {
                asked = text.to_string();
            }
        }
        if line["type"] != "event_msg" {
            continue;
        }
        match event["type"].as_str() {
            Some("task_started" | "turn_aborted") => (ended, error) = (None, None),
            Some("error") => error = event["message"].as_str().map(String::from),
            Some("task_complete") => {
                ended = Some(
                    event["last_agent_message"]
                        .as_str()
                        .unwrap_or_default()
                        .into(),
                );
            }
            _ => {}
        }
    }
    if !whole || asked == OPENER {
        return None;
    }
    match (ended.filter(|said| !said.is_empty()), error) {
        (Some(said), _) => Some(outcome_of(said)),
        (None, Some(error)) => Some(stopped(error)),
        (None, None) => None,
    }
}

impl Agent for Codex {
    fn screen(&self, shown: &str) -> Screen {
        let lines: Vec<&str> = shown.lines().collect();
        let foot = &lines[lines.len().saturating_sub(25)..];
        let menu: Vec<(&str, bool)> = foot.iter().filter_map(|l| option(l)).collect();
        if let Some((_, pointed)) = menu.iter().find(|(o, _)| ANSWERS.contains(o)) {
            return Screen::Keys(vec![if *pointed { "Enter" } else { "Down" }.to_string()]);
        }
        if foot.iter().any(|l| l.contains("Ask Codex to do anything")) {
            return Screen::Ready;
        }
        Screen::Starting
    }

    fn outcome(&self, _folder: &str, session: &str) -> Result<Option<Outcome>, String> {
        let Some(path) = rollout(&self.sessions, session)? else {
            return Ok(None);
        };
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(outcome_in(&text))
    }

    /// Its slash command, as Claude's: the first Enter closes the menu, the next one quits (recorded 2026-11-08).
    fn stop_words(&self) -> String {
        "/exit".into()
    }

    fn limits(&self) -> Result<Vec<Limit>, String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH);
        let now = now.map_err(|e| e.to_string())?.as_secs();
        let result = ask(&self.bin, "account/rateLimits/read")?;
        Ok(limit_in(&result, now).into_iter().collect())
    }

    /// What a ChatGPT login talks to.
    fn reachable(&self) -> bool {
        connects("chatgpt.com:443")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/codex")
            .join(name);
        std::fs::read_to_string(&path).expect("fixture")
    }

    fn codex(sessions: &Path) -> Codex {
        Codex {
            bin: "codex".into(),
            sessions: sessions.to_path_buf(),
        }
    }

    #[dash_test]
    fn each_start_dialog_is_answered_as_recorded_and_the_prompt_reads_ready() {
        let agent = codex(Path::new("/none"));
        let keys = |key: &str| Screen::Keys(vec![key.to_string()]);
        assert_eq!(agent.screen("$ make codex"), Screen::Starting);
        assert_eq!(
            agent.screen(&fixture("update.txt")),
            keys("Down"),
            "never npm"
        );
        let skip = fixture("update.txt")
            .replace("› 1. Update", "  1. Update")
            .replace("  2. Skip\n", "› 2. Skip\n");
        assert_eq!(agent.screen(&skip), keys("Enter"));
        assert_eq!(agent.screen(&fixture("trust.txt")), keys("Enter"));
        assert_eq!(agent.screen(&fixture("hooks.txt")), keys("Down"));
        assert_eq!(agent.screen(&fixture("hooks-down.txt")), keys("Enter"));
        assert_eq!(agent.screen(&fixture("ready.txt")), Screen::Ready);
        assert_eq!(agent.screen(&fixture("done.txt")), Screen::Ready);
        let quoted = "› Skip the tests, Yes, continue\n› Ask Codex to do anything";
        assert_eq!(agent.screen(quoted), Screen::Ready, "a sentence is no menu");
    }

    #[dash_test]
    fn the_outcome_is_the_last_ended_turn_of_its_recorded_rollout() {
        let recorded = fixture("rollout.jsonl");
        let lines: Vec<&str> = recorded.lines().collect();
        let upto = |n: usize| lines[..n].join("\n");
        assert_eq!(outcome_in(&upto(1)), None, "its first turn at work");
        let done = Some(Outcome::Done("ok\nTASK_DONE".into()));
        assert_eq!(outcome_in(&upto(4)), done);
        assert_eq!(outcome_in(&upto(5)), None, "a second turn at work");
        assert_eq!(outcome_in(&recorded), Some(Outcome::Ended("EXTRA".into())));
        // The opener's own turn, ended before the brief came: no outcome. The brief typed into it, or a turn of its own: the report.
        let prompt = |text: &str| {
            let content = json!([{"type": "input_text", "text": text}]);
            let message = json!({"type": "message", "role": "user", "content": content});
            json!({"type": "response_item", "payload": message}).to_string()
        };
        let event = |kind: &str, said: &str| {
            let payload = json!({"type": kind, "last_agent_message": said});
            json!({"type": "event_msg", "payload": payload}).to_string()
        };
        let report = "ok\nTASK_DONE";
        let started = || event("task_started", "");
        let opener = [
            started(),
            prompt(OPENER),
            event("task_complete", "Waiting."),
        ]
        .join("\n");
        assert_eq!(
            outcome_in(&opener),
            None,
            "the opener's turn answered no brief"
        );
        let brief = prompt("the brief");
        let into = [
            started(),
            prompt(OPENER),
            brief.clone(),
            event("task_complete", report),
        ];
        assert_eq!(outcome_in(&into.join("\n")), done);
        let after = [started(), brief, event("task_complete", report)].join("\n");
        assert_eq!(outcome_in(&format!("{opener}\n{after}")), done);
        let half = format!("{}\n{{\"type\":\"ev", upto(4));
        assert_eq!(outcome_in(&half), None, "half written");
        let aborted = json!({"type": "event_msg", "payload": {"type": "turn_aborted"}});
        let aborted = format!("{}\n{aborted}", upto(4));
        assert_eq!(outcome_in(&aborted), None, "he interrupted it");
        let error = |message: &str| {
            let error =
                json!({"type": "event_msg", "payload": {"type": "error", "message": message}});
            outcome_in(&format!("{}\n{error}", upto(6)))
        };
        let spent = "You've hit your usage limit. Upgrade to Plus to continue using Codex";
        assert_eq!(error(spent), Some(Outcome::Limited(spent.into())));
        let curly = "You’ve hit your usage limit for GPT-5.6-Terra";
        assert_eq!(error(curly), Some(Outcome::Limited(curly.into())));
        let dropped = "stream disconnected before completion: error sending request";
        assert_eq!(error(dropped), Some(Outcome::Offline(dropped.into())));
        let refused = Some(Outcome::Ended("bad request".into()));
        assert_eq!(error("bad request"), refused);
    }

    #[dash_test]
    fn a_session_s_rollout_is_found_by_its_id_and_none_yet_waits() {
        let dir = TestDir::new("codex-rollout");
        let agent = codex(&dir.0);
        let session = "01a1bc43-39c8-7be0-ae2c-15d36567944d";
        assert_eq!(
            agent.outcome("/w", session).expect("read"),
            None,
            "no sessions folder yet"
        );
        let day = dir.0.join("2026/11/08");
        std::fs::create_dir_all(&day).expect("dirs");
        std::fs::create_dir_all(dir.0.join("2026/11/09")).expect("a later day");
        let file = day.join(format!("rollout-2026-11-08T18-44-44-{session}.jsonl"));
        std::fs::write(&file, fixture("rollout.jsonl")).expect("rollout");
        assert_eq!(
            agent.outcome("/w", session).expect("read"),
            Some(Outcome::Ended("EXTRA".into()))
        );
        assert_eq!(agent.outcome("/w", "other").expect("read"), None);
    }

    #[dash_test]
    fn the_recorded_reply_reads_as_its_30_day_window() {
        let reply: Value = serde_json::from_str(&fixture("rate-limits.json")).expect("json");
        let result = &reply["result"];
        let limit = limit_in(result, 1_794_160_000).expect("a limit");
        assert_eq!(
            limit,
            Limit {
                key: false,
                account: "codex".into(),
                five_hours: 0.0,
                five_hours_resets: 0,
                week: 0.0,
                week_resets: 1_796_751_652,
                models: vec![],
            }
        );
        let paid = json!({"rateLimits": {
            "primary": {"usedPercent": 40, "windowDurationMins": 300, "resetsAt": 2000},
            "secondary": {"usedPercent": 100, "windowDurationMins": 10080, "resetsAt": 9000}}});
        let limit = limit_in(&paid, 1000).expect("a limit");
        assert_eq!(
            (limit.five_hours, limit.week, limit.week_resets),
            (40.0, 100.0, 9000)
        );
        assert!(!limit.room());
        let past = limit_in(&paid, 5000).expect("a limit");
        assert_eq!(
            (past.five_hours, past.week),
            (0.0, 100.0),
            "its 5 hours reset"
        );
        assert_eq!(limit_in(&json!({}), 0), None, "no window, no reading");
        let unstated = json!({"rateLimits": {"primary": {"usedPercent": 100, "resetsAt": 9000}}});
        let limit = limit_in(&unstated, 1000).expect("a limit");
        assert_eq!(
            (limit.five_hours, limit.week),
            (0.0, 100.0),
            "a window of no length holds"
        );
    }

    /// The exchange itself, on a stand-in server that answers as the recorded one did.
    #[dash_test]
    fn limits_come_from_app_server_s_reply() {
        let dir = TestDir::new("codex-ask");
        let reply = dir.0.join("reply.json");
        std::fs::write(&reply, fixture("rate-limits.json")).expect("reply");
        let bin = dir.0.join("codex");
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = app-server ] || exit 2\nread a\necho '{{\"id\":0,\"result\":{{}}}}'\nread b\nread c\n\
             case \"$c\" in *account/rateLimits/read*) cat '{}';; esac\ncat >/dev/null\n",
            reply.display()
        );
        std::fs::write(&bin, script).expect("script");
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        let agent = Codex {
            bin: bin.display().to_string(),
            sessions: dir.0.clone(),
        };
        let limits = agent.limits().expect("limits");
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].account, "codex");
        assert_eq!(limits[0].week_resets, 1_796_751_652);
        let missing = Codex {
            bin: dir.0.join("none").display().to_string(),
            sessions: dir.0.clone(),
        };
        assert!(missing.limits().is_err());
    }
}
