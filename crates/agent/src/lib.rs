//! Core's Agent for Claude Code: its start screen, and a session's outcome from its transcript. `codex` is the second kind.
//!
//! Point p-outcome: one folder holds every account's transcripts, since Dashboard.ClaudeAccounts links `projects` into each
//! profile, so which account the launch picked changes nothing here. A task's folder is resolved at `dash task add`: Claude
//! names the transcript's folder after the real path.
pub mod login;

use dash_core::days;
use dash_core::tasks::{Agent, Limit, Outcome, Scoped, Screen};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod codex;

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

/// Each login, as `Dashboard.ClaudeAccounts`' profiles: `default` (`claude` itself), then each folder of `claude/accounts`
/// named as its @name allows, sorted.
pub fn logins(claude: &Path) -> Vec<String> {
    let folders = std::fs::read_dir(claude.join("accounts"))
        .into_iter()
        .flatten();
    let mut names: Vec<String> = (folders.flatten())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| login::shaped(n))
        .collect();
    names.sort();
    std::iter::once("default".to_string())
        .chain(names)
        .collect()
}

/// What `usage` sends: Claude Code's own `get_usage` control request, as `Dashboard.ClaudeAccounts.usage_command` sends it.
/// No model call, no 5-hour window opened, the CLI reading its own login: nothing here reads a token. `skip_behaviors` spares
/// the scan of every transcript (8.5 GB in 26 s, task usage-sync-reads-transcripts).
const USAGE: &str = r#"{"type":"control_request","request_id":"usage","request":{"subtype":"get_usage","skip_behaviors":true}}"#;

/// A login's 5 hours and week as Claude answers `get_usage` under `config` (`None`: `~/.claude`). Some 7.6 s a call
/// (2026-11-08): a page's read runs it off its routing thread. `exec`s down to claude, so the limit's kill ends claude itself.
pub fn usage(account: &str, config: Option<&Path>) -> Result<Limit, String> {
    let script = format!("exec claude \"$@\" <<'EOF'\n{USAGE}\nEOF\n");
    let set = config.map(|dir| format!("CLAUDE_CONFIG_DIR={}", dir.display()));
    let mut args: Vec<&str> = match &set {
        Some(set) => vec![set],
        None => vec!["-u", "CLAUDE_CONFIG_DIR"],
    };
    args.extend([
        "sh",
        "-c",
        &script,
        "sh",
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
    ]);
    let limit = Duration::from_secs(120);
    let out = dash_process::run("env", &args, Some(&std::env::temp_dir()), limit)?;
    usage_of(account, &out)
}

/// Unix seconds of `get_usage`'s `2026-11-15T04:59:59.694536+00:00`, a fraction rounded up: the status line writes that
/// reset as 05:00:00, so both read the same minute.
fn unix_of(iso: &str) -> Option<u64> {
    // Only ASCII from here: a byte range then never splits a character.
    let time = iso.get(10..19).filter(|_| iso.is_ascii())?;
    let digits = |r: std::ops::Range<usize>| {
        let field = &time[r];
        field
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| field.parse::<i64>().ok())?
    };
    if time.as_bytes()[0] != b'T' || time.as_bytes()[3] != b':' || time.as_bytes()[6] != b':' {
        return None;
    }
    let (h, mi, s) = (digits(1..3)?, digits(4..6)?, digits(7..9)?);
    let rest = &iso[19..];
    let (fraction, zone) = match rest.strip_prefix('.') {
        Some(r) => r.split_at(r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len())),
        None => ("", rest),
    };
    let zone_part = |r: std::ops::Range<usize>| {
        let field = zone.get(r)?;
        field
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| field.parse::<i64>().ok())?
    };
    let offset = match zone {
        "Z" => 0,
        z if z.len() == 6
            && z.as_bytes()[3] == b':'
            && (z.starts_with('+') || z.starts_with('-')) =>
        {
            let (zh, zm) = (zone_part(1..3)?, zone_part(4..6)?);
            (zh * 3600 + zm * 60) * if z.starts_with('-') { -1 } else { 1 }
        }
        _ => return None,
    };
    let up = i64::from(fraction.bytes().any(|b| b != b'0'));
    let at = days(iso)? * 86_400 + h * 3600 + mi * 60 + s + up - offset;
    (h < 24 && mi < 60 && s < 61 && at > 0).then_some(at as u64)
}

