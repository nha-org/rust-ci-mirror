//! A pane's open question dialog (Claude Code's AskUserQuestion): read off its screen and its session's transcript, listed in
//! the orchestrator's wake, and answered by `dash pane pick` (the older app's `Dashboard.Orch.Dialog`, ported, task
//! rust-watch-port; @nha, 2026-11-11: « see blocked pages. Is the rust watcher telling you already ? »).
//!
//! The screen says a dialog is open and which question it shows; the transcript holds the whole AskUserQuestion input: every
//! tab, each option's description. The transcript is used when one of its questions is the one on screen, else the screen alone.
//! A pick is refused before any key when anything does not hold, so a refusal types nothing (a stray « 1 » was left in a pane
//! on 2026-11-11).
use dash_core::tasks::Terminal;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

/// Its usage, which bin/dash also looks for in this binary before handing it `pane pick`.
pub const USAGE: &str = "dash pane pick <pane|task id> <n>|--text <words>...";

/// What the wake tells the orchestrator to run.
pub const DASH: &str = "~/repo/site_perso/private/dashboard/bin/dash";

/// The transcript's tail read: the open question is its last tool call, so a long session costs no full read.
const TAIL: u64 = 2_000_000;

/// Reads of the screen while it redraws, `pause` apart, before a step gives up.
const TRIES: u32 = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Opt {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    /// Its tab's header.
    pub header: Option<String>,
    pub question: String,
    /// Several options may be picked.
    pub multi: bool,
    pub options: Vec<Opt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub header: String,
    pub answered: bool,
}

/// What the screen shows of a dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub tabs: Vec<Tab>,
    pub question: String,
    pub options: Vec<Opt>,
    /// The option under the cursor, its index.
    pub focused: Option<usize>,
}

/// A pane's open dialog: its questions, the one on screen (`current`), the tabs answered there and how many the screen shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub pane: String,
    pub questions: Vec<Question>,
    pub current: usize,
    pub answered: usize,
    pub tabs: usize,
    /// Read from the transcript, else the screen alone.
    pub whole: bool,
}

/// One answer: an option's number, or words for its « Type something. » line.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Number(usize),
    Text(String),
}

// ------------------------------------------------------------------ the screen

fn is_rule(l: &str) -> bool {
    l.contains("───")
}

