//! Core's Agent for Claude Code: its start screen, and a session's outcome from its transcript.
//!
//! Point p-outcome: one folder holds every account's transcripts, since Dashboard.ClaudeAccounts links `projects` into each
//! profile, so which account the launch picked changes nothing here. A task's folder is resolved at `dash task add`: Claude
//! names the transcript's folder after the real path.
use dash_core::tasks::{Agent, Limit, Outcome, Screen};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const TRUST: &str = "Yes, I trust this folder";

pub struct ClaudeCode {
    /// `~/.claude/projects`.
    pub projects: PathBuf,
    /// `~/.claude/weekly-limit`, what the status line writes per session, each file naming its account's folder.
    pub limits: PathBuf,
}

/// The status line's files read, newest first, for each account's newest. Older ones are of sessions long gone.
const READINGS: usize = 200;

/// An account's name from its folder, as `dash claude-accounts` prints it.
fn account_of(config_dir: &str) -> String {
    match config_dir.split_once("/accounts/") {
        Some((_, name)) => name.trim_end_matches('/').to_string(),
        None => "default".to_string(),
    }
}

/// Newest first, each account's first file: its spend then, a window past its reset read as empty.
fn limits_in(newest_first: impl Iterator<Item = String>, now: u64) -> Vec<Limit> {
    let mut seen = HashMap::new();
    for text in newest_first {
        let Ok(reading) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let rates = &reading["rate_limits"];
        let Some(config) = reading["config_dir"].as_str().filter(|_| rates.is_object()) else {
            continue;
        };
        let used = |window: &str| {
            let resets = rates[window]["resets_at"].as_u64().unwrap_or(0);
            let used = rates[window]["used_percentage"].as_f64().unwrap_or(0.0);
            if resets > now { used } else { 0.0 }
        };
        let account = account_of(config);
        let limit = Limit {
            account: account.clone(),
            five_hours: used("five_hour"),
            week: used("seven_day"),
            week_resets: rates["seven_day"]["resets_at"].as_u64().unwrap_or(0),
        };
        seen.entry(account).or_insert(limit);
    }
    seen.into_values().collect()
}

fn newest(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    Ok(files.into_iter().take(READINGS).map(|(_, p)| p).collect())
}

/// Claude's folder name for a working directory: every character but a letter or digit becomes a dash.
fn slug(folder: &str) -> String {
    let dash = |c: char| if c.is_ascii_alphanumeric() { c } else { '-' };
    folder.chars().map(dash).collect()
}

/// Whether this line is the trust menu's option, and has the pointer. A conversation quoting it is no menu: only a line
/// holding nothing else counts.
fn trust_option(line: &str) -> Option<bool> {
    let line = line.trim_matches(|c: char| c.is_whitespace() || c == '│');
    let rest = line.strip_prefix('❯').map(str::trim_start);
    let words = rest.unwrap_or(line);
    let words = words.trim_start_matches(|c: char| c.is_ascii_digit());
    (words.strip_prefix(". ").unwrap_or(words) == TRUST).then_some(rest.is_some())
}

/// A session idle with no block is `Ended`, even with background work of its own: phase 2's watch judges those.
fn outcome_of(said: String) -> Outcome {
    // The word alone on its line, as Dashboard.Wake's @marks reads it: a sentence naming it is no block.
    let is_block = |l: &&str| matches!(l.trim(), "TASK_DONE" | "TASK_BLOCKED");
    match said.lines().rev().find(is_block) {
        Some(l) if l.trim() == "TASK_DONE" => Outcome::Done(said),
        Some(_) => Outcome::Blocked(said),
        None => Outcome::Ended(said),
    }
}

impl Agent for ClaudeCode {
    fn screen(&self, shown: &str) -> Screen {
        let lines: Vec<&str> = shown.lines().collect();
        let foot = &lines[lines.len().saturating_sub(25)..];
        if let Some(pointed) = foot.iter().find_map(|l| trust_option(l)) {
            // A launch here only opens his own projects, already under --dangerously-skip-permissions, as session.sh does.
            return Screen::Keys(vec![if pointed { "Enter" } else { "Down" }.to_string()]);
        }
        if foot
            .iter()
            .any(|l| l.contains("bypass permissions") || l.contains("? for shortcuts"))
        {
            return Screen::Ready;
        }
        Screen::Starting
    }