/// A `get_usage` answer's figures, as `Dashboard.ClaudeAccounts.usage_figures` reads them. A window with no reset and 0 % is
/// none open: 0 %, its reset `1`, long past. Any other window with no reset it can read is no reading: its reset 0.
fn usage_of(account: &str, out: &str) -> Result<Limit, String> {
    let answers = out
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok());
    let mut answers = answers.filter(|l| {
        let r = &l["response"];
        l["type"] == "control_response" && r["request_id"] == "usage" && r["subtype"] == "success"
    });
    let answer = answers.next().ok_or("no get_usage answer")?;
    let rates = &answer["response"]["response"]["rate_limits"];
    // A changed shape reads as no figures, never as 0 %: the request is marked experimental in Claude Code.
    if !rates["seven_day"].is_object() {
        return Err("no week in the get_usage answer".into());
    }
    let window = |name: &str| {
        let w = &rates[name];
        let used = w["utilization"].as_f64();
        match (used, w["resets_at"].as_str().and_then(unix_of)) {
            (Some(used), Some(resets)) => (used, resets),
            (Some(used), None) if used == 0.0 && w["resets_at"].is_null() => (0.0, 1),
            _ => (0.0, 0),
        }
    };
    let ((five_hours, five_hours_resets), (week, week_resets)) =
        (window("five_hour"), window("seven_day"));
    Ok(Limit {
        key: false,
        account: account.into(),
        five_hours,
        five_hours_resets,
        week,
        week_resets,
        models: scoped_of(&rates["model_scoped"]),
    })
}

/// Each model's own allowance in a `get_usage` answer (RFC 20h: Fable's, 2026-11-10: `model_scoped`), named as `claude --model`
/// takes it. One with no reset it can read keeps 0, a reset long past: no reading.
fn scoped_of(models: &Value) -> Vec<Scoped> {
    let models = models.as_array().into_iter().flatten();
    let one = |m: &Value| {
        Some(Scoped {
            model: m["display_name"].as_str()?.to_ascii_lowercase(),
            used: m["utilization"].as_f64()?,
            resets: m["resets_at"].as_str().and_then(unix_of).unwrap_or(0),
        })
    };
    models.filter_map(one).collect()
}

/// What asks a login's figures: `usage` live, a stand-in in a test.
pub type Usage = fn(&str, Option<&Path>) -> Result<Limit, String>;

/// How long a login's remembered model allowances serve before the live picker asks `get_usage` again: some 7.6 s a login.
pub const MODELS_FRESH: Duration = Duration::from_secs(10 * 60);

/// Keeps `limit`'s model allowances in `dir`, a file per login, for the pick: the status line's readings carry none.
pub fn remember(dir: &Path, limit: &Limit) -> Result<(), String> {
    let models: Vec<Value> = (limit.models.iter())
        .map(|m| serde_json::json!({"model": m.model, "used": m.used, "resets": m.resets}))
        .collect();
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{}.json", limit.account));
    let text = serde_json::json!({"models": models}).to_string();
    // a rename, so a pick reading it at that moment reads the old answer or the new, never half of one
    let fresh = dir.join(format!(".{}.{}.json", limit.account, std::process::id()));
    let failed = |e: std::io::Error| format!("{}: {e}", file.display());
    std::fs::write(&fresh, text).map_err(failed)?;
    std::fs::rename(&fresh, &file).map_err(failed)
}

/// The model allowances `remember` kept for `account` in `dir`; none when no file reads.
pub fn remembered(dir: &Path, account: &str) -> Vec<Scoped> {
    let text = std::fs::read_to_string(dir.join(format!("{account}.json")));
    let value: Value = text
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let models = value["models"].as_array().into_iter().flatten();
    let one = |m: &Value| {
        Some(Scoped {
            model: m["model"].as_str()?.into(),
            used: m["used"].as_f64()?,
            resets: m["resets"].as_u64()?,
        })
    };
    models.filter_map(one).collect()
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
            five_hours_resets: rates["five_hour"]["resets_at"].as_u64().unwrap_or(0),
            week: used("seven_day"),
            week_resets: rates["seven_day"]["resets_at"].as_u64().unwrap_or(0),
            key: false,
            models: vec![],
        };
        seen.entry(account).or_insert(limit);
    }
    seen.into_values().collect()
}

