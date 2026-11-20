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

/// A question: from its sentence's start (a dot inside a word, a letter or digit after it as v1.2 or rfc-x.html, is no end;
/// one before markup, `.**`, `.)` or `."`, is) to a `?` followed by a space, a closing mark or the end, never a URL's query.
static QUESTION: LazyLock<Regex> = LazyLock::new(|| {
    // a dot ends a sentence before a space or a closing mark (`.**`, `.)`, `."`), never before a letter, a comma (`i.e.,`)
    // or a path's slash (/code-review)
    Regex::new(r#"(?:[^.!?\n]|\.[^\s.!?*)"'`»_])*\?(?:[\s)»*"'`_]|$)"#).expect("question")
});
/// The words that make a question a pick between options, English and French.
static PICK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(or|ou|which|prefer|pick|choose|option|lequel|laquelle|lesquels|préfères?|préférez|choisis|choisissez)\b")
        .expect("pick")
});
/// A doc the message cites: an RFC by name, or a file of a doc kind.
static DOC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(rfc-[a-z0-9-]*[a-z0-9]|[A-Za-z0-9_-]+\.(?:html|md|edn))\b").expect("doc")
});
/// A PR the message cites: « PR 123 », « PR #123 », « #123 », a commit title's « (#123) » or a pull link.
static PR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:\bPR\s*#?|/pull/|(?:^|[\s(\[])#)(\d+)\b").expect("pr"));

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

/// The commands that print a file or a page (and `dash store text`): a doc named in a simple command one of them starts
/// was read, never one only listed, grepped, edited (`sed -i`) or written (`cat > x`).
pub const SHOWS: [&str; 7] = ["cat", "sed", "head", "tail", "less", "bat", "curl"];

