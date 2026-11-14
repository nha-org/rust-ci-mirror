//! What wakes the orchestrator besides his comments, Rust's own (the older app's `Dashboard.Watch` and the PM loop's
//! to_process requests, ported, task rust-watch-port; @nha, 2026-11-11: « see blocked pages. Is the rust watcher telling you
//! already ? »): a block, a lesson, a Datadog push. Each is one row, once by its key, beside the comment queue, and rides the
//! comment queue's wake, batched as `Dashboard.Watch` batched them: a block or push due once the newest is a quiet minute old
//! or the oldest 5 minutes, a lesson at most every 30 minutes, every open row in the wake that is due. A task pane's question
//! is no wake: it is a comment row the question pass rules (`crate::panes`, task rust-questions-orchestrator-first). Nothing
//! here writes a file of to_process.
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
/// A row's words in a wake, at most: its link holds the rest.
const CLIP: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Block,
    Lesson,
    /// A Datadog monitor alerting, pushed to his phone.
    Push,
    /// A task pane's screen matched a failure signature (`crate::fails`): due at once, never a quiet minute.
    Pane,
    /// A pane whose program said it is blocked or done (task rust-watch-osc7501).
    Stop,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Block => "block",
            Kind::Lesson => "lesson",
            Kind::Push => "datadog push",
            Kind::Pane => "pane failure",
            Kind::Stop => "stop",
        }
    }

    fn of(word: &str) -> Option<Kind> {
        [
            Kind::Block,
            Kind::Lesson,
            Kind::Push,
            Kind::Pane,
            Kind::Stop,
        ]
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
    /// Why it closed: `told`, each row is told once.
    pub closed: Option<String>,
    /// What the woken orchestrator did about it (`dash watch outcome`): a told row with none after `OUTCOME_MS` is raised on
    /// his inbox (task pane-fail-detect; the task dialog's answer, 2026-11-13: every watcher wake).
    pub outcome: Option<String>,
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
            outcome: None,
        }
    }
}

/// A task's page, where its run, dialog and PR show.
pub fn task_link(id: &str) -> String {
    format!("{BASE}/projects/site_perso?issue={}", crate::segment(id))
}