fn newest(dir: &Path, cap: usize) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    Ok(files.into_iter().take(cap).map(|(_, p)| p).collect())
}

/// Claude's folder name for a working directory: every character but a letter or digit becomes a dash.
fn slug(folder: &str) -> String {
    let dash = |c: char| if c.is_ascii_alphanumeric() { c } else { '-' };
    folder.chars().map(dash).collect()
}

/// Whether this line is a menu's option `option`, and has the pointer. A conversation quoting it is no menu: only a line
/// holding nothing else counts.
fn menu_option(line: &str, option: &str) -> Option<bool> {
    let line = line.trim_matches(|c: char| c.is_whitespace() || c == '│');
    let rest = line.strip_prefix('❯').map(str::trim_start);
    let words = rest.unwrap_or(line);
    let words = words.trim_start_matches(|c: char| c.is_ascii_digit());
    (words.strip_prefix(". ").unwrap_or(words) == option).then_some(rest.is_some())
}

/// The trust menu's option.
fn trust_option(line: &str) -> Option<bool> {
    menu_option(line, TRUST)
}

/// The limit menu's first option, as Claude Code draws it (merge-gate's run sat on it, 2026-11-09).
const WAIT: &str = "Stop and wait for limit to reset";

/// The pane's last 25 lines, the blank ones a tall pane pads its foot with left out: where a menu or prompt shows.
fn foot(shown: &str) -> Vec<&str> {
    let lines: Vec<&str> = shown.trim_end().lines().collect();
    lines[lines.len().saturating_sub(25)..].to_vec()
}

/// Whether a line is Claude Code's limit screen, as `Dashboard.Sessions`' @limit reads it: it starts its line after Claude's frame,
/// so a sentence naming a limit is none. A monthly spend limit alone is money, which no reset brings back, so it is none either.
fn limit_line(line: &str) -> bool {
    let line = line.trim_start_matches(|c: char| {
        c.is_whitespace() || c.is_ascii_digit() || "⎿│●❯.".contains(c)
    });
    let line = line.to_lowercase();
    if let Some(rest) = line.strip_prefix("you've hit your ") {
        if rest.starts_with("monthly spend limit") {
            return rest.contains("session limit resets") || rest.contains("weekly limit resets");
        }
        let rest = ["usage ", "session ", "weekly ", "opus ", "sonnet "]
            .iter()
            .find_map(|kind| rest.strip_prefix(kind))
            .unwrap_or(rest);
        return rest
            .strip_prefix("limit")
            .is_some_and(|tail| !tail.starts_with(|c: char| c.is_alphanumeric() || c == '_'));
    }
    let line = line.strip_prefix("claude ").unwrap_or(&line);
    let resets = line.strip_prefix("your ").unwrap_or(line);
    line.starts_with("usage limit reached")
        || resets.starts_with("session limit resets")
        || resets.starts_with("weekly limit resets")
        || line.starts_with("stop and wait for limit to reset")
}

/// Whether a line is Claude Code's words when it cannot connect, as `Dashboard.Network`'s @error reads them.
fn offline_line(line: &str) -> bool {
    let line = line.trim_start_matches(|c: char| c.is_whitespace() || "⏺●".contains(c));
    line.replace('’', "'")
        .starts_with("API Error: Can't reach the API server")
        || line.starts_with("Unable to connect to API")
}

/// What `reachable` connects to, as `Dashboard.Network`'s probe: its name looked up too, since a DNS failure is one of these stops.
const API: &str = "api.anthropic.com:443";

