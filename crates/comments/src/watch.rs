//! What wakes the orchestrator besides his comments, Rust's own (the older app's `Dashboard.Watch` and the PM loop's
//! to_process requests, ported, task rust-watch-port; @nha, 2026-11-11: « see blocked pages. Is the rust watcher telling you
//! already ? »): a task pane's question dialog, a block, a lesson, a Datadog push. Each is one row, once by its key, beside the
//! comment queue, and rides the comment queue's wake, batched as `Dashboard.Watch` batched them: a question, block or push
//! due once the newest is a quiet minute old or the oldest 5 minutes, a lesson at most every 30 minutes, every open row in
//! the wake that is due. Nothing here writes a file of to_process.
use crate::{BASE, Row, changes as changed, read_table, write_table};
use dash_core::FieldWrite;
use dash_core::OpLog;
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;

const TABLE: &str = "watch_queue";
/// As `Dashboard.Watch`'s @quiet_s and @held_s.
pub const QUIET_MS: i64 = crate::QUIET_MS;
pub const HELD_MS: i64 = crate::HELD_MS;
/// Lessons wake at most this often (its @lessons_s).
pub const LESSONS_MS: i64 = 30 * 60_000;
/// A question still shown this long after its wake wakes again.
pub const AGAIN_MS: i64 = crate::AGAIN_MS;
/// A row's words in a wake, at most: its link holds the rest.
const CLIP: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A task pane's open dialog: open while the pane shows it.
    Question,
    Block,
    Lesson,
    /// A Datadog monitor alerting, pushed to his phone.
    Push,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Question => "question",
            Kind::Block => "block",
            Kind::Lesson => "lesson",
            Kind::Push => "datadog push",
        }
    }

    fn of(word: &str) -> Option<Kind> {
        [Kind::Question, Kind::Block, Kind::Lesson, Kind::Push]
            .into_iter()
            .find(|k| k.word() == word)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: String,
    pub kind: Kind,
    pub task: Option<String>,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    pub landed_ms: i64,
    /// Its last wake.
    pub told_ms: Option<i64>,
    /// Why it closed: `told` (a notice, told once) or `gone` (the pane no longer shows the question).
    pub closed: Option<String>,
}

impl Item {
    pub fn new(
        key: &str,
        kind: Kind,
        task: Option<&str>,
        title: &str,
        body: &str,
        now_ms: i64,
    ) -> Item {
        Item {
            key: key.to_string(),
            kind,
            task: task.map(str::to_string),
            title: title.to_string(),
            body: body.to_string(),
            link: task.map(task_link),
            landed_ms: now_ms,
            told_ms: None,
            closed: None,
        }
    }
}

/// A task's page, where its run, dialog and PR show.
pub fn task_link(id: &str) -> String {
    format!("{BASE}/projects/site_perso?issue={}", crate::segment(id))
}

fn to_json(i: &Item) -> Json {
    json!({"key": i.key, "kind": i.kind.word(), "task": i.task, "title": i.title, "body": i.body, "link": i.link,
        "landed_ms": i.landed_ms, "told_ms": i.told_ms, "closed": i.closed})
}

fn from_json(j: &Json) -> Option<Item> {
    let m: &Map<String, Json> = j.as_object()?;
    let owned = |key: &str| m.get(key).and_then(Json::as_str).map(str::to_string);
    Some(Item {
        key: owned("key")?,
        kind: Kind::of(m.get("kind")?.as_str()?)?,
        task: owned("task"),
        title: owned("title")?,
        body: owned("body").unwrap_or_default(),
        link: owned("link"),
        landed_ms: m.get("landed_ms")?.as_i64()?,
        told_ms: m.get("told_ms").and_then(Json::as_i64),
        closed: owned("closed"),
    })
}

/// The rows, by key.
/// ponytail: no row is pruned and the whole log is read each pass and each land, as the comment queue's `load` does: tens of
/// rows a day, fine for months; a read port with a cut by `landed_ms` once a pass shows its time on /watchers.
pub fn load(log: &dyn OpLog) -> Result<BTreeMap<String, Item>, String> {
    read_table(log, TABLE, from_json)
}

