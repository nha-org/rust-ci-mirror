//! `dash task add|update|done|ended` and a fold's raise answered from Rust's task store, the older app's copy after, off the
//! caller's path (task rust-task-writes-port; @nha, 2026-11-10: « remember we move to rust. no more elixir, so if something
//! needs fixing port and then fix »). On 2026-11-12 the older app's store lock convoyed 77 s and task writes needed 300 s
//! timeouts all day: a write now waits on Rust's store alone, and the older app gets each one in order once it answers.
//!
//! The door answers every problem at once before it files (`refusal`: the flags, their types, an add's --start or --later
//! and priority, TaskStyle's bars ported in `style`, the ids it names), then an add asks the older app's own dry run
//! (`checked`), so its other rules (paths, AI tells, a dup pick) refuse the caller too (task rust-copy-refusal-answered: 12 of
//! 23 copies were refused after an « added »). An update or an into has no dry run there: a refusal its copy meets later
//! flags its task (`told`), and a copy the older app's store interrupts is sent again (`passing`).

use dash_store::older::{OWNERS, STATUSES};
use dash_store::tasks::{Copy, SqliteTasks};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};
use std::time::Duration;

/// The older app's `@new_gone` (Dashboard.Records), said the same way here.
const NEW_GONE: &str =
    "--new: gone; to file past a close task, --distinct-from <id>[,<id>] --because <why> (logged)";

/// Each flag `dash task add|update` takes and its type, the older app's `@task_flags` (Dashboard.Records).
const FLAGS: [(&str, Kind); 28] = [
    ("title", Kind::Text),
    ("why", Kind::Text),
    ("done-when", Kind::Many),
    ("project", Kind::Text),
    ("added", Kind::Text),
    ("blocked-by", Kind::Text),
    ("main-fix", Kind::Text),
    ("dropped", Kind::Text),
    ("status", Kind::Status),
    ("epic", Kind::Word),
    ("parent", Kind::Word),
    ("priority", Kind::Whole),
    ("estimate", Kind::Whole),
    ("lesson-turns", Kind::Whole),
    ("roadmap", Kind::Text),
    ("related", Kind::Words),
    ("duplicate-of", Kind::Word),
    ("owner", Kind::Owner),
    ("plan-first", Kind::Boolean),
    ("labels", Kind::Many),
    ("every", Kind::Text),
    ("later", Kind::Text),
    ("prompt", Kind::Text),
    ("model", Kind::Text),
    ("effort", Kind::Text),
    ("agent", Kind::Text),
    ("max-turns", Kind::Whole),
    ("max-price", Kind::Number),
];

/// The flags of `@task_flags` that sit in `:run`, also given as `--run.<name>`.
const RUN: [&str; 6] = [
    "prompt",
    "model",
    "effort",
    "agent",
    "max-turns",
    "max-price",
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Text,
    /// given once a value, each kept: done-when's checks, a label each
    Many,
    Status,
    Word,
    Words,
    Whole,
    Number,
    Boolean,
    Owner,
}

/// The value `v` of `--name`, refused as the older app's `flag_value/2` refuses it.
fn value(name: &str, kind: Kind, v: &str) -> Result<(), String> {
    let word = |v: &str| {
        let w = v.trim_start_matches(':');
        let ok = w.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && w.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        match ok {
            true => Ok(w.to_string()),
            false => Err(format!("{v}: one word of a-z, 0-9 and -")),
        }
    };
    if v == "nil" {
        return Ok(());
    }
    match kind {
        Kind::Text | Kind::Many => Ok(()),
        Kind::Word | Kind::Words => word(v).map(drop),
        Kind::Status if v.trim_start_matches(':') == "done" => {
            Err("--status done: close it with dash task done <id> <pr> merged|review <note>".into())
        }
        Kind::Status => match word(v)?.as_str() {
            "open" | "dropped" => Ok(()),
            w if STATUSES.contains(&w) => {
                Err(format!("--status {v}: a write sets open or dropped"))
            }
            _ => Err(format!(
                "--status {v}: none of the list's statuses: {}",
                STATUSES.join(", ")
            )),
        },
        Kind::Owner => match OWNERS.contains(&word(v)?.as_str()) {
            true => Ok(()),
            false => Err(format!("--owner {v}: one of {}", OWNERS.join(", "))),
        },
        Kind::Whole => (v.parse::<i64>().map(drop)).map_err(|_| format!("{v}: not a whole number")),
        Kind::Number => (v.parse::<f64>().map(drop)).map_err(|_| format!("{v}: not a number")),
        Kind::Boolean => match v {
            "true" | "false" => Ok(()),
            _ => Err(format!("{v}: true or false")),
        },
    }
    .map_err(|why| match why.starts_with("--") {
        true => why,
        false => format!("--{name} {why}"),
    })
}

fn is_flag(w: &str) -> bool {
    w.starts_with("--")
}

/// `words`, the flags after an add's id or an update's ids, refused as the older app's `go_pick`, `dup_pick` and `task_flags`
/// refuse them, every problem named at once (task rust-copy-refusal-answered): a flag's value or a flag given twice is one
/// line each, and a broken pair ends the reading, since the words after it no longer pair. `add`: its own words too
/// (--start, --later, --distinct-from, --because, --into).
fn flags(words: &[String], add: bool) -> Vec<String> {
    let mut seen: Vec<&str> = vec![];
    let mut out = vec![];
    let mut at = 0;
    while at < words.len() {
        let w = words[at].as_str();
        let next = words.get(at + 1).map(String::as_str);
        match (w, add) {
            ("--new", _) => {
                out.push(NEW_GONE.into());
                at += 1;
                continue;
            }
            ("--start", true) => {
                at += 1 + usize::from(next.is_some_and(|n| !is_flag(n)));
                continue;
            }
            ("--later", true) if next.is_none_or(is_flag) => {
                out.push("--later: give why it waits, --later <why>".into());
                at += 1;
                continue;
            }
            ("--later" | "--distinct-from" | "--because" | "--into", true) => {}
            _ if !is_flag(w) => {
                let rest = words[at..].join(" ");
                out.push(format!("{rest}: flags come in pairs, --<field> <text>"));
                return out;
            }
            _ => {
                let name = w.trim_start_matches("--");
                let name = name
                    .strip_prefix("run.")
                    .filter(|n| RUN.contains(n))
                    .unwrap_or(name);
                let Some((_, kind)) = FLAGS.iter().find(|(n, _)| *n == name) else {
                    let mut all: Vec<&str> = FLAGS.iter().map(|(n, _)| *n).collect();
                    all.sort_unstable();
                    out.push(format!(
                        "{w}: not a task flag; one of --{}",
                        all.join(" --")
                    ));
                    // its text skipped, never the next flag (/code-review medium)
                    at += 1 + usize::from(next.is_some_and(|n| !is_flag(n)));
                    continue;
                };
                let Some(v) = next else {
                    out.push(format!("{w}: flags come in pairs, --<field> <text>"));
                    return out;
                };
                if is_flag(v) {
                    out.push(format!("{w}: no text before {v}"));
                    at += 1;
                    continue;
                }
                if seen.contains(&name) && !matches!(kind, Kind::Many | Kind::Words) {
                    out.push(format!("--{name}: given twice"));
                }
                seen.push(name);
                out.extend(value(name, *kind, v).err());
            }
        }
        if next.is_none() {
            out.push(format!("{w}: flags come in pairs, --<field> <text>"));
            return out;
        }
        at += 2;
    }
    out
}