/// ponytail: a lookup on a dead network may outlast the connect's timeout; only a run's own wait sits behind it.
fn connects(address: &str) -> bool {
    let timeout = Duration::from_secs(5);
    let reached = |a| TcpStream::connect_timeout(&a, timeout).is_ok();
    address
        .to_socket_addrs()
        .is_ok_and(|mut addrs| addrs.any(reached))
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

impl ClaudeCode {
    /// `limits` at `now` (Unix seconds), from the `cap` newest files: a pick reads `READINGS`, the /agents page every one,
    /// since a login idle for a day sits past them (default's newest was file 274 of 1,955 on 2026-11-08). Each login's
    /// remembered model allowances join its reading.
    pub fn limits_at(&self, now: u64, cap: usize) -> Result<Vec<Limit>, String> {
        let files = newest(&self.limits, cap)?.into_iter();
        let mut limits = limits_in(files.filter_map(|f| std::fs::read_to_string(f).ok()), now);
        for limit in &mut limits {
            limit.models = remembered(&self.models(), &limit.account);
        }
        Ok(limits)
    }

    /// Where each login's model allowances are kept, beside the status line's readings.
    pub fn models(&self) -> PathBuf {
        self.limits.with_file_name("model-limits")
    }

    /// Asks `ask` again for each login whose remembered allowances are older than `fresh`, every login at once, and keeps what
    /// it answers. A failed ask keeps the last answer, only its time renewed, so a login gone costs one ask per `fresh`, never
    /// one per pass, and a spent Fable never reads as whole for want of an answer.
    /// ponytail: the pass waits on the slowest ask, some 8 s every 10 minutes; a thread of its own if a pass's time shows it.
    pub fn freshen(&self, ask: Usage, fresh: Duration) {
        let (Some(claude), dir) = (self.limits.parent(), self.models()) else {
            return;
        };
        let old = |login: &String| {
            let at =
                std::fs::metadata(dir.join(format!("{login}.json"))).and_then(|m| m.modified());
            at.map_or(true, |at| at.elapsed().map_or(true, |age| age > fresh))
        };
        let stale: Vec<String> = logins(claude).into_iter().filter(old).collect();
        std::thread::scope(|s| {
            for login in &stale {
                let dir = &dir;
                s.spawn(move || {
                    let config = (login != "default").then(|| claude.join("accounts").join(login));
                    let kept = match ask(login, config.as_deref()) {
                        Ok(asked) => remember(dir, &asked),
                        Err(e) => {
                            eprintln!("models: {login}: {e}");
                            let last = Limit {
                                account: login.clone(),
                                models: remembered(dir, login),
                                ..Limit::default()
                            };
                            remember(dir, &last)
                        }
                    };
                    if let Err(e) = kept {
                        eprintln!("models: {e}");
                    }
                });
            }
        });
    }
}

impl Agent for ClaudeCode {
    fn screen(&self, shown: &str) -> Screen {
        let foot = foot(shown);
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
        let mut server_error = false;
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
            // An error the API answers with (a 500, an overload), which no word of the session's can write, waits as a dead network does.
            server_error = line["isApiErrorMessage"] == true && line["error"] == "server_error";
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
        // Claude Code's own stop, never a report quoting one: the session wrote no stop_sequence turn.
        let limited = stop.as_str() == Some("stop_sequence")
            && said.iter().any(|s| s.lines().any(limit_line));
        let offline = stop.as_str() == Some("stop_sequence")
            && (server_error || said.iter().any(|s| s.lines().any(offline_line)));
        Ok((whole && ended && !said.is_empty()).then(|| {
            let words = said.join("\n");
            if limited {
                Outcome::Limited(words)
            } else if offline {
                Outcome::Offline(words)
            } else {
                outcome_of(words)
            }
        }))
    }

    fn stop_words(&self) -> String {
        "/exit".into()
    }

    // Each assistant message once (its blocks come on lines of their own, each carrying its usage): input, output and cache
    // writes, cache reads left out (they cost a tenth and would drown the rest).
    fn tokens(&self, folder: &str, session: &str) -> Result<u64, String> {
        let path = self
            .projects
            .join(slug(folder))
            .join(format!("{session}.jsonl"));
        let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut seen = std::collections::HashSet::new();
        let mut tokens = 0;
        for line in BufReader::new(file).split(b'\n').map_while(Result::ok) {
            let Ok(line) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            let message = &line["message"];
            // a row with no id is its own message: counted each time
            let id = &message["id"];
            if line["type"] != "assistant" || (!id.is_null() && !seen.insert(id.to_string())) {
                continue;
            }
            let usage = &message["usage"];
            let spent = [
                "input_tokens",
                "output_tokens",
                "cache_creation_input_tokens",
            ];
            tokens += spent.iter().filter_map(|k| usage[k].as_u64()).sum::<u64>();
        }
        Ok(tokens)
    }

    fn limits(&self) -> Result<Vec<Limit>, String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH);
        self.limits_at(now.map_err(|e| e.to_string())?.as_secs(), READINGS)
    }

    fn reachable(&self) -> bool {
        connects(API)
    }

    /// Its limit menu's first option on one of the last lines, read as the trust menu's: alone on its line.
    fn limit_menu(&self, shown: &str) -> bool {
        foot(shown).iter().any(|l| menu_option(l, WAIT).is_some())
    }
}