/// « 1. label », maybe after the cursor ❯: its number and label.
fn option(l: &str) -> Option<(usize, &str)> {
    let l = l.trim_start();
    let l = l.strip_prefix('❯').map_or(l, str::trim_start);
    let digits = l.find(|c: char| !c.is_ascii_digit())?;
    let n = l[..digits].parse().ok()?;
    let rest = l[digits..].strip_prefix('.')?;
    let label = rest.trim();
    (rest.starts_with(char::is_whitespace) && !label.is_empty()).then_some((n, label))
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A line of a live dialog's chrome below its options: blank, a rule, « Chat about this », the notes line, a preview box's
/// rows, the keys footer (« Enter to select », « Esc to cancel »).
fn chrome(l: &str) -> bool {
    let t = l.trim();
    let after_number = option(l).map_or(t, |(_, label)| label);
    let keys = ["Enter", "Esc", "Space", "Tab", "Ctrl", "↑", "←"];
    t.is_empty()
        || (t.chars().count() >= 3 && t.chars().all(|c| c == '─' || c == '╌'))
        || matches!(after_number, "Chat about this" | "Type something.")
        || t.starts_with("Notes: press n")
        || (l.starts_with("          ") && t.starts_with(['┌', '└', '│']))
        || (keys.iter().any(|k| t.starts_with(k)) && t.contains(" to "))
}

/// The dialog's header: the last line with a ☐ or ☒ tab.
fn tab_line(lines: &[&str]) -> Option<usize> {
    lines.iter().rposition(|l| {
        l.char_indices().any(|(at, c)| {
            matches!(c, '☐' | '☒') && {
                let rest = &l[at + c.len_utf8()..];
                rest.starts_with(char::is_whitespace) && !rest.trim().is_empty()
            }
        })
    })
}

/// The tabs of the header line, Submit (✔) left out.
fn tabs(line: &str) -> Vec<Tab> {
    let marks: Vec<(usize, char)> = (line.char_indices())
        .filter(|(_, c)| matches!(c, '☐' | '☒' | '✔'))
        .collect();
    let mut out = vec![];
    for (i, &(at, mark)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(line.len(), |(next, _)| *next);
        let text = &line[at + mark.len_utf8()..end];
        let header = text.split('→').next().unwrap_or_default().trim();
        if mark != '✔' && !header.is_empty() {
            out.push(Tab {
                header: header.to_string(),
                answered: mark == '☒',
            });
        }
    }
    out
}

/// Beside the options a preview is a box (┌ … └) from one column on: the lines from its top are cut there, the left kept.
fn left_of_box(lines: &[&str]) -> Vec<String> {
    let Some(top) = lines.iter().position(|l| l.contains('┌')) else {
        return lines.iter().map(|l| l.to_string()).collect();
    };
    let col = lines[top]
        .split('┌')
        .next()
        .unwrap_or_default()
        .chars()
        .count();
    let cut = |l: &str| {
        l.chars()
            .take(col)
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    let above = lines[..top].iter().map(|l| l.to_string());
    above.chain(lines[top..].iter().map(|l| cut(l))).collect()
}

/// The options from the first « 1. » on: an indented line under one is its description, or with a preview beside them its
/// label going on. « Type something. » and « Chat about this » are left out.
fn options(lines: &[String], preview: bool) -> (Vec<Opt>, Option<usize>) {
    let mut out: Vec<Opt> = vec![];
    let mut focused = None;
    for l in lines {
        match option(l) {
            Some((n, label)) if n == out.len() + 1 => {
                if l.trim_start().starts_with('❯') {
                    focused = Some(out.len());
                }
                out.push(Opt {
                    label: label.to_string(),
                    description: None,
                });
            }
            _ if l.trim().is_empty() => {}
            _ => {
                let more = l.trim();
                if let Some(o) = out.last_mut() {
                    match (&mut o.description, preview) {
                        (_, true) => o.label = format!("{} {more}", o.label),
                        (Some(d), false) => *d = format!("{d} {more}"),
                        (d @ None, false) => *d = Some(more.to_string()),
                    }
                }
            }
        }
    }
    out.retain(|o| !matches!(o.label.as_str(), "Type something." | "Chat about this"));
    (out, focused)
}

/// The dialog on a screen, or None: a dialog with the agent's output or the input box under it is scrollback, not live.
pub fn parse(screen: &str) -> Option<Shown> {
    let lines: Vec<&str> = screen.lines().collect();
    let t = tab_line(&lines)?;
    let preview = lines[t + 1..].iter().any(|l| l.contains('┌'));
    let left = left_of_box(&lines[t + 1..]);
    let body: Vec<&String> = left.iter().take_while(|l| !is_rule(l)).collect();
    // the last « 1. » above the dialog's rule: a numbered list in the question comes before it
    let first = body
        .iter()
        .rposition(|l| option(l).is_some_and(|(n, _)| n == 1))?;
    let opts: Vec<String> = body[first..].iter().map(|l| l.to_string()).collect();
    let below = &left[body.len()..];
    let output = |l: &String| l.trim_start().starts_with('⏺') || l.contains('⎿');
    if opts.iter().any(output) || !below.iter().all(|l| chrome(l)) {
        return None;
    }
    let question = body[..first]
        .iter()
        .map(|l| l.replace('│', ""))
        .collect::<Vec<_>>();
    let (options, focused) = options(&opts, preview);
    Some(Shown {
        tabs: tabs(lines[t]),
        question: one_line(&question.join(" ")),
        options,
        focused,
    })
}

/// Whether the screen's question (wrapped, maybe cut to its first lines) is `question`: markdown markers left out, as the pane
/// renders them.
pub fn same(question: &str, shown: &str) -> bool {
    let plain = |s: &str| one_line(s).replace(['`', '*'], "");
    let (q, s) = (plain(question), plain(shown));
    !s.is_empty() && q.starts_with(&s)
}

// ------------------------------------------------------------------ the transcript

/// The questions of the session's last AskUserQuestion still unanswered (no tool_result names it), read from the end of its
/// transcript `<projects>/*/<session>.jsonl`, or None.
pub fn transcript(projects: &Path, session: &str) -> Option<Vec<Question>> {
    let safe = !session.is_empty()
        && session
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return None;
    }
    let name = format!("{session}.jsonl");
    let file = (std::fs::read_dir(projects).ok()?.flatten())
        .map(|d| d.path().join(&name))
        .find(|p| p.is_file())?;
    let mut f = std::fs::File::open(file).ok()?;
    let size = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(size.saturating_sub(TAIL))).ok()?;
    let mut bytes = vec![];
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let (id, input) = (text.lines())
        .filter(|l| l.contains(r#""name":"AskUserQuestion""#))
        .flat_map(asks)
        .next_back()?;
    let answered = format!(r#""tool_use_id":"{id}""#);
    (!text.contains(&answered)).then(|| questions(&input))?
}

fn asks(line: &str) -> Vec<(String, Value)> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let content = v["message"]["content"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    (content.into_iter())
        .filter(|b| b["type"] == "tool_use" && b["name"] == "AskUserQuestion")
        .filter_map(|b| Some((b["id"].as_str()?.to_string(), b["input"].clone())))
        .collect()
}

fn questions(input: &Value) -> Option<Vec<Question>> {
    let qs = input["questions"].as_array().filter(|qs| !qs.is_empty())?;
    let text = |v: &Value| v.as_str().map(str::to_string);
    Some(
        qs.iter()
            .map(|q| Question {
                header: text(&q["header"]),
                question: text(&q["question"]).unwrap_or_default(),
                multi: q["multiSelect"] == true,
                options: (q["options"].as_array().into_iter().flatten())
                    .map(|o| Opt {
                        label: text(&o["label"]).unwrap_or_default(),
                        description: text(&o["description"]),
                    })
                    .collect(),
            })
            .collect(),
    )
}

// ------------------------------------------------------------------ the dialog

/// The dialog `screen` shows in `pane`, its questions from the transcript when one of them is on screen.
pub fn from(pane: &str, shown: &Shown, transcript: Option<Vec<Question>>) -> Dialog {
    let at = (transcript.as_ref())
        .and_then(|qs| qs.iter().position(|q| same(&q.question, &shown.question)));
    let (questions, current, whole) = match (transcript, at) {
        (Some(qs), Some(i)) => (qs, i, true),
        _ => {
            let header = shown
                .tabs
                .iter()
                .find(|t| !t.answered)
                .map(|t| t.header.clone());
            let multi = (shown.options.iter()).any(|o| {
                ["[ ]", "[x]", "[✔]", "[✓]"]
                    .iter()
                    .any(|b| o.label.starts_with(b))
            });
            let q = Question {
                header,
                question: shown.question.clone(),
                multi,
                options: shown.options.clone(),
            };
            (vec![q], 0, false)
        }
    };
    Dialog {
        pane: pane.to_string(),
        questions,
        current,
        answered: shown.tabs.iter().filter(|t| t.answered).count(),
        tabs: shown.tabs.len(),
        whole,
    }
}

/// The dialog open in `pane`, or why there is none: its agent must be asking and its screen show one live.
pub fn read(terminal: &dyn Terminal, pane: &str, projects: &Path) -> Result<Dialog, String> {
    let seen = terminal.agent(pane)?.ok_or(format!("no agent in {pane}"))?;
    if !seen.asking {
        return Err(format!("{pane} shows no question dialog"));
    }
    let screen = terminal.read(pane)?;
    let shown = parse(&screen).ok_or(format!("{pane} shows no question dialog"))?;
    let qs = seen
        .session
        .as_deref()
        .and_then(|s| transcript(projects, s));
    Ok(from(pane, &shown, qs))
}

/// Why `dialog` cannot take these picks from outside the pane, or None.
pub fn refusal(d: &Dialog, picks: &[Pick]) -> Option<String> {
    let why = if d.answered > 0 || d.current != 0 {
        "partly answered in the pane: finish it there"
    } else if d.tabs > d.questions.len() {
        "no transcript holds its other tabs: answer it in the pane"
    } else if d.questions.iter().any(|q| q.multi) {
        "a question takes several picks: answer it in the pane"
    } else if picks.len() != d.questions.len() {
        "one answer per question"
    } else if !(d.questions.iter().zip(picks)).all(|(q, p)| match p {
        Pick::Number(n) => (1..=q.options.len()).contains(n),
        Pick::Text(_) => true,
    }) {
        "an answer is not one of its options"
    } else if picks
        .iter()
        .any(|p| matches!(p, Pick::Text(w) if w.is_empty() || w.len() > 200))
    {
        "a typed answer is empty or over 200 bytes, which keys would cut: answer it in the pane"
    } else {
        return None;
    };
    Some(why.to_string())
}

/// `pane pick`'s words after the pane: an option number per question, or `--text <words>` for its « Type something. ».
pub fn picks(args: &[&str]) -> Option<Vec<Pick>> {
    let mut out = vec![];
    let mut rest = args;
    while let Some((first, more)) = rest.split_first() {
        match (*first, more) {
            ("--text", [words, more @ ..]) => {
                out.push(Pick::Text(one_line(words)));
                rest = more;
            }
            (n, more) => {
                out.push(Pick::Number(n.parse().ok().filter(|n| *n > 0)?));
                rest = more;
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The line the orchestrator runs to answer it: one placeholder per question.
pub fn pick_line(d: &Dialog) -> String {
    let each = vec!["<n>"; d.questions.len()].join(" ");
    format!("{DASH} pane pick {} {each}", d.pane)
}

/// The dialog in words: every tab's question and options, then its pick line.
pub fn words(d: &Dialog) -> String {
    let qs: Vec<String> = (d.questions.iter().enumerate())
        .map(|(i, q)| {
            let header = q
                .header
                .as_deref()
                .map(|h| format!("[{h}] "))
                .unwrap_or_default();
            let several = if q.multi { " (several picks)" } else { "" };
            let opts: Vec<String> = (q.options.iter().enumerate())
                .map(|(n, o)| match &o.description {
                    Some(d) => format!("{}. {} ({})", n + 1, o.label, one_line(d)),
                    None => format!("{}. {}", n + 1, o.label),
                })
                .collect();
            format!(
                "Q{} {header}{}{several} Options: {}.",
                i + 1,
                one_line(&q.question),
                opts.join("; ")
            )
        })
        .collect();
    let partly = match d.whole {
        true => "",
        false => " Only the tab on screen was read, no transcript: answer the others in the pane.",
    };
    format!(
        "{} Pick: {} (one option number per question, or --text '<words>' for its « Type something. »).{partly}",
        qs.join(" "),
        pick_line(d)
    )
}

// ------------------------------------------------------------------ the answer

/// The screen once `ok` holds on it, read up to `TRIES` times.
fn screen_when(
    t: &dyn Terminal,
    pane: &str,
    pause: Duration,
    ok: &dyn Fn(&str) -> bool,
    why: &str,
) -> Result<String, String> {
    for i in 0..TRIES {
        let screen = t.read(pane)?;
        if ok(&screen) {
            return Ok(screen);
        }
        if i + 1 < TRIES {
            sleep(pause);
        }
    }
    Err(why.to_string())
}

fn on(q: &Question) -> impl Fn(&str) -> bool + '_ {
    move |s: &str| parse(s).is_some_and(|shown| same(&q.question, &shown.question))
}

/// Enter only while the pane still shows the question the number answered, once it had time to redraw; `moves`: the number
/// should change the screen, so a screen still the same after the last read is a slow pane, and nothing is pressed.
fn enter_while(
    t: &dyn Terminal,
    pane: &str,
    before: &str,
    moves: bool,
    still: &dyn Fn(&str) -> bool,
    pause: Duration,
) -> Result<(), String> {
    for i in 0..TRIES {
        let screen = t.read(pane)?;
        if screen == before && i + 1 < TRIES {
            sleep(pause);
            continue;
        }
        if screen == before && moves {
            return Err("the pane did not redraw after the number: answer it there".into());
        }
        return if still(&screen) {
            t.key(pane, "Enter")
        } else {
            Ok(())
        };
    }
    Ok(())
}

fn type_one(
    t: &dyn Terminal,
    pane: &str,
    q: &Question,
    pick: &Pick,
    pause: Duration,
) -> Result<(), String> {
    let on_q = on(q);
    let before = screen_when(t, pane, pause, &on_q, "the pane left this question")?;
    match pick {
        Pick::Number(n) => {
            t.key(pane, &n.to_string())?;
            let moves = parse(&before).and_then(|s| s.focused) != Some(n - 1);
            enter_while(t, pane, &before, moves, &on_q, pause)
        }
        Pick::Text(words) => {
            let n = (before.lines().rev())
                .find_map(|l| {
                    option(l)
                        .filter(|(_, label)| *label == "Type something.")
                        .map(|(n, _)| n)
                })
                .ok_or("the pane shows no « Type something. »: answer it there")?;
            let focus = move |s: &str| {
                (s.lines().rev())
                    .find_map(|l| l.trim_start().starts_with('❯').then(|| option(l)).flatten())
                    .is_some_and(|(at, _)| at == n)
            };
            if !focus(&before) {
                t.key(pane, &n.to_string())?;
            }
            screen_when(
                t,
                pane,
                pause,
                &focus,
                "the pane did not focus « Type something. »: answer it there",
            )?;
            t.send(pane, words)?;
            sleep(pause);
            t.key(pane, "Enter")
        }
    }
}

/// Types the answer into `d`'s pane: each question's pick in order, then Submit on the review tab when there are several.
/// Called once `refusal` said nothing: an error after the first key says the dialog is half answered.
pub fn answer(t: &dyn Terminal, d: &Dialog, picks: &[Pick], pause: Duration) -> Result<(), String> {
    for (i, (q, p)) in d.questions.iter().zip(picks).enumerate() {
        if let Err(why) = type_one(t, &d.pane, q, p, pause) {
            return Err(match i {
                0 => why,
                _ => format!("partly answered, finish it in the pane: {why}"),
            });
        }
    }
    if d.questions.len() > 1 {
        let submit = |s: &str| {
            s.lines()
                .any(|l| option(l).is_some_and(|(_, label)| label.starts_with("Submit answers")))
        };
        let screen = screen_when(
            t,
            &d.pane,
            pause,
            &submit,
            "the pane shows no « Submit answers »",
        )
        .map_err(|why| format!("partly answered, finish it in the pane: {why}"))?;
        let n = (screen.lines())
            .find_map(|l| {
                option(l)
                    .filter(|(_, label)| label.starts_with("Submit answers"))
                    .map(|(n, _)| n)
            })
            .unwrap_or(1);
        t.key(&d.pane, &n.to_string())?;
        // already focused, its number leaves the screen as it was: a slow pane then still takes the Enter (/code-review)
        let focused = (screen.lines())
            .any(|l| l.trim_start().starts_with('❯') && option(l).is_some_and(|(at, _)| at == n));
        enter_while(t, &d.pane, &screen, !focused, &submit, pause)?;
    }
    Ok(())
}

/// `dash pane pick`: every check made before the first key, so a refusal types nothing.
pub fn pick(
    t: &dyn Terminal,
    pane: &str,
    picks: &[Pick],
    projects: &Path,
    pause: Duration,
) -> Result<(), String> {
    let d = read(t, pane, projects)?;
    if let Some(why) = refusal(&d, picks) {
        return Err(format!("{pane}: {why}"));
    }
    answer(t, &d, picks, pause)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::Seen;
    use dash_meter::dash_test;
    use std::cell::RefCell;

    /// Two tabs, as a pane showed AskUserQuestion with a preview-less first tab.
    const TWO_TABS: &str = "\
⏺ Two things to settle before the port.

 ←  ☐ Scope  ☐ Wake  ✔ Submit  →

Which slice ships first?

❯ 1. Dialog reader (Recommended)
     Port the screen and transcript reader first.
  2. The queue
     The batched wake first.
  3. Type something.
──────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";

    struct Pane {
        screens: RefCell<Vec<String>>,
        keys: RefCell<Vec<String>>,
        asking: bool,
    }

    impl Terminal for Pane {
        fn open(&self, _: &str, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn run(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn agent(&self, _: &str) -> Result<Option<Seen>, String> {
            Ok(Some(Seen {
                idle: false,
                asking: self.asking,
                session: Some("s1".into()),
            }))
        }
        fn sessions(&self) -> Result<usize, String> {
            Ok(1)
        }
        fn read(&self, _: &str) -> Result<String, String> {
            let mut s = self.screens.borrow_mut();
            Ok(if s.len() > 1 {
                s.remove(0)
            } else {
                s[0].clone()
            })
        }
        fn send(&self, _: &str, text: &str) -> Result<(), String> {
            self.keys.borrow_mut().push(format!("text:{text}"));
            Ok(())
        }
        fn key(&self, _: &str, key: &str) -> Result<(), String> {
            self.keys.borrow_mut().push(key.into());
            Ok(())
        }
        fn close(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
    }

    fn pane(screens: &[&str], asking: bool) -> Pane {
        Pane {
            screens: RefCell::new(screens.iter().map(|s| s.to_string()).collect()),
            keys: RefCell::default(),
            asking,
        }
    }

    /// A projects folder whose session s1 asked the two questions of `TWO_TABS`, unanswered.
    fn projects(name: &str) -> dash_meter::TestDir {
        let dir = dash_meter::TestDir::new(&format!("dialog-{name}"));
        std::fs::create_dir_all(dir.0.join("-w-a")).expect("a folder");
        let ask = serde_json::json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "AskUserQuestion",
            "id": "toolu_1", "input": {"questions": [
                {"header": "Scope", "question": "Which slice ships first?", "multiSelect": false, "options": [
                    {"label": "Dialog reader (Recommended)", "description": "Port the screen and transcript reader first."},
                    {"label": "The queue", "description": "The batched wake first."}]},
                {"header": "Wake", "question": "Where does the wake go?", "multiSelect": false, "options": [
                    {"label": "The comment wake", "description": "One line with his comments."},
                    {"label": "A file of its own", "description": "A second wake file."}]}]}}]}});
        std::fs::write(dir.0.join("-w-a/s1.jsonl"), format!("{{}}\n{ask}\n"))
            .expect("a transcript");
        dir
    }

    /// Done-when 1: the wake's words hold every tab's question and options, and the pick line.
    #[dash_test]
    fn a_two_tab_dialog_reads_every_question_its_options_and_the_pick_line() {
        let shown = parse(TWO_TABS).expect("a live dialog");
        assert_eq!(shown.tabs.len(), 2);
        assert_eq!(shown.question, "Which slice ships first?");
        assert_eq!(
            shown.options[1].description.as_deref(),
            Some("The batched wake first.")
        );
        let dir = projects("two-tabs");
        let d = read(&pane(&[TWO_TABS], true), "w3:p1", &dir.0).expect("its dialog");
        assert!(d.whole, "both tabs from the transcript");
        let said = words(&d);
        for part in [
            "Q1 [Scope] Which slice ships first? Options: 1. Dialog reader (Recommended) (Port the screen and transcript reader first.); 2. The queue (The batched wake first.).",
            "Q2 [Wake] Where does the wake go? Options: 1. The comment wake (One line with his comments.); 2. A file of its own (A second wake file.).",
            "Pick: ~/repo/site_perso/private/dashboard/bin/dash pane pick w3:p1 <n> <n>",
        ] {
            assert!(said.contains(part), "{part} in {said}");
        }
    }

    /// Screens real panes showed (the older app's `Dashboard.DialogScreens`, 2026-10-21 and 10-24), `RULE` a rule of 120.
    fn real(screen: &str) -> String {
        screen.replace("RULE", &"─".repeat(120))
    }

    const TABS: &str = "     355:  @doc \"`comments` each with its task's state\"
RULE
←  ☒ Merging  ☐ whatsnr  ✔ Submit  →

│ geodb-whatsnr is shared with asiegf: the RFC (section 9) holds its import and Backlog.md removal on his yes, then
│ asiegf's. How do I treat that part?

❯ 1. Hold it, named as waiting (Recommended)
     geodb-whatsnr is not imported and keeps its folder and its board reads it from its files; its removal is not
     delegated. My report names it as waiting on him and asiegf, not done.
  2. Waive it from this task
     Same build, but the part is waived here and moves to a later task the orchestrator names.
  3. Do it now
     Import geodb-whatsnr and delegate its removal now, without asiegf's yes.
  4. Type something.
RULE
  5. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";

    const CUT: &str =
        "⏺ Committed. Now the dialog: approval of the 23 changed blocks, and the explainers' scope.

RULE
←  ☐ Chat blocks  ☐ Explainers  ✔ Submit  →

│ The chat's words after a humanizer pass?

❯ 1. Approve all 23               ┌──────────────────────────────────────────┐
    (Recommended)                 │ (preview cannot be shown in full)        │
  2. Keep the old wording         └──────────────────────────────────────────┘

                                  Notes: press n to add notes

RULE
  Chat about this

Enter to select · ↑/↓ to navigate · n to add notes · Tab to switch questions · Esc to cancel
";

    /// A permission prompt counting down is no question dialog (wJ6:p1, 2026-10-24).
    const SAFETY: &str = " Bash command
 ⚠ Claude Code will automatically deny this request in 0:03, to avoid blocking progress on an unattended session

 Do you want to proceed?
 ❯ 1. Yes
   2. No
";

    #[dash_test]
    fn real_screens_read_as_the_older_app_read_them() {
        let tabs = parse(&real(TABS)).expect("a dialog");
        let headers: Vec<(&str, bool)> = tabs
            .tabs
            .iter()
            .map(|t| (t.header.as_str(), t.answered))
            .collect();
        assert_eq!(headers, [("Merging", true), ("whatsnr", false)]);
        assert!(
            tabs.question
                .starts_with("geodb-whatsnr is shared with asiegf: ")
                && tabs.question.ends_with("How do I treat that part?")
        );
        let labels: Vec<&str> = tabs.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Hold it, named as waiting (Recommended)",
                "Waive it from this task",
                "Do it now"
            ]
        );
        assert!(
            tabs.options[0]
                .description
                .as_deref()
                .is_some_and(|d| d.ends_with("waiting on him and asiegf, not done."))
        );
        let cut = parse(&real(CUT)).expect("a dialog beside a preview");
        assert_eq!(cut.question, "The chat's words after a humanizer pass?");
        let labels: Vec<&str> = cut.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Approve all 23 (Recommended)", "Keep the old wording"]
        );
        assert_eq!(parse(SAFETY), None);
        assert_eq!(parse("Steps:\n1. read\n2. fix\n"), None);
    }

    /// Scrollback is no dialog: output under the options, or the input box under them.
    #[dash_test]
    fn a_dialog_in_scrollback_is_none() {
        let gone = TWO_TABS.replace("Enter to select", "⏺ Answered.\n> ");
        assert_eq!(parse(&gone), None);
    }

    /// Done-when 4: each refusal of `dash pane pick` types nothing (a stray « 1 » was left in a pane, 2026-11-11).
    #[dash_test]
    fn a_refused_pick_types_nothing() {
        let dir = projects("refused");
        let n = |n| Pick::Number(n);
        for (asking, picks, expected) in [
            (false, vec![n(1), n(1)], "shows no question dialog"),
            (true, vec![n(1)], "one answer per question"),
            (
                true,
                vec![n(1), n(3)],
                "an answer is not one of its options",
            ),
            (
                true,
                vec![n(1), Pick::Text(String::new())],
                "a typed answer is empty",
            ),
        ] {
            let p = pane(&[TWO_TABS], asking);
            let why = pick(&p, "w3:p1", &picks, &dir.0, Duration::ZERO).expect_err("refused");
            assert!(why.contains(expected), "{expected} in {why}");
            assert!(
                p.keys.borrow().is_empty(),
                "nothing typed on « {why} »: {:?}",
                p.keys.borrow()
            );
        }
        let screen_only = pane(&[TWO_TABS], true);
        let why = pick(
            &screen_only,
            "w3:p1",
            &[n(1), n(1)],
            Path::new("/nowhere"),
            Duration::ZERO,
        )
        .expect_err("refused");
        assert!(why.contains("no transcript holds its other tabs"), "{why}");
        assert!(screen_only.keys.borrow().is_empty());
    }

    /// Submit already focused on a slow pane: its number leaves the screen as it was, and Enter still goes (/code-review).
    #[dash_test]
    fn a_focused_submit_on_a_slow_pane_still_takes_its_enter() {
        let dir = projects("slow-submit");
        let second = TWO_TABS
            .replace("☐ Scope", "☒ Scope")
            .replace("Which slice ships first?", "Where does the wake go?");
        let submit = " ←  ☒ Scope  ☒ Wake  ✔ Submit  →\n\nReview your answers\n\n❯ 1. Submit answers\n  2. Cancel\n";
        let p = pane(&[TWO_TABS, TWO_TABS, &second, &second, submit], true);
        let picks = [Pick::Number(2), Pick::Number(1)];
        pick(&p, "w3:p1", &picks, &dir.0, Duration::ZERO).expect("answered");
        assert_eq!(*p.keys.borrow(), ["2", "1", "1", "Enter"]);
    }

    /// A pick accepted types each number, Enter while the question stays, then Submit.
    #[dash_test]
    fn a_pick_types_each_number_then_submits() {
        let dir = projects("typed");
        let second = TWO_TABS
            .replace("☐ Scope", "☒ Scope")
            .replace("Which slice ships first?", "Where does the wake go?");
        let submit = " ←  ☒ Scope  ☒ Wake  ✔ Submit  →\n\nReview your answers\n\n❯ 1. Submit answers\n  2. Cancel\n";
        let p = pane(
            &[TWO_TABS, TWO_TABS, &second, &second, submit, submit, "done"],
            true,
        );
        pick(
            &p,
            "w3:p1",
            &[Pick::Number(2), Pick::Number(1)],
            &dir.0,
            Duration::ZERO,
        )
        .expect("answered");
        assert_eq!(*p.keys.borrow(), ["2", "1", "1"]);
    }
}
