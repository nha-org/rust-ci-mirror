//! `dash task add|update|done` and a fold's raise answered from Rust's task store, the older app's copy sent after, off the
//! caller's path (task rust-task-writes-port; @nha, 2026-11-10: « remember we move to rust. no more elixir, so if something
//! needs fixing port and then fix »). On 2026-11-12 the older app's store lock convoyed 77 s and task writes needed 300 s
//! timeouts all day: a write now waits on Rust's store alone, and the older app gets each one in order once it answers.
//!
//! The door checks the shape (`refusal`: the flags, their types, the ids it names); the older app's own checks (TaskStyle's
//! rules, a dup pick) run at the copy. ponytail: a refused copy flags its task to the orchestrator instead of refusing the
//! caller; port TaskStyle (792 lines of Elixir) to the door if those flags come often.

use dash_store::older::OWNERS;
use dash_store::tasks::{Copy, SqliteTasks};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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
            _ => Err(format!("--status {v}: open or dropped")),
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
/// refuse them. `add`: its own words too (--start, --later, --distinct-from, --because, --into).
fn flags(words: &[String], add: bool) -> Result<(), String> {
    let mut seen: Vec<&str> = vec![];
    let mut at = 0;
    while at < words.len() {
        let w = words[at].as_str();
        let next = words.get(at + 1).map(String::as_str);
        match (w, add) {
            ("--new", _) => return Err(NEW_GONE.into()),
            ("--start", true) => {
                at += 1 + usize::from(next.is_some_and(|n| !is_flag(n)));
                continue;
            }
            ("--later", true) if next.is_none_or(is_flag) => {
                return Err("--later: give why it waits, --later <why>".into());
            }
            ("--later" | "--distinct-from" | "--because" | "--into", true) => {}
            _ if !is_flag(w) => {
                let rest = words[at..].join(" ");
                return Err(format!("{rest}: flags come in pairs, --<field> <text>"));
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
                    return Err(format!(
                        "{w}: not a task flag; one of --{}",
                        all.join(" --")
                    ));
                };
                let Some(v) = next else {
                    return Err(format!("{w}: flags come in pairs, --<field> <text>"));
                };
                if is_flag(v) {
                    return Err(format!("{w}: no text before {v}"));
                }
                if seen.contains(&name) && !matches!(kind, Kind::Many | Kind::Words) {
                    return Err(format!("--{name}: given twice"));
                }
                seen.push(name);
                value(name, *kind, v)?;
            }
        }
        if next.is_none() {
            return Err(format!("{w}: flags come in pairs, --<field> <text>"));
        }
        at += 2;
    }
    Ok(())
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
        "done" => Some(("done", vec![rest[0].as_str()])),
        _ => None,
    }
}

/// Why the door refuses `args`, a write `write_of` read, before anything is filed: its shape, or a task it names that the list
/// does not hold (an update, a done) or holds already (an add). `known`: whether a task is filed or its add waits for its copy.
/// Exit 2 for a misuse, as the older app's usage, 1 for a refusal.
pub fn refusal(
    args: &[String],
    known: &dyn Fn(&str) -> Result<bool, String>,
) -> Option<(i32, String)> {
    let (verb, ids) = write_of(args)?;
    let rest = &args[2..];
    let usage = match verb {
        "add" | "into" => "task add <id> --<field> <text> ...",
        "update" => "task update <id> [<id> ...] --<field> <text> ...",
        _ => "task done <id> <pr> merged|review <note>",
    };
    let shape = match verb {
        "done" => match rest {
            [_, _, state, note, ..]
                if ["merged", "review"].contains(&state.as_str()) && !note.is_empty() =>
            {
                Ok(())
            }
            _ => return Some((2, format!("usage: dash {usage}"))),
        },
        _ => {
            let words = &rest[ids.len().min(rest.len())..];
            if ids.is_empty() || words.is_empty() || ids.iter().any(|id| is_flag(id)) {
                return Some((2, format!("usage: dash {usage}")));
            }
            flags(words, verb != "update")
        }
    };
    if let Err(why) = shape {
        return Some((1, why));
    }
    for id in ids {
        // an add on a list it cannot read is filed, as the layer check files it: its copy's landing check reads the list again
        let held = match (known(id), verb) {
            (Ok(held), _) => held,
            (Err(_), "add") => false,
            (Err(e), _) => {
                return Some((1, format!("{id}: refused, the list could not be read: {e}")));
            }
        };
        match (verb, held) {
            ("add", true) => return Some((1, format!("{id}: refused, it exists"))),
            ("update" | "done" | "into", false) => {
                return Some((1, format!("{id}: refused, the list holds no such task")));
            }
            _ => {}
        }
    }
    None
}

