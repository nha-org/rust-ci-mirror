//! `dash gate unread-choice`, the orchestrator's Stop hook: a last message that asks him to pick between options is
//! refused unless it cites a doc or PR this session read (task stop-refuses-unread-choice; @nha, 2026-11-14: « why do you
//! want me to review if you have not read my previous guidance ? »: an A or B on SDK ports rfc-rust-plugins-ui section 3
//! had settled, asked unread). « Read » is a tool call of the transcript naming it whole: a Read or WebFetch of the doc, a
//! Bash that prints it ([`SHOWS`]), a `gh pr view|diff` or a `/pull/<n>` of the PR. Each refusal is a firing,
//! `unread_choice`, counted by `dash gate firings` and /status/hooks. .claude/settings.json runs it in the main checkout
//! only, as `dash hook words`; registered here, since the binary only shrinks.
use regex::Regex;
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::sync::LazyLock;

use crate::firings::UNREAD_CHOICE as FIRING;

/// A question's end: a `?` followed by a space, a closing mark or the end, never a URL's query.
static QUESTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^.!?\n]*\?(?:[\s)»*]|$)").expect("question"));
/// The words that make a question a pick between options, English and French.
static PICK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(or|ou|which|prefer|pick|choose|option|lequel|laquelle|lesquels|préfères?|préférez|choisis|choisissez)\b")
        .expect("pick")
});
/// A doc the message cites: an RFC by name, or a file of a doc kind.
static DOC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(rfc-[a-z0-9-]*[a-z0-9]|[A-Za-z0-9_-]+\.(?:html|md|edn))\b").expect("doc")
});
/// A PR the message cites: « PR 123 », « PR #123 », « #123 » or a pull link.
static PR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:\bPR\s*#?|/pull/|(?:^|\s)#)(\d+)\b").expect("pr"));

/// Whether `text` asks him to pick: one of its questions holds a word of [`PICK`].
pub fn asks_a_choice(text: &str) -> bool {
    QUESTION.find_iter(text).any(|q| PICK.is_match(q.as_str()))
}

/// What `text` cites: its docs by name and its PR numbers.
pub fn cited(text: &str) -> (Vec<String>, Vec<String>) {
    let docs = DOC.captures_iter(text).map(|c| c[1].to_string()).collect();
    let prs = PR.captures_iter(text).map(|c| c[1].to_string()).collect();
    (docs, prs)
}

/// The Bash words that print a file or a page: a doc named beside one of them was read, never one only listed or grepped.
pub const SHOWS: [&str; 8] = [
    "cat ",
    "sed ",
    "head ",
    "tail ",
    "less ",
    "bat ",
    "store text",
    "curl ",
];

/// Whether `input` names `name` whole: no letter, digit, `-` or `_` touches it, so rfc-rust is not rfc-rust-plugins-ui
/// and PR 166 is not 1663.
fn names(input: &str, name: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    input.match_indices(name).any(|(i, _)| {
        !input[..i].chars().next_back().is_some_and(word)
            && !input[i + name.len()..].chars().next().is_some_and(word)
    })
}

/// Whether one tool call reads a cited doc or PR: a Read or WebFetch naming it, a Bash printing the doc or viewing the PR.
fn reads(call: &Value, docs: &[String], prs: &[String]) -> bool {
    let input = &call["input"];
    let pull = |text: &str| {
        prs.iter()
            .any(|n| names(text, &format!("pull/{n}")) || names(text, &format!("pulls/{n}")))
    };
    match call["name"].as_str().unwrap_or_default() {
        "Read" | "WebFetch" => {
            let text = input.to_string();
            docs.iter().any(|d| names(&text, d)) || pull(&text)
        }
        "Bash" => {
            let command = input["command"].as_str().unwrap_or_default();
            let shows = SHOWS.iter().any(|w| command.contains(w));
            let views = command.contains("pr view") || command.contains("pr diff");
            shows && docs.iter().any(|d| names(command, d))
                || views && prs.iter().any(|n| names(command, n))
                || pull(command)
        }
        _ => false,
    }
}

/// Whether a tool call of `transcript` (Claude Code's JSON Lines) read one of the docs or PRs `message` cites. Only a line
/// holding a tool call and a cited name is decoded: a transcript can reach 400 MB, most of it tool results.
pub fn read_this_session(transcript: impl BufRead, message: &str) -> bool {
    let (docs, prs) = cited(message);
    if docs.is_empty() && prs.is_empty() {
        return false;
    }
    let names: Vec<&String> = docs.iter().chain(&prs).collect();
    transcript
        .lines()
        .map_while(Result::ok)
        .filter(|l| l.contains("\"tool_use\"") && names.iter().any(|n| l.contains(n.as_str())))
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .any(|line| {
            let content = line["message"]["content"].as_array().into_iter().flatten();
            content
                .filter(|c| c["type"] == "tool_use")
                .any(|c| reads(c, &docs, &prs))
        })
}