/// Writes each row that differs from `kept`, and says how many.
pub fn save(
    log: &mut dyn OpLog,
    kept: &BTreeMap<String, Item>,
    rows: &BTreeMap<String, Item>,
) -> Result<usize, String> {
    write_table(log, TABLE, kept, rows, to_json)
}

/// His comments and the watcher's rows saved in one write: both or neither, so a wake never marks one table told while the
/// other failed (/code-review of rust-watch-port).
pub fn save_both(
    log: &mut dyn OpLog,
    (kept, rows): (&BTreeMap<String, Row>, &BTreeMap<String, Row>),
    (kept_items, items): (&BTreeMap<String, Item>, &BTreeMap<String, Item>),
) -> Result<usize, String> {
    let mut writes: Vec<FieldWrite> = crate::comment_changes(kept, rows);
    writes.extend(changed(TABLE, kept_items, items, to_json));
    crate::write(log, writes)
}

/// Lands `item` once by its key: true when it is new. A key seen before, open or closed, changes nothing.
pub fn land(log: &mut dyn OpLog, item: Item) -> Result<bool, String> {
    let kept = load(log)?;
    if kept.contains_key(&item.key) {
        return Ok(false);
    }
    let mut rows = kept.clone();
    rows.insert(item.key.clone(), item);
    save(log, &kept, &rows).map(|_| true)
}

/// The questions after one look at the panes: each shown lands once, a closed one shown again opens as new, and an open one
/// closes once its pane no longer asks. `asking`: every pane herdr says asks, read or not, so a read that failed once (a
/// redraw, a claims read) closes nothing (/code-review: a row closed on one failed read never came back).
pub fn questions(rows: &mut BTreeMap<String, Item>, shown: Vec<Item>, asking: &[String]) {
    let keys: Vec<String> = shown.iter().map(|i| i.key.clone()).collect();
    let pane_of = |key: &str| key.split(' ').nth(1).map(str::to_string);
    for i in rows.values_mut() {
        let still = keys.contains(&i.key) || pane_of(&i.key).is_some_and(|p| asking.contains(&p));
        if i.kind == Kind::Question && i.closed.is_none() && !still {
            i.closed = Some("gone".into());
        }
    }
    for i in shown {
        match rows.get_mut(&i.key) {
            Some(kept) if kept.closed.is_some() => *kept = i,
            Some(_) => {}
            None => {
                rows.insert(i.key.clone(), i);
            }
        }
    }
}

/// The rows a wake names now, in the order they landed: every open row not yet told once a question, block or push is
/// quiet a minute (or held 5 minutes), or a lesson waits and none woke in 30 minutes; a question still shown 30 minutes
/// after its wake on its own.
pub fn due(rows: &BTreeMap<String, Item>, now_ms: i64) -> Vec<&Item> {
    let open: Vec<&Item> = rows.values().filter(|i| i.closed.is_none()).collect();
    let new: Vec<&Item> = open
        .iter()
        .copied()
        .filter(|i| i.told_ms.is_none())
        .collect();
    let urgent: Vec<&Item> = new
        .iter()
        .copied()
        .filter(|i| i.kind != Kind::Lesson)
        .collect();
    let newest = urgent.iter().map(|i| i.landed_ms).max();
    let oldest = urgent.iter().map(|i| i.landed_ms).min();
    let quiet = newest.is_some_and(|n| now_ms - n >= QUIET_MS)
        || oldest.is_some_and(|o| now_ms - o >= HELD_MS);
    let last_lesson = (rows.values())
        .filter(|i| i.kind == Kind::Lesson)
        .filter_map(|i| i.told_ms)
        .max();
    let lesson = new.iter().any(|i| i.kind == Kind::Lesson)
        && last_lesson.is_none_or(|t| now_ms - t >= LESSONS_MS);
    let again = (open.iter().copied())
        .filter(|i| i.kind == Kind::Question && i.told_ms.is_some_and(|t| now_ms - t >= AGAIN_MS));
    let mut due: Vec<&Item> = again
        .chain(new.into_iter().filter(|_| quiet || lesson))
        .collect();
    due.sort_by_key(|i| (i.landed_ms, i.key.clone()));
    due
}

