//! The comments plugin (rfc-rust-unified section 20b): each comment of his, on any page, is one row of a queue the watcher
//! wakes the orchestrator on, never a task (@nha, 2026-11-08: « this should still be a watcher but just a simple queue
//! behind it. the rust version can do that. »). It depends on the core alone: its table is written and read through the op
//! log port, and the binary opens it with `SCHEMA`. The older app still keeps the comments (`Dashboard.Comments`, where
//! `dash comment answer` writes) and still routes each to a task card (`Dashboard.Watch`) until this replaces it:
//! `comments` reads its export, and an answer there closes the row on the next pass.
use dash_core::{FieldWrite, OpLog, Value};
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;

pub const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS comment_queue (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";
const TABLE: &str = "comment_queue";
const FIELD: &str = "record";
const PAGE: i64 = 1000;

/// A pass this often, so a comment is a row within a minute of its post.
pub const POLL_MS: i64 = 30_000;
// Within a minute: a comment posted just after a pass lands on the next one, a read of a fraction of a second later.
const _: () = assert!(POLL_MS < 60_000);
/// He comments in batches (@nha, 2026-10-01): a wake waits until the newest new row is this old, as `Dashboard.Watch`'s
/// @quiet_s, or the oldest is `HELD_MS` old (its @held_s), so a stream that never stops still wakes.
pub const QUIET_MS: i64 = 60_000;
pub const HELD_MS: i64 = 5 * 60_000;
/// A row still open this long after its wake wakes again.
pub const AGAIN_MS: i64 = 30 * 60_000;
/// Where his phone reaches the app (`Dashboard.Phone.link/1`).
pub const BASE: &str = "http://100.72.158.115:7788";
/// A comment's words in a wake, at most: the link holds the rest.
const CLIP: usize = 300;

/// One comment as the older app's export holds it. `waiting`: not resolved, and his word last (`Comments.untreated_comments`).
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: String,
    /// A repo path, a page path (`/d/<id>`, `/dashboard/tasks/<id>`) or a web page's URL.
    pub file: String,
    pub quote: String,
    pub text: String,
    /// When he posted it: its time, else its day.
    pub at: String,
    pub waiting: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub page: String,
    pub quote: String,
    pub text: String,
    pub link: String,
    pub at: String,
    pub landed_ms: i64,
    /// Its last wake.
    pub told_ms: Option<i64>,
    /// Why it closed: `answered` (Claude's word last, or resolved), or `gone` from the export.
    pub closed: Option<String>,
    /// Where it came from: `older` (the older app's export, its answer written there) or `compass` (a Rust page's sidebar,
    /// worked and answered here by the queue's agent).
    pub from: String,
    /// The agent working it, and since when.
    pub worker: Option<String>,
    pub worked_ms: Option<i64>,
    /// The agent's answer, shown under the comment in the compass.
    pub answer: Option<String>,
}

/// A row the compass posted.
pub const COMPASS: &str = "compass";
const OLDER: &str = "older";

/// The older app's keywords are « ~:name ».
fn text(m: &Map<String, Json>, key: &str) -> String {
    let s = m.get(key).and_then(Json::as_str).unwrap_or_default();
    s.strip_prefix("~:").unwrap_or(s).to_string()
}

/// `dash store text live/comments.edn`: every comment, in its order.
pub fn comments(export: &Json) -> Result<Vec<Comment>, String> {
    let all = export.as_array().ok_or("comments: not a list")?;
    let read = |c: &Json| {
        let m = c.as_object()?;
        let last = m
            .get("replies")
            .and_then(Json::as_array)
            .and_then(|r| r.last())
            .and_then(Json::as_object);
        let last_by = last.map(|r| text(r, "by"));
        let at = Some(text(m, "at")).filter(|a| !a.is_empty());
        // His last word: his reply that reopened the thread, else the comment.
        let said = match last {
            Some(r) if last_by.as_deref() != Some("claude") => text(r, "text"),
            _ => text(m, "text"),
        };
        Some(Comment {
            id: Some(text(m, "id")).filter(|id| !id.is_empty())?,
            file: text(m, "file"),
            quote: text(m, "quote"),
            text: said,
            at: at.unwrap_or_else(|| text(m, "date")),
            waiting: text(m, "status") != "resolved" && last_by.as_deref() != Some("claude"),
        })
    };
    // A record with no id is no comment anyone can answer: left out, the rest still land.
    Ok(all.iter().filter_map(read).collect())
}

