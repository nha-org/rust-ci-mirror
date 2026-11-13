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
use dash_comments::watch;
use dash_store::oplog::Replica;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const READ: Duration = Duration::from_millis(comments::READ_MS as u64);
/// The queue's one agent, as /agents and the compass name it.
pub const AGENT: &str = "comment-agent";
/// One comment's run at most.
const WORK: Duration = Duration::from_secs(15 * 60);
/// A comment's words and its quote at most: a post past them is refused, never cut.
const TEXT_LIMIT: usize = 8000;
const QUOTE_LIMIT: usize = 2000;

/// A day and time in UTC, as the older app's `at` reads: « 2026-11-09T10:00:00Z ».
pub(crate) fn iso(now_ms: i64) -> String {
    dash_core::iso(now_ms.div_euclid(1000))
}

/// One row written alone.
fn put(log: &mut Replica, row: comments::Row) -> Result<(), String> {
    let one = BTreeMap::from([(row.id.clone(), row)]);
    comments::save(log, &BTreeMap::new(), &one).map(|_| ())
}

/// Where a comment's files are kept: `<data>/comment-files/<comment id>/`.
const FILES: &str = "comment-files";
/// Where a comment's file opens: `/compass/files/<comment id>/<name>`.
const FILES_AT: &str = "/compass/files/";
/// A comment's files at most, and each one's size: together within the body `POST /compass` reads (main.rs's
/// COMPASS_FILES_BODY), so each limit is the one that refuses.
pub const FILES_MOST: usize = 5;
pub const FILE_MOST: usize = 5 * 1024 * 1024;

/// A file's MIME type, by its extension, when a comment may attach it (dash_surface::documents::ATTACHABLE_KINDS): else none.
fn attachable(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let kinds = dash_surface::documents::ATTACHABLE_KINDS;
    kinds.iter().find(|(e, _)| *e == ext).map(|(_, kind)| *kind)
}

/// A name a file is kept under: letters, digits, dot, dash and underscore, never starting with a dot, so it is one entry of its
/// comment's folder and nothing else; its place first, so two files of the same name are both kept.
fn kept_name(i: usize, name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let safe: String = (base.chars())
        .map(
            |c| match c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                true => c,
                false => '_',
            },
        )
        .collect();
    format!("{i}-{}", safe.trim_start_matches('.'))
}

/// `POST /compass`: his comment on `page`, with its quote and the images and PDFs he attached, each a name and its bytes, one
/// new row. Its id. A file of any other kind, past FILE_MOST or past FILES_MOST of them refuses the whole comment: nothing lands.
pub fn post(
    dir: &Path,
    page: &str,
    quote: &str,
    text: &str,
    files: &[(String, &[u8])],
    now_ms: i64,
) -> Result<String, String> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let text = text.trim();
    // a backslash is no path of ours: a browser reads « /\\host » as another host (/code-review); a control character
    // neither: the post's 303 names the page in its Location header (task rust-task-brief-linear)
    if !page.starts_with('/')
        || page.starts_with("//")
        || page.contains('\\')
        || page.contains(char::is_control)
    {
        return Err("a comment's page is a path of this app".into());
    }
    if text.is_empty() || text.len() > TEXT_LIMIT || quote.len() > QUOTE_LIMIT {
        return Err(format!(
            "a comment is 1 to {TEXT_LIMIT} bytes, its quote {QUOTE_LIMIT} at most"
        ));
    }
    // judged on the name it is kept under, the one it is served by: « .png » is kept as « 0-png », no image (/code-review)
    let kept: Vec<String> = (files.iter().enumerate())
        .map(|(i, (name, _))| kept_name(i, name))
        .collect();
    let refused = files.len() > FILES_MOST
        || kept.iter().any(|name| attachable(name).is_none())
        || files.iter().any(|(_, bytes)| bytes.len() > FILE_MOST);
    if refused {
        return Err(format!(
            "a comment attaches {FILES_MOST} files at most, each an image or a PDF ({}) of {} MB at most",
            dash_surface::documents::ATTACHABLE,
            FILE_MOST / 1024 / 1024
        ));
    }
    let id = format!(
        "compass-{now_ms:x}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    // the files first: a row never names a file that is not kept, and a comment that does not land keeps none (/code-review)
    let folder = dir.join(FILES).join(&id);
    let landed = (|| {
        if !files.is_empty() {
            std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        }
        for (name, (_, bytes)) in kept.iter().zip(files) {
            std::fs::write(folder.join(name), bytes).map_err(|e| e.to_string())?;
        }
        let row = comments::Row {
            files: kept.clone(),
            ..comments::posted(&id, page, quote.trim(), text, &iso(now_ms), now_ms)
        };
        put(&mut log(dir)?, row)
    })();
    if let Err(e) = landed {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(e);
    }
    Ok(id)
}

/// A comment's file, as the rail links it: its name shown (its place dropped), where it opens, its MIME type.
fn file_of(id: &str, kept: &str) -> (String, String, String) {
    let shown = kept.split_once('-').map_or(kept, |(_, n)| n);
    let kind = attachable(kept).unwrap_or("application/octet-stream");
    (shown.into(), format!("{FILES_AT}{id}/{kept}"), kind.into())
}

/// `GET /compass/files/<id>/<name>`: the file on disk and its MIME type, when the request names one a comment keeps. Each
/// segment is one entry of the folder (kept_name's letters), so no request reads outside it.
pub fn attached(dir: &Path, line: &str) -> Option<(PathBuf, &'static str)> {
    let mut words = line.split_whitespace();
    (words.next() == Some("GET")).then_some(())?;
    let path = words.next()?.split('?').next()?;
    let (id, name) = path.strip_prefix(FILES_AT)?.split_once('/')?;
    let one = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    (one(id) && one(name)).then_some(())?;
    let file = dir.join(FILES).join(id).join(name);
    let kind = attachable(name)?;
    file.is_file().then_some((file, kind))
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
                .filter(|r| r.closed.is_none() && r.from == comments::OLDER)
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
    let read = crate::pages::serve_from_dir().and_then(|dir| comments::load(&log(dir)?));
    read.map_or_else(
        |e| format!("The comment queue: {e}"),
        |rows| agent_line(&rows),
    )
}