/// The older app's `start_decision/3` and `decided/3`: new work comes with its priority and --start or --later <why>, once;
/// checks added `--into` a filed task start nothing, so they take neither.
fn decided(words: &[String], into: Option<&str>) -> Option<String> {
    let goes = words
        .iter()
        .filter(|w| ["--start", "--later"].contains(&w.as_str()))
        .count();
    if let Some(into) = into {
        return (goes > 0).then(|| {
            format!("task add: refused, --into files nothing to start; to start {into}: dash task start {into}")
        });
    }
    let priority = (words.windows(2))
        .find(|p| p[0] == "--priority")
        .is_some_and(|p| p[1].parse::<i64>().is_ok_and(|n| (1..=4).contains(&n)));
    let missing: Vec<&str> = [
        (!priority).then_some("--priority <1-4>"),
        (goes == 0).then_some("--start (launch it now) or --later <why> (why it waits)"),
    ]
    .into_iter()
    .flatten()
    .collect();
    match (goes, missing.is_empty()) {
        (2.., _) => Some("task add: refused, --start or --later <why>, once and not both".into()),
        (_, true) => None,
        _ => Some(format!("task add: refused, give {}", missing.join(" and "))),
    }
}

/// Task `before` (none for an add) as `words` would write it: --title, --why, --done-when and the prompt, « nil » clearing.
fn written(before: crate::style::Task, words: &[String]) -> crate::style::Task {
    let mut t = before;
    // each value of any of `names`, in the order given: the older app writes the flags in turn, so the last one holds
    let given = |names: &[&str]| -> Vec<&str> {
        (words.windows(2))
            .filter(|p| names.iter().any(|n| p[0] == format!("--{n}")) && !is_flag(&p[1]))
            .map(|p| p[1].as_str())
            .collect()
    };
    let set = |field: &mut Option<String>, vs: Vec<&str>| {
        if let Some(v) = vs.last() {
            *field = (*v != "nil").then(|| v.to_string());
        }
    };
    set(&mut t.title, given(&["title"]));
    set(&mut t.why, given(&["why"]));
    set(&mut t.prompt, given(&["prompt", "run.prompt"]));
    // the older app waives the bars for his words filed as they are (`Tasks.unplanned?/1`)
    t.unplanned |= (given(&["labels"]).iter()).any(|l| l.eq_ignore_ascii_case("unplanned"));
    let checks = given(&["done-when"]);
    if !checks.is_empty() {
        t.done_when = (checks != ["nil"]).then(|| crate::style::numbered(&checks));
    }
    t
}

/// A task write this door answers: its verb and the tasks it names. `None`: not one (a read, a dry run, another command), which
/// goes on to the older app as before.
pub fn write_of(args: &[String]) -> Option<(&str, Vec<&str>)> {
    let [task, verb, rest @ ..] = args else {
        return None;
    };
    if task != "task" || rest.is_empty() {
        return None;
    }
    match verb.as_str() {
        "add" if rest.iter().any(|w| w == "--dry-run") => None,
        // checks added `--into` a filed task make no task: the write is that task's
        "add" => match rest.iter().position(|w| w == "--into") {
            Some(at) => Some(("into", vec![rest.get(at + 1).map_or("", String::as_str)])),
            None => Some(("add", vec![rest[0].as_str()])),
        },
        "update" => {
            let ids = rest.iter().take_while(|w| !is_flag(w)).map(String::as_str);
            Some(("update", ids.collect()))
        }
        "done" | "ended" => Some((verb.as_str(), vec![rest[0].as_str()])),
        _ => None,
    }
}

/// Why the door refuses `args`, a write `write_of` read, before anything is filed, every problem at once, one a line (task
/// rust-copy-refusal-answered): its shape, an add's start decision, the style bars on the task it writes (`filed`: a filed
/// task's fields, read from the older app's list), and each task it names that the list does not hold (an update, a done) or
/// holds already (an add). `known`: whether a task is filed or its add waits for its copy. Exit 2 for a misuse, as the older
/// app's usage, 1 for a refusal.
pub fn refusal(
    args: &[String],
    known: &dyn Fn(&str) -> Result<bool, String>,
    filed: &dyn Fn(&str) -> Option<crate::style::Task>,
) -> Option<(i32, String)> {
    let (verb, ids) = write_of(args)?;
    let rest = &args[2..];
    let usage = match verb {
        "add" | "into" => "task add <id> --<field> <text> ...",
        "update" => "task update <id> [<id> ...] --<field> <text> ...",
        "done" => "task done <id> <pr> merged|review <note>",
        _ => "task ended <id> <why>",
    };
    // an add's flags follow its id, an update's its ids
    let words = &rest[ids.len().min(rest.len())..];
    let shaped = match verb {
        "done" => matches!(rest, [_, _, state, note, ..]
                if ["merged", "review"].contains(&state.as_str()) && !note.is_empty()),
        _ => !ids.is_empty() && !words.is_empty() && !ids.iter().any(|id| is_flag(id)),
    };
    if !shaped {
        return Some((2, format!("usage: dash {usage}")));
    }
    // an ended names its task and its why, nothing more: no list read, so a store it cannot read never refuses it
    if verb == "ended" {
        return None;
    }
    // a done's words are no flags
    let mut out = if verb == "done" {
        vec![]
    } else {
        flags(words, verb != "update")
    };
    if ["add", "into"].contains(&verb) {
        out.extend(decided(words, (verb == "into").then(|| ids[0])));
    }
    // the style bars beside the flags' problems, so one answer names them all (done-when 4)
    out.extend(styled(verb, &rest[0], &ids, words, filed));
    for id in ids {
        // an add on a list it cannot read is filed, as the layer check files it: its copy's landing check reads the list again
        let held = match (known(id), verb) {
            (Ok(held), _) => held,
            (Err(_), "add") => false,
            (Err(e), _) => {
                out.push(format!("{id}: refused, the list could not be read: {e}"));
                continue;
            }
        };
        match (verb, held) {
            ("add", true) => out.push(format!("{id}: refused, it exists")),
            ("update" | "done" | "into", false) => {
                out.push(format!("{id}: refused, the list holds no such task"));
            }
            _ => {}
        }
    }
    (!out.is_empty()).then(|| (1, out.join("\n")))
}