/// Its place on the app, as `Dashboard.Comments.link/2` makes it: a web page as it is, a page path as it is, a repo path
/// under /v/ with each segment encoded; then the comment's anchor.
pub fn link(file: &str, id: &str) -> String {
    if file.starts_with("http://") || file.starts_with("https://") {
        return file.to_string();
    }
    let page = if file.starts_with('/') {
        file.to_string()
    } else {
        let segment = |s: &str| {
            s.bytes()
                .map(|b| match b {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'~' | b'_' | b'-' | b'.' => {
                        (b as char).to_string()
                    }
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        };
        let segments: Vec<String> = file.split('/').map(segment).collect();
        format!("/v/{}", segments.join("/"))
    };
    format!("{BASE}{page}#c-{id}")
}

fn to_json(r: &Row) -> Json {
    json!({"id": r.id, "page": r.page, "quote": r.quote, "text": r.text, "link": r.link, "at": r.at,
        "landed_ms": r.landed_ms, "told_ms": r.told_ms, "closed": r.closed, "from": r.from, "worker": r.worker,
        "worked_ms": r.worked_ms, "answer": r.answer})
}

fn from_json(j: &Json) -> Option<Row> {
    let m = j.as_object()?;
    let owned = |key: &str| m.get(key).and_then(Json::as_str).map(str::to_string);
    Some(Row {
        id: owned("id")?,
        page: owned("page")?,
        quote: owned("quote")?,
        text: owned("text")?,
        link: owned("link")?,
        at: owned("at")?,
        landed_ms: m.get("landed_ms")?.as_i64()?,
        told_ms: m.get("told_ms").and_then(Json::as_i64),
        closed: owned("closed"),
        // a row written before the compass came from the export
        from: owned("from").unwrap_or_else(|| OLDER.into()),
        worker: owned("worker"),
        worked_ms: m.get("worked_ms").and_then(Json::as_i64),
        answer: owned("answer"),
    })
}

/// The table, read from the log.
/// ponytail: a row whose record does not read is left out, so its comment lands again as new on the next pass, a second
/// wake at worst; the whole log is read each pass, fine at a few thousand rows, a read port once the core has one.
pub fn load(log: &dyn OpLog) -> Result<BTreeMap<String, Row>, String> {
    let mut rows = BTreeMap::new();
    let mut after: Option<(i64, String)> = None;
    loop {
        let ops = log.ops_after(after.as_ref().map(|(s, w)| (*s, w.as_str())), PAGE)?;
        let Some(last) = ops.last() else {
            return Ok(rows);
        };
        after = Some((last.stamp, last.writer.clone()));
        for w in ops.iter().map(|op| &op.write) {
            if let (TABLE, FIELD, Value::Text(t)) = (w.table.as_str(), w.field.as_str(), &w.value) {
                match serde_json::from_str(t).ok().and_then(|j| from_json(&j)) {
                    Some(r) => rows.insert(w.row.clone(), r),
                    None => rows.remove(&w.row),
                };
            }
        }
    }
}

/// Writes each row that differs from `kept`, and says how many.
pub fn save(
    log: &mut dyn OpLog,
    kept: &BTreeMap<String, Row>,
    rows: &BTreeMap<String, Row>,
) -> Result<usize, String> {
    let writes: Vec<FieldWrite> = rows
        .values()
        .filter(|r| kept.get(&r.id) != Some(r))
        .map(|r| FieldWrite {
            table: TABLE.into(),
            row: r.id.clone(),
            field: FIELD.into(),
            value: Value::Text(to_json(r).to_string()),
        })
        .collect();
    if !writes.is_empty() {
        log.write(&writes)?;
    }
    Ok(writes.len())
}

/// The queue after one look at the comments: a waiting comment with no open row lands as one (his reply on a closed
/// thread opens it again), and an open row whose comment no longer waits closes. Nothing else is made of a comment. A
/// compass row is never the export's: it stays as it is.
pub fn step(
    kept: &BTreeMap<String, Row>,
    comments: &[Comment],
    now_ms: i64,
) -> BTreeMap<String, Row> {
    let mut rows = kept.clone();
    for c in comments {
        let open = rows.get(&c.id).is_some_and(|r| r.closed.is_none());
        if c.waiting && !open {
            let row = Row {
                id: c.id.clone(),
                page: c.file.clone(),
                quote: c.quote.clone(),
                text: c.text.clone(),
                link: link(&c.file, &c.id),
                at: c.at.clone(),
                landed_ms: now_ms,
                told_ms: None,
                closed: None,
                from: OLDER.into(),
                worker: None,
                worked_ms: None,
                answer: None,
            };
            rows.insert(c.id.clone(), row);
        }
    }
    let by_id: BTreeMap<&str, &Comment> = comments.iter().map(|c| (c.id.as_str(), c)).collect();
    for r in rows
        .values_mut()
        .filter(|r| r.closed.is_none() && r.from != COMPASS)
    {
        r.closed = match by_id.get(r.id.as_str()) {
            None => Some("gone".into()),
            Some(c) if c.waiting => None,
            Some(_) => Some("answered".into()),
        };
    }
    rows
}

/// The rows a wake names now, oldest comment first. New rows wait for the quiet minute (or 5 minutes held); a row woken
/// 30 minutes ago and still open is due again on its own. A compass row wakes nobody: the queue's agent works it.
pub fn due(rows: &BTreeMap<String, Row>, now_ms: i64) -> Vec<&Row> {
    let open: Vec<&Row> = (rows.values())
        .filter(|r| r.closed.is_none() && r.from != COMPASS)
        .collect();
    let new: Vec<&Row> = open
        .iter()
        .copied()
        .filter(|r| r.told_ms.is_none())
        .collect();
    let newest = new.iter().map(|r| r.landed_ms).max();
    let oldest = new.iter().map(|r| r.landed_ms).min();
    let quiet = newest.is_some_and(|n| now_ms - n >= QUIET_MS)
        || oldest.is_some_and(|o| now_ms - o >= HELD_MS);
    let again = open
        .iter()
        .copied()
        .filter(|r| r.told_ms.is_some_and(|t| now_ms - t >= AGAIN_MS));
    let mut due: Vec<&Row> = again.chain(new.into_iter().filter(|_| quiet)).collect();
    due.sort_by(|a, b| (&a.at, &a.id).cmp(&(&b.at, &b.id)));
    due
}

fn one_line(s: &str, limit: usize) -> String {
    let words = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match words.char_indices().nth(limit) {
        Some((at, _)) => format!("{}…", &words[..at]),
        None => words,
    }
}

/// One line for the orchestrator's wake file, each row with its page, quote and link, in the order they drain.
/// « Woken by the dashboard » is what `Dashboard.Watch.Wakes` counts.
pub fn wake(due: &[&Row]) -> String {
    let items: Vec<String> = due
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let again = if r.told_ms.is_some() {
                "still open 30 minutes after its wake: "
            } else {
                ""
            };
            let quote = if r.quote.is_empty() {
                "the whole page".to_string()
            } else {
                format!("« {} »", one_line(&r.quote, CLIP))
            };
            format!(
                "{}. {again}comment {} on {}, {quote}: « {} » ({}).",
                i + 1,
                r.id,
                r.page,
                one_line(&r.text, CLIP),
                r.link
            )
        })
        .collect();
    format!(
        "Woken by the dashboard's comment queue ({}): {} Drain them in this order. Do what each asks, or answer why not, \
         with private/dashboard/bin/dash comment answer <comment id> '<what you did or why not>', a link to each task, \
         step, PR or page it names; the answer closes its row, and a row still open 30 minutes after a wake wakes again.",
        due.len(),
        items.join(" ")
    )
}