/// The last text this turn wrote in `transcript`: its last text block after his last prompt, empty when the turn wrote
/// none, so a pick of an earlier turn is never judged again. A tool's result is a user line too, never a prompt.
fn last_text(transcript: impl BufRead) -> String {
    let mut last = String::new();
    for line in transcript.lines().map_while(Result::ok) {
        let prompt = line.contains("\"type\":\"user\"") && !line.contains("\"tool_result\"");
        if !prompt && !(line.contains("\"assistant\"") && line.contains("\"text\"")) {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match line["type"].as_str() {
            Some("user") if prompt => last.clear(),
            Some("assistant") => {
                let content = line["message"]["content"].as_array().into_iter().flatten();
                for c in content.filter(|c| c["type"] == "text") {
                    last = c["text"].as_str().unwrap_or_default().to_string();
                }
            }
            _ => {}
        }
    }
    last
}

/// The tail a turn's last message is looked for in: the last text block sits near the end.
pub const TAIL: u64 = 4 << 20;

/// An opener: the transcript at `path`, its last `tail` bytes (0, the whole), its first line cut short and so skipped.
pub type Open<'a> = &'a dyn Fn(&Path, u64) -> Option<Box<dyn BufRead>>;

/// The file opener [`main`] uses.
pub fn open_file(path: &Path, tail: u64) -> Option<Box<dyn BufRead>> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if tail > 0 && len > tail {
        file.seek(SeekFrom::Start(len - tail)).ok()?;
    }
    Some(Box::new(std::io::BufReader::new(file)))
}

/// The Stop event's verdict: a block when its last message asks a pick and cites nothing read, recorded in `firings`;
/// None lets the turn end. The event's `last_assistant_message` is used when Claude Code gives it, else the transcript's
/// tail; the whole transcript is read only for a pick that cites something. A stop this hook already blocked once
/// (`stop_hook_active`) ends: its block made the session read or decide, and a hook judging its own rewrite could hold the
/// turn for ever. A transcript that does not open lets the turn end: an error never holds it.
pub fn answer(event: &Value, open: Open, firings: Option<&Path>) -> Option<Value> {
    if event["stop_hook_active"] == true {
        return None;
    }
    let transcript = Path::new(event["transcript_path"].as_str()?);
    let message = match event["last_assistant_message"].as_str() {
        Some(m) => m.to_string(),
        None => last_text(open(transcript, TAIL)?),
    };
    if !asks_a_choice(&message) || read_this_session(open(transcript, 0)?, &message) {
        return None;
    }
    if let Some(file) = firings {
        let _ = crate::firings::record(file, FIRING);
    }
    Some(json!({
        "decision": "block",
        "reason": "unread-choice: this message asks Nicolas to pick between options, and cites no doc or PR this session \
                   read. Read the RFC, doc or PR that settles it and cite it by name (rfc-<name>, <file>.html, PR <n>), \
                   or decide it yourself (CLAUDE.md: « Never end a turn with a choice »)."
    }))
}

pub const USAGE: &str =
    "dash gate unread-choice (the orchestrator's Stop hook: its input on stdin, a block on stdout)";

dash_core::submit! {
    dash_core::Command { usage: USAGE, words: &["gate", "unread-choice"], run: Some(|_, _, out| main(out)) }
}