/// The rows of `due`, woken at `now_ms`: a question stays open while its pane shows it, anything else is told once.
pub fn told(rows: &mut BTreeMap<String, Item>, due: &[String], now_ms: i64) {
    for key in due {
        if let Some(i) = rows.get_mut(key) {
            i.told_ms = Some(now_ms);
            if i.kind != Kind::Question {
                i.closed = Some("told".into());
            }
        }
    }
}

fn clip(s: &str) -> String {
    let words = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match words.char_indices().nth(CLIP) {
        Some((at, _)) => format!("{}…", &words[..at]),
        None => words,
    }
}

/// The wake's words for `due`, numbered from `from`: each row's kind and link, its title and words.
pub fn wake(due: &[&Item], from: usize) -> String {
    let items: Vec<String> = (due.iter().enumerate())
        .map(|(n, i)| {
            let kind = match &i.link {
                Some(l) => format!("{} ({l})", i.kind.word()),
                None => i.kind.word().to_string(),
            };
            let again = match i.told_ms {
                Some(_) => "still shown 30 minutes after its wake: ",
                None => "",
            };
            let body = match i.body.trim() {
                "" => String::new(),
                b => format!(" {}", clip(b)),
            };
            format!("{}. {kind}: {again}{}.{body}", from + n, clip(&i.title))
        })
        .collect();
    format!(
        "{} Done when each question is answered with its pick line, each lesson folded, each block and Datadog push acted on, \
         as the orchestrator skill's step 3 says.",
        items.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::MemoryLog as Log;
    use dash_meter::dash_test;

    const MIN: i64 = 60_000;

    /// A pass as the binary runs it on these rows: due, told, saved. The wake's words, if any.
    fn pass(log: &mut Log, now: i64) -> Option<String> {
        let kept = load(log).expect("loads");
        let mut rows = kept.clone();
        let due: Vec<Item> = due(&rows, now).into_iter().cloned().collect();
        let line = (!due.is_empty()).then(|| wake(&due.iter().collect::<Vec<_>>(), 1));
        let keys: Vec<String> = due.into_iter().map(|i| i.key).collect();
        told(&mut rows, &keys, now);
        save(log, &kept, &rows).expect("saves");
        line
    }

    fn block(key: &str, now: i64) -> Item {
        Item::new(
            key,
            Kind::Block,
            Some("rust-x"),
            &format!("Task rust-x is flagged: {key}"),
            "",
            now,
        )
    }

    /// Done-when 2, blocks: two blocks land once each, wake together a quiet minute after the newest, then never again.
    #[dash_test]
    fn blocks_wake_once_batched_a_quiet_minute_after_the_newest() {
        let mut log = Log::default();
        assert!(land(&mut log, block("b1", 0)).expect("lands"));
        assert!(!land(&mut log, block("b1", 5)).expect("once by its key"));
        assert!(land(&mut log, block("b2", 30_000)).expect("lands"));
        assert_eq!(pass(&mut log, MIN), None, "b2 is not quiet a minute yet");
        let line = pass(&mut log, 30_000 + MIN).expect("both, batched");
        assert!(
            line.starts_with(
                "1. block (http://100.72.158.115:7788/projects/site_perso?issue=rust-x): Task rust-x is flagged: b1. \
                 2. block (http://100.72.158.115:7788/projects/site_perso?issue=rust-x): Task rust-x is flagged: b2."
            ),
            "{line}"
        );
        assert_eq!(pass(&mut log, 100 * MIN), None, "told once");
    }

    /// A stream that never stops still wakes, 5 minutes after its oldest.
    #[dash_test]
    fn a_stream_that_never_stops_wakes_at_five_minutes() {
        let mut log = Log::default();
        for s in 0..7 {
            land(&mut log, block(&format!("b{s}"), s * 50_000)).expect("lands");
            let now = s * 50_000 + 10_000;
            assert_eq!(pass(&mut log, now).is_some(), now >= HELD_MS, "at {now}");
        }
    }

    /// Done-when 2, lessons: a lesson wakes alone when none woke in 30 minutes, else rides the next due wake.
    #[dash_test]
    fn lessons_wake_at_most_every_30_minutes() {
        let mut log = Log::default();
        let lesson = |key: &str, now| {
            Item::new(
                key,
                Kind::Lesson,
                Some("rust-y"),
                "Task rust-y's lesson",
                "pin the herdr read",
                now,
            )
        };
        land(&mut log, lesson("l1", 0)).expect("lands");
        let line = pass(&mut log, 0).expect("no lesson woke yet: at once");
        assert!(line.starts_with("1. lesson (http://100.72.158.115:7788/projects/site_perso?issue=rust-y): Task rust-y's lesson. pin the herdr read"), "{line}");
        land(&mut log, lesson("l2", MIN)).expect("lands");
        assert_eq!(pass(&mut log, 2 * MIN), None, "one woke 2 minutes ago");
        land(&mut log, block("b", 3 * MIN)).expect("lands");
        let line = pass(&mut log, 4 * MIN).expect("the block is due, the lesson rides it");
        assert!(
            line.contains("1. lesson") && line.contains("2. block"),
            "{line}"
        );
        land(&mut log, lesson("l3", 5 * MIN)).expect("lands");
        assert_eq!(pass(&mut log, 20 * MIN), None);
        assert!(
            pass(&mut log, 34 * MIN).is_some(),
            "30 minutes after the last lesson's wake"
        );
    }

    /// Done-when 2, Datadog pushes: each alert pushed is one row, woken with its monitor's link.
    #[dash_test]
    fn a_datadog_push_wakes_batched_with_its_words() {
        let mut log = Log::default();
        let words = "Datadog: « dash serve p99 » is alerting on command:serve. https://app.datadoghq.eu/monitors/7";
        land(
            &mut log,
            Item::new("push 7 command:serve 1", Kind::Push, None, words, "", 0),
        )
        .expect("lands");
        land(&mut log, block("b", 10_000)).expect("lands");
        assert_eq!(pass(&mut log, 30_000), None);
        let line = pass(&mut log, 10_000 + MIN).expect("due");
        assert!(
            line.starts_with(&format!("1. datadog push: {words}. 2. block")),
            "{line}"
        );
    }

    /// Done-when 1: a question stays open while shown or unread, closes once its pane stops asking, opens again when shown again,
    /// and wakes again at 30 minutes.
    #[dash_test]
    fn a_question_wakes_while_shown_and_closes_once_gone() {
        let q = |now| {
            Item::new(
                "question w3:p1 1",
                Kind::Question,
                Some("rust-z"),
                "Task rust-z's pane w3:p1 asks",
                "Q1 …",
                now,
            )
        };
        let mut rows = BTreeMap::new();
        let w3 = ["w3:p1".to_string()];
        questions(&mut rows, vec![q(0)], &w3);
        assert_eq!(due(&rows, 30_000).len(), 0);
        let keys = vec!["question w3:p1 1".to_string()];
        assert_eq!(due(&rows, MIN).len(), 1);
        told(&mut rows, &keys, MIN);
        questions(&mut rows, vec![q(2 * MIN)], &w3);
        assert_eq!(rows["question w3:p1 1"].landed_ms, 0, "landed once");
        questions(&mut rows, vec![], &w3);
        assert_eq!(
            rows["question w3:p1 1"].closed, None,
            "still asking, its read failed: kept open"
        );
        assert_eq!(due(&rows, 10 * MIN).len(), 0);
        assert_eq!(due(&rows, 31 * MIN).len(), 1, "still shown 30 minutes on");
        questions(&mut rows, vec![], &[]);
        assert_eq!(rows["question w3:p1 1"].closed.as_deref(), Some("gone"));
        assert_eq!(due(&rows, 90 * MIN).len(), 0);
        questions(&mut rows, vec![q(91 * MIN)], &w3);
        assert_eq!(
            rows["question w3:p1 1"].closed, None,
            "shown again: open as new"
        );
        assert_eq!(due(&rows, 92 * MIN).len(), 1);
    }
}