/// The rows of `due`, woken at `now_ms`.
pub fn told(rows: &mut BTreeMap<String, Row>, due: &[String], now_ms: i64) {
    for id in due {
        if let Some(r) = rows.get_mut(id) {
            r.told_ms = Some(now_ms);
        }
    }
}

/// A comment the compass posted on `page` (a path of the app): one row, open, for the queue's agent.
pub fn posted(id: &str, page: &str, quote: &str, text: &str, at: &str, now_ms: i64) -> Row {
    Row {
        id: id.into(),
        page: page.into(),
        quote: quote.into(),
        text: text.into(),
        link: link(page, id),
        at: at.into(),
        landed_ms: now_ms,
        told_ms: None,
        closed: None,
        from: COMPASS.into(),
        worker: None,
        worked_ms: None,
        answer: None,
    }
}

fn open_compass(rows: &BTreeMap<String, Row>) -> impl Iterator<Item = &Row> {
    (rows.values()).filter(|r| r.from == COMPASS && r.closed.is_none())
}

/// The row the agent is on: the one it took, else none.
pub fn working(rows: &BTreeMap<String, Row>) -> Option<&Row> {
    open_compass(rows).find(|r| r.worker.is_some())
}

/// The row the one agent works next: the one it took and did not finish (a restart takes it again), else the oldest open.
pub fn next(rows: &BTreeMap<String, Row>) -> Option<&Row> {
    working(rows).or_else(|| open_compass(rows).min_by_key(|r| (r.landed_ms, r.id.clone())))
}