/// A Bash command's simple commands, each its words, as bash splits them (`guard::shell`): at a pipe, `;`, `&` or a line
/// break outside quotes, a backslash-continued line one line, so a word of one never counts for another (`gh pr view 1 |
/// head -n 40` does not view PR 40) and a quoted `;` or `&` splits nothing (/code-review of PR 1669's recovery).
fn simple_commands(command: &str) -> Vec<Vec<String>> {
    // two blanks for the backslash and its line break: each char keeps its place for `spans`
    let typed: Vec<char> = command.replace("\\\n", "  ").chars().collect();
    let code = crate::guard::shell::code(&typed.iter().collect::<String>());
    (crate::guard::shell::spans(&code).into_iter())
        .map(|r| typed[r].iter().collect::<String>())
        .map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether the simple command `words` prints `doc`: a printer starts it and names it, not as a redirect's target.
fn prints(words: &[&str], doc: &str) -> bool {
    let verb = words[0].rsplit('/').next().unwrap_or_default();
    let edits = verb == "sed"
        && words
            .iter()
            .any(|w| w.starts_with("-i") || w.starts_with("--in-place"));
    let printer = SHOWS.contains(&verb) && !edits
        || verb == "dash" && words.get(1..3) == Some(&["store", "text"][..]);
    printer
        && (1..words.len()).any(|i| {
            names(words[i], doc) && !words[i].contains('>') && !words[i - 1].ends_with('>')
        })
}

/// Whether the simple command `words` views PR `n`: `gh pr view|diff <n|its link>`, `gh api …/pulls/<n>` or a curl of
/// its page.
fn views(words: &[&str], n: &str) -> bool {
    let verb = words[0].rsplit('/').next().unwrap_or_default();
    let pull = |w: &str| names(w, &format!("pull/{n}")) || names(w, &format!("pulls/{n}"));
    match (verb, words.get(1).copied(), words.get(2).copied()) {
        ("gh", Some("pr"), Some("view" | "diff")) => words[3..].iter().any(|w| *w == n || pull(w)),
        ("gh", Some("api"), _) | ("curl", _, _) => words[1..].iter().any(|w| pull(w)),
        _ => false,
    }
}

/// Whether `input` names `name` whole: no letter, digit, `-` or `_` touches it, so rfc-rust is not rfc-rust-plugins-ui
/// and PR 166 is not 1663.
fn names(input: &str, name: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    input.match_indices(name).any(|(i, _)| {
        !input[..i].chars().next_back().is_some_and(word)
            && !input[i + name.len()..].chars().next().is_some_and(word)
    })
}

/// Whether one tool call reads a cited doc or PR: a Read of the doc's file, a WebFetch of the doc or the PR's page (its
/// path or URL, never its prompt's words), a Bash simple command printing the doc or viewing the PR.
fn reads(call: &Value, docs: &[String], prs: &[String]) -> bool {
    let input = &call["input"];
    let target = |field: &str| input[field].as_str().unwrap_or_default();
    match call["name"].as_str().unwrap_or_default() {
        "Read" => docs.iter().any(|d| names(target("file_path"), d)),
        "WebFetch" => {
            let url = target("url");
            docs.iter().any(|d| names(url, d))
                || prs
                    .iter()
                    .any(|n| names(url, &format!("pull/{n}")) || names(url, &format!("pulls/{n}")))
        }
        "Bash" => simple_commands(target("command")).iter().any(|words| {
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            docs.iter().any(|d| prints(&words, d)) || prs.iter().any(|n| views(&words, n))
        }),
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

/// Whether a transcript entry starts a turn: a user line that is neither a tool's result nor Claude Code's own text
/// (`isMeta`: a skill's body, a hook's context), so his prompt, a task's notification or a peer's message.
fn starts_turn(entry: &Value) -> bool {
    let content = entry["message"]["content"].as_array().into_iter().flatten();
    entry["type"] == "user"
        && entry["isMeta"] != true
        && !content.into_iter().any(|c| c["type"] == "tool_result")
}

/// Whether a transcript entry is the session's own reply, as dash-hooks' `takeover::last_report` reads one: an assistant
/// line, never Claude Code's own (a usage limit, an API error) nor a subagent's. Its twin, since neither crate may take the
/// other's (dash-gate's NAMED_EDGES, rust/plugin-deps-held.txt) and the sdk names no tool.
fn is_reply(entry: &Value) -> bool {
    entry["type"] == "assistant"
        && entry["message"]["model"] != "<synthetic>"
        && entry["isApiErrorMessage"] != true
        && entry["isSidechain"] != true
}

/// The last text this turn wrote in `transcript`: its last reply's text after the turn's start, empty when the turn wrote
/// none, so a pick of an earlier turn is never judged again.
fn last_text(transcript: impl BufRead) -> String {
    let mut last = String::new();
    for line in transcript.lines().map_while(Result::ok) {
        // only a line that may start a turn or hold a reply's text is decoded: most of a transcript is tool results
        let user = line.contains("\"type\":\"user\"") && !line.contains("\"tool_result\"");
        if !user && !line.contains("\"text\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if starts_turn(&entry) {
            last.clear();
        } else if is_reply(&entry) {
            let content = entry["message"]["content"].as_array().into_iter().flatten();
            for c in content.filter(|c| c["type"] == "text") {
                last = c["text"].as_str().unwrap_or_default().to_string();
            }
        }
    }
    last
}

/// The transcript at `path` from its last `tail` bytes (0, the whole). The line the cut lands in is dropped as bytes: a
/// character cut in two would end `lines()` at once, and the turn would never be judged.
fn open(path: &Path, tail: u64) -> Option<Box<dyn BufRead>> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let cut = tail > 0 && file.metadata().ok()?.len() > tail;
    if cut {
        file.seek(SeekFrom::End(-(tail as i64))).ok()?;
    }
    let mut reader = std::io::BufReader::new(file);
    if cut {
        reader.read_until(b'\n', &mut Vec::new()).ok()?;
    }
    Some(Box::new(reader))
}

/// The tail a turn's last message is looked for in: the last text block sits near the end.
pub const TAIL: u64 = 4 << 20;

/// The Stop event's verdict: a block when its last message asks a pick and cites nothing read, recorded in `firings`;
/// None lets the turn end. The event's `last_assistant_message` is used when Claude Code gives it, else the transcript's
/// tail; the whole transcript is read only for a pick that cites something. A stop this hook already blocked once ends:
/// its block made the session read or decide, and a hook judging its own rewrite could hold the turn for ever. Claude
/// Code's `stop_hook_active` is set after any Stop hook's block, so [`mark`] says whether that block was this one's; with
/// no mark to keep (no session id, no firings file) any blocked stop ends. A transcript that does not open lets the turn
/// end: an error never holds it.
pub fn answer(event: &Value, firings: Option<&Path>) -> Option<Value> {
    let mark = mark(event, firings);
    // ours is the only block a stop's mark can come from; a mark left by an interrupted turn is dropped on a fresh stop
    let ours = mark
        .as_deref()
        .is_some_and(|m| std::fs::remove_file(m).is_ok());
    if event["stop_hook_active"] == true && (ours || mark.is_none()) {
        return None;
    }
    let path = Path::new(event["transcript_path"].as_str()?);
    let message = match event["last_assistant_message"].as_str() {
        Some(m) => m.to_string(),
        None => last_text(open(path, TAIL)?),
    };
    if !asks_a_choice(&message) || read_this_session(open(path, 0)?, &message) {
        return None;
    }
    // a mark that cannot be written could never end the next stop: no block then, never one that loops (/code-review of
    // PR 1669's recovery)
    if let Some(m) = &mark
        && std::fs::write(m, "").is_err()
    {
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

/// The mark of this hook's block of the event's session, beside the firings file: written on a block, removed by the next
/// stop. None without a plain session id or a firings file.
fn mark(event: &Value, firings: Option<&Path>) -> Option<std::path::PathBuf> {
    let session = (event["session_id"].as_str())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))?;
    Some(firings?.with_file_name(format!("unread-choice-{session}.blocked")))
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
    if let Some(block) = answer(&event, firings.as_deref()) {
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
            tool_line(
                "Bash",
                json!({"command": "gh pr view 1663 --json body | head -n 40"}),
            ),
            tool_line(
                "Bash",
                json!({"command": "grep -n x private/docs/rfc-grepped.html | head -5"}),
            ),
            tool_line(
                "Bash",
                json!({"command": "cat > private/docs/rfc-written.html <<'EOF'\nx\nEOF"}),
            ),
            tool_line(
                "Bash",
                json!({"command": "sed -i '' s/a/b/ private/docs/rfc-edited.html"}),
            ),
            tool_line(
                "Bash",
                json!({"command": "echo https://github.com/o/r/pull/1601"}),
            ),
            tool_line(
                "WebFetch",
                json!({"url": "https://x/a", "prompt": "what does rfc-prompted say"}),
            ),
            tool_line(
                "Bash",
                json!({"command": "private/dashboard/bin/dash store text handoff.md"}),
            ),
            tool_line("Bash", json!({"command": "gh api repos/o/r/pulls/1802"})),
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
        let event = |last: Option<&str>| {
            let mut e = json!({"hook_event_name": "Stop", "transcript_path": transcript.to_str()});
            if let Some(m) = last {
                e["last_assistant_message"] = json!(m);
            }
            e
        };
        // the pick, from the transcript's last text, cites nothing: refused, one firing
        let refused = answer(&event(None), Some(&log)).expect("a pick citing nothing is refused");
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
            // a word of another simple command, a grep, a write, an edit, an echoed link, a WebFetch prompt's words
            "PR 40 has the context.",
            "See rfc-grepped.",
            "See rfc-written.",
            "See rfc-edited.",
            "PR 1601 has the context.",
            "See rfc-prompted.",
        ] {
            let msg = format!("{CHOICE} {unread}");
            assert!(
                answer(&event(Some(&msg)), Some(&log)).is_some(),
                "{unread} is not read"
            );
        }
        assert_eq!(count(&log), 12);
        // the same pick citing what it read, printed or viewed: the turn ends, no firing
        for cites in [
            "rfc-rust-plugins-ui section 3 leaves it open.",
            "rfc-printed.html leaves it open.",
            "PR #1663 leaves it open.",
            "PR 1777 leaves it open.",
            "rust-merge-gate-one-rule (#1663) leaves it open.",
            "handoff.md leaves it open.",
            "PR 1802 leaves it open.",
        ] {
            let msg = format!("{CHOICE} {cites}");
            assert_eq!(answer(&event(Some(&msg)), Some(&log)), None, "{msg}");
        }
        // no pick: never judged, even citing nothing
        let report =
            "Merged PR 1700. Checked: https://x/status?view=text. What you need to do: nothing.";
        assert_eq!(answer(&event(Some(report)), Some(&log)), None);
        // a stop it already blocked ends; a transcript that does not open holds nothing
        let mut again = event(Some(CHOICE));
        again["stop_hook_active"] = json!(true);
        assert_eq!(answer(&again, Some(&log)), None, "a second stop ends");
        let gone = json!({"transcript_path": dir.0.join("gone.jsonl").to_str(), "last_assistant_message": format!("{CHOICE} See rfc-x.")});
        assert_eq!(answer(&gone, Some(&log)), None, "no transcript, no block");
        assert_eq!(count(&log), 12);
        // with a session id, its own block is marked: the next stop ends; another hook's block leaves no mark, so judged
        let mut first = event(Some(CHOICE));
        first["session_id"] = json!("s-1");
        assert!(answer(&first, Some(&log)).is_some(), "blocked, marked");
        let mut rewrite = first.clone();
        rewrite["stop_hook_active"] = json!(true);
        assert_eq!(answer(&rewrite, Some(&log)), None, "its own block ends");
        assert!(
            answer(&rewrite, Some(&log)).is_some(),
            "another hook's block is judged"
        );
        assert!(dir.0.join("unread-choice-s-1.blocked").exists());
        assert_eq!(count(&log), 14);
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
            answer(&json!({"transcript_path": later.to_str()}), Some(&log)),
            None
        );
    }

    /// /code-review of PR 1669's recovery: a read is split as bash splits it, so a quoted `;` or `&` and a continued line keep
    /// it one command; a WebFetch of the API's `pulls/<n>` views the PR; `sed --in-place` and a `2>` redirect read nothing; a
    /// mark that cannot be written blocks nothing.
    #[dash_test]
    fn a_read_is_split_as_bash_splits_it() {
        let bash = |c: &str| json!({"name": "Bash", "input": {"command": c}});
        let doc = |d: &str| (vec![d.to_string()], vec![]);
        for (call, (docs, prs), read) in [
            (
                bash("sed -n '1,40p;/x/p' private/docs/rfc-x.html"),
                doc("rfc-x"),
                true,
            ),
            (
                bash("curl -s 'http://h/v/handoff.md?a=1&b=2'"),
                doc("handoff.md"),
                true,
            ),
            (
                bash("cat \\\n  private/docs/rfc-x.html"),
                doc("rfc-x"),
                true,
            ),
            (
                bash("sed --in-place s/a/b/ private/docs/rfc-x.html"),
                doc("rfc-x"),
                false,
            ),
            (
                bash("cat foo 2>private/docs/rfc-x.html"),
                doc("rfc-x"),
                false,
            ),
            (
                json!({"name": "WebFetch", "input": {"url": "https://api.github.com/repos/o/r/pulls/1802"}}),
                (vec![], vec!["1802".to_string()]),
                true,
            ),
        ] {
            assert_eq!(reads(&call, &docs, &prs), read, "{call}");
        }
        let dir = TestDir::new("unread-choice-unmarked");
        let transcript = dir.0.join("t.jsonl");
        std::fs::write(&transcript, text_line(CHOICE)).expect("write");
        // the firings file's folder is gone, so the mark beside it cannot be written
        let firings = dir.0.join("gone/firings.tsv");
        let event = json!({"session_id": "s-2", "transcript_path": transcript.to_str(), "last_assistant_message": CHOICE});
        assert_eq!(answer(&event, Some(&firings)), None, "no mark, no block");
    }

    /// A turn's last text survives Claude Code's own lines after it (a skill's body is `isMeta`); a task's notification
    /// starts a turn of its own.
    #[dash_test]
    fn the_last_text_is_this_turn_s_meta_lines_aside() {
        let meta = json!({"type": "user", "isMeta": true, "message": {"content": "Base directory for this skill"}});
        let notice = json!({"type": "user", "origin": {"kind": "task-notification"}, "message": {"content": "done"}});
        let read = |lines: &[String]| last_text(lines.join("\n").as_bytes());
        assert_eq!(read(&[text_line(CHOICE), meta.to_string()]), CHOICE);
        assert_eq!(read(&[text_line(CHOICE), notice.to_string()]), "");
        // Claude Code's own line (an API error) is not the turn's text
        let error = json!({"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "API Error"}]}});
        assert_eq!(read(&[text_line(CHOICE), error.to_string()]), CHOICE);
        // a tail cut inside « è »'s two bytes (byte 9 of the 16-byte first line) still reads every whole line after it
        let dir = TestDir::new("unread-choice-tail");
        let file = dir.0.join("t.jsonl");
        std::fs::write(&file, format!("« première »\n{}\n", text_line(CHOICE))).expect("write");
        let tail = text_line(CHOICE).len() as u64 + 1 + 7;
        assert_eq!(last_text(open(&file, tail).expect("opens")), CHOICE);
        assert!(open(&dir.0.join("gone"), 0).is_none());
    }

    #[dash_test]
    fn a_pick_is_a_question_holding_an_option_word() {
        assert!(asks_a_choice("Option A or option B?"));
        assert!(asks_a_choice("Tu préfères la 1 ou la 2 ?"));
        assert!(asks_a_choice("**Which one do you want?**"));
        assert!(!asks_a_choice("Should I merge it? Done."));
        // a dot inside a word is no sentence end; a quote or backtick may close the question
        assert!(asks_a_choice("Keep v1.2 or move to v1.3?\" he said"));
        assert!(asks_a_choice("Which do you want: `A` or `B?`"));
        // a sentence ends at a dot before markup: the bold item is no part of the question after it (review of PR 1776)
        assert!(!asks_a_choice("1. **Option A merged.** Ready to ship?"));
        assert!(!asks_a_choice("Option A merged (see PR 3.) Ready to ship?"));
        assert!(!asks_a_choice("He said \"pick option A.\" Ready to ship?"));
        // a dot inside a sentence before a comma or a slash ends nothing (/code-review)
        assert!(asks_a_choice("Keep A or B (i.e., the old one)?"));
        assert!(asks_a_choice("Run ./a.sh or ./b.sh?"));
        assert!(!asks_a_choice(
            "Open http://h/status?view=text or the page."
        ));
        assert_eq!(
            cited(
                "See rfc-rust-plugins-ui, rfc-x.html, ARCHITECTURE.md, PR 12, #34, /pull/56 and a title (#78)."
            ),
            (
                vec![
                    "rfc-rust-plugins-ui".into(),
                    "rfc-x".into(),
                    "ARCHITECTURE.md".into()
                ],
                vec!["12".into(), "34".into(), "56".into(), "78".into()]
            )
        );
    }
}