/// The style refusals of a write: an add's task as its words write it, an update's each filed task as it would become, an
/// into's task with the add's checks after its own (the older app's `into/5`). `first`: the word after the verb, an add's id.
fn styled(
    verb: &str,
    first: &str,
    ids: &[&str],
    words: &[String],
    filed: &dyn Fn(&str) -> Option<crate::style::Task>,
) -> Vec<String> {
    use crate::style::{Task, as_checks, new_problems, numbered, problems, refused};
    let mut out = vec![];
    match verb {
        "add" => {
            let ps = problems(&written(Task::default(), words));
            if !ps.is_empty() {
                out.push(refused(first, &ps));
            }
        }
        "update" => {
            for id in ids {
                let Some(before) = filed(id) else { continue };
                let ps = new_problems(&before, &written(before.clone(), words));
                if !ps.is_empty() {
                    out.push(refused(id, &ps));
                }
            }
        }
        "into" => {
            let into = ids[0];
            let Some(before) = filed(into) else {
                return out;
            };
            let mine = written(Task::default(), words).done_when;
            let all = [
                as_checks(before.done_when.as_deref()),
                as_checks(mine.as_deref()),
            ]
            .concat();
            let all: Vec<&str> = all.iter().map(String::as_str).collect();
            let after = Task {
                done_when: Some(numbered(&all)),
                ..before.clone()
            };
            let ps = new_problems(&before, &after);
            if !ps.is_empty() {
                let why = refused(into, &ps);
                out.push(format!("--into {into}, with {first}'s checks added: {why}"));
            }
        }
        _ => {}
    }
    out
}

/// How long an add waits on the older app's own dry run: past it, the add is filed and its copy judged when sent.
pub const CHECK_LIMIT: Duration = Duration::from_secs(20);

/// The older app's own checks on an add, before it is filed (its `--dry-run`: style, overlap, start, every field): its
/// refusal, or a line said beside the verdict when it gave none within `limit` (busy, down, jammed), the copy then judged when
/// sent and a refusal flagged on its task. `form`: the add as the older app gets it.
pub fn checked(port: u16, form: &str, limit: Duration) -> Result<String, String> {
    match post(port, &format!("{form}&args%5B%5D=--dry-run"), limit) {
        Sent::Took => Ok(String::new()),
        Sent::Refused(why) => Err(why),
        Sent::Wait(why) | Sent::Odd(why) => Ok(format!(
            "the older app's checks gave no verdict ({why}): its copy is checked when sent, a refusal flagged on the task\n"
        )),
    }
}

/// The door's verdict on a filed write, said last.
pub fn verdict(verb: &str, ids: &[&str]) -> String {
    let did = match verb {
        "add" => "added",
        "into" => "added its checks into",
        "update" => "updated",
        _ => verb,
    };
    format!(
        "{did} {}: in Rust's task store; the older app's list gets its copy in order\n",
        ids.join(" ")
    )
}

/// `form` without the caller's environment: the older app reads a task write's folder, words and stdin, never its
/// environment, and a session's variables (a token among them) are never kept in the store.
pub fn kept_form(form: &str) -> String {
    let env = |pair: &&str| {
        let key = pair.split_once('=').map_or(*pair, |(k, _)| k);
        crate::pages::form_decoded(key) == "env[]"
    };
    form.split('&')
        .filter(|p| !p.is_empty() && !env(p))
        .collect::<Vec<_>>()
        .join("&")
}

pub use crate::pages::form_of;

/// Write `form`, the `verb` of `tasks` (a multi-id update names each, space-separated), into the task store in `data`, and wake
/// the copier: it is the store's from here on. Returns the copier's stall, said before the verdict (`stall`), or nothing.
pub fn file(data: &Path, verb: &str, tasks: &str, form: &str) -> Result<String, String> {
    let mut store = crate::tasks::open_tasks(data)?;
    store.file_copy(tasks, verb, &kept_form(form))?;
    wake();
    let waiting = store.uncopied()?;
    let oldest = waiting.first().map_or(0, |c| c.at_ms);
    Ok(stall(waiting.len(), oldest, crate::tasks::now_ms()).unwrap_or_default())
}

/// How long the older app may take no copy before every write says so (rust/CLAUDE.md rule 5: a watcher's silence is shown
/// where it is read).
const STALL_MS: i64 = 60_000;

/// The line a write says when the oldest of the `waiting` copies was filed at `oldest_ms`, past `STALL_MS` before `now_ms`.
pub fn stall(waiting: usize, oldest_ms: i64, now_ms: i64) -> Option<String> {
    let late = now_ms - oldest_ms;
    (waiting > 0 && late > STALL_MS).then(|| {
        format!(
            "the older app has taken no task write for {} s: {waiting} wait in Rust's task store, sent in order once it answers\n",
            late / 1000
        )
    })
}

/// A fold's raise of task `id`, `flags` its update (task rust-task-writes-port: `dash task run --folded` waited on the older
/// app through `dash task update`), filed as the door files one.
pub fn file_update(id: &str, flags: &[(&str, &str)]) -> Result<(), String> {
    let data = crate::pages::serve_from_dir()?;
    let mut args = vec!["task", "update", id];
    args.extend(flags.iter().flat_map(|(k, v)| [*k, *v]));
    file(data, "update", id, &form_of(&crate::tasks::repo(), &args)).map(drop)
}

/// `dash pane pick`'s rewrite of task `id` after an answer moving scope out of it (task move-names-task): the form of an
/// update giving its new title and part `r.part`'s new text, every other part of `done_when` (its filed one) kept, as the door
/// files one from `cwd`. Refused before any key: no such part, a done-when opening with words before « 1. » (an update by
/// parts would drop them), or a write to `id` still waiting for the older app in the store at `data` (its row is not the
/// filed one yet: rewriting it would undo that write).
pub fn rewrite_form(
    data: &Path,
    cwd: &Path,
    id: &str,
    done_when: &str,
    r: &dash_terminal::dialog::Rewrite,
) -> Result<String, String> {
    let mut parts = crate::reports::parts(done_when);
    if !done_when.trim_start().starts_with("1. ") {
        return Err(format!(
            "{id}'s done-when opens with words before « 1. »: rewrite it with dash task update"
        ));
    }
    let Some(part) = parts.get_mut(r.part.wrapping_sub(1)) else {
        return Err(format!(
            "{id} has no done-when part {}: it holds {}",
            r.part,
            parts.len()
        ));
    };
    *part = r.text.clone();
    let waiting = crate::tasks::open_tasks(data)?.uncopied()?;
    if waiting.iter().any(|c| c.task.split(' ').any(|t| t == id)) {
        return Err(format!(
            "{id} has a write waiting for the older app: rewrite it once that lands"
        ));
    }
    let mut args = vec!["task", "update", id, "--title", &r.title];
    args.extend(parts.iter().flat_map(|p| ["--done-when", p.as_str()]));
    Ok(form_of(cwd, &args))
}

