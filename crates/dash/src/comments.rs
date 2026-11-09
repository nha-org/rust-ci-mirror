//! The binary's wiring of the comments plugin: its table in `comments.sqlite` on the op log, the older app's comments read
//! through its CLI (`dash store text live/comments.edn`, `DASH_CLI` names another), and each wake appended to the
//! orchestrator's wake file, the one `<repo>/.claude/run/owner` names while its supervisor lives (bin/comment-wake's first
//! choice; `DASH_REPO` names another checkout). `dash orch` runs a pass every `POLL_MS` on a thread of its own;
//! `dash comment pass --data <dir>` runs one.
//!
//! The compass (task rust-compass-queue): `dash serve` lands each comment a page's sidebar posts as a row of the same table,
//! and answers what the sidebar lists; `dash orch`'s one agent works those rows oldest first, its answer closing each.
//! `dash comment work --data <dir>` works one.
use crate::tasks::{now_ms, older_app, older_db, repo};
use crate::watchers;
use dash_comments as comments;
use dash_store::oplog::Replica;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const READ: Duration = Duration::from_millis(comments::READ_MS as u64);
/// The queue's one agent, as /agents and the compass name it.
pub const AGENT: &str = "comment-agent";
/// One comment's run at most.
const WORK: Duration = Duration::from_secs(15 * 60);
/// The answered comments /comments lists, newest first.
const ANSWERED: usize = 20;
/// A comment's words and its quote at most: a post past them is refused, never cut.
const TEXT_LIMIT: usize = 8000;
const QUOTE_LIMIT: usize = 2000;

/// The folder `dash serve` keeps the queue in: its `--data`.
static DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn serve_from(dir: &Path) {
    let _ = DIR.set(dir.to_path_buf());
}

pub fn serve_from_dir() -> Result<&'static Path, String> {
    DIR.get()
        .map(PathBuf::as_path)
        .ok_or_else(|| "no comment queue in this process".to_string())
}