/// The door's verdict on a filed write, said last.
pub fn verdict(verb: &str, ids: &[&str]) -> String {
    let did = match verb {
        "add" => "added",
        "into" => "added its checks into",
        "update" => "updated",
        _ => "done",
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
    let kept: Vec<&str> = form
        .split('&')
        .filter(|p| !p.is_empty() && !env(p))
        .collect();
    kept.join("&")
}

/// A form bin/dash would send for `args` run in `cwd`.
pub fn form_of(cwd: &Path, args: &[&str]) -> String {
    let encoded = |s: &str| -> String {
        (s.bytes())
            .map(
                |b| match b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    true => (b as char).to_string(),
                    false => format!("%{b:02X}"),
                },
            )
            .collect()
    };
    let mut form = format!("cwd={}", encoded(&cwd.display().to_string()));
    for a in args {
        form.push_str(&format!("&args%5B%5D={}", encoded(a)));
    }
    form
}

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
        Some(_) => Sent::Refused(body(&raw)),
    }
}

/// Whether the older app's list holds copy `c` already: an add's task filed, a done's task done. An update is sent again,
/// which writes the same fields. ponytail: checks added `--into` a task are sent again too, so a send cut past `SEND_LIMIT`
/// after the older app filed them adds them twice; a copy id the older app records would end it.
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
    let ended = round(
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
    )?;
    Ok(ended)
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
    if let Some(copier) = COPIER.get() {
        copier.unpark();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use std::net::TcpListener;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(String::from).collect()
    }

    #[test]
    fn the_door_refuses_a_write_s_shape_as_the_older_app_does() {
        let _budget = TestBudget::new();
        let known = |id: &str| Ok(id == "old");
        let refused = |line: &str| refusal(&words(line), &known).map(|(_, why)| why);
        assert_eq!(refused("task add new --title A --priority 2 --start"), None);
        assert_eq!(
            refused("task add new --title A --later why --run.prompt go"),
            None
        );
        assert_eq!(refused("task update old --done-when a --done-when b"), None);
        assert_eq!(refused("task done old 12 merged it landed"), None);
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
            refused("task add old --title A").as_deref(),
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
            refused("task add new --title A --title B").as_deref(),
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
            refusal(&words("task done old 12 shipped x"), &known).map(|(c, _)| c),
            Some(2)
        );
    }

    #[test]
    fn a_kept_form_drops_the_caller_s_environment() {
        let _budget = TestBudget::new();
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
    #[test]
    fn a_jammed_older_app_loses_no_copy_and_gets_them_in_order() {
        let _budget = TestBudget::new();
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

    /// One copy the older app answers with no verdict (a 500 page) is refused after `ODD_TRIES` answers, so it never holds the
    /// rest back; a busy or down app is waited for however long. A stall past a minute is said by every write.
    #[test]
    fn an_answer_with_no_verdict_refuses_its_copy_after_twelve_and_a_stall_is_said() {
        let _budget = TestBudget::new();
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
    #[test]
    fn a_refused_add_takes_its_later_copies_with_it() {
        let _budget = TestBudget::new();
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
    #[test]
    fn a_copy_that_landed_is_not_sent_twice() {
        let _budget = TestBudget::new();
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