/// Section 20b's key kinds, each the variable its tool reads the key from, as bin/secret's file names it too. Gemini and OpenAI come
/// with their tool's agent (rust-codex-agent).
pub fn key_var(kind: &str) -> Option<&'static str> {
    (kind == "anthropic").then_some("ANTHROPIC_API_KEY")
}

/// A headless run's arguments: one prompt, its answer one JSON value. No tools, so nothing it runs inherits the key or prints
/// it into a transcript, and `budget` dollars at most, which Claude Code stops at itself (`--max-budget-usd`, print mode only).
/// A prompt is never read as a flag: the caller refuses a leading dash, and it comes after `-p`, which takes no value.
pub fn headless(prompt: &str, budget: f64) -> Vec<String> {
    let budget = format!("{budget:.4}");
    let args = [
        "--tools",
        "",
        "--max-budget-usd",
        &budget,
        "--output-format",
        "json",
        "-p",
        prompt,
    ];
    args.map(String::from).to_vec()
}

/// What a headless run said and what it cost in US dollars, from its answer: one result object, or a list whose last result is
/// (the probe of 2026-11-05, section 20b). Err when no cost is there: a run whose spend is unknown is never counted as free.
pub fn headless_outcome(answer: &str) -> Result<(String, f64), String> {
    let value: Value = serde_json::from_str(answer.trim()).map_err(|e| format!("not JSON: {e}"))?;
    let result = match &value {
        Value::Array(entries) => entries.iter().rev().find(|e| e["type"] == "result"),
        object => Some(object),
    };
    let result = result.ok_or("no result in its answer")?;
    let usd = result["total_cost_usd"].as_f64();
    let usd = usd.ok_or("no total_cost_usd in its result")?;
    let said = result["result"].as_str().unwrap_or_default().to_string();
    Ok((said, usd))
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
            "a spend limit's message ends the turn, and no reset brings it back"
        );
        for limit in [
            "You've hit your weekly limit · resets Nov 8 at 6am (Europe/Rome)",
            "You've hit your limit · resets 3pm",
            "You've hit your monthly spend limit · your session limit resets 4:40pm",
            // merge-gate's transcript under Claude's limit menu, 2026-11-09 10:27.
            "You've hit your monthly spend limit · raise it at https://claude.ai/settings/usage?from=cc_cli_limit_message · your weekly limit resets Nov 11 at 5pm (Europe/Paris)",
            "⎿ Claude usage limit reached",
        ] {
            assert_eq!(
                read(&[message("m1", "stop_sequence", limit)]),
                Some(Outcome::Limited(limit.into())),
                "{limit}"
            );
        }
        let quoted = "You've hit your limit · resets 3pm";
        assert_eq!(
            read(&[said(quoted)]),
            Some(Outcome::Ended(quoted.into())),
            "a session's own end_turn quoting it is no limit"
        );
        let named = "Your limits reset at 3pm, or so it says";
        assert_eq!(
            read(&[message("m1", "stop_sequence", named)]),
            Some(Outcome::Ended(named.into())),
            "only its own words"
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

    /// merge-gate's run sat on this menu as a question (2026-11-09): its first option on a line of its own reads as the menu.
    #[test]
    fn claude_s_limit_menu_is_read_from_its_last_lines() {
        let _budget = TestBudget::new();
        let agent = ClaudeCode {
            projects: PathBuf::new(),
            limits: PathBuf::new(),
        };
        let menu = "⎿ You've hit your limit · resets 5pm (Europe/Paris)\n\n  What do you want to do?\n\n  ❯ 1. Stop and wait for limit to reset\n    2. Upgrade your plan\n";
        assert!(agent.limit_menu(menu));
        assert!(
            !agent.limit_menu("● I would Stop and wait for limit to reset here\n"),
            "a sentence is no menu"
        );
        let listed = "  1. Stop and wait for limit to reset, then retry\n";
        assert!(!agent.limit_menu(listed), "a list item of a reply is none");
        let tall = format!("{menu}{}", "\n".repeat(40));
        assert!(agent.limit_menu(&tall), "a tall pane's blank foot");
        let scrolled = format!(
            "❯ 1. Stop and wait for limit to reset\n{}",
            "x\n".repeat(30)
        );
        assert!(!agent.limit_menu(&scrolled), "a menu long gone");
    }

    /// Task rust-pr-one-owner, done-when 4: a session's tokens, each message once though its blocks repeat its usage, cache
    /// reads left out; a transcript that is not there says so.
    #[test]
    fn a_session_s_tokens_count_each_message_once_without_cache_reads() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agent-tokens");
        std::fs::create_dir_all(dir.0.join("-a")).expect("dirs");
        let agent = ClaudeCode {
            projects: dir.0.clone(),
            limits: PathBuf::new(),
        };
        let usage = json!({"input_tokens": 10, "output_tokens": 200, "cache_creation_input_tokens": 1000,
            "cache_read_input_tokens": 50_000});
        let line = |id: &str| {
            json!({"type": "assistant", "message": {"id": id, "usage": usage}}).to_string()
        };
        let user = json!({"type": "user", "message": {"usage": usage}}).to_string();
        let no_id = json!({"type": "assistant", "message": {"usage": usage}}).to_string();
        let rows = [
            line("m1"),
            line("m1"),
            user,
            line("m2"),
            no_id.clone(),
            no_id,
            "{half".into(),
        ];
        std::fs::write(dir.0.join("-a/s.jsonl"), rows.join("\n")).expect("transcript");
        assert_eq!(agent.tokens("/a", "s"), Ok(4 * 1_210));
        assert!(agent.tokens("/a", "none").is_err());
    }

    /// Case: rust-linear-view, session 543b81aa, 2026-11-07 17:54: the row as its transcript holds it, its stop a stop_sequence.
    #[test]
    fn an_api_or_dns_error_reads_as_offline_and_a_session_quoting_one_does_not() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agent-offline");
        std::fs::create_dir_all(dir.0.join("-a")).expect("dirs");
        let agent = ClaudeCode {
            projects: dir.0.clone(),
            limits: PathBuf::new(),
        };
        let read = |row: String| {
            std::fs::write(dir.0.join("-a/s.jsonl"), row).expect("transcript");
            agent.outcome("/a", "s").expect("read")
        };
        let dns = "API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)";
        for words in [
            dns,
            "⏺ API Error: Can’t reach the API server",
            "Unable to connect to API (ECONNREFUSED)",
        ] {
            assert_eq!(
                read(message("m1", "stop_sequence", words)),
                Some(Outcome::Offline(words.into())),
                "{words}"
            );
        }
        let answered = "API Error: 529 Overloaded";
        let row = |error: &str| {
            let message = json!({"id": "m1", "stop_reason": "stop_sequence",
                "content": [{"type": "text", "text": answered}]});
            json!({"type": "assistant", "isApiErrorMessage": true, "error": error, "message": message})
                .to_string()
        };
        assert_eq!(
            read(row("server_error")),
            Some(Outcome::Offline(answered.into()))
        );
        assert_eq!(
            read(row("invalid_request")),
            Some(Outcome::Ended(answered.into())),
            "waiting cannot mend a refused request"
        );
        assert_eq!(
            read(message("m1", "end_turn", dns)),
            Some(Outcome::Ended(dns.into())),
            "a session's own end_turn quoting it is no drop"
        );
    }

    #[test]
    fn a_headless_answer_says_its_words_and_cost_and_one_with_no_cost_is_refused() {
        let _budget = TestBudget::new();
        let one = json!({"type": "result", "result": "hi", "total_cost_usd": 0.00055}).to_string();
        assert_eq!(headless_outcome(&one), Ok(("hi".into(), 0.00055)));
        let list = json!([{"type": "system"}, {"type": "result", "result": "yo", "total_cost_usd": 0.5},
            {"type": "other"}]);
        assert_eq!(headless_outcome(&list.to_string()), Ok(("yo".into(), 0.5)));
        assert!(headless_outcome(&json!({"type": "result"}).to_string()).is_err());
        assert!(headless_outcome("[]").is_err());
        assert!(headless_outcome("half {").is_err());
        assert_eq!(key_var("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(key_var("sk-ant-api03-x"), None);
        let args = headless("hi", 0.5);
        assert_eq!(
            args.join(" "),
            "--tools  --max-budget-usd 0.5000 --output-format json -p hi"
        );
    }

    #[test]
    fn what_connects_is_reachable_and_nothing_else() {
        let _budget = TestBudget::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address").to_string();
        assert!(connects(&address));
        drop(listener);
        assert!(!connects(&address), "nothing listens there now");
        assert!(!connects("not an address"));
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
        assert_eq!(limits[0].five_hours_resets, 2000);
        let week_only = json!({"config_dir": "/h/.claude", "rate_limits": {
            "seven_day": {"used_percentage": 80, "resets_at": 2000}}});
        let limits = limits_in([week_only.to_string()].into_iter(), 1000);
        assert_eq!(
            (limits[0].five_hours, limits[0].five_hours_resets),
            (0.0, 0),
            "no 5-hour reading: no reset either"
        );
    }

    /// Case: default's answer on 2026-11-08, its other windows cut: no 5 hours, its week at 1 %.
    #[test]
    fn a_get_usage_answer_reads_as_its_two_windows_and_a_changed_shape_as_none() {
        let _budget = TestBudget::new();
        let answer = |rates: serde_json::Value| {
            let response = json!({"subtype": "success", "request_id": "usage",
                "response": {"rate_limits_available": true, "rate_limits": rates}});
            let init = json!({"type": "system", "subtype": "init"});
            format!(
                "{init}\nnot json\n{}\n",
                json!({"type": "control_response", "response": response})
            )
        };
        let week = json!({"utilization": 1, "resets_at": "2026-11-15T04:59:59.694536+00:00"});
        let read = usage_of(
            "default",
            &answer(json!({"five_hour": null, "seven_day": week})),
        )
        .expect("figures");
        assert_eq!(
            read,
            Limit {
                key: false,
                account: "default".into(),
                five_hours: 0.0,
                five_hours_resets: 0,
                week: 1.0,
                week_resets: 1_794_718_800,
                models: vec![],
            },
            "no 5 hours is no reading; its reset rounded up to the status line's 05:00"
        );
        let none_open = json!({"utilization": 0, "resets_at": null});
        let read = usage_of(
            "a",
            &answer(json!({"five_hour": none_open, "seven_day": week})),
        );
        assert_eq!(
            read.map(|l| (l.five_hours, l.five_hours_resets)),
            Ok((0.0, 1)),
            "none open: 0 %, past"
        );
        let unread = json!({"utilization": 40, "resets_at": "soon"});
        let read = usage_of(
            "a",
            &answer(json!({"five_hour": unread, "seven_day": week})),
        );
        assert_eq!(
            read.map(|l| l.five_hours_resets),
            Ok(0),
            "a reset it cannot read is no reading"
        );
        assert!(
            usage_of("a", &answer(json!({"seven_day": null}))).is_err(),
            "a changed shape"
        );
        assert!(usage_of("a", "").is_err(), "no answer");
    }

    /// Task rust-pools-fable, done-when 1: default's real answer of 2026-11-10 22:40 UTC, recorded whole (fixtures/claude): Fable's
    /// own allowance sits in `model_scoped`, 2 % used, as a pool the route names; kept per login for the pick, whose status-line
    /// readings carry none, and joined to that login's reading.
    #[test]
    fn fable_s_allowance_is_read_per_login_from_a_recorded_get_usage_answer() {
        let _budget = TestBudget::new();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/claude/get-usage-2026-11-10.jsonl");
        let recorded = std::fs::read_to_string(fixture).expect("the recorded answer");
        let read = usage_of("default", &recorded).expect("figures");
        let fable = Scoped {
            model: "fable".into(),
            used: 2.0,
            resets: unix_of("2026-11-15T05:00:00.246797+00:00").expect("its reset"),
        };
        assert_eq!(read.models, vec![fable.clone()]);
        assert_eq!(
            (read.week, read.five_hours_resets),
            (37.0, 0),
            "the windows as before"
        );
        // kept per login, then joined to that login's status-line reading, the one a pick reads
        let dir = TestDir::new("models-kept");
        let limits = dir.0.join("weekly-limit");
        std::fs::create_dir_all(&limits).expect("dir");
        let agent = ClaudeCode {
            projects: dir.0.join("projects"),
            limits: limits.clone(),
        };
        remember(&agent.models(), &read).expect("kept");
        let reading = json!({"config_dir": "/h/.claude", "rate_limits": {
            "seven_day": {"used_percentage": 37, "resets_at": u64::MAX}}});
        std::fs::write(limits.join("a.json"), reading.to_string()).expect("reading");
        let other = json!({"config_dir": "/h/.claude/accounts/second", "rate_limits": {
            "seven_day": {"used_percentage": 5, "resets_at": u64::MAX}}});
        std::fs::write(limits.join("b.json"), other.to_string()).expect("reading");
        let mut seen = agent.limits_at(1_000, usize::MAX).expect("limits");
        seen.sort_by(|a, b| a.account.cmp(&b.account));
        assert_eq!(seen[0].models, vec![fable], "default's own");
        assert_eq!(
            seen[1].models,
            vec![],
            "second has none kept: no reading, never another's"
        );
        // a shape with no model_scoped reads as none, never as a failure
        let bare = recorded.replace("model_scoped", "renamed");
        assert_eq!(usage_of("default", &bare).map(|l| l.models), Ok(vec![]));
    }

    /// /code-review of rust-pools-fable, finding 2: an ask that fails keeps the last answer, so a Fable spent never reads as whole
    /// for want of one; a fresh file is not asked again.
    #[test]
    fn a_failed_ask_keeps_the_last_fable_reading() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("models-freshen");
        let limits = dir.0.join("weekly-limit");
        std::fs::create_dir_all(&limits).expect("dir");
        let agent = ClaudeCode {
            projects: dir.0.join("projects"),
            limits,
        };
        fn spent(account: &str, _: Option<&Path>) -> Result<Limit, String> {
            let fable = Scoped {
                model: "fable".into(),
                used: 100.0,
                resets: 9,
            };
            let models = vec![fable];
            let account = account.into();
            Ok(Limit {
                account,
                models,
                ..Limit::default()
            })
        }
        fn fails(_: &str, _: Option<&Path>) -> Result<Limit, String> {
            Err("timed out".into())
        }
        agent.freshen(spent, Duration::ZERO);
        let kept = remembered(&agent.models(), "default");
        assert_eq!(kept.len(), 1);
        agent.freshen(fails, Duration::ZERO);
        assert_eq!(
            remembered(&agent.models(), "default"),
            kept,
            "the last answer stands"
        );
        fn never(_: &str, _: Option<&Path>) -> Result<Limit, String> {
            panic!("a fresh file is not asked again")
        }
        agent.freshen(never, Duration::from_secs(600));
    }

    #[test]
    fn a_reset_time_reads_in_any_zone() {
        let _budget = TestBudget::new();
        assert_eq!(unix_of("2026-11-11T16:00:00Z"), Some(1_794_412_800));
        assert_eq!(unix_of("2026-11-11T17:00:00+01:00"), Some(1_794_412_800));
        assert_eq!(
            unix_of("2026-11-11T15:59:59.000001+00:00"),
            Some(1_794_412_800)
        );
        assert_eq!(
            unix_of("1970-01-01T00:00:00.000Z"),
            None,
            "no time before it"
        );
        for bad in [
            "2026-13-01T00:00:00Z",
            "2026-11-11",
            "2026-11-11T16:00:00+0100",
            "2026-02-31T16:00:00Z",
            "2026-11-11T16-00-00Z",
            "2026-11-11T+6:00:00Z",
            "2026-11-11T16:00:00+0é:0",
            "",
        ] {
            assert_eq!(unix_of(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_logins_are_default_then_each_named_profile_folder() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agent-logins");
        assert_eq!(logins(&dir.0), ["default"], "no accounts folder");
        for folder in ["second", "a-3", "Upper", "-dash"] {
            std::fs::create_dir_all(dir.0.join("accounts").join(folder)).expect("dirs");
        }
        std::fs::write(dir.0.join("accounts/switches.jsonl"), "").expect("a file");
        assert_eq!(logins(&dir.0), ["default", "a-3", "second"]);
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