/// A day and time in UTC, as the older app's `at` reads: « 2026-11-09T10:00:00Z ».
pub(crate) fn iso(now_ms: i64) -> String {
    let secs = now_ms.div_euclid(1000);
    let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let date = dash_core::date(days);
    format!("{date}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// One row written alone.
fn put(log: &mut Replica, row: comments::Row) -> Result<(), String> {
    let one = BTreeMap::from([(row.id.clone(), row)]);
    comments::save(log, &BTreeMap::new(), &one).map(|_| ())
}

/// `POST /compass`: his comment on `page`, with its quote, one new row. Its id.
pub fn post(
    dir: &Path,
    page: &str,
    quote: &str,
    text: &str,
    now_ms: i64,
) -> Result<String, String> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let text = text.trim();
    // a backslash is no path of ours: a browser reads « /\\host » as another host (/code-review)
    if !page.starts_with('/') || page.starts_with("//") || page.contains('\\') {
        return Err("a comment's page is a path of this app".into());
    }
    if text.is_empty() || text.len() > TEXT_LIMIT || quote.len() > QUOTE_LIMIT {
        return Err(format!(
            "a comment is 1 to {TEXT_LIMIT} bytes, its quote {QUOTE_LIMIT} at most"
        ));
    }
    let id = format!(
        "compass-{now_ms:x}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let row = comments::posted(&id, page, quote.trim(), text, &iso(now_ms), now_ms);
    put(&mut log(dir)?, row)?;
    Ok(id)
}

/// The open comments on `page`, the compass's and the older app's: a draft's page holds Approve back while one is open.
pub fn open_on(dir: &Path, page: &str) -> Result<usize, String> {
    let rows = comments::load(&log(dir)?)?;
    Ok(comments::queue(&rows)
        .iter()
        .filter(|r| r.page == page)
        .count())
}

/// What the queue's agent is on, one line.
pub fn agent_line(rows: &BTreeMap<String, comments::Row>) -> String {
    let waiting = comments::waiting(rows);
    match comments::working(rows) {
        Some(r) => format!(
            "The comment agent works comment {} on {}: « {} » ({waiting} open).",
            r.id,
            r.page,
            comments::clipped(&r.text)
        ),
        None if waiting > 0 => format!("The comment agent starts on the oldest of {waiting} open."),
        None => "The comment agent is idle: no comment waits.".into(),
    }
}

/// What feeds the comment watcher, for its row on /watchers (task rust-watchers-port: « fed by rust-compass-queue »): the
/// compass's open rows and its agent, and the old app's open comments.
pub fn fed(dir: &Path) -> String {
    match log(dir).and_then(|log| comments::load(&log)) {
        Ok(rows) => {
            let older = (rows.values())
                .filter(|r| r.closed.is_none() && r.from != comments::COMPASS)
                .count();
            format!(
                "the compass on every Rust page ({} open; {}) and the old app's comments ({older} open)",
                comments::waiting(&rows),
                agent_line(&rows)
            )
        }
        Err(e) => format!("the queue did not read: {e}"),
    }
}

/// /agents' line on the queue, from the folder `dash serve` reads.
pub fn agent_now() -> String {
    let read = serve_from_dir().and_then(|dir| comments::load(&log(dir)?));
    read.map_or_else(
        |e| format!("The comment queue: {e}"),
        |rows| agent_line(&rows),
    )
}

/// `GET /compass?page=<path>`: the page's comments, each with its answer or where it stands.
pub fn compass(dir: &Path, page: &str) -> Result<String, String> {
    let rows = comments::load(&log(dir)?)?;
    let notes: Vec<serde_json::Value> = comments::on_page(&rows, page)
        .into_iter()
        .map(|r| {
            let (state, answer) = match (&r.answer, &r.worker) {
                (Some(a), _) => ("answered", a.clone()),
                (None, Some(_)) => ("working", "The agent is on it.".to_string()),
                (None, None) => ("waiting", "Waiting for the agent.".to_string()),
            };
            serde_json::json!({"id": r.id, "quote": r.quote, "text": r.text, "state": state, "answer": answer})
        })
        .collect();
    Ok(serde_json::json!({"notes": notes}).to_string())
}

/// The page at /comments (task rust-agents-comments-tabs; @nha, 2026-11-09: « should be an agents tab, and then a comments
/// queue »): the queue's agent, what the old app's comments add to it, then every open comment oldest first, a row each of
/// the design system's table that opens it where it was left (task rust-comments-table; @nha, 2026-11-09: « there is nothing
/// card-like to comments. they are a queue. a table. »).
pub fn page(dir: &Path) -> Result<dash_surface::Drawing, String> {
    let rows = comments::load(&log(dir)?)?;
    Ok(dash_surface::Drawing::Tree {
        title: "Comments".into(),
        tree: queue_tree(&rows),
    })
}

fn queue_tree(rows: &BTreeMap<String, comments::Row>) -> String {
    use serde_json::json;
    let text = |t: &str| json!({"type": "text", "props": {"text": t}});
    let mut elements = serde_json::Map::new();
    elements.insert("agent".into(), text(&agent_line(rows)));
    let queue = comments::queue(rows);
    let table: Vec<serde_json::Value> = (queue.iter())
        .map(|r| {
            let (state, href) = match (r.from == comments::COMPASS, &r.worker) {
                (true, Some(_)) => ("the agent works it", format!("{}#c-{}", r.page, r.id)),
                (true, None) => ("waits for the agent", format!("{}#c-{}", r.page, r.id)),
                (false, _) => ("the orchestrator's, from the old app", r.link.clone()),
            };
            // to the minute: « 2026-11-09 10:00 »
            let since = r.at.get(..16).unwrap_or(&r.at).replace('T', " ");
            let cells = [
                comments::clipped(&r.text),
                r.page.clone(),
                state.into(),
                since,
            ];
            json!({"cells": cells, "href": href})
        })
        .collect();
    let list = if table.is_empty() {
        text("No comment waits.")
    } else {
        let columns = ["Comment", "Page", "State", "Since"];
        json!({"type": "table", "props": {"columns": columns, "rows": table}})
    };
    elements.insert("queue".into(), list);
    // what the agent answered, newest first, each with the task it filed (task rust-comment-files-task)
    let mut done = vec![];
    for (i, r) in comments::answered_newest(rows, ANSWERED).iter().enumerate() {
        let since = r.at.get(..16).unwrap_or(&r.at).replace('T', " ");
        let (filed, href) = match &r.task {
            Some(id) => (
                format!("task {id}"),
                format!("/projects/site_perso?issue={id}"),
            ),
            None => (
                "answered, no task".to_string(),
                format!("{}#c-{}", r.page, r.id),
            ),
        };
        let line = format!("{filed}, on {}, asked {since}", r.page);
        let name = format!("answered-{i}");
        let card = json!({"type": "card", "props": {"title": comments::clipped(&r.text), "line": line, "href": href}});
        elements.insert(name.clone(), card);
        done.push(name);
    }
    if !done.is_empty() {
        elements.insert(
            "answered-list".into(),
            json!({"type": "list", "children": done}),
        );
        let part = json!({"type": "section", "props": {"title": "Answered"}, "children": ["answered-list"]});
        elements.insert("answered".into(), part);
    }
    // no heading: the menu's Comments entry names the page (@nha, 2026-11-09: « remove that we have a tab now »)
    let mut children = vec!["agent"];
    // the agent's line counts the compass's rows; the old app's are the orchestrator's, said apart so the two agree
    let older = queue.iter().filter(|r| r.from != comments::COMPASS).count();
    if older > 0 {
        let said = format!("{older} more from the old app, the orchestrator's.");
        elements.insert("older".into(), text(&said));
        children.push("older");
    }
    children.push("queue");
    if !done.is_empty() {
        children.push("answered");
    }
    let page = json!({"type": "stack", "children": children});
    elements.insert("page".into(), page);
    json!({"root": "page", "elements": elements}).to_string()
}

/// The agent's answer to one prompt: Claude Code in print mode, in the repo, its tools those that only read, and Bash for
/// the older app's `task add` and Claude Code's own read-only commands, any other refused unasked (task
/// rust-comment-files-task: 10 answers on 2026-11-09 said « no shell, no dash task add »). `DASH_CLAUDE` names another binary; a stand-in that prints plain words
/// is read as them.
fn asked(prompt: &str) -> Result<String, String> {
    let claude = std::env::var("DASH_CLAUDE").unwrap_or_else(|_| "claude".into());
    let files = format!("Bash({} task add:*)", older_app().bin);
    let args = [
        "--tools",
        "Read,Grep,Glob,Bash",
        "--allowedTools",
        &files,
        "--permission-mode",
        "dontAsk",
        "--max-budget-usd",
        "1",
        "--output-format",
        "json",
        "-p",
        prompt,
    ];
    let out = dash_process::run(&claude, &args, Some(&repo()), WORK)?;
    match dash_agent::headless_outcome(&out) {
        Ok((said, _)) => Ok(said),
        // Claude's JSON with no result (an error, a budget stop) is no answer, never posted as one (/code-review)
        Err(e) if serde_json::from_str::<serde_json::Value>(out.trim()).is_ok() => {
            Err(format!("Claude answered no result: {e}"))
        }
        Err(_) => Ok(out),
    }
}

/// One comment worked: the oldest open compass row (or the one taken before a restart) taken, asked, answered, closed. A
/// run that fails gives the row back, still open, and says why: an outage closes none of his comments (/code-review). The
/// row's id, or none waits.
/// ponytail: one agent, one process: a second `dash orch` on the same folder would take the same row too.
pub fn work_once(
    dir: &Path,
    ask: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Option<String>, String> {
    let mut log = log(dir)?;
    let rows = comments::load(&log)?;
    let Some(row) = comments::next(&rows) else {
        return Ok(None);
    };
    let row = comments::took(row, AGENT, now_ms());
    put(&mut log, row.clone())?;
    let answer = ask(&comments::prompt(&row, &older_app().bin)).and_then(|a| {
        Some(a)
            .filter(|a| !a.trim().is_empty())
            .ok_or_else(|| "the agent said nothing".to_string())
    });
    match answer {
        Ok(answer) => {
            // the task its answer files, once the older app's list holds it: a name made up shows as no task. A list
            // that cannot be read keeps the name, said: the prompt writes it only after task add said so (/code-review)
            let task = comments::task_named(&answer).filter(|id| {
                dash_store::older::filed(&older_db(), id).unwrap_or_else(|e| {
                    eprintln!("comment: {}: task {id} kept unchecked: {e}", row.id);
                    true
                })
            });
            let row = comments::Row {
                task,
                ..comments::closed_with(&row, &answer)
            };
            put(&mut log, row)?;
        }
        Err(e) => {
            put(&mut log, comments::given_back(&row))?;
            return Err(format!("{}: {e}", row.id));
        }
    }
    Ok(Some(row.id))
}

/// `dash comment work --data <dir>`: one comment worked, its id said.
pub fn work_one(dir: &Path, mut out: File) {
    match work_once(dir, &asked) {
        Ok(Some(id)) => writeln!(out, "answered {id}").expect("stdout"),
        Ok(None) => writeln!(out, "no comment waits").expect("stdout"),
        Err(e) => {
            eprintln!("comment: {e}");
            std::process::exit(1);
        }
    }
}

/// The one agent, for as long as `dash orch` runs: a comment after another, oldest first, a look every `POLL_MS` when none
/// waits, and `AGAIN_MS` after a run that failed, so an outage costs two tries an hour, never one a pass.
pub fn work(dir: PathBuf) {
    std::thread::spawn(move || {
        loop {
            let ran = std::panic::catch_unwind(|| work_once(&dir, &asked));
            let wait = match ran {
                Ok(Ok(Some(id))) => {
                    eprintln!("comment agent: answered {id}");
                    continue;
                }
                Ok(Ok(None)) => comments::POLL_MS,
                Ok(Err(e)) => {
                    eprintln!("comment agent: {e}; the next try in 30 minutes");
                    comments::AGAIN_MS
                }
                Err(_) => {
                    eprintln!("comment agent: a run panicked; the next try in 30 minutes");
                    comments::AGAIN_MS
                }
            };
            std::thread::sleep(Duration::from_millis(wait as u64));
        }
    });
}

fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("comments.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(comments::SCHEMA)
        .map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// The orchestrator's wake file: the owner's, while its supervisor lives. None, and the wake waits for the next pass.
/// ponytail: the owner only; `Attention.wake_file`'s fallback to the newest live session if the owner is often unset.
pub(crate) fn wake_file(run: &Path) -> Option<PathBuf> {
    let owner = std::fs::read_to_string(run.join("owner")).ok()?;
    let pid: i32 = owner.trim().parse().ok().filter(|p| *p > 0)?;
    let alive = run.join(format!("{pid}.state")).exists() && unsafe { libc::kill(pid, 0) } == 0;
    alive.then(|| run.join(format!("{pid}.wake")))
}

/// One pass: read his comments, land and close rows, wake on what is due, keep it. A line saying what it did.
pub fn pass(dir: &Path, now_ms: i64) -> Result<String, String> {
    let mut log = log(dir)?;
    let kept = comments::load(&log)?;
    let cli = older_app().bin;
    let text = dash_process::run(&cli, &["store", "text", "live/comments.edn"], None, READ)?;
    let export = serde_json::from_str(&text).map_err(|e| format!("the comments: {e}"))?;
    let rows = comments::step(&kept, &comments::comments(&export)?, now_ms);
    let open = (rows.values())
        .filter(|r| r.closed.is_none() && r.from != comments::COMPASS)
        .count();
    let due = comments::due(&rows, now_ms);
    // read before this pass's own beat, so a queue that was stopped says so in its first wake back
    let line = (!due.is_empty()).then(|| match watchers::stopped(dir, now_ms).as_slice() {
        [] => comments::wake(&due),
        s => format!(
            "{} Watchers that stopped: {}.",
            comments::wake(&due),
            s.join("; ")
        ),
    });
    let due: Vec<String> = due.iter().map(|r| r.id.clone()).collect();
    let file = line
        .as_ref()
        .and_then(|_| wake_file(&repo().join(".claude/run")));
    let (Some(line), Some(file)) = (line, file) else {
        let written = comments::save(&mut log, &kept, &rows)?;
        let held = if due.is_empty() {
            String::new()
        } else {
            format!(", {} due and no live owner to wake", due.len())
        };
        return Ok(format!("{open} open, {written} written{held}"));
    };
    // Kept as woken before the wake is written: a save that fails then writes no wake, so a failing store never repeats
    // one every pass. A wake that fails to write puts the rows back unwoken, for the next pass.
    let mut woken = rows.clone();
    comments::told(&mut woken, &due, now_ms);
    let written = comments::save(&mut log, &kept, &woken)?;
    let said = |e: std::io::Error| format!("{}: {e}", file.display());
    let wrote = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = wrote {
        comments::save(&mut log, &woken, &rows)?;
        return Err(said(e));
    }
    // /watchers' « what it woke »: a failed record costs that line only, never the wake.
    let what = format!(
        "the orchestrator ({}) with {}",
        file.display(),
        due.join(" ")
    );
    if let Err(e) = watchers::woke(dir, &watchers::COMMENTS, now_ms, &what) {
        eprintln!("comment: the wake's record: {e}");
    }
    Ok(format!(
        "{open} open, {written} written, woke {} with {}",
        file.display(),
        due.join(" ")
    ))
}

/// A pass and its beat on /watchers, whatever it said.
fn beating(dir: &Path) -> Result<String, String> {
    let ran = pass(dir, now_ms());
    let said = ran
        .as_ref()
        .map_or_else(|e| format!("comment: {e}"), String::clone);
    // stamped at its end, so a slow pass is not read LATE; a failed one is NOT OK on /watchers
    let ok = ran.is_ok();
    if let Err(e) = watchers::beat(dir, &watchers::COMMENTS, now_ms(), ok, &said) {
        eprintln!("comment: the beat: {e}");
    }
    ran
}

pub fn once(dir: &Path, mut out: File) {
    match beating(dir) {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("comment: {e}");
            std::process::exit(1);
        }
    }
}

/// A pass every `POLL_MS`, for as long as the process runs; a line on stderr each time what it says changes.
pub fn watch(dir: PathBuf) {
    std::thread::spawn(move || {
        let mut said = String::new();
        loop {
            // A pass that panics is said and the next one runs: the queue never stops while orch runs.
            let ran = std::panic::catch_unwind(|| beating(&dir));
            let line = match ran {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => format!("comment: {e}"),
                Err(_) => "comment: a pass panicked, the next one runs".to_string(),
            };
            if line != said {
                eprintln!("comment queue: {line}");
                said = line;
            }
            std::thread::sleep(Duration::from_millis(comments::POLL_MS as u64));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// /code-review of rust-agents-comments-tabs: the agent's count and the list agree, an old app's row says whose it is,
    /// and a long comment is cut in its cell. rust-comments-table: a table's rows, oldest first, in both forms.
    #[test]
    fn the_queue_page_says_the_old_app_s_rows_apart_and_cuts_a_long_comment() {
        let _budget = TestBudget::new();
        let long = "word ".repeat(200);
        let ours = comments::posted("a", "/deps", "", &long, "2026-11-09T10:00:00Z", 1);
        let older = comments::Row {
            from: "older".into(),
            link: "/tasks/x".into(),
            ..comments::posted("b", "/x", "", "theirs", "2026-11-09T11:00:00Z", 2)
        };
        let rows = BTreeMap::from([("b".to_string(), older), ("a".to_string(), ours)]);
        let tree = queue_tree(&rows);
        let text = dash_surface::text(&tree).expect("its text");
        let lines: Vec<&str> = text.lines().collect();
        // no heading: the menu's entry names the page (task rust-comments-no-title)
        assert!(!text.starts_with("Comments\n"), "{text}");
        assert_eq!(
            lines[0],
            "The comment agent starts on the oldest of 1 open."
        );
        assert_eq!(lines[1], "1 more from the old app, the orchestrator's.");
        assert_eq!(lines[2], "Comment\tPage\tState\tSince\tLink");
        let row: Vec<&str> = lines[3].split('\t').collect();
        assert!(
            row[0].ends_with('…') && row[0].chars().count() == 301,
            "{}",
            row[0]
        );
        assert_eq!(
            row[1..],
            [
                "/deps",
                "waits for the agent",
                "2026-11-09 10:00",
                "/deps#c-a"
            ]
        );
        assert_eq!(
            lines[4],
            "theirs\t/x\tthe orchestrator's, from the old app\t2026-11-09 11:00\t/tasks/x"
        );
        let html = dash_surface::screen::page("Comments", &tree).expect("its HTML");
        let (a, b) = (
            html.find(r#"href="/deps#c-a">"#),
            html.find(r#"href="/tasks/x">theirs"#),
        );
        assert!(a.is_some() && a < b, "oldest first, each its link: {html}");
        assert!(html.contains(r#"class="table" id="queue""#), "{html}");
    }
}