/// A page's comments as the rail draws them (task rust-comment-answer-on-page): each a thread anchored `c-<id>`, the passage
/// quoted, his words, then the agent's answer or where it stands. None when the queue cannot be read: the page still serves.
pub fn threads_on(page: &str) -> Vec<dash_surface::documents::Thread> {
    use dash_surface::documents::{Said, Thread};
    // a read never makes the store: a folder with no queue has no comments (opening it would create comments.sqlite)
    let read =
        crate::pages::serve_from_dir().and_then(|dir| match dir.join("comments.sqlite").exists() {
            true => comments::load(&log(dir)?).map(Some),
            false => Ok(None),
        });
    let Ok(Some(rows)) = read else {
        return vec![];
    };
    let when = |at: &str| at.get(..16).unwrap_or(at).replace('T', " ");
    // the older app's threads moved here (`dash comment move`), each whole, before the ones written since
    let moved = (crate::pages::serve_from_dir()
        .and_then(|dir| comments::threads::load(&log(dir)?)))
    .unwrap_or_default();
    let who = |by: &str| match by {
        "nicolas" => "Nicolas".to_string(),
        "claude" => "Claude".to_string(),
        other => other.to_string(),
    };
    let older = comments::threads::on_page(&moved, page)
        .into_iter()
        .map(|t| Thread {
            id: format!("c-{}", t.id),
            quote: t.quote.clone(),
            said: (t.said.iter())
                .map(|s| Said {
                    who: who(&s.by),
                    when: when(&s.on),
                    words: s.text.clone(),
                })
                .collect(),
            open: t.open,
            files: vec![],
        });
    let new = comments::on_page(&rows, page).into_iter().map(|r| {
        let answer = match (&r.answer, &r.worker) {
            (Some(a), _) => a.clone(),
            (None, Some(_)) => "The agent is on it.".to_string(),
            (None, None) => "Waiting for the agent.".to_string(),
        };
        let said = |who: &str, words: String, at: &str| Said {
            who: who.into(),
            when: when(at),
            words,
        };
        Thread {
            id: format!("c-{}", r.id),
            quote: r.quote.clone(),
            said: vec![
                said("Nicolas", r.text.clone(), &r.at),
                said("Comment agent", answer, ""),
            ],
            open: r.closed.is_none(),
            files: r.files.iter().map(|f| file_of(&r.id, f)).collect(),
        }
    });
    older.chain(new).collect()
}

/// `dash comment move --data <dir> [--from <export>]`: every thread of the older app's comments, resolved ones too, moved
/// into this folder's store (task editor-in-rfcs, done-when 2), the export read through its CLI unless a file is named. A
/// line saying how many, on how many pages.
pub fn move_threads(dir: &Path, from: Option<&Path>, mut out: File) {
    let moved = (|| {
        let text = match from {
            Some(f) => std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?,
            None => dash_process::run(
                &older_app().bin,
                &["store", "text", "live/comments.edn"],
                None,
                READ,
            )?,
        };
        move_from(dir, &text)
    })();
    match moved {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("dash comment move: {e}");
            std::process::exit(1);
        }
    }
}

/// The older app's export, as `dash store text live/comments.edn` prints it, moved into `dir`'s store: a line saying how many.
pub(crate) fn move_from(dir: &Path, text: &str) -> Result<String, String> {
    let export = serde_json::from_str(text).map_err(|e| format!("the comments: {e}"))?;
    let threads = comments::threads::moved(&export)?;
    let pages = (threads.iter())
        .map(|t| t.page.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mut log = log(dir)?;
    let kept = comments::threads::load(&log)?;
    let n = threads.len();
    let written = comments::threads::save(&mut log, &kept, threads)?;
    Ok(format!(
        "moved {n} threads on {pages} pages, {written} written"
    ))
}

/// The rail's threads for an agent: a heading, then a record a comment, his words and the answer a tab apart; nothing when none.
pub fn threads_text(threads: &[dash_surface::documents::Thread]) -> String {
    if threads.is_empty() {
        return String::new();
    }
    let mut out = format!("\nComments on this page ({})\n", threads.len());
    for t in threads {
        // his words and the answer, then each file he attached, where it opens (task rust-comment-files-filter)
        let words: Vec<String> = (t.said.iter())
            .map(|s| s.words.replace(['\n', '\t'], " "))
            .chain(t.files.iter().map(|(_, href, _)| href.clone()))
            .collect();
        let quote = match t.quote.as_str() {
            "" => "Whole page".to_string(),
            q => q.replace(['\n', '\t'], " "),
        };
        out += &format!("{}\t{quote}\t{}\n", t.id, words.join("\t"));
    }
    out
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
            let files: Vec<String> = r.files.iter().map(|f| file_of(&r.id, f).1).collect();
            serde_json::json!({"id": r.id, "quote": r.quote, "text": r.text, "state": state, "answer": answer, "files": files})
        })
        .collect();
    Ok(serde_json::json!({"notes": notes}).to_string())
}

/// The page at /comments (task rust-agents-comments-tabs; @nha, 2026-11-09: « should be an agents tab, and then a comments
/// queue »): the queue's agent, what the old app's comments add to it, then every open comment oldest first, a row each of
/// the design system's table that opens it where it was left (task rust-comments-table; @nha, 2026-11-09: « there is nothing
/// card-like to comments. they are a queue. a table. »).
pub fn page(dir: &Path) -> Result<dash_surface::Drawing, String> {
    // the page's store reads, under the request's root span (task rust-dd-spans): a slow /comments says which read waited
    let read = dash_statsd::current().map(|parent| parent.child("store.read", "comments"));
    let loaded = log(dir).and_then(|log| comments::load(&log));
    // each task's place: the older app's list, where the comment agent files it, its stage and roadmap horizon (task
    // comments-board-pill), then the tasks Rust's store finished, merged or ended, closed too. A list that fails to read leaves
    // the cards under Task created, never a page that fails.
    // ponytail: the older app's file read on each load, WAL so never waiting on its writer; gone with it once tasks are filed here
    let ids: Vec<String> = loaded.as_ref().map_or(vec![], |rows| {
        rows.values().filter_map(|r| r.task.clone()).collect()
    });
    let mut places: Places = dash_store::older::places(&older_db(), &ids)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let finished = dash_store::tasks::open(dir, dir).and_then(|t| t.rows());
    for r in finished.unwrap_or_default() {
        if r.status == "finished" {
            places.entry(r.id).or_default().closed = true;
        }
    }
    if let Some(span) = read {
        span.finish();
    }
    let rows = loaded?;
    Ok(dash_surface::Drawing::Tree {
        title: "Comments".into(),
        tree: queue_tree(&rows, &places),
    })
}

/// His comments as a board, as Linear's (task comments-by-state; @nha, 2026-11-09: « Comments should be: backlog (unsorted),
/// answered, task created (+ pill now/next/later + link => this is a perview view of the task, not a separate component), done
/// (again same as task created a view of the task) => start with the design system And this CAN become a board of comments if we
/// want to, similar to the linear board. Again, reuse reuse reuse. »): the tasks board's columns and cards, every comment on it,
/// none dropped (« whatever you end up doing don't drop comments »). A filed comment is its task's card.
type Places = std::collections::HashMap<String, dash_store::older::Place>;