/// Open compass rows, the one in work included.
pub fn waiting(rows: &BTreeMap<String, Row>) -> usize {
    open_compass(rows).count()
}

/// The compass rows of one page, oldest first: what its sidebar lists, each with its answer once there.
pub fn on_page<'a>(rows: &'a BTreeMap<String, Row>, page: &str) -> Vec<&'a Row> {
    let mut on: Vec<&Row> = (rows.values())
        .filter(|r| r.from == COMPASS && r.page == page)
        .collect();
    on.sort_by_key(|r| (r.landed_ms, r.id.clone()));
    on
}

/// The row taken by `agent` at `now_ms`.
pub fn took(row: &Row, agent: &str, now_ms: i64) -> Row {
    Row {
        worker: Some(agent.into()),
        worked_ms: Some(now_ms),
        ..row.clone()
    }
}

/// The row given back after a run that failed: open, no agent on it.
pub fn given_back(row: &Row) -> Row {
    Row {
        worker: None,
        worked_ms: None,
        ..row.clone()
    }
}

/// The row answered: its answer kept, the row closed.
pub fn closed_with(row: &Row, answer: &str) -> Row {
    Row {
        answer: Some(answer.trim().into()),
        closed: Some("answered".into()),
        worker: None,
        ..row.clone()
    }
}

