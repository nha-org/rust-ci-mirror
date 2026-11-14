//! His words in two tiers (rfc-context-alignment sections 5 and 10, phase 1, task words-capture; @nha, 2026-11-02: « My
//! comment should be a verbatim addentum here because it keeps getting lost and costing us. »).
//!
//! 1. **The feed** (`words_feed`): every message of his, word for word, with its date, session and how it came: typed at
//!    the prompt, or typed while the session was busy (Claude Code's `queued_command` attachment, which no
//!    UserPromptSubmit hook sees: 35 of them were skipped on 2026-11-02). Its id is the transcript line's uuid, so a
//!    second read of a transcript adds nothing and a row is never edited.
//! 2. **The entries:** one comment-queue row per point of a message (`from: words`), open until it lands. Its id is
//!    `<feed id>.<n>`: the id is the link to its feed line (`feed_of`).
use crate::{BASE, Row, changes, posted, read_table, write};
use dash_core::OpLog;
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The feed's table, in `crate::SCHEMA`.
pub const FEED: &str = "words_feed";
/// An entry's `from` and page.
pub const WORDS: &str = "words";

/// One message of his, as the feed keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Said {
    pub id: String,
    pub at: String,
    pub session: String,
    /// `typed` at the prompt, or `busy`: typed while the session worked (`queued_command`).
    pub how: String,
    pub text: String,
}

/// What a machine types into his prompt, by its start (`Dashboard.Insights.Chat`'s @machine and @echo, ported): herdr's
/// wake, restart and briefs, an agent's signed relay, Claude Code's own echoes. None of it is his.
const MACHINE: &[&str] = &[
    "Woken by ",
    "Restarted ",
    "Read .claude/run/",
    "bash '",
    "# Prior context digest",
    "# Episode window",
    "# Agent-authored skills",
    "/goal ",
    "PM check:",
    "From the PM check:",
    "Orchestrator:",
    "Orchestrator tick:",
    "From the orchestrator",
    "From the task ",
    "The orchestrator:",
    "Coordinator:",
    "Follow-up from site_perso:",
    "Herdr key check:",
    "Below are messages a user typed to their assistant",
    "The message, one line per row",
    "Sandbox reloaded:",
    "Reply with exactly",
    "Run the shell command:",
    "Run exactly this shell command",
    "<ao-session-delivery",
    "<command-",
    "<local-command-",
    "[Request interrupted",
    "<task-notification",
    "This session is being continued",
];

/// Whether a machine typed `text` into his prompt.
pub fn machine(text: &str) -> bool {
    let t = text.trim_start();
    MACHINE.iter().any(|p| t.starts_with(p))
}

/// A message's text: a string, or its text blocks joined; none when it holds a tool's result.
fn text_of(content: &Json) -> Option<String> {
    match content {
        Json::String(s) => Some(s.clone()),
        Json::Array(blocks) => {
            let mut texts = vec![];
            for b in blocks {
                match b["type"].as_str() {
                    Some("text") => texts.push(b["text"].as_str().unwrap_or_default()),
                    Some("tool_result") => return None,
                    _ => {}
                }
            }
            Some(texts.join("\n"))
        }
        _ => None,
    }
}

/// His message on one transcript line, if it holds one: a prompt he typed, or one he typed while the session was busy.
pub fn said(line: &Json) -> Option<Said> {
    let (source, how) = match line["type"].as_str()? {
        "user" if !line["isMeta"].as_bool().unwrap_or(false) => (line, "typed"),
        "attachment" if line["attachment"]["type"] == "queued_command" => {
            (&line["attachment"], "busy")
        }
        _ => return None,
    };
    if source["origin"]["kind"] != "human" {
        return None;
    }
    let text = match how {
        "typed" => text_of(&line["message"]["content"])?,
        _ => text_of(&source["prompt"])?,
    };
    // a slash command with words after it is his: « /merge and … » (20 such lines were dropped, /code-review)
    let text = typed_after_command(&text).unwrap_or(text);
    if text.trim().is_empty() || machine(&text) {
        return None;
    }
    Some(Said {
        id: format!("w-{}", line["uuid"].as_str()?),
        at: line["timestamp"].as_str().unwrap_or_default().into(),
        session: line["sessionId"].as_str().unwrap_or_default().into(),
        how: how.into(),
        text,
    })
}