fn queue_tree(rows: &BTreeMap<String, comments::Row>, places: &Places) -> String {
    use serde_json::json;
    let text = |t: &str| json!({"type": "text", "props": {"text": t}});
    let since = |at: &str| at.get(..16).unwrap_or(at).replace('T', " ");
    let mut elements = serde_json::Map::new();
    elements.insert("agent".into(), text(&agent_line(rows)));
    // (column key, its status for the icon, its name, its cards)
    let mut columns: [(&str, &str, &str, Vec<String>); 4] = [
        ("backlog", "backlog", "Backlog", vec![]),
        ("answered", "in-review", "Answered", vec![]),
        ("created", "todo", "Task created", vec![]),
        ("done", "done", "Done", vec![]),
    ];
    let mut card = |column: usize, name: String, row: serde_json::Value| {
        elements.insert(name.clone(), row);
        columns[column].3.push(name);
    };
    let queue = comments::queue(rows);
    for (i, r) in queue.iter().enumerate() {
        let (state, href) = match (r.from.as_str(), &r.worker) {
            (comments::COMPASS, Some(_)) => {
                ("the agent works it", format!("{}#c-{}", r.page, r.id))
            }
            (comments::COMPASS, None) => ("waits for the agent", format!("{}#c-{}", r.page, r.id)),
            (comments::CHAT, _) => ("his question in chat, the orchestrator's", r.link.clone()),
            (comments::PANE, _) => ("a task's question, the question pass's", r.link.clone()),
            _ => ("the orchestrator's, from the old app", r.link.clone()),
        };
        let row = json!({"type": "issue-row", "props": {
            "id": r.page, "title": comments::clipped(&r.text), "status": "backlog",
            "says": format!("{state}, {}", since(&r.at)), "comments": 0, "href": href,
        }});
        card(0, format!("queued-{i}"), row);
    }
    for (i, r) in comments::answered_newest(rows, comments::COMPASS, usize::MAX)
        .iter()
        .enumerate()
    {
        let says = format!("on {}, {}", r.page, since(&r.at));
        let (column, row) = match &r.task {
            // its task's card: the task's key, his words, the task's issue page (the comment agent files in the older app), its
            // roadmap bucket the card's label (@nha, 2026-11-09: « task created (+ pill now/next/later + link »)
            Some(id) => {
                let place = places.get(id).cloned().unwrap_or_default();
                let status = if place.closed { "done" } else { "todo" };
                let row = json!({"type": "issue-row", "props": {
                    "id": id, "title": comments::clipped(&r.text), "status": status, "says": says, "comments": 0,
                    "project": "site_perso", "labels": place.horizon.into_iter().collect::<Vec<_>>(),
                }});
                (if place.closed { 3 } else { 2 }, row)
            }
            // the agent's answer under his words, clipped as they are: the page's thread holds it whole (task
            // comments-answered-shows-answer; @nha, 2026-11-10: « I have no way to see the answers. »)
            None => {
                let row = json!({"type": "issue-row", "props": {
                    "id": r.page, "title": comments::clipped(&r.text), "status": "in-review", "says": says,
                    "comments": 0, "href": format!("{}#c-{}", r.page, r.id), "answer": comments::clipped(r.answer.as_deref().unwrap_or_default()),
                }});
                (1, row)
            }
        };
        card(column, format!("answered-{i}"), row);
    }
    let mut groups = vec![];
    for (key, status, title, names) in columns {
        let group = json!({"type": "issue-group", "props": {"status": status, "title": title}, "children": names});
        elements.insert(key.into(), group);
        groups.push(key);
    }
    elements.insert(
        "board".into(),
        json!({"type": "board", "props": {}, "children": groups}),
    );
    // no heading: the menu's Comments entry names the page (@nha, 2026-11-09: « remove that we have a tab now »)
    let mut children = vec!["agent"];
    // the agent's line counts the compass's rows; the old app's are the orchestrator's, said apart so the two agree
    let older = queue.iter().filter(|r| r.from == comments::OLDER).count();
    if older > 0 {
        let said = format!("{older} more from the old app, the orchestrator's.");
        elements.insert("older".into(), text(&said));
        children.push("older");
    }
    children.push("board");
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
    let mut prompt = comments::prompt(&row, &older_app().bin);
    // a comment on a document's words: the agent rewrites that section, Rust applies it (task rust-doc-cowrite)
    let section = crate::doc_store::section(dir, &repo(), &row.page, &row.quote);
    if let Some(s) = &section {
        prompt += &s.asked();
    }
    // what he attached, where the agent's Read opens it: a picture or PDF is half his comment
    let folder = dir.join(FILES).join(&row.id);
    let folder = folder.canonicalize().unwrap_or(folder);
    let files: Vec<String> = (row.files.iter())
        .map(|f| folder.join(f).display().to_string())
        .collect();
    if !files.is_empty() {
        prompt += &format!(" He attached, read each first: {}.", files.join(", "));
    }
    let answer = ask(&prompt).and_then(|a| {
        Some(a)
            .filter(|a| !a.trim().is_empty())
            .ok_or_else(|| "the agent said nothing".to_string())
    });
    match answer {
        Ok(answer) => {
            let answer = match &section {
                Some(s) => s.applied(dir, &repo(), &answer),
                None => answer,
            };
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
    crate::watch::log(dir)
}

/// The orchestrator's wake file: the owner's, while its supervisor lives. None, and the wake waits for the next pass.
/// ponytail: the owner only; `Attention.wake_file`'s fallback to the newest live session if the owner is often unset.
pub(crate) fn wake_file(run: &Path) -> Option<PathBuf> {
    let owner = std::fs::read_to_string(run.join("owner")).ok()?;
    let pid: i32 = owner.trim().parse().ok().filter(|p| *p > 0)?;
    let alive = run.join(format!("{pid}.state")).exists() && unsafe { libc::kill(pid, 0) } == 0;
    alive.then(|| run.join(format!("{pid}.wake")))
}

/// The panes asking a question, each a task's, as backlog rows (`dash_comments::panes`): every tab's question, options and the
/// pick line; and every pane asking, read or not. None when herdr cannot be read, so no question closes on a failed look. A pane of no task (the
/// orchestrator's, his own) is left out.
/// ponytail: herdr's list only, as `dash pane say` reads it; hc's when DASH_TERMINAL=hc runs the tasks.
fn questions(dir: &Path, now_ms: i64) -> Option<(Vec<comments::Row>, Vec<String>)> {
    let herdr = dash_terminal::Herdr::from_env();
    let asking = herdr
        .asking()
        .map_err(|e| eprintln!("comment: herdr: {e}"))
        .ok()?;
    let panes: Vec<String> = asking.iter().map(|(p, _)| p.clone()).collect();
    if asking.is_empty() {
        return Some((vec![], panes));
    }
    let mut tasks: BTreeMap<String, String> = dash_store::older::claimed_panes(&older_db())
        .unwrap_or_else(|e| {
            eprintln!("comment: the older claims: {e}");
            vec![]
        })
        .into_iter()
        .collect();
    let rust = crate::tasks::open_tasks(dir).and_then(|t| dash_core::tasks::TaskStore::claims(&t));
    match rust {
        Ok(claims) => tasks.extend(claims.into_iter().filter_map(|c| Some((c.pane?, c.id)))),
        Err(e) => eprintln!("comment: Rust's claims: {e}"),
    }
    let rows = shown(&herdr, &asking, &tasks, &crate::watch::projects(), now_ms);
    Some((rows, panes))
}

/// The questions `asking` panes show, those of a task in `tasks` (pane to task id).
fn shown(
    terminal: &dyn dash_core::tasks::Terminal,
    asking: &[(String, Option<String>)],
    tasks: &BTreeMap<String, String>,
    projects: &Path,
    now_ms: i64,
) -> Vec<comments::Row> {
    use dash_terminal::dialog;
    (asking.iter())
        .filter_map(|(pane, session)| {
            let task = tasks.get(pane)?;
            // herdr's list said it asks, its session with it: no second lookup
            let screen = terminal.read(pane).ok()?;
            let on_screen = dialog::parse(&screen)?;
            let transcript = session
                .as_deref()
                .and_then(|s| dialog::transcript(projects, s));
            let d = dialog::from(pane, &on_screen, transcript);
            // the question on screen in the key, never the words: a transcript read once and missed the next pass changes
            // the words, never which question shows (/code-review)
            let key = dash_comments::panes::key(pane, &on_screen.question);
            let n = d.questions.len();
            let title = format!(
                "task {task}'s pane {pane} waits on {n} question{}",
                if n == 1 { "" } else { "s" }
            );
            Some(dash_comments::panes::asked(
                &key,
                task,
                &title,
                &dialog::words(&d),
                now_ms,
            ))
        })
        .collect()
}

/// The wake's line: his comments' words, then the watcher's rows (task rust-watch-port), one line, one wake.
fn words(due: &[&comments::Row], items: &[&watch::Item]) -> String {
    let mut parts = vec![];
    if !due.is_empty() {
        parts.push(comments::wake(due));
    }
    if !items.is_empty() {
        parts.push(format!(
            "Woken by the dashboard's Rust watcher ({}): {}",
            items.len(),
            watch::wake(items, 1)
        ));
    }
    parts.join(" ")
}

/// One pass: read his comments and the panes' questions, land and close rows, wake on what is due, keep it. A pane's question
/// is a backlog row the inbox's pass rules after this one (`crate::inbox`), never a wake. A line saying what
/// it did.
pub fn pass(dir: &Path, now_ms: i64) -> Result<String, String> {
    let mut log = log(dir)?;
    let kept = comments::load(&log)?;
    let cli = older_app().bin;
    let text = dash_process::run(&cli, &["store", "text", "live/comments.edn"], None, READ)?;
    let export = serde_json::from_str(&text).map_err(|e| format!("the comments: {e}"))?;
    let mut rows = comments::step(&kept, &comments::comments(&export)?, now_ms);
    if let Some((shown, asking)) = questions(dir, now_ms) {
        dash_comments::panes::landed(&mut rows, shown, &asking);
    }
    let items = watch::load(&log)?;
    let open = (rows.values())
        .filter(|r| r.closed.is_none() && r.from == comments::OLDER)
        .count();
    let due = comments::due(&rows, now_ms);
    let due_items = watch::due(&items, now_ms);
    // read before this pass's own beat, so a queue that was stopped says so in its first wake back
    let line = (!due.is_empty() || !due_items.is_empty()).then(|| {
        match watchers::stopped(dir, now_ms).as_slice() {
            [] => words(&due, &due_items),
            s => format!(
                "{} Watchers that stopped: {}.",
                words(&due, &due_items),
                s.join("; ")
            ),
        }
    });
    let due: Vec<String> = due.iter().map(|r| r.id.clone()).collect();
    let due_items: Vec<String> = due_items.iter().map(|i| i.key.clone()).collect();
    let file = line
        .as_ref()
        .and_then(|_| wake_file(&repo().join(".claude/run")));
    let (Some(line), Some(file)) = (line, file) else {
        let written = comments::save(&mut log, &kept, &rows)?;
        let n = due.len() + due_items.len();
        let held = if n == 0 {
            String::new()
        } else {
            format!(", {n} due and no live owner to wake")
        };
        return Ok(format!("{open} open, {written} written{held}"));
    };
    // Kept as woken before the wake is written: a save that fails then writes no wake, so a failing store never repeats
    // one every pass. A wake that fails to write puts the rows back unwoken, for the next pass.
    let mut woken = rows.clone();
    comments::told(&mut woken, &due, now_ms);
    let mut woken_items = items.clone();
    watch::told(&mut woken_items, &due_items, now_ms);
    let written = watch::save_both(&mut log, (&kept, &woken), (&items, &woken_items))?;
    let said = |e: std::io::Error| format!("{}: {e}", file.display());
    let wrote = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = wrote {
        watch::save_both(&mut log, (&woken, &rows), (&woken_items, &items))?;
        return Err(said(e));
    }
    let all: Vec<String> = due.into_iter().chain(due_items).collect();
    // /watchers' « what it woke »: a failed record costs that line only, never the wake.
    let what = format!(
        "the orchestrator ({}) with {}",
        file.display(),
        all.join(" ")
    );
    if let Err(e) = watchers::woke(dir, &watchers::COMMENTS, now_ms, &what) {
        eprintln!("comment: the wake's record: {e}");
    }
    Ok(format!(
        "{open} open, {written} written, woke {} with {}",
        file.display(),
        all.join(" ")
    ))
}

/// A pass and its beat on /watchers, whatever it said.
fn beating(dir: &Path) -> Result<String, String> {
    beat_with(dir, background_passes)
}

/// His inbox's pass and the live check's (task done-checked-live), each on a thread of its own.
fn background_passes(dir: &Path) -> String {
    let inbox = crate::inbox::in_background(dir);
    format!(
        "{inbox}; live check: {}",
        crate::live_check::in_background(dir)
    )
}

/// A pass, his inbox's pass beside it (task rust-inbox-triage): `asked` runs it and says its line, which the beat writes on
/// /watchers, so one that fails is read there and never stops the comments. After the comment pass, so his comment never
/// waits on the inbox's reads; `dash orch`'s beat starts it on a thread of its own, `dash comment pass` waits for it.
fn beat_with(dir: &Path, asked: fn(&Path) -> String) -> Result<String, String> {
    let ran = pass(dir, now_ms());
    let resolved = resolve_older(&older_db(), &repo(), now_ms());
    let asked = asked(dir);
    let said = ran.as_ref().map_or_else(
        |e| format!("comment: {e}; {resolved}"),
        |line| format!("{line}; {resolved}; inbox: {asked}"),
    );
    // stamped at its end, so a slow pass is not read LATE; a failed one is NOT OK on /watchers
    let ok = ran.is_ok();
    if let Err(e) = watchers::beat(dir, &watchers::COMMENTS, now_ms(), ok, &said) {
        eprintln!("comment: the beat: {e}");
    }
    ran
}

/// The commits judged not on main, with the HEAD they were judged against: a sha that never lands (another repository's, a
/// typo) is asked of git once per HEAD, never every pass (/code-review).
static NOT_LANDED: std::sync::Mutex<(String, std::collections::BTreeSet<String>)> =
    std::sync::Mutex::new((String::new(), std::collections::BTreeSet::new()));

/// One git call in `repo`, its stdout trimmed when it exits 0.
fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether commit `sha` is in `repo`'s HEAD, the code the app runs (`Comments.landed?/1`).
fn landed(repo: &Path, sha: &str) -> bool {
    let head = git(repo, &["rev-parse", "HEAD"]).unwrap_or_default();
    let mut memo = NOT_LANDED.lock().unwrap_or_else(|e| e.into_inner());
    if memo.0 != head {
        *memo = (head, Default::default());
    }
    if memo.1.contains(sha) {
        return false;
    }
    let on = git(repo, &["merge-base", "--is-ancestor", sha, "HEAD"]).is_some();
    if !on {
        memo.1.insert(sha.to_string());
    }
    on
}

/// The older app's comments resolved by Rust's pass (task rust-comments-resolve-port), `Comments.resolve_merged/1` and
/// `resolve_landed/1` ported: those of merged tasks, then those whose fix's commit is in `repo`'s HEAD, one read of `db` for
/// both. Each resolve is a line on stderr naming Rust's pass, so the log says who resolved it; what it says is the beat's too.
/// A failed read is said there, never failing the comment pass. ponytail: read every pass (30 s), 16 ms on the live file of
/// 2026-11-12 (860 comments); read less often if /watchers shows the pass slow.
fn resolve_older(db: &Path, repo: &Path, now_ms: i64) -> String {
    let today = crate::audit::today(now_ms);
    let owner = crate::access::owner(repo);
    let is_landed = |sha: &str| landed(repo, sha);
    let ran = dash_store::older_comments::resolve(db, &owner, &today, now_ms, &is_landed);
    let ran = match ran {
        Ok(r) => r,
        Err(e) => {
            eprintln!("comments: Rust's resolve: {e}");
            return format!("older comments: resolve failed: {e}");
        }
    };
    let mut said = vec![];
    for (ran, why) in [
        (ran.merged, "its task merged"),
        (ran.landed, "the fix its answer names is on main"),
    ] {
        match ran {
            Ok(ids) => {
                for r in &ids {
                    eprintln!("comments: {} resolved by Rust's pass, {why}", r.id);
                }
                said.push(format!("{} resolved, {why}", ids.len()));
            }
            Err(e) => {
                eprintln!("comments: Rust's resolve ({why}): {e}");
                said.push(format!("resolve ({why}) failed: {e}"));
            }
        }
    }
    format!("older comments: {}", said.join(", "))
}

/// `dash hook chat`, Claude Code's UserPromptSubmit hook with its JSON on stdin (task rust-chat-questions-tracked): each
/// question in his message lands as a chat row, then every chat row still open is said, so it heads the turn until
/// `dash comment answer` closes it. What it prints goes into the turn's context.
pub fn chat(dir: &Path, input: &str, now_ms: i64) -> Result<String, String> {
    let message: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
    let message = message["prompt"].as_str().unwrap_or_default();
    // an agent's words in his box (`dash pane say`, signed) are not his: none of their questions lands (/code-review)
    let signed = ["From the orchestrator: ", "From the task "]
        .iter()
        .any(|s| message.trim_start().starts_with(s));
    let message = if signed { "" } else { message };
    let mut log = log(dir)?;
    let kept = comments::load(&log)?;
    let mut rows = kept.clone();
    for (i, q) in comments::questions(message).into_iter().enumerate() {
        // the same words still open are one question, asked again
        if comments::open_chat(&rows).iter().any(|r| r.text == q) {
            continue;
        }
        let id = format!("chat-{now_ms:x}-{i}");
        rows.insert(id.clone(), comments::asked(&id, &q, &iso(now_ms), now_ms));
    }
    comments::save(&mut log, &kept, &rows)?;
    Ok(comments::heads(&comments::open_chat(&rows)))
}

/// `dash comment answer <id> <words>` on a chat row: the answer kept, the row closed and shown on /inbox. Any other row is
/// the older app's (its `dash comment answer`) or the compass agent's to answer.
pub fn answer(dir: &Path, id: &str, words: &str, now_ms: i64) -> Result<String, String> {
    let words = words.trim();
    if words.is_empty() {
        return Err("an answer has words".into());
    }
    let mut log = log(dir)?;
    let rows = comments::load(&log)?;
    let row = match rows.get(id) {
        Some(r) if r.from == comments::CHAT => r,
        Some(_) => {
            return Err(format!(
                "{id} is no chat question: the older app's dash comment answer takes it"
            ));
        }
        None => return Err(format!("no comment {id}")),
    };
    let was = row.closed.is_some();
    let done = comments::Row {
        worked_ms: Some(now_ms),
        ..comments::closed_with(row, words)
    };
    put(&mut log, done)?;
    Ok(match was {
        true => format!("{id}: answered again, on /inbox"),
        false => format!("{id}: answered, closed, on /inbox"),
    })
}

/// `dash comment note`: a thread Claude opens on `page`, about `quote` (empty: the page as a whole), open until he answers or
/// resolves it: a choice made for him said where he reads it (task design-doc-tree-menu; the orchestrator, 2026-11-12, from
/// him: « make the layout choice visible where it is read »). Its id. No agent is woken: the words are Claude's already.
pub fn note(
    dir: &Path,
    page: &str,
    quote: &str,
    words: &str,
    now_ms: i64,
) -> Result<String, String> {
    let words = words.trim();
    if words.is_empty() || !page.starts_with('/') || page.starts_with("//") || page.contains('\\') {
        return Err("a note has words, on a path of this app".into());
    }
    let mut log = log(dir)?;
    let kept = dash_comments::threads::load(&log)?;
    // an id no kept thread holds: two notes in one millisecond are two threads (/code-review)
    let id = (0..)
        .map(|n| match n {
            0 => format!("claude-{now_ms:x}"),
            n => format!("claude-{now_ms:x}-{n}"),
        })
        .find(|id| !kept.contains_key(id))
        .expect("a free id");
    let said = dash_comments::threads::Said {
        by: "claude".into(),
        text: words.into(),
        on: iso(now_ms),
    };
    let thread = dash_comments::threads::Thread {
        id: id.clone(),
        page: page.into(),
        quote: quote.trim().into(),
        said: vec![said],
        open: true,
    };
    dash_comments::threads::save(&mut log, &kept, vec![thread])?;
    Ok(id)
}

inventory::submit! { crate::usage::Command::usage("dash comment note --data <dir> <page> <quote> <words>|- (a thread Claude opens on a page)") }
inventory::submit! { crate::usage::Command::usage("dash comment pass|work --data <dir>") }
inventory::submit! { crate::usage::Command::usage("dash comment answer [--data <dir>] <chat id> <words>|- (his chat question answered and closed)") }
inventory::submit! { crate::usage::Command::usage("dash hook chat [--data <dir>] (Claude Code's UserPromptSubmit hook: his questions as chat rows)") }
pub fn once(dir: &Path, mut out: File) {
    match beat_with(dir, crate::inbox::waited) {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("comment: {e}");
            std::process::exit(1);
        }
    }
}

/// A pass every `POLL_MS`, for as long as the process runs; a line on stderr each time what it says changes.
pub fn watch(dir: PathBuf) {
    let sleep = Duration::from_millis(comments::POLL_MS as u64);
    watchers::run(dir, &watchers::COMMENTS, sleep, beating);
}

inventory::submit! {
    crate::pages::Page { path: "/comments", step: "route_comments", reads: crate::pages::Reads::Nothing, redraw: true, serve: |a| a.form(Some(crate::pages::serve_from_dir().and_then(page))) }
}
// /watchers is registered here, not in watchers.rs: it reads this module's served folder and feed, and this module reads
// watchers, so the other way is a module cycle. The server's data folder, never the board's: with no --port there is no
// board, and the page still answers (/code-review of rust-watchers-port, its finding 2, shown by compass.rs' test).
/// His switch on /watchers (`POST /watchers/pause?name=`, task rust-automations): flips `name`, then sends him back to the
/// page, so a reload or a back never posts the switch again.
/// `tab`: the tab it was flipped on, « automations », or none.
pub(crate) fn switched(data: &Path, name: &str, tab: Option<&str>) -> crate::pages::Response {
    match watchers::toggle(data, name, &crate::automations::keys()) {
        Err(e) => ("400 Bad Request", crate::pages::TEXT, e),
        // back to the tab it was flipped on
        Ok(_) if tab == Some("automations") => (
            "303 See Other",
            crate::pages::TEXT,
            watchers::AUTOMATIONS_TAB.into(),
        ),
        Ok(_) => ("303 See Other", crate::pages::TEXT, "/watchers".into()),
    }
}

inventory::submit! {
    crate::pages::Page { path: "/watchers", step: "route_watchers", reads: crate::pages::Reads::Nothing, redraw: true, serve: |a| {
        a.form(Some(crate::pages::serve_from_dir().and_then(|dir| watchers::page(dir, &fed(dir), crate::automations::rows(dir), &a.param("tab").unwrap_or_default()))))
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// A pane whose agent asks the two-tab dialog of `TWO_TABS`, its session's transcript holding both questions.
    struct Asking(String);

    impl dash_core::tasks::Terminal for Asking {
        fn open(&self, _: &str, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn run(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn agent(&self, _: &str) -> Result<Option<dash_core::tasks::Seen>, String> {
            let session = Some("s2".to_string());
            Ok(Some(dash_core::tasks::Seen {
                idle: false,
                asking: true,
                session,
            }))
        }
        fn sessions(&self) -> Result<usize, String> {
            Ok(1)
        }
        fn read(&self, _: &str) -> Result<String, String> {
            Ok(self.0.clone())
        }
        fn send(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!("the watcher types nothing")
        }
        fn key(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!("the watcher types nothing")
        }
        fn close(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
    }

    const TWO_TABS: &str = " ←  ☐ Scope  ☐ Wake  ✔ Submit  →

Which slice ships first?

❯ 1. Dialog reader
     Port the reader first.
  2. The queue
     The batched wake first.
  3. Type something.
──────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel
";

    /// Task rust-watch-port, done-when 1, and rust-questions-orchestrator-first, done-when 1: a task pane blocked on a
    /// two-tab dialog is one backlog row of the comment queue holding every tab's question, its options and the pick line; it
    /// wakes nobody; a pane of no task is left out.
    #[dash_test]
    fn a_pane_blocked_on_two_tabs_is_one_backlog_row_with_each_question_its_options_and_the_pick_line()
     {
        let dir = dash_meter::TestDir::new("watch-two-tabs");
        std::fs::create_dir_all(dir.0.join("-w-a")).expect("a folder");
        let ask = serde_json::json!({"type": "assistant", "message": {"content": [{"type": "tool_use",
            "name": "AskUserQuestion", "id": "toolu_2", "input": {"questions": [
                {"header": "Scope", "question": "Which slice ships first?", "options": [
                    {"label": "Dialog reader", "description": "Port the reader first."},
                    {"label": "The queue", "description": "The batched wake first."}]},
                {"header": "Wake", "question": "Where does the wake go?", "options": [
                    {"label": "The comment wake"}, {"label": "A file of its own"}]}]}}]}});
        std::fs::write(dir.0.join("-w-a/s2.jsonl"), format!("{ask}\n")).expect("a transcript");
        let asking = [
            ("w3:p1".to_string(), Some("s2".to_string())),
            ("w9:his".to_string(), None),
        ];
        let tasks = BTreeMap::from([("w3:p1".to_string(), "rust-x".to_string())]);
        let shown = shown(&Asking(TWO_TABS.into()), &asking, &tasks, &dir.0, 0);
        assert_eq!(shown.len(), 1, "his own pane is no task's");
        let mut rows = BTreeMap::new();
        dash_comments::panes::landed(&mut rows, shown, &["w3:p1".to_string()]);
        let row = rows.values().next().expect("one row");
        assert_eq!(
            (row.from.as_str(), row.task.as_deref()),
            (comments::PANE, Some("rust-x"))
        );
        assert_eq!(row.quote, "task rust-x's pane w3:p1 waits on 2 questions");
        let expected = "Q1 [Scope] Which slice ships first? Options: 1. Dialog reader (Port the reader first.); 2. The queue \
            (The batched wake first.). Q2 [Wake] Where does the wake go? Options: 1. The comment wake; 2. A file of its own. \
            Pick: ~/repo/site_perso/private/dashboard/bin/dash pane pick w3:p1 <n> <n> (one option number per question, or \
            --text '<words>' for its « Type something. »).";
        assert_eq!(row.text, expected);
        assert!(
            comments::due(&rows, 90 * comments::QUIET_MS).is_empty(),
            "no wake"
        );
    }

    /// /code-review of rust-questions-orchestrator-first: a question the pass cannot rule is told to the orchestrator once, as
    /// a block, and stays open for the next try; nothing reaches his list.
    #[dash_test]
    fn a_question_the_pass_cannot_rule_is_told_to_the_orchestrator_once() {
        let dir = dash_meter::TestDir::new("question-unruled");
        let row = dash_comments::panes::asked(
            "question w3:p1 00ab",
            "rust-x",
            "task rust-x's pane w3:p1 waits on 1 question",
            "Q1 … pick w3:p1 <n>",
            0,
        );
        put(&mut log(&dir.0).expect("log"), row).expect("lands");
        let mut asks = BTreeMap::new();
        for now in [60_000, 60_000 + comments::AGAIN_MS] {
            let (ruled, _) = crate::watch::rule_questions(
                &dir.0,
                &mut asks,
                &mut |_| Err("no key".into()),
                3,
                now,
            )
            .expect("a pass");
            assert_eq!(ruled, 0);
        }
        let items = crate::watch::watched(&dir.0).expect("the watcher's rows");
        let blocks: Vec<&watch::Item> = items.values().collect();
        assert_eq!(blocks.len(), 1, "told once");
        assert_eq!(blocks[0].kind, watch::Kind::Block);
        assert_eq!(
            blocks[0].title,
            "the question pass could not answer task rust-x's pane w3:p1 waits on 1 question: no key"
        );
        let rows = comments::load(&log(&dir.0).expect("log")).expect("rows");
        assert_eq!(
            rows["question w3:p1 00ab"].closed, None,
            "open for the next try"
        );
        assert!(asks.is_empty());
    }

    /// /code-review of rust-agents-comments-tabs: the agent's count and the list agree, an old app's row says whose it is,
    /// and a long comment is cut in its cell. rust-comments-table: a table's rows, oldest first, in both forms.
    #[dash_test]
    fn the_queue_page_says_the_old_app_s_rows_apart_and_cuts_a_long_comment() {
        let long = "word ".repeat(200);
        let ours = comments::posted("a", "/deps", "", &long, "2026-11-09T10:00:00Z", 1);
        let older = comments::Row {
            from: "older".into(),
            link: "/tasks/x".into(),
            ..comments::posted("b", "/x", "", "theirs", "2026-11-09T11:00:00Z", 2)
        };
        let rows = BTreeMap::from([("b".to_string(), older), ("a".to_string(), ours)]);
        let tree = queue_tree(&rows, &Default::default());
        let text = dash_surface::text(&tree).expect("its text");
        let lines: Vec<&str> = text.lines().collect();
        // no heading: the menu's entry names the page (task rust-comments-no-title)
        assert!(!text.starts_with("Comments\n"), "{text}");
        assert_eq!(
            lines[0],
            "The comment agent starts on the oldest of 1 open."
        );
        assert_eq!(lines[1], "1 more from the old app, the orchestrator's.");
        // the board's first column (task comments-by-state), every queued comment a card, oldest first: its page, its words,
        // its state, where it opens
        assert_eq!(lines[3], "Backlog (2)");
        let row: Vec<&str> = lines[4].split('\t').collect();
        assert_eq!(row[0], "/deps");
        assert!(
            row[1].ends_with('…') && row[1].chars().count() == 301,
            "{}",
            row[1]
        );
        assert_eq!(
            row[2..],
            ["waits for the agent, 2026-11-09 10:00", "/deps#c-a"]
        );
        assert_eq!(
            lines[5],
            "/x\ttheirs\tthe orchestrator's, from the old app, 2026-11-09 11:00\t/tasks/x"
        );
        let html =
            dash_surface::markup(&dash_surface::screen::page("Comments", &tree).expect("its HTML"));
        let (a, b) = (
            html.find(r#"href="/deps#c-a""#),
            html.find(r#"href="/tasks/x""#),
        );
        assert!(a.is_some() && a < b, "oldest first, each its link: {html}");
        assert!(html.contains(r#"class="board""#), "{html}");
    }

    /// `dash comment note`: Claude's own open thread on a page, beside the ones kept; a page not of this app, or no words, refused.
    #[dash_test]
    fn a_note_is_claude_s_open_thread_on_its_page() {
        let dir = dash_meter::TestDir::new("comment-note");
        let id = note(&dir.0, "/design", "Doc tree", "Layout A picked.", 1_000).expect("noted");
        let threads = dash_comments::threads::load(&log(&dir.0).expect("log")).expect("read");
        let on = dash_comments::threads::on_page(&threads, "/design");
        assert_eq!(on.len(), 1);
        let t = on[0];
        assert_eq!(
            (t.id.as_str(), t.quote.as_str(), t.open),
            (id.as_str(), "Doc tree", true)
        );
        assert_eq!(
            (t.said[0].by.as_str(), t.said[0].text.as_str()),
            ("claude", "Layout A picked.")
        );
        // a second note keeps the first
        note(&dir.0, "/design", "", "Another.", 2_000).expect("noted");
        let threads = dash_comments::threads::load(&log(&dir.0).expect("log")).expect("read");
        assert_eq!(
            dash_comments::threads::on_page(&threads, "/design").len(),
            2
        );
        // the same millisecond again: a third thread, none overwritten (/code-review)
        note(&dir.0, "/design", "", "Same time.", 2_000).expect("noted");
        let threads = dash_comments::threads::load(&log(&dir.0).expect("log")).expect("read");
        assert_eq!(
            dash_comments::threads::on_page(&threads, "/design").len(),
            3
        );
        assert!(
            note(&dir.0, "//evil", "", "x", 3_000).is_err()
                && note(&dir.0, "/x", "", " ", 3_000).is_err()
        );
    }

    /// Task comments-by-state, done-when 1 and 2: one title (the menu's), the comments under Backlog, Answered, Task created and
    /// Done, a filed comment drawn as its task's own row, its status the task's: todo while open, done once Rust finished it or
    /// the older app closed it (task comments-board-pill, done-when 2), its roadmap bucket its label (done-when 1).
    #[dash_test]
    fn comments_stand_under_backlog_answered_task_created_and_done() {
        let mut rows = BTreeMap::new();
        let mut row = |id: &str, answer: Option<&str>, task: Option<&str>| {
            let r = comments::Row {
                answer: answer.map(String::from),
                closed: answer.map(|_| "answered".to_string()),
                task: task.map(String::from),
                worked_ms: Some(1),
                ..comments::posted(
                    id,
                    "/x",
                    "",
                    &format!("said {id}"),
                    "2026-11-09T10:00:00Z",
                    1,
                )
            };
            rows.insert(id.to_string(), r);
        };
        row("waits", None, None);
        row("asked", Some("yes"), None);
        row("filed", Some("filed it"), Some("t-open"));
        row("shipped", Some("filed it"), Some("t-done"));
        row("closed", Some("filed it"), Some("t-older"));
        // a stand-in list (task comments-board-pill): Rust finished t-done, the older app closed t-older, t-open waits for next
        let place = |closed: bool, horizon: &str| dash_store::older::Place {
            closed,
            horizon: Some(horizon.to_string()),
        };
        let places = Places::from([
            ("t-open".to_string(), place(false, "next")),
            ("t-done".to_string(), place(true, "now")),
            ("t-older".to_string(), place(true, "later")),
        ]);
        let tree = queue_tree(&rows, &places);
        let text = dash_surface::text(&tree).expect("its text");
        let at = |s: &str, n: usize| {
            text.find(&format!("\n{s} ({n})\n"))
                .unwrap_or_else(|| panic!("{s} in {text}"))
        };
        // the board's four columns, none dropped; the task the older app closed stands under Done with Rust's finished one
        assert!(
            at("Backlog", 1) < at("Answered", 1)
                && at("Answered", 1) < at("Task created", 1)
                && at("Task created", 1) < at("Done", 2)
        );
        let done = &text[at("Done", 2)..];
        assert!(
            done.contains("t-older") && done.contains("t-done") && !done.contains("t-open"),
            "{text}"
        );
        // each task card's roadmap bucket is the board card's label (the face draws labels as its tags, screen.rs'
        // the_board_s_cards_are_design_s_linear_cards), in the tree and the text form
        for bucket in ["next", "now", "later"] {
            let label = format!(r#""labels":["{bucket}"]"#);
            assert!(tree.contains(&label), "{label} in {tree}");
            assert!(text.contains(&format!("#{bucket}")), "#{bucket} in {text}");
        }
        assert!(!text.contains("Comments\n"), "{text}");
        // task comments-answered-shows-answer: the Answered card shows the agent's answer under his words, in both forms
        let answered = &text[at("Answered", 1)..at("Task created", 1)];
        assert!(answered.contains("said asked\tanswer: yes\t"), "{text}");
        // the tree the HTML draws from: the card's answer prop, which screen.rs' an_issue_row_s_answer_stands_under_his_words
        // draws under the title
        assert!(tree.contains(r#""answer":"yes""#), "{tree}");
        assert!(tree.contains(r#""type":"board""#), "{tree}");
        assert!(
            tree.contains(r#""id":"t-open""#) && tree.contains(r#""status":"todo""#),
            "{tree}"
        );
        assert!(
            tree.contains(r#""id":"t-done""#) && tree.contains(r#""status":"done""#),
            "{tree}"
        );
    }

    /// Task rust-doc-cowrite, done-when 3: his comment on a document's words asking a change makes the comment agent rewrite
    /// that section: its prompt names the section and its passages, its edit is applied to the document (kept, its HTML file
    /// written again), and the answer under his comment names the diff. An edit outside the section changes nothing.
    #[dash_test]
    fn a_comment_asking_a_change_makes_the_agent_edit_that_section() {
        let dir = dash_meter::TestDir::new("comment-cowrite");
        let (repo, data) = (dir.0.join("repo"), dir.0.join("data"));
        std::fs::create_dir_all(repo.join("private/docs")).expect("repo");
        std::fs::create_dir_all(&data).expect("data");
        let file = repo.join("private/docs/rfc-t.html");
        std::fs::write(&file, include_str!("../tests/fixtures/rfc-t.html")).expect("the RFC");
        // SAFETY: nextest runs each test in its own process
        unsafe { std::env::set_var("DASH_REPO", &repo) };
        let (page, quote) = ("/v/private/docs/rfc-t.html", "drops what is settled");
        let section = crate::doc_store::section(&data, &repo, page, quote).expect("its section");
        assert_eq!(section.name, "1. Intent");
        assert!(
            section.fields.iter().all(|(_, w)| w != "2. The page"),
            "up to the next heading of its level"
        );
        let (n, old) = (section.fields.iter())
            .find(|(_, w)| w.contains(quote))
            .cloned()
            .expect("the passage");
        // words holding a fence of their own: the edit is still read whole (/code-review)
        let new = "The check pass drops what is settled: ```a settled row``` asks nothing.";
        let id = post(&data, page, quote, "Say why it drops them.", &[], 1_000).expect("posted");
        let asked = std::cell::RefCell::new(String::new());
        let reply = format!("Said why.\n```edit\n{{\"b{n}\": \"{new}\"}}\n```\n");
        let ask = |p: &str| {
            *asked.borrow_mut() = p.to_string();
            Ok(reply.clone())
        };
        assert_eq!(work_once(&data, &ask).expect("worked"), Some(id.clone()));
        let prompt = asked.borrow().clone();
        assert!(
            prompt.contains("his comment is on its section « 1. Intent »")
                && prompt.contains(&format!("\"b{n}\":\"{old}\"")),
            "{prompt}"
        );
        let written = std::fs::read_to_string(&file).expect("its file");
        assert!(written.contains(new), "the file written again: {written}");
        let rows = comments::load(&log(&data).expect("log")).expect("rows");
        assert_eq!(
            rows[&id].answer.as_deref(),
            Some(
                format!(
                    "Said why.\n\nEdited « 1. Intent » in /v/private/docs/rfc-t.html:\n````diff\n- {old}\n+ {new}\n````"
                )
                .as_str()
            ),
            "the answer names the diff"
        );

        // a passage of another section: refused, nothing written
        let doc = crate::doc_store::stored(&data, "private/docs/rfc-t.html")
            .expect("reads")
            .expect("kept")
            .doc;
        let m = (dash_surface::documents::passages(&doc).iter())
            .position(|(_, w)| w == "Ours alone.")
            .expect("a glossary cell");
        let id = post(&data, page, quote, "And the glossary.", &[], 2_000).expect("posted");
        let outside = format!("Done.\n```edit\n{{\"b{m}\": \"Ours.\"}}\n```");
        work_once(&data, &|_: &str| Ok(outside.clone())).expect("worked");
        let rows = comments::load(&log(&data).expect("log")).expect("rows");
        assert_eq!(
            rows[&id].answer.as_deref(),
            Some(format!("Done.\n\nNot applied: b{m} is not a passage of « 1. Intent »").as_str())
        );
        assert_eq!(std::fs::read_to_string(&file).expect("kept"), written);
    }

    /// A fix's commit is on main once in HEAD; a sha git never finds is asked once per HEAD, then remembered (/code-review).
    #[dash_test]
    fn a_commit_not_on_main_is_asked_once_per_head() {
        let dir = dash_meter::TestDir::new("comment-landed");
        let repo = dir.0.as_path();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git");
            assert!(
                ok.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&ok.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "one"]);
        let head = super::git(repo, &["rev-parse", "HEAD"]).expect("head");
        assert!(landed(repo, &head[..8]));
        assert!(!landed(repo, "deadbeef1"));
        let memo = |sha: &str| NOT_LANDED.lock().expect("memo").1.contains(sha);
        assert!(memo("deadbeef1") && !memo(&head[..8]));
        git(&["commit", "-q", "--allow-empty", "-m", "two"]);
        assert!(!landed(repo, "deadbeef2"));
        assert!(!memo("deadbeef1"), "a new HEAD judges again");
    }
}