/// What the agent is asked for one comment.
pub fn prompt(row: &Row) -> String {
    let quote = if row.quote.is_empty() {
        "the whole page".to_string()
    } else {
        format!("this part of it: « {} »", row.quote)
    };
    format!(
        "Nicolas commented on the page {} of his dashboard ({}), about {quote}. His comment: « {} ». Do what it asks if \
         reading the repository is enough, else say exactly what needs doing and where. Your reply is posted under his \
         comment, as it is: a few short lines, in his language, no preamble.",
        row.page, row.link, row.text
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::Op;
    use dash_meter::memory::TestBudget;

    const MIN: i64 = 60_000;

    /// The op log in memory: its stamps count up, as one replica's do.
    #[derive(Default)]
    struct Log(Vec<Op>);

    impl OpLog for Log {
        fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String> {
            let ops: Vec<Op> = writes
                .iter()
                .map(|w| Op {
                    stamp: self.0.len() as i64 + 1,
                    writer: "t".into(),
                    write: w.clone(),
                })
                .collect();
            self.0.extend(ops.iter().cloned());
            Ok(ops)
        }
        fn apply(&mut self, _ops: &[Op]) -> Result<usize, String> {
            unreachable!()
        }
        fn ops_after(&self, after: Option<(i64, &str)>, limit: i64) -> Result<Vec<Op>, String> {
            let from = after.map_or(0, |(s, _)| s);
            let ops = self.0.iter().filter(|o| o.stamp > from);
            Ok(ops.take(limit as usize).cloned().collect())
        }
        fn state_hash(&self) -> Result<u64, String> {
            unreachable!()
        }
    }

    /// One pass as the binary runs it: read, step, wake what is due, mark it, save. The wake line, if any.
    fn pass(log: &mut Log, export: &Json, now: i64) -> Option<String> {
        let kept = load(log).expect("loads");
        let mut rows = step(&kept, &comments(export).expect("reads"), now);
        let due: Vec<Row> = due(&rows, now).into_iter().cloned().collect();
        let line = (!due.is_empty()).then(|| wake(&due.iter().collect::<Vec<_>>()));
        let ids: Vec<String> = due.into_iter().map(|r| r.id).collect();
        told(&mut rows, &ids, now);
        save(log, &kept, &rows).expect("saves");
        line
    }

    fn open(log: &Log) -> Vec<String> {
        let rows = load(log).expect("loads");
        rows.into_values()
            .filter(|r| r.closed.is_none())
            .map(|r| r.id)
            .collect()
    }

    fn answered(mut export: Json, id: &str, answer: &str) -> Json {
        for c in export.as_array_mut().expect("a list") {
            if c["id"] == id {
                let reply = json!({"by": "~:claude", "text": answer, "date": "2026-11-08"});
                c.as_object_mut()
                    .expect("a map")
                    .entry("replies")
                    .or_insert(json!([]))
                    .as_array_mut()
                    .expect("replies")
                    .push(reply);
            }
        }
        export
    }

    /// The two comments he wrote on the Hostman draft on 2026-11-08, before Claude answered them, as the export held them.
    fn hostman() -> Json {
        json!([
            {"id": "f99c7dcc", "file": "/d/hostman-grant-reply",
             "quote": "6. Call: I am available Tuesday 10 or Wednesday 11 November, 10:00 to 12:00 or 14:00 to 17:00 CET. Happy to adapt.",
             "text": "Nice but try to get a time for people not just before their lunch. I believe hungry people are not as nice. their timezone if we can find it.",
             "date": "2026-11-08", "at": "2026-11-08T09:51:35.212087Z", "status": "~:open", "path": "#body-read", "replies": []},
            {"id": "d0e47ac7", "file": "/d/hostman-grant-reply", "quote": "1. Website and users:",
             "text": "count users and link to app store too",
             "date": "2026-11-08", "at": "2026-11-08T09:51:48.043917Z", "status": "~:open", "path": "#body-read", "replies": []},
        ])
    }

    #[test]
    fn a_comment_on_any_page_is_one_row_and_nothing_else() {
        let _budget = TestBudget::new();
        let export = json!([
            {"id": "a1", "file": "/d/some-draft", "quote": "q", "text": "on a draft", "date": "2026-11-08", "status": "~:open"},
            {"id": "b2", "file": "private/docs/rfc x.html", "quote": "", "text": "on a doc", "date": "2026-11-07", "status": "~:open"},
            {"id": "c3", "file": "/dashboard/tasks/rust-poc", "quote": "", "text": "on an issue", "date": "2026-11-06",
             "status": "~:open", "replies": [{"by": "~:claude", "text": "old"}, {"by": "~:nicolas", "text": "again"}]},
            {"id": "d4", "file": "/d/x", "text": "answered", "status": "~:open", "replies": [{"by": "~:claude", "text": "done"}]},
            {"id": "e5", "file": "/d/x", "text": "resolved", "status": "~:resolved"},
            {"file": "/d/x", "text": "no id"},
        ]);
        let mut log = Log::default();
        assert_eq!(pass(&mut log, &export, 0), None, "the quiet minute first");
        assert_eq!(open(&log), ["a1", "b2", "c3"]);
        let rows = load(&log).expect("loads");
        assert_eq!(rows.len(), 3, "an answered or resolved comment never lands");
        assert_eq!(rows["a1"].link, format!("{BASE}/d/some-draft#c-a1"));
        assert_eq!(
            rows["b2"].link,
            format!("{BASE}/v/private/docs/rfc%20x.html#c-b2")
        );
        assert_eq!(
            rows["c3"].link,
            format!("{BASE}/dashboard/tasks/rust-poc#c-c3")
        );
        assert_eq!(link("https://x.org/p", "z"), "https://x.org/p");
        let before = log.0.len();
        pass(&mut log, &export, POLL_MS);
        assert_eq!(log.0.len(), before, "a second look adds no row");
    }

    #[test]
    fn new_rows_wake_once_batched_oldest_first_after_a_quiet_minute() {
        let _budget = TestBudget::new();
        let mut log = Log::default();
        let first = json!([{"id": "late", "file": "/d/a", "quote": "two", "text": "second\nline", "at": "2026-11-08T10:00:00Z", "status": "~:open"}]);
        assert_eq!(pass(&mut log, &first, 0), None);
        let both = json!([first[0],
            {"id": "early", "file": "/d/b", "quote": "", "text": "first", "at": "2026-11-08T09:00:00Z", "status": "~:open"}]);
        assert_eq!(
            pass(&mut log, &both, 30_000),
            None,
            "a new row holds the batch"
        );
        assert_eq!(pass(&mut log, &both, 60_000), None);
        let line = pass(&mut log, &both, 90_000).expect("one wake for both");
        assert!(line.starts_with("Woken by the dashboard's comment queue (2): 1. comment early on /d/b, the whole page: « first »"), "{line}");
        assert!(
            line.contains(&format!(
                "2. comment late on /d/a, « two »: « second line » ({BASE}/d/a#c-late)."
            )),
            "{line}"
        );
        assert!(!line.contains('\n'));
        assert_eq!(pass(&mut log, &both, 120_000), None, "told once");

        // A stream that never stops still wakes 5 minutes after its oldest row.
        let mut log = Log::default();
        let mut export = vec![];
        let mut woke = None;
        for i in 0..12 {
            export.push(json!({"id": format!("s{i:02}"), "file": "/d/s", "text": "more", "status": "~:open"}));
            woke = woke.or(pass(&mut log, &Json::Array(export.clone()), i * 30_000).map(|_| i));
        }
        assert_eq!(woke, Some(10));
    }

    #[test]
    fn an_answer_closes_its_row_and_an_open_row_wakes_again_at_30_minutes() {
        let _budget = TestBudget::new();
        let mut log = Log::default();
        let export = json!([
            {"id": "x", "file": "/d/a", "text": "one", "at": "1", "status": "~:open"},
            {"id": "y", "file": "/d/a", "text": "two", "at": "2", "status": "~:open"},
        ]);
        pass(&mut log, &export, 0);
        assert!(pass(&mut log, &export, MIN).is_some());
        let export = answered(export, "x", "Done: see PR 1");
        assert_eq!(pass(&mut log, &export, 2 * MIN), None);
        assert_eq!(open(&log), ["y"]);
        assert_eq!(load(&log).unwrap()["x"].closed.as_deref(), Some("answered"));
        assert_eq!(
            pass(&mut log, &export, 30 * MIN),
            None,
            "29 minutes after its wake"
        );
        let again = pass(&mut log, &export, 31 * MIN).expect("30 minutes after its wake");
        assert!(
            again.contains("(1): 1. still open 30 minutes after its wake: comment y"),
            "{again}"
        );
        assert_eq!(pass(&mut log, &export, 32 * MIN), None);

        // His reply on an answered thread opens it again; a resolved or vanished comment closes.
        let mut export = answered(export, "y", "Done too");
        pass(&mut log, &export, 33 * MIN);
        assert!(open(&log).is_empty());
        export[0]["replies"]
            .as_array_mut()
            .unwrap()
            .push(json!({"by": "~:nicolas", "text": "not quite"}));
        pass(&mut log, &export, 34 * MIN);
        assert_eq!(open(&log), ["x"]);
        assert_eq!(load(&log).unwrap()["x"].text, "not quite", "his last word");
        export[0]["status"] = json!("~:resolved");
        pass(&mut log, &Json::Array(vec![export[0].clone()]), 35 * MIN);
        let rows = load(&log).unwrap();
        assert!(open(&log).is_empty());
        assert_eq!(rows["x"].closed.as_deref(), Some("answered"));
        assert_eq!(rows["y"].closed.as_deref(), Some("answered"));
    }

    #[test]
    fn the_two_hostman_comments_of_2026_11_08_land_wake_and_close() {
        let _budget = TestBudget::new();
        let mut log = Log::default();
        let export = hostman();
        assert_eq!(pass(&mut log, &export, 0), None);
        assert_eq!(open(&log), ["d0e47ac7", "f99c7dcc"]);
        let line = pass(&mut log, &export, MIN).expect("one wake");
        let first = line.find("comment f99c7dcc").expect("names the first");
        let second = line.find("comment d0e47ac7").expect("names the second");
        assert!(first < second, "posted first, drained first: {line}");
        assert!(line.contains("(2): 1. comment f99c7dcc on /d/hostman-grant-reply, « 6. Call: I am available Tuesday 10"));
        assert!(
            line.contains(&format!("({BASE}/d/hostman-grant-reply#c-d0e47ac7)")),
            "{line}"
        );
        assert!(line.contains("« count users and link to app store too »"));
        // Claude's two answers that day, as `dash comment answer` wrote them.
        let export = answered(
            export,
            "f99c7dcc",
            "Done: his mail is sent from UTC-8, so likely US West Coast.",
        );
        let export = answered(
            export,
            "d0e47ac7",
            "Done: the App Store link is in, and the one count we can read.",
        );
        assert_eq!(pass(&mut log, &export, 2 * MIN), None);
        assert!(open(&log).is_empty());
        assert_eq!(
            pass(&mut log, &export, 40 * MIN),
            None,
            "closed rows never wake again"
        );
    }

    /// Done-when 1 and 2 of rust-compass-queue: a compass comment is one row the export never closes and no wake names;
    /// the one agent takes the oldest first, keeps the one it took across a restart, and its answer closes the row.
    #[test]
    fn a_compass_row_waits_for_the_agent_oldest_first_and_its_answer_closes_it() {
        let _budget = TestBudget::new();
        let mut log = Log::default();
        let mut rows = load(&log).expect("loads");
        let late = posted(
            "compass-b",
            "/tasks",
            "",
            "second",
            "2026-11-09T10:01:00Z",
            2 * MIN,
        );
        let early = posted(
            "compass-a",
            "/tasks/x",
            "a line",
            "first",
            "2026-11-09T10:00:00Z",
            MIN,
        );
        for r in [late, early] {
            rows.insert(r.id.clone(), r);
        }
        save(&mut log, &BTreeMap::new(), &rows).expect("saves");
        assert_eq!(
            rows["compass-a"].link,
            format!("{BASE}/tasks/x#c-compass-a")
        );
        // the export knows neither: they stay open, and wake nobody however long they wait
        assert_eq!(pass(&mut log, &json!([]), 60 * MIN), None);
        assert_eq!(open(&log), ["compass-a", "compass-b"]);
        let rows = load(&log).expect("loads");
        assert_eq!(rows["compass-a"].from, COMPASS);
        assert_eq!(next(&rows).map(|r| r.id.as_str()), Some("compass-a"));
        assert!(working(&rows).is_none());
        assert!(prompt(&rows["compass-a"]).contains("« a line »"));
        assert!(prompt(&rows["compass-a"]).contains("« first »"));

        let mut taken = rows.clone();
        let row = took(&rows["compass-a"], "comment-agent", 61 * MIN);
        taken.insert(row.id.clone(), row);
        save(&mut log, &rows, &taken).expect("saves");
        let rows = load(&log).expect("loads");
        assert_eq!(working(&rows).map(|r| r.id.as_str()), Some("compass-a"));
        assert_eq!(
            next(&rows).map(|r| r.id.as_str()),
            Some("compass-a"),
            "the one it took, again"
        );
        assert_eq!(waiting(&rows), 2);

        let mut done = rows.clone();
        let row = closed_with(&rows["compass-a"], " Done: see /tasks/x \n");
        done.insert(row.id.clone(), row);
        save(&mut log, &rows, &done).expect("saves");
        let rows = load(&log).expect("loads");
        assert_eq!(
            rows["compass-a"].answer.as_deref(),
            Some("Done: see /tasks/x")
        );
        assert_eq!(rows["compass-a"].closed.as_deref(), Some("answered"));
        assert_eq!(next(&rows).map(|r| r.id.as_str()), Some("compass-b"));
        let page: Vec<&str> = on_page(&rows, "/tasks/x")
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(page, ["compass-a"], "an answered comment stays on its page");
    }
}