/// How the older app took one copy.
#[derive(Debug, PartialEq)]
pub enum Sent {
    /// It filed it (exit 0).
    Took,
    /// It said no (any other exit it ran to): its words.
    Refused(String),
    /// No verdict: down, busy (exit 75), cut or jammed. Sent again next round.
    Wait(String),
    /// An answer with no verdict (a 500 page, a 404): sent again, until `ODD_TRIES` of them in a row refuse it (`judged`).
    Odd(String),
}

/// How many answers with no verdict in a row refuse a copy: one copy the older app cannot run never holds the rest back.
const ODD_TRIES: u32 = 12;

/// `sent` as the round reads it, `odd` the count of answers with no verdict this copy got in a row.
pub fn judged(sent: Sent, odd: &mut u32) -> Sent {
    match sent {
        Sent::Odd(line) => {
            *odd += 1;
            match *odd >= ODD_TRIES {
                true => Sent::Refused(format!(
                    "{ODD_TRIES} answers with no verdict, the last: {line}"
                )),
                false => Sent::Wait(line),
            }
        }
        sent => {
            *odd = 0;
            sent
        }
    }
}

/// One round: each copy the older app lacks, oldest first, sent through `send` until one waits, so a task's writes land in
/// the order they were made. `landed`: whether an earlier send took it already (an answer cut after the older app filed
/// it), checked before each send, so nothing is filed twice. `refused`: told each refusal. A task whose add was refused has
/// its later copies of the round refused with it, unsent and untold: one flag for one cause. Returns how many ended.
pub fn round(
    tasks: &mut SqliteTasks,
    landed: &dyn Fn(&Copy) -> bool,
    send: &mut dyn FnMut(&Copy) -> Sent,
    refused: &mut dyn FnMut(&Copy, &str),
) -> Result<usize, String> {
    let mut ended = 0;
    let mut unadded: Vec<String> = vec![];
    for copy in tasks.uncopied()? {
        let gone = copy.task.split(' ').any(|t| unadded.iter().any(|u| u == t));
        let state = match (gone, landed(&copy)) {
            (true, _) => "refused: its add was refused".to_string(),
            (false, true) => "copied".to_string(),
            (false, false) => match send(&copy) {
                Sent::Took => "copied".to_string(),
                Sent::Refused(why) => {
                    if copy.verb == "add" {
                        unadded.push(copy.task.clone());
                    }
                    refused(&copy, &why);
                    format!("refused: {why}")
                }
                Sent::Wait(_) | Sent::Odd(_) => break,
            },
        };
        tasks.copy_ended(&copy.row, &state)?;
        ended += 1;
    }
    Ok(ended)
}

/// bin/dash's exit code in the older app's answer, its `x-exit` header; `None` when it has none.
pub fn exit_of(raw: &[u8]) -> Option<i32> {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..at]).ok()?;
    head.lines().find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("x-exit")
            .then(|| value.trim().parse().ok())?
    })
}

/// How long one copy may take: past it the older app is jammed, and the copy waits for the next round.
const SEND_LIMIT: Duration = Duration::from_secs(120);

/// `form` posted to the older app's CLI on `port` as bin/dash posts it, its verdict read within `limit`.
pub fn post(port: u16, form: &str, limit: Duration) -> Sent {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = match TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
        Ok(stream) => stream,
        Err(e) => return Sent::Wait(format!("no answer at {address}: {e}")),
    };
    let _ = stream.set_read_timeout(Some(limit));
    let _ = stream.set_write_timeout(Some(limit));
    let request = format!(
        "POST /dashboard/cli HTTP/1.1\r\nHost: 127.0.0.1\r\nx-dashboard-cli: 1\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
        form.len()
    );
    let mut raw = vec![];
    if let Err(e) =
        (stream.write_all(request.as_bytes())).and_then(|()| stream.read_to_end(&mut raw))
    {
        return Sent::Wait(format!("cut: {e}"));
    }
    let body = |raw: &[u8]| {
        let at = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map_or(raw.len(), |a| a + 4);
        String::from_utf8_lossy(&raw[at..]).trim().to_string()
    };
    let line = || {
        let text = String::from_utf8_lossy(&raw);
        text.lines().next().unwrap_or("an empty answer").to_string()
    };
    match exit_of(&raw) {
        Some(0) => Sent::Took,
        Some(75) => Sent::Wait(line()),
        None if raw.is_empty() => Sent::Wait("an empty answer".into()),
        None => Sent::Odd(line()),
        Some(_) if passing(&body(&raw)) => Sent::Wait(line()),
        Some(_) => Sent::Refused(body(&raw)),
    }
}

/// Whether the older app's refusal is its store's passing state, no verdict on the write: an interrupted read or a busy
/// database (task rust-copy-refusal-answered: 7 copies of 2026-11-12 were refused « refused by the store: interrupted »), so
/// the copy is sent again, in order, until it lands. Only an answer opening on the store's words counts, « refused by the
/// store: » or « dash: task <verb> failed: », so a style refusal quoting « busy » is no such state.
pub fn passing(body: &str) -> bool {
    static STORE: LazyLock<regex::Regex> = LazyLock::new(|| {
        let words = r"(?i)^(?:refused by the store|dash: task \w+ failed): (?:interrupted|busy|database busy|database is locked)\b";
        regex::Regex::new(words).expect("the store's words")
    });
    STORE.is_match(body.lines().next().unwrap_or_default())
}

/// Whether the older app's list holds copy `c` already: an add's task filed, a done's task done. An update is sent again,
/// which writes the same fields. ponytail: `--into` checks and an `ended` are sent again too, so a send cut past `SEND_LIMIT`
/// after the older app filed one adds or flags it twice; a copy id the older app records would end it.
fn landed(db: &Path, c: &Copy) -> bool {
    match c.verb.as_str() {
        "add" => dash_store::older::filed(db, &c.task) == Ok(true),
        "done" => matches!(dash_store::older::stage(db, &c.task), Ok(Some((s, _))) if s == "done"),
        _ => false,
    }
}