/// What he typed when Claude Code recorded a slash command: its name then its words, none when he gave no words.
fn typed_after_command(text: &str) -> Option<String> {
    let inside = |tag: &str| {
        let (_, rest) = text.split_once(&format!("<{tag}>"))?;
        Some(rest.split_once(&format!("</{tag}>"))?.0.trim())
    };
    if !text.trim_start().starts_with("<command-") {
        return None;
    }
    let args = inside("command-args").filter(|a| !a.is_empty())?;
    let name = inside("command-name").unwrap_or_default();
    Some(format!("{name} {args}").trim().to_string())
}

/// Whether a line opens a point: at its start, a bullet (« - », « * », « • ») or a number of one or two digits (« 1. »,
/// « 12) ») then a space; « 2026. » or « 404. » is prose. His sub-bullets (« -- », or indented) stay in their point.
fn opens(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let rest = &line[digits..];
    let marked = match digits {
        0 => ["- ", "* ", "• "].iter().find_map(|m| rest.strip_prefix(m)),
        3.. => None,
        _ => [". ", ") "].iter().find_map(|m| rest.strip_prefix(m)),
    };
    marked.is_some_and(|r| !r.trim().is_empty())
}

/// A message's points, each word for word: a point per top-level bullet or number, the words before the first one a point
/// of their own; a message with none is one point. A pasted block and a code fence are never split: they belong to the
/// point they sit in.
/// ponytail: numbering and bullets only; the splitter for casual paragraphs (Jev) is phase 2's, task words-landing-check.
pub fn points(text: &str) -> Vec<String> {
    let (mut found, mut point, mut pasted, mut fenced) = (vec![], String::new(), false, false);
    // once he numbers, a flush-left bullet is a part of its number, never a point of its own (/code-review)
    let mut numbered = false;
    for line in text.split_inclusive('\n') {
        let number = line.starts_with(|c: char| c.is_ascii_digit());
        if !pasted && !fenced && opens(line) && (number || !numbered) {
            numbered |= number;
            if !point.trim().is_empty() {
                found.push(std::mem::take(&mut point));
            }
        }
        point.push_str(line);
        pasted = (pasted || line.contains("<pasted_content")) && !line.contains("</pasted_content");
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
    }
    found.push(point);
    found
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// The feed line an entry was cut from.
pub fn feed_of(entry: &str) -> &str {
    entry.rsplit_once('.').map_or(entry, |(feed, _)| feed)
}

fn to_json(s: &Said) -> Json {
    json!({"id": s.id, "at": s.at, "session": s.session, "how": s.how, "text": s.text})
}

fn from_json(j: &Json) -> Option<Said> {
    let owned = |k: &str| j[k].as_str().map(str::to_string);
    Some(Said {
        id: owned("id")?,
        at: owned("at")?,
        session: owned("session")?,
        how: owned("how")?,
        text: owned("text")?,
    })
}

/// The message an entry was cut from, or a feed line by its own id: what `dash words show` prints.
pub fn message(log: &dyn OpLog, id: &str) -> Result<Option<Said>, String> {
    let mut feed = feed(log)?;
    Ok(feed.remove(id).or_else(|| feed.remove(feed_of(id))))
}

/// The feed, read from the log.
pub fn feed(log: &dyn OpLog) -> Result<BTreeMap<String, Said>, String> {
    read_table(log, FEED, from_json)
}

/// Each message of his in `transcript` (Claude Code's JSON Lines) whose time `keep` takes and that the feed lacks: a feed
/// line, then an open entry per point. Says how many of each it wrote. A line cut at the start of a tail read is no JSON
/// and is skipped.
pub fn capture(
    log: &mut dyn OpLog,
    rows: &BTreeMap<String, Row>,
    transcript: &str,
    keep: &dyn Fn(&str) -> bool,
    now_ms: i64,
) -> Result<(usize, usize), String> {
    let kept = feed(log)?;
    let mut new_feed = kept.clone();
    let mut new_rows = BTreeMap::new();
    // his lines only are decoded: a tool's result is most of a transcript's bytes
    for line in transcript
        .lines()
        .filter(|l| l.contains(r#""kind":"human""#))
    {
        let Some(s) = serde_json::from_str(line).ok().as_ref().and_then(said) else {
            continue;
        };
        if !keep(&s.at) || new_feed.contains_key(&s.id) {
            continue;
        }
        for (n, p) in points(&s.text).into_iter().enumerate() {
            let id = format!("{}.{}", s.id, n + 1);
            if !rows.contains_key(&id) {
                let row = Row {
                    link: format!("{BASE}/comments#c-{id}"),
                    from: WORDS.into(),
                    ..posted(&id, WORDS, "", &p, &s.at, now_ms)
                };
                new_rows.insert(id, row);
            }
        }
        new_feed.insert(s.id.clone(), s);
    }
    let mut writes = changes(FEED, &kept, &new_feed, to_json);
    let fed = writes.len();
    writes.extend(changes(
        crate::TABLE,
        &BTreeMap::new(),
        &new_rows,
        crate::to_json,
    ));
    write(log, writes)?;
    Ok((fed, new_rows.len()))
}

/// A turn's end reads this much of its transcript's tail, and the messages of this last day in it: a session can run for
/// days and its transcript reach 400 MB (/code-review). A Stop that failed is made up by the next, inside the day.
/// ponytail: a turn whose tool output passes 64 MB after his message misses it; a mark per transcript if that is seen.
const HOOK_TAIL: u64 = 64 << 20;
const HOOK_DAY_MS: i64 = 24 * 3_600_000;

/// `dash words capture` (`at`: the whole transcript, the messages whose time starts with it, empty for all) and `dash hook
/// words` (`at` none: Claude Code's Stop hook, the tail and the last day): `capture` on the file `transcript`, `rows` the
/// queue's (task words-capture). Says what it wrote. Here, not in the binary, which only shrinks (rust-binary-ratchet).
pub fn capture_file(
    log: &mut dyn OpLog,
    rows: &BTreeMap<String, Row>,
    transcript: &Path,
    at: Option<&str>,
    now_ms: i64,
) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    let failed = |e: std::io::Error| format!("{}: {e}", transcript.display());
    let mut file = std::fs::File::open(transcript).map_err(failed)?;
    if at.is_none() {
        let len = file.metadata().map_err(failed)?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(HOOK_TAIL)))
            .map_err(failed)?;
    }
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).map_err(failed)?;
    let text = String::from_utf8_lossy(&bytes);
    let since = dash_core::iso((now_ms - HOOK_DAY_MS).div_euclid(1000));
    let keep = |t: &str| match at {
        Some(prefix) => t.starts_with(prefix),
        None => t >= since.as_str(),
    };
    let (fed, opened) = capture(log, rows, &text, &keep, now_ms)?;
    Ok(format!(
        "words: {fed} messages to the feed, {opened} entries open\n"
    ))
}