fn to_json(i: &Item) -> Json {
    json!({"key": i.key, "kind": i.kind.word(), "task": i.task, "title": i.title, "body": i.body, "link": i.link,
        "landed_ms": i.landed_ms, "told_ms": i.told_ms, "closed": i.closed, "outcome": i.outcome})
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
        // a row told before outcomes were asked has no field: its wake named no key to record one with, so none is due
        outcome: match m.contains_key("outcome") || m.get("told_ms").is_none_or(Json::is_null) {
            true => owned("outcome"),
            false => Some(BEFORE.into()),
        },
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

/// The rows a wake names now, in the order they landed: every open row not yet told once a block or push is quiet a minute
/// (or held 5 minutes), or a lesson waits and none woke in 30 minutes.
pub fn due(rows: &BTreeMap<String, Item>, now_ms: i64) -> Vec<&Item> {
    let new: Vec<&Item> = (rows.values())
        .filter(|i| i.closed.is_none() && i.told_ms.is_none())
        .collect();
    let urgent: Vec<&Item> = new
        .iter()
        .copied()
        .filter(|i| i.kind != Kind::Lesson)
        .collect();
    let newest = urgent.iter().map(|i| i.landed_ms).max();
    let oldest = urgent.iter().map(|i| i.landed_ms).min();
    let quiet = newest.is_some_and(|n| now_ms - n >= QUIET_MS)
        || oldest.is_some_and(|o| now_ms - o >= HELD_MS)
        || urgent.iter().any(|i| i.kind == Kind::Pane);
    let last_lesson = (rows.values())
        .filter(|i| i.kind == Kind::Lesson)
        .filter_map(|i| i.told_ms)
        .max();
    let lesson = new.iter().any(|i| i.kind == Kind::Lesson)
        && last_lesson.is_none_or(|t| now_ms - t >= LESSONS_MS);
    let mut due: Vec<&Item> = new.into_iter().filter(|_| quiet || lesson).collect();
    due.sort_by_key(|i| (i.landed_ms, i.key.clone()));
    due
}

/// The rows of `due`, woken at `now_ms`: each is told once.
pub fn told(rows: &mut BTreeMap<String, Item>, due: &[String], now_ms: i64) {
    for key in due {
        if let Some(i) = rows.get_mut(key) {
            i.told_ms = Some(now_ms);
            i.closed = Some("told".into());
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
            let body = match i.body.trim() {
                "" => String::new(),
                b => format!(" {}", clip(b)),
            };
            format!(
                "{}. {kind} [{}]: {}.{body}",
                from + n,
                i.key,
                clip(&i.title)
            )
        })
        .collect();
    format!(
        "{} Done when each lesson is folded, each block, stop, pane failure and Datadog push acted on, \
         as the orchestrator skill's step 3 says, and each one's outcome recorded: \
         dash watch outcome '<its [key]>' '<what you did>'; one with none in 15 minutes goes on his inbox.",
        items.join(" ")
    )
}

/// A told row with no outcome this long is raised on his inbox.
pub const OUTCOME_MS: i64 = 15 * 60_000;

/// The outcome of a row told before outcomes were asked.
pub const BEFORE: &str = "told before wakes recorded outcomes";

/// Records `words` as the outcome of each told row `name` names: its key, whatever it holds; or its task or a pane failure's
/// pane, when it has none yet (a generic word of a key, such as a signature, names nothing, and no outcome is overwritten).
/// How many.
pub fn outcome(rows: &mut BTreeMap<String, Item>, name: &str, words: &str) -> usize {
    let pane = |i: &Item| i.kind == Kind::Pane && i.key.split(' ').nth(1) == Some(name);
    let by_word = |i: &Item| i.task.as_deref() == Some(name) || pane(i);
    let named = |i: &Item| i.key == name || (i.outcome.is_none() && by_word(i));
    let mut n = 0;
    for i in rows
        .values_mut()
        .filter(|i| i.told_ms.is_some() && named(i))
    {
        i.outcome = Some(words.to_string());
        n += 1;
    }
    n
}

/// The wake's line: his comments' words, then the watcher's rows (task rust-watch-port), one line, one wake.
pub fn line(due: &[&Row], items: &[&Item]) -> String {
    let mut parts = vec![];
    if !due.is_empty() {
        parts.push(crate::wake(due));
    }
    if !items.is_empty() {
        parts.push(format!(
            "Woken by the dashboard's Rust watcher ({}): {}",
            items.len(),
            wake(items, 1)
        ));
    }
    parts.join(" ")
}

/// A pass's wake: `due` comments and `due_items` rows marked told and both tables saved, then `line` appended to `file`.
/// Kept as woken before the wake is written: a save that fails then writes no wake, so a failing store never repeats one
/// every pass. A wake that fails to write puts the rows back unwoken, for the next pass. How many rows were written.
pub fn tell(
    log: &mut dyn OpLog,
    (kept, rows): (&BTreeMap<String, Row>, &BTreeMap<String, Row>),
    items: &BTreeMap<String, Item>,
    (due, due_items): (&[String], &[String]),
    (file, line): (&std::path::Path, &str),
    now_ms: i64,
) -> Result<usize, String> {
    use std::io::Write;
    let mut woken = rows.clone();
    crate::told(&mut woken, due, now_ms);
    let mut woken_items = items.clone();
    told(&mut woken_items, due_items, now_ms);
    let written = save_both(log, (kept, &woken), (items, &woken_items))?;
    let wrote = (std::fs::OpenOptions::new().create(true).append(true))
        .open(file)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = wrote {
        save_both(log, (&woken, rows), (&woken_items, items))?;
        return Err(format!("{}: {e}", file.display()));
    }
    Ok(written)
}

/// `dash watch outcome`'s write: `words` as the outcome of each told row `name` names (`outcome`), in one read and one write.
/// How many rows took it, or that none is named.
pub fn record(log: &mut dyn OpLog, name: &str, words: &str) -> Result<String, String> {
    let kept = load(log)?;
    let mut rows = kept.clone();
    match outcome(&mut rows, name, words) {
        0 => Err(format!("no woken row is named {name}")),
        n => save(log, &kept, &rows).map(|_| format!("{n} recorded")),
    }
}

/// Each told row with no outcome after `OUTCOME_MS` raised on `asks` once (the task dialog's answer, 2026-11-13: every
/// watcher wake), and the asks a recorded outcome settles. None when the rows do not read, said on stderr.
pub fn raise(
    log: &dyn OpLog,
    asks: &mut BTreeMap<String, crate::asks::Ask>,
    now_ms: i64,
) -> std::collections::BTreeSet<String> {
    match load(log) {
        Ok(rows) => {
            let raised = unrecorded(&rows, asks, now_ms);
            asks.extend(raised.into_iter().map(|a| (a.id.clone(), a)));
            recorded(&rows)
        }
        Err(e) => {
            eprintln!("inbox: the wakes with no outcome: {e}");
            Default::default()
        }
    }
}

/// The asks a row's outcome settles: `wake <key>` of each row that has one, so a late outcome drops its ask.
pub fn recorded(rows: &BTreeMap<String, Item>) -> std::collections::BTreeSet<String> {
    (rows.values())
        .filter(|i| i.outcome.is_some())
        .map(|i| format!("wake {}", i.key))
        .collect()
}

/// The ask that raises a told row with no outcome after `OUTCOME_MS`, once: its id is `wake <key>`, so an ask already on
/// `asks`, open or closed, is never raised again.
pub fn unrecorded(
    rows: &BTreeMap<String, Item>,
    asks: &BTreeMap<String, crate::asks::Ask>,
    now_ms: i64,
) -> Vec<crate::asks::Ask> {
    (rows.values())
        .filter(|i| i.outcome.is_none())
        .filter(|i| i.told_ms.is_some_and(|t| now_ms - t >= OUTCOME_MS))
        .filter(|i| !asks.contains_key(&format!("wake {}", i.key)))
        .map(|i| {
            let what = format!("The orchestrator was woken and recorded nothing: {}", clip(&i.title));
            crate::asks::Ask {
                why: Some("his-hands".into()),
                pick: format!(
                    "Ask the orchestrator what it did, then: dash watch outcome '{}' '<what was done>'",
                    i.key
                ),
                link: i.link.clone().unwrap_or_else(|| format!("{BASE}/watchers")),
                task: i.task.clone(),
                kind: "wake".into(),
                ..crate::asks::Ask::raised(&format!("wake {}", i.key), "watcher", &what, now_ms)
            }
        })
        .collect()
}

/// Task rust-watch-osc7501: each task's pane turned blocked or done lands one watcher row, once by its pane, word, the time
/// that word was said and its session (a reused pane id or a restarted count never matches an older stop), which the comment
/// wake tells. Left out: a pane of no task, the orchestrator's own (its stop would wake it again, every turn) or his, and the
/// panes in `ruled`, a task's question the question pass rules (`crate::panes`). A stop not yet told whose pane now
/// says something else closes, `moved on`, so no wake names a pane back at work. One read and one write of the log a pass
/// (/code-review of PR 1570). How many landed.
pub fn land_stops(
    log: &mut dyn OpLog,
    statuses: &[dash_core::tasks::Status],
    tasks: &BTreeMap<String, String>,
    ruled: &[String],
    now_ms: i64,
) -> usize {
    let kept = match load(log) {
        Ok(kept) => kept,
        Err(e) => {
            eprintln!("comment: the stops: {e}");
            return 0;
        }
    };
    let mut rows = kept.clone();
    let stops: Vec<Item> = (statuses.iter())
        .filter(|s| matches!(s.state.as_str(), "blocked" | "done"))
        .filter(|s| !ruled.contains(&s.pane))
        .filter_map(|s| {
            let task = tasks.get(&s.pane)?;
            let on = (s.kind.as_deref())
                .map(|k| format!(" on a {k}"))
                .unwrap_or_default();
            let title = format!("task {task}'s pane {} is {}{on}", s.pane, s.state);
            let session = s.session.as_deref().unwrap_or("-");
            let key = format!("stop {} {} {} {session}", s.pane, s.state, s.at);
            Some(Item::new(&key, Kind::Stop, Some(task), &title, "", now_ms))
        })
        .collect();
    let seen: Vec<&str> = statuses.iter().map(|s| s.pane.as_str()).collect();
    for row in rows.values_mut() {
        let pane = row.key.split(' ').nth(1).unwrap_or_default();
        let moved = row.kind == Kind::Stop
            && row.closed.is_none()
            && seen.contains(&pane)
            && !stops.iter().any(|s| s.key == row.key);
        if moved {
            row.closed = Some("moved on".into());
        }
    }
    let mut landed = 0;
    for item in stops {
        if !rows.contains_key(&item.key) {
            rows.insert(item.key.clone(), item);
            landed += 1;
        }
    }
    if let Err(e) = save(log, &kept, &rows) {
        eprintln!("comment: the stops: {e}");
        return 0;
    }
    landed
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
                "1. block (http://100.72.158.115:7788/projects/site_perso?issue=rust-x) [b1]: Task rust-x is flagged: b1. \
                 2. block (http://100.72.158.115:7788/projects/site_perso?issue=rust-x) [b2]: Task rust-x is flagged: b2."
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
        assert!(line.starts_with("1. lesson (http://100.72.158.115:7788/projects/site_perso?issue=rust-y) [l1]: Task rust-y's lesson. pin the herdr read"), "{line}");
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
            line.starts_with(&format!(
                "1. datadog push [push 7 command:serve 1]: {words}. 2. block"
            )),
            "{line}"
        );
    }

    /// Done-when 1: a pane failure is due on the pass that lands it, batched with what else is new, and told once.
    #[dash_test]
    fn a_pane_failure_wakes_at_once() {
        let mut log = Log::default();
        land(&mut log, block("b", 0)).expect("lands");
        let fail = crate::fails::row("w1:p2", "rust-x", "error-143", "Error 143", 10_000);
        land(&mut log, fail.clone()).expect("lands");
        let line = pass(&mut log, 10_000).expect("no quiet minute");
        assert!(
            line.contains("1. block") && line.contains("2. pane failure"),
            "{line}"
        );
        assert!(
            line.contains("[pane w1:p2 rust-x error-143]") && line.contains("dash watch outcome"),
            "{line}"
        );
        assert!(!land(&mut log, fail).expect("once by its key"));
        assert_eq!(pass(&mut log, 20_000), None, "told once");
    }

    /// Done-when 2: a told row with no outcome after 15 minutes is one ask, raised once; one with an outcome is none.
    #[dash_test]
    fn a_wake_with_no_outcome_is_raised_once() {
        let mut log = Log::default();
        land(&mut log, block("b", 0)).expect("lands");
        land(
            &mut log,
            crate::fails::row("w1:p2", "rust-x", "bare-prompt", "% ", 0),
        )
        .expect("lands");
        pass(&mut log, 0).expect("woken");
        let mut rows = load(&log).expect("loads");
        assert_eq!(
            outcome(&mut rows, "w1:p2", "restarted it"),
            1,
            "named by its pane"
        );
        let mut asks = BTreeMap::new();
        assert!(
            unrecorded(&rows, &asks, OUTCOME_MS - 1).is_empty(),
            "not 15 minutes yet"
        );
        let raised = unrecorded(&rows, &asks, OUTCOME_MS);
        assert_eq!(raised.len(), 1, "{raised:?}");
        assert_eq!(raised[0].id, "wake b");
        assert!(
            crate::asks::lacks(&raised[0]).is_empty(),
            "{:?}",
            crate::asks::lacks(&raised[0])
        );
        asks.insert(raised[0].id.clone(), raised[0].clone());
        assert!(
            unrecorded(&rows, &asks, 2 * OUTCOME_MS).is_empty(),
            "raised once"
        );
        let kept = load(&log).expect("loads");
        save(&mut log, &kept, &rows).expect("saves");
        assert_eq!(
            load(&log).expect("loads")["pane w1:p2 rust-x bare-prompt"]
                .outcome
                .as_deref(),
            Some("restarted it")
        );
        assert!(recorded(&rows).contains("wake pane w1:p2 rust-x bare-prompt"));
        assert!(!recorded(&rows).contains("wake b"), "no outcome yet");
        assert_eq!(
            outcome(&mut rows, "bare-prompt", "x"),
            0,
            "a signature names no row"
        );
        assert_eq!(
            outcome(&mut rows, "w1:p2", "again"),
            0,
            "a word never overwrites"
        );
        assert_eq!(outcome(&mut rows, "b", "read it"), 1, "its key does");
        assert!(
            recorded(&rows).contains("wake b"),
            "a late outcome settles its ask"
        );
    }

    /// A row told before outcomes were asked raises nothing: its wake named no key.
    #[dash_test]
    fn a_row_told_before_outcomes_raises_nothing() {
        let old = json!({"key": "b", "kind": "block", "title": "t", "landed_ms": 0, "told_ms": 1, "closed": "told"});
        assert_eq!(
            from_json(&old).expect("row").outcome.as_deref(),
            Some(BEFORE)
        );
        let untold = json!({"key": "b", "kind": "block", "title": "t", "landed_ms": 0});
        assert_eq!(
            from_json(&untold).expect("row").outcome,
            None,
            "an old row told now asks one"
        );
    }
}