/// A refusal: its task flagged on Rust's watcher, so the orchestrator reads it, and a refused add's layer or deadline, kept for
/// a task the list never filed, forgotten (task rust-front-door-known-id).
/// Told inside the round, before the copy's state is written: a reader that sees « refused » sees the layer gone too.
fn told(data: &Path, c: &Copy, why: &str) {
    let what = format!("the older app refused its {} copy: {why}", c.verb);
    for task in c.task.split(' ') {
        if let Err(e) = crate::tasks::flag_at(data, task, &what, crate::tasks::now_ms()) {
            eprintln!("copies: {task}: its flag: {e}");
        }
    }
    if c.verb == "add" && dash_store::older::filed(&crate::tasks::older_db(), &c.task) == Ok(false)
    {
        let forgot = crate::tasks::open_tasks(data)
            .and_then(|mut t| t.forget(std::slice::from_ref(&c.task)));
        if let Err(e) = forgot {
            eprintln!("copies: {}: its layer or deadline stays: {e}", c.task);
        }
    }
}

/// One round against the older app on `port`, the store in `data`.
pub fn once(data: &Path, port: u16) -> Result<usize, String> {
    let db = crate::tasks::older_db();
    let mut tasks = crate::tasks::open_tasks(data)?;
    // the answers with no verdict each copy got in a row, across rounds while this process lives
    static ODD: std::sync::Mutex<Option<(String, u32)>> = std::sync::Mutex::new(None);
    let mut odd = ODD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    round(
        &mut tasks,
        &|c| landed(&db, c),
        &mut |c| {
            let tries = match &mut *odd {
                Some((row, tries)) if *row == c.row => tries,
                _ => &mut odd.insert((c.row.clone(), 0)).1,
            };
            judged(post(port, &c.form, SEND_LIMIT), tries)
        },
        &mut |c, why| told(data, c, why),
    )
}

static COPIER: OnceLock<std::thread::Thread> = OnceLock::new();

/// The copier: a round at once on each filed write, and every 5 s while the older app has not taken them all.
pub fn start(data: PathBuf, port: u16) {
    let copier = std::thread::spawn(move || {
        loop {
            // a panic in a round (a store read) never ends the copier: the next round tries again
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| once(&data, port)));
            match ran {
                Ok(Err(e)) => eprintln!("copies: {e}"),
                Err(_) => eprintln!("copies: a round panicked"),
                Ok(Ok(_)) => {}
            }
            std::thread::park_timeout(Duration::from_secs(5));
        }
    });
    let _ = COPIER.set(copier.thread().clone());
}