/// `dash gate unread-choice`: the Stop event on stdin, its block on stdout, or nothing. Always exit 0: a hook error never
/// holds his turn's end.
pub fn main(out: &mut dyn Write) {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(event) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    let firings = crate::firings::log_from_env();
    if let Some(block) = answer(&event, &open_file, firings.as_deref()) {
        let _ = writeln!(out, "{block}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    const CHOICE: &str =
        "Two ways to port the SDK. Should the plugins take the Host by value, or by reference?";

    fn tool_line(tool: &str, input: Value) -> String {
        json!({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "t1", "name": tool, "input": input}]}})
        .to_string()
    }

    fn text_line(text: &str) -> String {
        json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}})
            .to_string()
    }

    fn count(log: &Path) -> usize {
        std::fs::read_to_string(log).map_or(0, |t| t.lines().count())
    }

    /// Done-when 1, each way: a pick citing nothing read is refused and counted; the same pick citing the RFC the
    /// session read, or a PR it viewed, ends the turn; a message asking no pick is never judged.
    #[dash_test]
    fn a_pick_is_refused_unless_it_cites_a_doc_or_pr_read_this_session() {
        let dir = TestDir::new("unread-choice");
        let transcript = dir.0.join("t.jsonl");
        let log = dir.0.join("firings.tsv");
        let lines = [
            tool_line(
                "Read",
                json!({"file_path": "/r/private/docs/rfc-rust-plugins-ui.html"}),
            ),
            tool_line("Bash", json!({"command": "gh pr view 1663 --json body"})),
            tool_line("Bash", json!({"command": "gh pr view --json body 1777"})),
            tool_line(
                "Write",
                json!({"file_path": "/r/private/docs/rfc-never-read.html"}),
            ),
            tool_line(
                "Bash",
                json!({"command": "ls private/docs/rfc-listed.html"}),
            ),
            tool_line(
                "Bash",
                json!({"command": "sed -n 1,40p private/docs/rfc-printed.html"}),
            ),
            text_line(CHOICE),
        ];
        std::fs::write(&transcript, lines.join("\n")).expect("write");
        let open = &open_file;
        let event = |last: Option<&str>| {
            let mut e = json!({"hook_event_name": "Stop", "transcript_path": transcript.to_str()});
            if let Some(m) = last {
                e["last_assistant_message"] = json!(m);
            }
            e
        };
        // the pick, from the transcript's last text, cites nothing: refused, one firing
        let refused =
            answer(&event(None), open, Some(&log)).expect("a pick citing nothing is refused");
        assert_eq!(refused["decision"], "block");
        assert!(
            refused["reason"]
                .as_str()
                .is_some_and(|r| r.contains("cites no doc or PR")),
            "{refused}"
        );
        assert_eq!(count(&log), 1);
        // cited but never read: a doc only written or listed, a PR never viewed, a prefix of what was read
        for unread in [
            "See rfc-never-read.",
            "See rfc-listed.",
            "PR 1600 has the context.",
            "PR 166 has the context.",
            "See rfc-rust.",
        ] {
            let msg = format!("{CHOICE} {unread}");
            assert!(
                answer(&event(Some(&msg)), open, Some(&log)).is_some(),
                "{unread} is not read"
            );
        }
        assert_eq!(count(&log), 6);
        // the same pick citing what it read, printed or viewed: the turn ends, no firing
        for cites in [
            "rfc-rust-plugins-ui section 3 leaves it open.",
            "rfc-printed.html leaves it open.",
            "PR #1663 leaves it open.",
            "PR 1777 leaves it open.",
        ] {
            let msg = format!("{CHOICE} {cites}");
            assert_eq!(answer(&event(Some(&msg)), open, Some(&log)), None, "{msg}");
        }
        // no pick: never judged, even citing nothing
        let report =
            "Merged PR 1700. Checked: https://x/status?view=text. What you need to do: nothing.";
        assert_eq!(answer(&event(Some(report)), open, Some(&log)), None);
        // a stop it already blocked ends; a transcript that does not open holds nothing
        let mut again = event(Some(CHOICE));
        again["stop_hook_active"] = json!(true);
        assert_eq!(answer(&again, open, Some(&log)), None, "a second stop ends");
        let gone = json!({"transcript_path": dir.0.join("gone.jsonl").to_str(), "last_assistant_message": format!("{CHOICE} See rfc-x.")});
        assert_eq!(
            answer(&gone, open, Some(&log)),
            None,
            "no transcript, no block"
        );
        assert_eq!(count(&log), 6);
        assert!(
            std::fs::read_to_string(&log)
                .expect("the firings")
                .starts_with("unread_choice\t")
        );
        // a pick of an earlier turn is never judged again: this turn, after his prompt, wrote no text
        let later = dir.0.join("later.jsonl");
        let prompt =
            json!({"type": "user", "message": {"role": "user", "content": "go on"}}).to_string();
        let result = json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}}).to_string();
        let turns = [
            text_line(CHOICE),
            prompt,
            tool_line("Bash", json!({"command": "ls"})),
            result,
        ];
        std::fs::write(&later, turns.join("\n")).expect("write");
        assert_eq!(
            answer(
                &json!({"transcript_path": later.to_str()}),
                open,
                Some(&log)
            ),
            None
        );
    }

    #[dash_test]
    fn a_pick_is_a_question_holding_an_option_word() {
        assert!(asks_a_choice("Option A or option B?"));
        assert!(asks_a_choice("Tu préfères la 1 ou la 2 ?"));
        assert!(asks_a_choice("**Which one do you want?**"));
        assert!(!asks_a_choice("Should I merge it? Done."));
        assert!(!asks_a_choice(
            "Open http://h/status?view=text or the page."
        ));
        assert_eq!(
            cited("See rfc-rust-plugins-ui, rfc-x.html, ARCHITECTURE.md, PR 12, #34 and /pull/56."),
            (
                vec![
                    "rfc-rust-plugins-ui".into(),
                    "rfc-x".into(),
                    "ARCHITECTURE.md".into()
                ],
                vec!["12".into(), "34".into(), "56".into()]
            )
        );
    }
}