/// `dash words show <entry or feed id>`: the message, word for word, with its time, session and how it came.
pub fn shown(log: &dyn OpLog, id: &str) -> Result<String, String> {
    let s = message(log, id)?.ok_or(format!("no message {id}"))?;
    Ok(format!(
        "{} ({}, session {}, {}):\n{}\n",
        s.id, s.at, s.session, s.how, s.text
    ))
}

/// The transcript a hook's JSON names.
pub fn transcript_of(input: &str) -> Option<PathBuf> {
    let hook: Json = serde_json::from_str(input).ok()?;
    hook["transcript_path"].as_str().map(PathBuf::from)
}

pub const CAPTURE_USAGE: &str =
    "usage: dash words capture [--data <dir>] [--at <prefix>] <transcript.jsonl>";

/// `dash words capture`'s words: its --data folder, its --at prefix (empty for all) and its transcript, or none on a misuse.
/// A flag with no value is a misuse, never the live store or every message (/code-review).
pub fn capture_args<'a>(args: &[&'a str]) -> Option<(Option<&'a str>, &'a str, &'a str)> {
    let (mut dir, mut at, mut file) = (None, "", None);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match (a, file) {
            ("--data", _) if it.len() > 0 => dir = it.next().copied(),
            ("--at", _) if it.len() > 0 => at = it.next().copied().unwrap_or_default(),
            (f, None) if !f.starts_with("--") => file = Some(f),
            _ => return None,
        }
    }
    Some((dir, at, file?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    #[dash_test]
    fn a_queued_command_from_him_is_his_and_a_notification_is_not() {
        let busy = json!({"type": "attachment", "uuid": "u1", "timestamp": "2026-11-02T07:58:16.475Z",
            "sessionId": "s", "attachment": {"type": "queued_command", "prompt": "check them",
            "commandMode": "prompt", "origin": {"kind": "human"}}});
        let s = said(&busy).expect("his");
        assert_eq!(
            (s.id.as_str(), s.how.as_str(), s.text.as_str()),
            ("w-u1", "busy", "check them")
        );
        let note = json!({"type": "attachment", "uuid": "u2", "attachment": {"type": "queued_command",
            "prompt": "<task-notification>x</task-notification>", "origin": {"kind": "task-notification"}}});
        assert_eq!(said(&note), None);
        let slash = |args: &str| {
            let text = format!(
                "<command-message>merge</command-message>\n<command-name>/merge</command-name>\n<command-args>{args}</command-args>"
            );
            json!({"type": "user", "uuid": "u5", "origin": {"kind": "human"}, "message": {"content": text}})
        };
        assert_eq!(
            said(&slash("is overly verbose")).map(|s| s.text),
            Some("/merge is overly verbose".into())
        );
        assert_eq!(said(&slash("")), None);
        let relay = json!({"type": "user", "uuid": "u3", "origin": {"kind": "human"},
            "message": {"content": "From the orchestrator, not Nicolas: merge main"}});
        assert_eq!(said(&relay), None);
        let result = json!({"type": "user", "uuid": "u4", "origin": {"kind": "human"},
            "message": {"content": [{"type": "tool_result", "content": "ok"}]}});
        assert_eq!(said(&result), None);
    }

    #[dash_test]
    fn points_split_on_top_level_bullets_and_numbers_only() {
        assert_eq!(points("1. a\n2. b\n3. c"), ["1. a", "2. b", "3. c"]);
        assert_eq!(
            points("Good\n\n- see both\n- the focus\n  -- its parts"),
            ["Good", "- see both", "- the focus\n  -- its parts"]
        );
        let pasted = "check:\n\n<pasted_content id=\"1\">\n- now\n- next\n</pasted_content>";
        assert_eq!(points(pasted), [pasted]);
        assert_eq!(points("> - quoted\n\nwhere ?"), ["> - quoted\n\nwhere ?"]);
        assert_eq!(points("-1 is fine"), ["-1 is fine"]);
        assert_eq!(
            points("Context:\n2026. was the year\n404. Not found"),
            ["Context:\n2026. was the year\n404. Not found"]
        );
        assert_eq!(points("10. ten\n11. eleven"), ["10. ten", "11. eleven"]);
        assert_eq!(
            points("1. I did ask:\n- carry the idea\n- in a skill\n2. next"),
            ["1. I did ask:\n- carry the idea\n- in a skill", "2. next"]
        );
    }

    #[dash_test]
    fn an_entry_names_its_feed_line() {
        assert_eq!(feed_of("w-c36cad25-3f14.3"), "w-c36cad25-3f14");
    }

    use dash_core::MemoryLog;

    /// A transcript as Claude Code writes it: his typed prompt, a tool's result, then what he typed while it worked.
    fn transcript(typed: &str, busy: &str) -> String {
        [
            json!({"type": "user", "uuid": "t1", "timestamp": "2026-11-02T07:50:06.072Z", "sessionId": "s",
                "origin": {"kind": "human"}, "message": {"role": "user", "content": typed}}),
            json!({"type": "user", "uuid": "r1", "timestamp": "2026-11-02T07:51:00.000Z",
                "message": {"content": [{"type": "tool_result", "content": "ok"}]}}),
            json!({"type": "attachment", "uuid": "q1", "timestamp": "2026-11-02T07:58:16.475Z", "sessionId": "s",
                "attachment": {"type": "queued_command", "prompt": busy, "commandMode": "prompt",
                "origin": {"kind": "human"}}}),
        ]
        .iter()
        .map(|l| l.to_string() + "\n")
        .collect()
    }

    fn open(log: &MemoryLog) -> BTreeMap<String, Row> {
        crate::load(log).expect("loads")
    }

    #[dash_test]
    fn a_queued_command_gives_a_feed_line_and_an_entry_with_its_words() {
        let busy = "It seems some of my comments got discarded - please check and act on them.\n\n<pasted_content id=\"9cf0\">\n- now\n- next\n</pasted_content>";
        let mut log = MemoryLog::default();
        let said = capture(
            &mut log,
            &BTreeMap::new(),
            &transcript("ok", busy),
            &|_| true,
            1,
        )
        .expect("captures");
        assert_eq!(said, (2, 2));
        let fed = feed(&log).expect("the feed");
        let line = &fed["w-q1"];
        assert_eq!(
            (line.text.as_str(), line.how.as_str(), line.at.as_str()),
            (busy, "busy", "2026-11-02T07:58:16.475Z")
        );
        let rows = open(&log);
        let entry = &rows["w-q1.1"];
        assert_eq!(
            (
                entry.text.as_str(),
                entry.from.as_str(),
                entry.closed.as_deref()
            ),
            (busy, WORDS, None)
        );
        // read again, nothing new: the feed is append-only, an entry is never opened twice
        assert_eq!(
            capture(&mut log, &rows, &transcript("ok", busy), &|_| true, 2),
            Ok((0, 0))
        );
    }

    #[dash_test]
    fn a_three_point_message_gives_three_open_entries_each_linked_to_its_feed_line() {
        let typed =
            "1. merge the open PRs\n2. keep the dash on sqlite\n3. start nanolathe, one session";
        let mut log = MemoryLog::default();
        capture(
            &mut log,
            &BTreeMap::new(),
            &transcript(typed, "later"),
            &|at| at.starts_with("2026-11-02T07:50"),
            1,
        )
        .expect("captures");
        let fed = feed(&log).expect("the feed");
        assert_eq!(
            fed.keys().collect::<Vec<_>>(),
            ["w-t1"],
            "--at keeps the 07:50 message only"
        );
        let rows = open(&log);
        let entries: Vec<(&str, &str, &str)> = (rows.values())
            .filter(|r| r.closed.is_none())
            .map(|r| (r.id.as_str(), feed_of(&r.id), r.text.as_str()))
            .collect();
        assert_eq!(
            entries,
            [
                ("w-t1.1", "w-t1", "1. merge the open PRs"),
                ("w-t1.2", "w-t1", "2. keep the dash on sqlite"),
                ("w-t1.3", "w-t1", "3. start nanolathe, one session"),
            ]
        );
        assert!(entries.iter().all(|(_, f, _)| fed.contains_key(*f)));
        let shown = message(&log, "w-t1.2")
            .expect("reads")
            .expect("its message");
        assert_eq!(shown.text, typed);
    }
}