fn wake() {
    COPIER.get().inspect(|copier| copier.unpark());
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::net::TcpListener;

    /// A line's words, `~` a space inside one; `FIELDS` the why and done-when every add needs.
    fn words(line: &str) -> Vec<String> {
        let fields =
            "--why (@nha,~2026-11-12:~«~fix~it~»)~A~refused~copy~is~lost. --done-when 1.~a";
        (line.replace("FIELDS", fields).split(' '))
            .map(|w| w.replace('~', " "))
            .collect()
    }

    fn none(_: &str) -> Option<crate::style::Task> {
        None
    }

    #[dash_test]
    fn the_door_refuses_a_write_s_shape_as_the_older_app_does() {
        let known = |id: &str| Ok(id == "old");
        let all = |line: &str| refusal(&words(line), &known, &none).map(|(_, why)| why);
        // the first problem named, as the older app names it
        let refused =
            |line: &str| all(line).map(|why| why.lines().next().unwrap_or_default().to_string());
        assert_eq!(
            refused("task add new --title A FIELDS --priority 2 --start"),
            None
        );
        assert_eq!(
            refused("task add new --title A FIELDS --priority 3 --later why --run.prompt go"),
            None
        );
        assert_eq!(refused("task update old --done-when a --done-when b"), None);
        assert_eq!(refused("task done old 12 merged it landed"), None);
        assert_eq!(
            refused("task ended ghost its~run~ended"),
            None,
            "no list check"
        );
        let unread = |_: &str| Err("the store is locked".to_string());
        assert_eq!(
            refusal(&words("task ended old why"), &unread, &none),
            None,
            "a list it cannot read never refuses an ended"
        );
        for no_id_or_why in ["task ended old", "task ended --help why"] {
            assert_eq!(
                refused(no_id_or_why).as_deref(),
                Some("usage: dash task ended <id> <why>")
            );
        }
        assert_eq!(
            refused("task add new --into old --title A"),
            None,
            "checks into a filed task"
        );
        assert_eq!(
            refused("task add new --into ghost --title A").as_deref(),
            Some("ghost: refused, the list holds no such task")
        );
        assert_eq!(
            refused("task add new --dry-run --title A"),
            None,
            "a dry run is the older app's"
        );
        assert_eq!(
            refused("task add old --title A FIELDS --priority 2 --start").as_deref(),
            Some("old: refused, it exists")
        );
        assert_eq!(
            refused("task update old ghost --title A").as_deref(),
            Some("ghost: refused, the list holds no such task")
        );
        assert_eq!(
            refused("task add new --priority high").as_deref(),
            Some("--priority high: not a whole number")
        );
        assert_eq!(
            refused("task add new --status done").map(|w| w.starts_with("--status done: close")),
            Some(true)
        );
        assert_eq!(
            refused("task add new --title A --title B FIELDS --priority 2 --start").as_deref(),
            Some("--title: given twice")
        );
        assert_eq!(
            refused("task add new --title").as_deref(),
            Some("--title: flags come in pairs, --<field> <text>")
        );
        assert_eq!(
            refused("task add new --title --why x").as_deref(),
            Some("--title: no text before --why")
        );
        assert!(
            refused("task add new --colour red")
                .is_some_and(|w| w.starts_with("--colour: not a task flag"))
        );
        assert_eq!(
            refused("task add new --new --title A").as_deref(),
            Some(NEW_GONE)
        );
        assert_eq!(
            refusal(&words("task done old 12 shipped x"), &known, &none).map(|(c, _)| c),
            Some(2)
        );
    }

    /// Task rust-copy-refusal-answered, done-when 1: an add without its start decision, an add or an update breaking a style
    /// bar, and checks added `--into` with a start are refused at the door, exit 1, before anything is filed.
    #[dash_test]
    fn the_door_refuses_a_missing_start_and_a_style_bar_before_it_files() {
        let known = |id: &str| Ok(id == "old");
        let filed = |id: &str| {
            (id == "old").then(|| {
                crate::style::Task::filed("Old", "(@nha, 2026-11-12: « x ») Ours.", "1. a", "")
            })
        };
        let refused = |line: &str| refusal(&words(line), &known, &filed);
        let start =
            "task add: refused, give --start (launch it now) or --later <why> (why it waits)";
        assert_eq!(
            refused("task add new --title A FIELDS --priority 2"),
            Some((1, start.into()))
        );
        assert_eq!(
            refused("task add new --title A FIELDS --priority 2 --start --later why"),
            Some((
                1,
                "task add: refused, --start or --later <why>, once and not both".into()
            ))
        );
        assert_eq!(
            refused("task add new --title A FIELDS --start").map(|(_, w)| w),
            Some("task add: refused, give --priority <1-4>".into())
        );
        assert_eq!(
            refused("task add new --into old FIELDS --start").map(|(_, w)| w),
            Some("task add: refused, --into files nothing to start; to start old: dash task start old".into())
        );
        let title = "t".repeat(85);
        let (code, why) = refused(&format!(
            "task add new --title {title} FIELDS --priority 2 --start"
        ))
        .expect("refused");
        assert_eq!(code, 1);
        assert!(
            why.starts_with("new: nothing written, 1 style problems: 1) :title: 85 characters, under 80; limits:"),
            "{why}"
        );
        let (_, why) =
            refused(&format!("task update old --title {}", "t".repeat(81))).expect("refused");
        assert!(
            why.starts_with("old: nothing written, 1 style problems: 1) :title: 81 characters"),
            "{why}"
        );
        let six = "1.~a~2.~b~3.~c~4.~d~5.~e";
        let (_, why) =
            refused(&format!("task add new --into old --done-when {six}")).expect("refused");
        assert!(
            why.starts_with("--into old, with new's checks added: old: nothing written, 1 style problems: 1) :done-when: 6 checks"),
            "{why}"
        );
    }

    /// Task rust-copy-refusal-answered, done-when 4: four problems at once, each named in the first answer.
    #[dash_test]
    fn an_add_with_four_problems_names_them_all_at_once() {
        // an unknown flag skips its text, never the next flag (/code-review medium)
        let one = refusal(
            &words("task update old --bogus --title A"),
            &|_| Ok(true),
            &none,
        );
        assert!(one.is_some_and(
            |(_, w)| w.starts_with("--bogus: not a task flag") && w.lines().count() == 1
        ));
        // his words filed Unplanned keep their bars waived; --run.prompt is --prompt, so both is one flag twice
        let unplanned = "task add new --title A --why (@nha,~2026-11-12:~«~x~») --labels Unplanned --priority 3 --later plan";
        assert_eq!(refusal(&words(unplanned), &|_| Ok(false), &none), None);
        let prompts =
            "task add new --title A FIELDS --priority 2 --start --run.prompt long --prompt short";
        assert_eq!(
            refusal(&words(prompts), &|_| Ok(false), &none),
            Some((1, "--prompt: given twice".into()))
        );
        let known = |_: &str| Ok(false);
        let line = format!(
            "task add new --title {} --why (@nha,~2026-11-12:~«~x~»)~Ours. --done-when 1.~{} --priority high",
            "t".repeat(85),
            "c".repeat(170)
        );
        let (code, why) = refusal(&words(&line), &known, &none).expect("refused");
        assert_eq!(code, 1);
        for problem in [
            "--priority high: not a whole number",
            "task add: refused, give --priority <1-4> and --start (launch it now) or --later <why>",
            ":title: 85 characters, under 80",
            ":done-when check 1: 170 characters, one line of at most 160",
        ] {
            assert!(why.contains(problem), "{problem} missing from: {why}");
        }
    }

    /// Task stress-run-scoped, done-when 2: dash task add refuses a done-when asking for a stress in the full parallel run,
    /// exit 1 before anything is filed, naming « cargo nextest run -p <crate> <test name> --stress-count 20 » instead.
    #[dash_test]
    fn an_add_asking_a_stress_in_the_full_parallel_run_is_refused() {
        let line = "task add new --title A --why (@nha,~2026-11-12:~«~x~»)~Ours. --done-when 1.~it~passes~20~runs~in~nextest's~full~parallel~run --priority 2 --start";
        let (code, why) = refusal(&words(line), &|_| Ok(false), &none).expect("refused");
        assert_eq!(code, 1);
        assert!(
            why.starts_with("new: nothing written, 1 style problems: 1) :done-when check 1: a stress in the full parallel run")
                && why.contains("« cargo nextest run -p <crate> <test name> --stress-count 20 »"),
            "{why}"
        );
        let scoped = line.replace("in~nextest's~full~parallel~run", "of~-p~dash~not_built_yet");
        assert_eq!(refusal(&words(&scoped), &|_| Ok(false), &none), None);
    }

    /// A stand-in older app on a port: each request's text kept, each answered with the next of `answers` (exit, body); past
    /// them it closes.
    fn older_app(answers: Vec<(i32, &'static str)>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let app = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = app.local_addr().expect("address").port();
        let asked = std::thread::spawn(move || {
            let mut asked = vec![];
            for (exit, body) in answers {
                let (mut s, _) = app.accept().expect("a request");
                let mut buf = [0u8; 8192];
                let n = s.read(&mut buf).expect("its form");
                asked.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nx-exit: {exit}\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
            }
            asked
        });
        (port, asked)
    }

    /// Task rust-copy-refusal-answered, done-when 1: an add runs the older app's own checks, its dry run, before it is filed;
    /// its refusal is the caller's. An app that gives no verdict in time lets the add through, said.
    #[dash_test]
    fn an_add_runs_the_older_app_s_dry_run_and_its_refusal_is_the_caller_s() {
        let refusal = "new: nothing written, 1 style problems: 1) a/b.rs: no such tracked file";
        let (port, asked) = older_app(vec![(1, refusal)]);
        let form = form_of(Path::new("/r"), &["task", "add", "new", "--title", "A"]);
        assert_eq!(
            checked(port, &form, Duration::from_secs(5)),
            Err(refusal.into())
        );
        let asked = asked.join().expect("the app");
        assert!(
            asked[0].ends_with("args%5B%5D=A&args%5B%5D=--dry-run"),
            "{asked:?}"
        );

        let (port, _) = older_app(vec![(0, "dry run, nothing written")]);
        assert_eq!(
            checked(port, &form, Duration::from_secs(5)),
            Ok(String::new())
        );
        let silent = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = silent.local_addr().expect("address").port();
        let said = checked(port, &form, Duration::from_millis(300)).expect("let through");
        assert!(
            said.starts_with("the older app's checks gave no verdict"),
            "{said}"
        );
    }

    /// Done-when 3 of move-names-task: a scope-moving pick's rewrite files the task's update, its new title and the part's new
    /// text in place, the other parts kept; a part the task lacks files nothing.
    #[dash_test]
    fn a_scope_moving_pick_rewrites_the_task_s_title_and_part() {
        let dir = dash_meter::TestDir::new("copies-rewrite");
        let r = dash_terminal::dialog::Rewrite {
            title: "Waivers quote their answer".into(),
            part: 3,
            text: "Moved to rust-pick by « Move to a new task »".into(),
        };
        let form = rewrite_form(
            &dir.0,
            &dir.0,
            "move-x",
            "1. a: a test 2. b 3. c: a test",
            &r,
        )
        .expect("its form");
        file(&dir.0, "update", "move-x", &form).expect("filed");
        let tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("the store");
        let filed = tasks.uncopied().expect("the copies");
        assert_eq!(
            (filed.len(), filed[0].verb.as_str(), filed[0].task.as_str()),
            (1, "update", "move-x")
        );
        let words: Vec<String> = (filed[0].form.split('&'))
            .filter_map(|p| p.strip_prefix("args%5B%5D="))
            .map(crate::pages::form_decoded)
            .collect();
        assert_eq!(&words[..3], ["task", "update", "move-x"]);
        let row = written(crate::style::Task::default(), &words[3..]);
        assert_eq!(row.title.as_deref(), Some("Waivers quote their answer"));
        assert_eq!(
            row.done_when.as_deref(),
            Some("1. a: a test 2. b 3. Moved to rust-pick by « Move to a new task »")
        );
        let why = rewrite_form(&dir.0, &dir.0, "other", "1. a 2. b", &r).expect_err("no part 3");
        assert_eq!(why, "other has no done-when part 3: it holds 2");
        let why = rewrite_form(&dir.0, &dir.0, "other", "Both lanes. 1. a 2. b 3. c", &r)
            .expect_err("a preamble");
        assert!(why.contains("opens with words before « 1. »"), "{why}");
        // its update above still waits for the older app: a second rewrite would undo it (/code-review)
        let why =
            rewrite_form(&dir.0, &dir.0, "move-x", "1. a 2. b 3. c", &r).expect_err("waiting");
        assert_eq!(
            why,
            "move-x has a write waiting for the older app: rewrite it once that lands"
        );
    }

    /// Task rust-copy-refusal-answered, done-when 5: a copy the older app's store interrupts or finds busy is no verdict: it
    /// waits, holds the later copies behind it, and lands in order on a later round.
    #[dash_test]
    fn an_interrupted_or_busy_copy_is_sent_again_in_order_until_it_lands() {
        let dir = dash_meter::TestDir::new("copies-interrupted");
        let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("the store");
        for task in ["a", "b"] {
            let form = form_of(&dir.0, &["task", "add", task]);
            tasks.file_copy(task, "add", &form).expect("filed");
        }
        let interrupted = "refused by the store: interrupted\nSELECT EXISTS (SELECT 1 FROM links)";
        let busy = "dash: task add failed: Database busy";
        let (port, asked) = older_app(vec![
            (1, interrupted),
            (1, busy),
            (0, "added a"),
            (0, "added b"),
        ]);
        let mut said = vec![];
        let mut send = |c: &Copy| post(port, &c.form, Duration::from_secs(5));
        for _ in 0..2 {
            let ended = round(&mut tasks, &|_| false, &mut send, &mut |c, why| {
                said.push(format!("{} {why}", c.task))
            })
            .expect("a round");
            assert_eq!(ended, 0, "no verdict ends nothing");
        }
        let ended = round(&mut tasks, &|_| false, &mut send, &mut |c, why| {
            said.push(format!("{} {why}", c.task))
        })
        .expect("a round");
        assert_eq!(ended, 2);
        assert_eq!(said, Vec::<String>::new(), "nothing refused");
        let asked = asked.join().expect("the app");
        let order: Vec<bool> = (asked.iter().zip(["a", "a", "a", "b"]))
            .map(|(a, t)| a.ends_with(&format!("args%5B%5D={t}")))
            .collect();
        assert_eq!(order, [true; 4], "{asked:?}");
        assert!(!passing(
            "new: nothing written, 1 style problems: 1) :title: « refused by the store: busy »"
        ));
    }

    /// Task rust-copy-refusal-answered, done-when 2: a copy refused after its answer raises a flag on Rust's watcher naming its
    /// task and the refusal, each task of a multi-id update flagged.
    #[dash_test]
    fn a_copy_refused_later_flags_its_task_with_the_refusal() {
        let dir = dash_meter::TestDir::new("copies-told");
        let copy = Copy {
            row: "r1".into(),
            task: "x y".into(),
            verb: "update".into(),
            form: String::new(),
            at_ms: 0,
        };
        told(
            &dir.0,
            &copy,
            "x: nothing written, 1 style problems: 1) :title: 81 characters, under 80",
        );
        let flags: Vec<(Option<String>, String)> = (crate::watch::watched(&dir.0)
            .expect("the watcher"))
        .into_values()
        .map(|i| (i.task, i.title))
        .collect();
        let said = "is flagged: the older app refused its update copy: x: nothing written, 1 style problems: 1) :title: 81 characters, under 80";
        assert_eq!(
            flags,
            [
                (Some("x".to_string()), format!("Task x {said}")),
                (Some("y".to_string()), format!("Task y {said}")),
            ]
        );
    }

    #[dash_test]
    fn a_kept_form_drops_the_caller_s_environment() {
        let form =
            "cwd=%2Fa&args%5B%5D=task&env%5B%5D=GH_TOKEN%3Dsecret&env[]=HOME%3D%2Fx&stdin=hi";
        assert_eq!(kept_form(form), "cwd=%2Fa&args%5B%5D=task&stdin=hi");
        assert_eq!(
            form_of(Path::new("/r p"), &["task", "update", "a", "--why", "a&b"]),
            "cwd=%2Fr%20p&args%5B%5D=task&args%5B%5D=update&args%5B%5D=a&args%5B%5D=--why&args%5B%5D=a%26b"
        );
    }

    /// Task rust-task-writes-port, done-when 2: the older app gets each write as a copy, in order; a jammed app (it takes the
    /// connection and never answers) loses none, and once it answers again every copy lands, a refusal flagged, never dropped.
    #[dash_test]
    fn a_jammed_older_app_loses_no_copy_and_gets_them_in_order() {
        let dir = dash_meter::TestDir::new("copies-jammed");
        let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("the store");
        for (verb, task) in [("add", "a"), ("update", "a"), ("done", "a")] {
            let form = form_of(&dir.0, &["task", verb, task]);
            tasks.file_copy(task, verb, &form).expect("filed");
        }

        // jammed: it accepts and answers nothing within the limit
        let jammed = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = jammed.local_addr().expect("address").port();
        let mut send = |c: &Copy| post(port, &c.form, Duration::from_millis(300));
        let ended = round(&mut tasks, &|_| false, &mut send, &mut |_, _| {}).expect("a round");
        assert_eq!(ended, 0, "nothing ends while it is jammed");
        assert_eq!(tasks.uncopied().expect("read").len(), 3, "each copy kept");
        drop(jammed);

        // stopped: the same
        let ended = round(&mut tasks, &|_| false, &mut send, &mut |_, _| {}).expect("a round");
        assert_eq!(ended, 0);

        // back: an app that takes the add and the update and refuses the done, each answer read as bin/dash reads it
        let back = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = back.local_addr().expect("address").port();
        let app = std::thread::spawn(move || {
            let mut asked = vec![];
            for exit in [0, 0, 1] {
                let (mut s, _) = back.accept().expect("a copy");
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).expect("its form");
                asked.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let body = if exit == 0 {
                    "ok"
                } else {
                    "task done: no PR 12"
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nx-exit: {exit}\r\nx-stdout-bytes: 0\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
            }
            asked
        });
        let mut said = vec![];
        let mut send = |c: &Copy| post(port, &c.form, Duration::from_secs(5));
        let ended = round(&mut tasks, &|_| false, &mut send, &mut |c, why| {
            said.push(format!("{} {why}", c.verb))
        })
        .expect("a round");
        let asked = app.join().expect("the app");
        assert_eq!(ended, 3);
        assert_eq!(tasks.uncopied().expect("read"), vec![]);
        let verbs: Vec<bool> = (asked.iter().zip(["add", "update", "done"]))
            .map(|(a, verb)| {
                a.contains(&format!("args%5B%5D={verb}")) && a.contains("x-dashboard-cli: 1")
            })
            .collect();
        assert_eq!(verbs, [true, true, true], "in the order filed: {asked:?}");
        assert_eq!(said, ["done task done: no PR 12"]);
    }

    /// Task rust-task-ended-own-store, done-when 2: a `task ended` filed while the older app is stopped waits in Rust's store,
    /// nothing posted on the caller's path; once the app answers, the next round sends it, its why word for word.
    #[dash_test]
    fn an_ended_copy_waits_for_the_older_app_and_lands_once_it_answers() {
        let dir = dash_meter::TestDir::new("copies-ended");
        let form = form_of(&dir.0, &["task", "ended", "t1", "its run ended: gave up"]);
        file(&dir.0, "ended", "t1", &form).expect("filed");
        let mut tasks = crate::tasks::open_tasks(&dir.0).expect("the store");

        let stopped = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = stopped.local_addr().expect("address").port();
        drop(stopped);
        let mut send = |c: &Copy| post(port, &c.form, Duration::from_secs(5));
        let ended = round(&mut tasks, &|_| false, &mut send, &mut |_, _| {}).expect("a round");
        assert_eq!(ended, 0, "it waits while the app is stopped");
        assert_eq!(tasks.uncopied().expect("read").len(), 1);

        let (port, asked) = older_app(vec![(0, "task ended: t1 is flagged")]);
        let mut send = |c: &Copy| post(port, &c.form, Duration::from_secs(5));
        let ended = round(&mut tasks, &|_| false, &mut send, &mut |_, _| {}).expect("a round");
        assert_eq!(ended, 1);
        assert_eq!(tasks.uncopied().expect("read"), vec![]);
        let asked = asked.join().expect("the app");
        assert!(
            asked[0].ends_with("args%5B%5D=task&args%5B%5D=ended&args%5B%5D=t1&args%5B%5D=its%20run%20ended%3A%20gave%20up"),
            "{asked:?}"
        );
    }

    /// One copy the older app answers with no verdict (a 500 page) is refused after `ODD_TRIES` answers, so it never holds the
    /// rest back; a busy or down app is waited for however long. A stall past a minute is said by every write.
    #[dash_test]
    fn an_answer_with_no_verdict_refuses_its_copy_after_twelve_and_a_stall_is_said() {
        let mut odd = 0;
        for _ in 1..ODD_TRIES {
            assert_eq!(
                judged(Sent::Odd("HTTP/1.1 500".into()), &mut odd),
                Sent::Wait("HTTP/1.1 500".into())
            );
        }
        let last = judged(Sent::Odd("HTTP/1.1 500".into()), &mut odd);
        assert_eq!(
            last,
            Sent::Refused("12 answers with no verdict, the last: HTTP/1.1 500".into())
        );
        let mut busy = 0;
        for _ in 0..50 {
            assert!(matches!(
                judged(Sent::Wait("busy".into()), &mut busy),
                Sent::Wait(_)
            ));
        }
        assert_eq!(stall(3, 0, 30_000), None, "recent: nothing said");
        assert_eq!(
            stall(3, 0, 90_000).as_deref(),
            Some(
                "the older app has taken no task write for 90 s: 3 wait in Rust's task store, sent in order once it answers\n"
            )
        );
        assert_eq!(stall(0, 0, 90_000), None);
    }

    /// A refused add takes its task's later copies with it, unsent and flagged once; a multi-id update's copy names each task.
    #[dash_test]
    fn a_refused_add_takes_its_later_copies_with_it() {
        let dir = dash_meter::TestDir::new("copies-unadded");
        let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("the store");
        tasks.file_copy("x", "add", "f1").expect("filed");
        tasks.file_copy("y x", "update", "f2").expect("filed");
        tasks.file_copy("y", "update", "f3").expect("filed");
        let mut sent = vec![];
        let mut told = vec![];
        let ended = round(
            &mut tasks,
            &|_| false,
            &mut |c| {
                sent.push(c.form.clone());
                match c.verb.as_str() {
                    "add" => Sent::Refused("style".into()),
                    _ => Sent::Took,
                }
            },
            &mut |c, why| told.push(format!("{} {why}", c.task)),
        )
        .expect("a round");
        assert_eq!(ended, 3);
        assert_eq!(sent, ["f1", "f3"], "the update naming x is not sent");
        assert_eq!(told, ["x style"], "one flag");
    }

    /// A copy whose earlier send landed before its answer was cut is not sent again.
    #[dash_test]
    fn a_copy_that_landed_is_not_sent_twice() {
        let dir = dash_meter::TestDir::new("copies-landed");
        let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("the store");
        tasks.file_copy("a", "add", "f").expect("filed");
        let mut sent = 0;
        let ended = round(
            &mut tasks,
            &|_| true,
            &mut |_| {
                sent += 1;
                Sent::Took
            },
            &mut |_, _| {},
        )
        .expect("a round");
        assert_eq!((ended, sent), (1, 0));
        assert!(!tasks.adding("a").expect("read"), "its add no longer waits");
    }
}