    fn outcome(&self, folder: &str, session: &str) -> Result<Option<Outcome>, String> {
        let path = self
            .projects
            .join(slug(folder))
            .join(format!("{session}.jsonl"));
        let failed = |e: std::io::Error| format!("{}: {e}", path.display());
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(failed(e)),
        };
        // ponytail: reads from the start on each idle look, one line held at a time; from the end if a look's time ever shows.
        // Of the last word in the conversation, its id, stop_reason and text: a tool's result or a new prompt after an end_turn
        // means a turn still runs. One message's blocks come on lines of their own, each carrying its stop_reason.
        let (mut id, mut stop, mut said) = (Value::Null, Value::Null, Vec::<String>::new());
        let mut whole = true;
        for line in BufReader::new(file).split(b'\n') {
            let parsed = serde_json::from_slice::<Value>(&line.map_err(failed)?);
            whole = parsed.is_ok();
            let Ok(line) = parsed else { continue };
            let assistant = line["type"] == "assistant";
            if !assistant && (line["type"] != "user" || line["isMeta"] == true) {
                continue;
            }
            let message = &line["message"];
            if message["id"] != id {
                id = message["id"].clone();
                said.clear();
            }
            stop = message["stop_reason"].clone();
            if assistant && let Some(blocks) = message["content"].as_array() {
                let text = blocks.iter().filter_map(|b| b["text"].as_str());
                said.extend(text.map(String::from));
            }
        }
        // A last line still being written: the next look reads it whole. A spend limit's words are a stop_sequence of Claude
        // Code's own, so a turn ended by one, or by a refusal, is read like an end_turn.
        let ended = matches!(
            stop.as_str(),
            Some("end_turn" | "stop_sequence" | "refusal")
        );
        Ok((whole && ended && !said.is_empty()).then(|| outcome_of(said.join("\n"))))
    }

    fn stop_words(&self) -> String {
        "/exit".into()
    }

    fn limits(&self) -> Result<Vec<Limit>, String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH);
        let now = now.map_err(|e| e.to_string())?.as_secs();
        let files = newest(&self.limits)?.into_iter();
        Ok(limits_in(
            files.filter_map(|f| std::fs::read_to_string(f).ok()),
            now,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use serde_json::json;

    /// A message as a transcript holds it: its thinking and its text on two lines, both with its stop_reason.
    fn message(id: &str, stop: &str, text: &str) -> String {
        let line = |block| {
            let message = json!({"id": id, "stop_reason": stop, "content": [block]});
            json!({"type": "assistant", "message": message}).to_string()
        };
        let (thinking, text) = (
            json!({"type": "thinking"}),
            json!({"type": "text", "text": text}),
        );
        [line(thinking), line(text)].join("\n")
    }

    #[test]
    fn the_outcome_is_the_last_message_of_an_ended_turn() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agent-outcome");
        let projects = dir.0.join("-Users-n--herdr-worktrees-site-perso-rust-t-1");
        std::fs::create_dir_all(&projects).expect("dirs");
        let agent = ClaudeCode {
            projects: dir.0.clone(),
            limits: PathBuf::new(),
        };
        let read = |transcript: &[String]| {
            std::fs::write(projects.join("s.jsonl"), transcript.join("\n")).expect("transcript");
            agent
                .outcome("/Users/n/.herdr/worktrees/site_perso/rust-t_1", "s")
                .expect("read")
        };
        let said = |text: &str| message("m1", "end_turn", text);
        let tail = || json!({"type": "system", "subtype": "turn_duration"}).to_string();
        let user = json!({"type": "user", "message": {"content": "more"}}).to_string();
        let meta = json!({"type": "user", "isMeta": true}).to_string();
        let half = "{\"type\":\"as".to_string();

        let done = "fine\nTASK_DONE\ntask: t";
        assert_eq!(
            read(&[said(done), tail(), meta]),
            Some(Outcome::Done(done.into()))
        );
        let asks = "TASK_BLOCKED\nneeds: x";
        let last = [
            message("m0", "end_turn", "first"),
            message("m1", "end_turn", asks),
        ];
        assert_eq!(
            read(&last),
            Some(Outcome::Blocked(asks.into())),
            "its last message alone"
        );
        assert_eq!(
            read(&[said("I stopped")]),
            Some(Outcome::Ended("I stopped".into()))
        );
        assert_eq!(
            read(&[message("m1", "tool_use", "looking")]),
            None,
            "mid-turn"
        );
        assert_eq!(
            read(&[said("x"), user]),
            None,
            "a new turn after an end_turn"
        );
        assert_eq!(
            read(&[said("x"), half.clone()]),
            None,
            "a line half written"
        );
        let limit = "You've hit your monthly spend limit";
        assert_eq!(
            read(&[message("m1", "stop_sequence", limit)]),
            Some(Outcome::Ended(limit.into())),
            "a spend limit's message ends the turn"
        );
        let sentence = "TASK_DONE is what I print once it is green";
        assert_eq!(
            read(&[said(sentence)]),
            Some(Outcome::Ended(sentence.into())),
            "a sentence naming it is no block"
        );
        let indented = "fine\n  TASK_DONE  \ntask: t";
        assert_eq!(
            read(&[said(indented)]),
            Some(Outcome::Done(indented.into())),
            "the word alone on its line, however indented"
        );
        let thinking = said("x").lines().next().expect("a line").to_string();
        assert_eq!(
            read(std::slice::from_ref(&thinking)),
            None,
            "its text not there yet"
        );
        assert_eq!(read(&[thinking, half]), None, "its text half written");
    }

    #[test]
    fn a_transcript_not_there_yet_waits_and_one_unreadable_says_so() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agent-read");
        let agent = ClaudeCode {
            projects: dir.0.clone(),
            limits: PathBuf::new(),
        };
        assert_eq!(agent.outcome("/a", "none").expect("read"), None);
        std::fs::create_dir_all(dir.0.join("-a/s.jsonl")).expect("a folder where a file goes");
        assert!(agent.outcome("/a", "s").is_err());
    }

    #[test]
    fn each_account_s_limit_is_its_newest_reading_a_window_past_its_reset_empty() {
        let _budget = TestBudget::new();
        let reading = |dir: &str, week: u32, week_resets: u64, five: u32, five_resets: u64| {
            json!({"config_dir": dir, "rate_limits": {
                "seven_day": {"used_percentage": week, "resets_at": week_resets},
                "five_hour": {"used_percentage": five, "resets_at": five_resets}}})
            .to_string()
        };
        let files = [
            reading("/h/.claude/accounts/second", 33, 2000, 10, 500),
            "not json".to_string(),
            json!({"config_dir": "/h/.claude"}).to_string(),
            reading("/h/.claude", 100, 3000, 50, 2000),
            reading("/h/.claude/accounts/second", 99, 2000, 99, 2000),
        ];
        let mut limits = limits_in(files.into_iter(), 1000);
        limits.sort_by(|a, b| a.account.cmp(&b.account));
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].account, "default");
        assert_eq!((limits[0].five_hours, limits[0].week), (50.0, 100.0));
        assert_eq!(limits[1].account, "second");
        assert_eq!(
            (limits[1].five_hours, limits[1].week),
            (0.0, 33.0),
            "newest, its 5 hours reset"
        );
        assert_eq!(limits[1].week_resets, 2000);
    }

    #[test]
    fn the_start_screen_reads_as_claude_draws_it() {
        let _budget = TestBudget::new();
        let agent = ClaudeCode {
            projects: PathBuf::new(),
            limits: PathBuf::new(),
        };
        let keys = |key: &str| Screen::Keys(vec![key.to_string()]);
        assert_eq!(agent.screen("$ make claude"), Screen::Starting);
        let numbered = "│ ❯ 1. No, exit │\n│   2. Yes, I trust this folder │\n";
        assert_eq!(agent.screen(numbered), keys("Down"));
        assert_eq!(
            agent.screen("  No, exit\n ❯ Yes, I trust this folder\n"),
            keys("Enter")
        );
        let quoted = "I read: Yes, I trust this folder is the option\n  ⏵⏵ bypass permissions on";
        assert_eq!(agent.screen(quoted), Screen::Ready);
        let far = format!("Yes, I trust this folder\n{}", "x\n".repeat(30));
        assert_eq!(
            agent.screen(&far),
            Screen::Starting,
            "only the screen's foot"
        );
    }
}
