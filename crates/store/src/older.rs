//! The older app's tasks.edn rows and leads for Rust, read from its SQLite file, read-only: no call to the app, so its stalls never hold a
//! pass (it answered no command for 120 s under disk load, 2026-11-09). Deleted once tasks are filed here.
//! ponytail: its schema is read as it stands (`tasks`, `links`, `project_settings`; for the chances `leads`, `events`,
//! `drafts`, `records` and their `data` keys); a change there fails the sync, named,
//! and the store keeps the last list.
use dash_core::tasks::{BUILDS_NOTHING, FreeKind, Queue, Queued, Rank};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

/// Open tasks Rust owns, in the list's order: neither labelled unplanned nor put off (`:later`). Its other holds (a project's,
/// the budget's, a file gone) are the older app's own starts', and hid ready Rust tasks (2026-11-09): Rust's plan holds by its
/// own rules. Its waits: each `waits-on` link to a task of the list not done, in the link's order, and `:blocked-by` as is.
const TASKS: &str = "SELECT t.id, coalesce(t.title, ''),
    -- only an ISO datetime with its offset, as `DateTime.from_iso8601` in `Runner.reopened_ms/1`: julianday() takes more
    CASE WHEN json_type(t.data, '$.since') = 'text' AND json_extract(t.data, '$.since') GLOB '????-??-??T*'
      AND (json_extract(t.data, '$.since') GLOB '*Z' OR json_extract(t.data, '$.since') GLOB '*[+-][0-9][0-9]:[0-9][0-9]')
      THEN CAST(round((julianday(json_extract(t.data, '$.since')) - 2440587.5) * 86400000) AS INTEGER) END,
    (SELECT group_concat(substr(l.\"to\", 7), char(10) ORDER BY l.pos) FROM links l
       WHERE l.\"from\" = 'tasks:' || t.id AND l.kind = 'waits-on' AND l.\"to\" LIKE 'tasks:%'
       AND NOT EXISTS (SELECT 1 FROM tasks d WHERE d.collection = 'tasks.edn' AND 'tasks:' || d.id = l.\"to\"
                       AND d.stage = 'done')),
    -- a list stays in data (`Row.fits?/2` keeps only a text in the column): 96 rows on 2026-11-09
    coalesce(t.blocked_by, (SELECT group_concat(value, ',') FROM json_each(t.data, '$.\"blocked-by\"')
                              WHERE json_type(t.data, '$.\"blocked-by\"') = 'array')),
    -- its labels, a line apart: `dash task add --labels Docs` files one
    (SELECT group_concat(value, char(10)) FROM json_each(t.data, '$.labels')
       WHERE json_type(t.data, '$.labels') = 'array'),
    -- what its rank reads (task rust-orch-pick-dynamic): priority 1 to 4, estimate in points, its epic, its own roadmap item
    -- each cast and bounded, so an odd value is none, never a failed sync
    CASE WHEN CAST(t.priority AS INTEGER) BETWEEN 1 AND 4 THEN CAST(t.priority AS INTEGER) END,
    CASE WHEN CAST(json_extract(t.data, '$.estimate') AS INTEGER) BETWEEN 1 AND 1000
      THEN CAST(json_extract(t.data, '$.estimate') AS INTEGER) END,
    t.epic, ltrim(json_extract(t.data, '$.roadmap'), ':')
  FROM tasks t
  WHERE t.collection = 'tasks.edn' AND t.stage = 'open' AND t.owner = 'rust'
    AND NOT EXISTS (SELECT 1 FROM json_each(json_extract(t.data, '$.labels')) WHERE lower(value) = 'unplanned')
    AND trim(coalesce(json_extract(t.data, '$.later'), '')) = ''
  ORDER BY t.pos";

/// The roadmap's items in its order (`Focus.roadmap/0`): each id, its horizon today, urgent through its `until` day, else its own
/// (urgent stored by hand reads as now), and what it covers, a line apart.
const ROADMAP: &str = "SELECT id,
    CASE WHEN json_extract(data, '$.until') >= date('now', 'localtime') THEN 'urgent'
      ELSE replace(ltrim(coalesce(json_extract(data, '$.horizon'), 'now'), ':'), 'urgent', 'now') END,
    (SELECT group_concat(value, char(10)) FROM json_each(data, '$.covers') WHERE json_type(data, '$.covers') = 'array')
  FROM records WHERE collection = 'focus-overall.edn' AND id IS NOT NULL ORDER BY pos";

/// A roadmap item: its id, horizon and covers.
type Item = (String, String, Vec<String>);

/// The item task `id` serves and its horizon, as `Focus.owner/2` finds it: its own `:roadmap` first (« none » serves none; one
/// the roadmap no longer has falls through, as an item renamed), else the first item covering its id or a prefix of it ending
/// in « * », else the first naming its epic (as id or cover) or a label.
fn item_of(
    items: &[Item],
    id: &str,
    epic: Option<&str>,
    labels: &[String],
    own: Option<&str>,
) -> Option<(String, String)> {
    let found = |item: &Item| Some((item.0.clone(), item.1.clone()));
    if own == Some("none") {
        return None;
    }
    if let Some(item) = own.and_then(|own| items.iter().find(|i| i.0 == own)) {
        return found(item);
    }
    let by_key = |c: &String| c == id || c.strip_suffix('*').is_some_and(|p| id.starts_with(p));
    let by_group = |item: &Item| {
        let names = epic.into_iter().chain(labels.iter().map(String::as_str));
        names
            .into_iter()
            .any(|n| item.0 == n || item.2.iter().any(|c| c == n))
    };
    (items.iter().find(|i| i.2.iter().any(by_key)))
        .or_else(|| items.iter().find(|i| by_group(i)))
        .and_then(found)
}

/// A name of `:blocked-by` the list holds done: met, as `Runner.finished/2` reads a done task.
const DONE: &str =
    "SELECT 1 FROM tasks WHERE collection = 'tasks.edn' AND id = ?1 AND stage = 'done'";

/// A link ref in the older store's text, `\u{E000}@waits-on/0\u{E001}`: its target is read from the links.
const REF: char = '\u{E000}';

/// The names `:blocked-by`'s words hold, as `Runner.names/1` splits them (« a, b and c »), its link refs left out.
fn names(words: &str) -> Vec<String> {
    // `,\s*|\s+and\s+`: any run of blanks around « and » is one
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words.replace(" and ", ",");
    let names = words.split(',').map(str::trim).filter(|n| !n.is_empty());
    names
        .filter(|n| !n.starts_with(REF))
        .map(String::from)
        .collect()
}

/// His seats, as `Dashboard.Orch.Admission.seats/1` counts them: 5 by day, 8 from 22:00 to 07:00 local, plus the grant the
/// forecast loop writes (`claude-boost`, « n » or « n unix », 0 to 3). None while `dash drain` holds (`Holds.drain/1`, project
/// `*`, field `drain`): Rust's orchestrator started five runs after a drain the older app alone read (@nha, 2026-11-09: « finish
/// merging. do not start new work. »). Rust's own seats come first: `dash project set site_perso rust-seats <n>` (0 to 9; the older app takes only its projects) runs Rust's
/// orchestrator alone, its n seats whatever the drain, while the drain keeps holding the older app's runner (@nha, 2026-11-10:
/// « continue with the comments tasks. orchestrate via rust and obeserve »).
const SEATS: &str = "SELECT coalesce((SELECT CAST(trim(value) AS INTEGER) FROM project_settings
      WHERE project = 'site_perso' AND field = 'rust-seats' AND trim(value) GLOB '[0-9]'), CASE WHEN EXISTS (SELECT 1 FROM project_settings
      WHERE project = '*' AND field = 'drain' AND coalesce(value, '') <> '') THEN 0 ELSE
  CASE WHEN CAST(strftime('%H', 'now', 'localtime') AS INTEGER) >= 22
      OR CAST(strftime('%H', 'now', 'localtime') AS INTEGER) < 7 THEN 8 ELSE 5 END
  + coalesce((SELECT CASE WHEN n BETWEEN 0 AND 3 THEN n ELSE 0 END FROM
      (SELECT CASE WHEN trim(value) GLOB '[0-3]' OR trim(value) GLOB '[0-3] *' THEN CAST(substr(trim(value), 1, 1) AS INTEGER)
              ELSE 0 END AS n FROM project_settings
       WHERE project = 'site_perso' AND field = 'claude-boost')), 0) END)";

fn opened(db: &Path) -> Result<Connection, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let db_ = Connection::open_with_flags(db, flags).map_err(named)?;
    db_.busy_timeout(Duration::from_secs(5)).map_err(named)?;
    Ok(db_)
}

/// The list in `db`, the older app's database.
pub fn queue(db: &Path) -> Result<Queue, String> {
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// Whether the list in `db` holds task `id`, at any stage: what a comment's answer links is a task filed, never a name made up.
pub fn filed(db: &Path, id: &str) -> Result<bool, String> {
    let sql = "SELECT 1 FROM tasks WHERE collection = 'tasks.edn' AND id = ?1";
    let found = opened(db)?.prepare(sql).and_then(|mut q| q.exists([id]));
    found.map_err(|e| format!("{}: {e}", db.display()))
}

/// A lead as the older app files it, what its chances read (crates/dash/src/chances.rs): its columns, its `data` as JSON,
/// whether a human answered on its timeline (a call or a LinkedIn entry not ours, `Fields.human?/1`) and whether one of its
/// drafts holds screening answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Lead {
    pub id: String,
    pub posted: Option<String>,
    pub remote: Option<String>,
    pub onsite: Option<String>,
    pub data: String,
    pub human: bool,
    pub answered: bool,
}

const LEADS: &str = "SELECT l.id, l.posted, l.remote, l.onsite_expectation, l.data,
    EXISTS (SELECT 1 FROM events e WHERE e.source = 'timeline:' || l.id
            AND json_extract(e.data, '$.channel') IN ('~:call', '~:linkedin', 'call', 'linkedin')
            AND coalesce(json_type(e.data, '$.by'), 'null') IN ('null', 'false')),
    -- `List.wrap(answers) != []`: any value but none or an empty list
    EXISTS (SELECT 1 FROM drafts d WHERE d.lead = l.id AND coalesce(json_type(d.data, '$.answers'), 'null') <> 'null'
            AND (json_type(d.data, '$.answers') <> 'array' OR json_array_length(d.data, '$.answers') > 0))
  FROM leads l WHERE l.collection = 'opportunities' AND l.id IS NOT NULL ORDER BY l.pos";

/// Every lead in `db`, the older app's database.
pub fn leads(db: &Path) -> Result<Vec<Lead>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let mut q = db_.prepare(LEADS).map_err(named)?;
    let rows = q.query_map([], |r| {
        Ok(Lead {
            id: r.get(0)?,
            posted: r.get(1)?,
            remote: r.get(2)?,
            onsite: r.get(3)?,
            data: r.get(4)?,
            human: r.get(5)?,
            answered: r.get(6)?,
        })
    });
    rows.and_then(Iterator::collect).map_err(named)
}

/// A map source of the model (`profile.edn`, `skills.edn`): each record's key and its data, JSON.
pub fn records(db: &Path, collection: &str) -> Result<Vec<(String, String)>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let sql = "SELECT coalesce(id, ltrim(key, ':')), data FROM records WHERE collection = ?1 ORDER BY pos";
    let db_ = opened(db)?;
    let mut q = db_.prepare(sql).map_err(named)?;
    let rows = q.query_map([collection], |r| Ok((r.get(0)?, r.get(1)?)));
    rows.and_then(Iterator::collect).map_err(named)
}

fn read(db: &Connection) -> rusqlite::Result<Queue> {
    let mut items = db.prepare(ROADMAP)?;
    let items = items.query_map([], |r| {
        let covers: Option<String> = r.get(2)?;
        let covers = covers
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        Ok((r.get(0)?, r.get(1)?, covers))
    })?;
    let items: Vec<Item> = items.collect::<rusqlite::Result<_>>()?;
    let mut rows = db.prepare(TASKS)?;
    let rows = rows.query_map([], |r| {
        let links: Option<String> = r.get(3)?;
        let links = links.unwrap_or_default();
        let labels: Option<String> = r.get(5)?;
        let labels: Vec<String> = labels
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        let light = |l: &String| BUILDS_NOTHING.iter().any(|b| b.eq_ignore_ascii_case(l));
        let id: String = r.get(0)?;
        let (epic, own): (Option<String>, Option<String>) = (r.get(8)?, r.get(9)?);
        let item = item_of(&items, &id, epic.as_deref(), &labels, own.as_deref());
        let task = Queued {
            held: None,
            title: r.get(1)?,
            reopened_ms: r.get(2)?,
            waits: links.lines().map(String::from).collect(),
            // the older app keeps no layer: Rust's store holds it, by `dash task add --layer`
            layer: None,
            builds_nothing: labels.iter().any(light),
            // its deadline is Rust's, kept by `dash task add --deadline`
            rank: Rank {
                item: item.as_ref().map(|i| i.0.clone()),
                horizon: item.map(|i| i.1),
                deadline: None,
                priority: r.get(6)?,
                estimate: r.get(7)?,
            },
            free: FreeKind::of(&labels),
            id,
        };
        Ok((task, r.get::<_, Option<String>>(4)?))
    })?;
    let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut done = db.prepare(DONE)?;
    let mut tasks = vec![];
    // A name in words that is no task of the list holds it, as the older app holds it, until the words are taken off.
    for (mut task, words) in rows {
        for name in names(&words.unwrap_or_default()) {
            if !done.exists([&name])? && !task.waits.contains(&name) {
                task.waits.push(name);
            }
        }
        tasks.push(task);
    }
    let seats: i64 = db.query_row(SEATS, [], |r| r.get(0))?;
    Ok(Queue {
        seats: seats as usize,
        tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// The older app's tables, as much of them as the read takes.
    const OLDER: &str =
        "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL,
        title TEXT, blocked_by TEXT, owner TEXT, priority INTEGER, epic TEXT);
      CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, data TEXT NOT NULL);
      CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
      CREATE TABLE project_settings (project TEXT, field TEXT, value TEXT NOT NULL);";

    fn task(
        db: &Connection,
        pos: i64,
        id: &str,
        stage: &str,
        owner: &str,
        data: &str,
        by: Option<&str>,
    ) {
        let sql = "INSERT INTO tasks (collection, pos, id, stage, data, title, blocked_by, owner) VALUES ('tasks.edn', ?1, ?2, ?3, ?4, ?2, ?5, ?6)";
        db.execute(sql, rusqlite::params![pos, id, stage, data, by, owner])
            .expect("a task");
    }

    #[test]
    fn rust_s_open_tasks_come_in_list_order_with_their_unmet_waits_and_reopen_time() {
        let _budget = TestBudget::new();
        let db = Connection::open_in_memory().expect("db");
        db.execute_batch(OLDER).expect("tables");
        task(
            &db,
            5,
            "b",
            "open",
            "rust",
            r#"{"since":"2026-11-08T21:26:52.543184Z","labels":["Feature","docs"]}"#,
            Some("\u{E000}@waits-on/0\u{E001} and \u{E000}@waits-on/1\u{E001}"),
        );
        task(&db, 1, "a", "open", "rust", "{}", None);
        task(&db, 2, "his", "open", "claude", "{}", None);
        task(
            &db,
            3,
            "later",
            "open",
            "rust",
            r#"{"later":"after a"}"#,
            None,
        );
        task(
            &db,
            4,
            "unplanned",
            "open",
            "rust",
            r#"{"labels":["Unplanned"]}"#,
            None,
        );
        task(&db, 6, "shut", "done", "rust", "{}", None);
        let words = "\u{E000}@waits-on/0\u{E001} and shut, his DNS steps and a";
        task(&db, 7, "words", "open", "rust", "{}", Some(words));
        db.execute(
            "INSERT INTO links VALUES ('tasks:words', 'waits-on', 'drafts:x', 0)",
            [],
        )
        .expect("a link to no task");
        task(&db, 8, "old", "dropped", "claude", "{}", None);
        let link = "INSERT INTO links VALUES ('tasks:b', 'waits-on', ?1, ?2)";
        for (to, pos) in [
            ("tasks:his", 1),
            ("tasks:shut", 0),
            ("tasks:old", 2),
            ("tasks:a", 3),
        ] {
            db.execute(link, rusqlite::params![to, pos])
                .expect("a link");
        }
        // task rust-orch-pick-dynamic, done-when 5: what the rank reads, from the task and the roadmap (focus-overall.edn)
        let item = "INSERT INTO records VALUES ('focus-overall.edn', ?1, ?2, ?3)";
        for (pos, id, data) in [
            (
                0,
                "dash-sqlite",
                r#"{"horizon":"later","covers":["dashboard","b*"]}"#,
            ),
            (1, "rust-poc", r#"{"covers":["rust"]}"#),
            (2, "trip", r#"{"horizon":"next","until":"2999-01-01"}"#),
        ] {
            db.execute(item, rusqlite::params![pos, id, data])
                .expect("an item");
        }
        db.execute("UPDATE tasks SET priority = 1, epic = 'rust', data = json_set(data, '$.estimate', 5) WHERE id = 'a'", [])
            .expect("a's fields");
        db.execute("UPDATE tasks SET epic = 'rust', data = json_set(data, '$.roadmap', ':trip') WHERE id = 'words'", [])
            .expect("its own item");
        // odd values are none or read as the number, never a failed sync; an own item gone falls through to its prefix
        let odd = "UPDATE tasks SET priority = 1.0, data = json_set(data, '$.estimate', 99999999999, '$.roadmap', ':gone')
          WHERE id = 'b'";
        db.execute(odd, []).expect("b's odd fields");
        let queue = read(&db).expect("the list");
        let ids: Vec<&str> = queue.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "words"]);
        let rank = |at: usize| {
            let r = &queue.tasks[at].rank;
            (
                r.item.as_deref(),
                r.horizon.as_deref(),
                r.priority,
                r.estimate,
            )
        };
        assert_eq!(
            rank(0),
            (Some("rust-poc"), Some("now"), Some(1), Some(5)),
            "its epic covered, no horizon is now"
        );
        assert_eq!(
            rank(1),
            (Some("dash-sqlite"), Some("later"), Some(1), None),
            "its own item gone, its id by a prefix; a real 1.0 is priority 1, an estimate past 1000 none"
        );
        assert_eq!(
            rank(2),
            (Some("trip"), Some("urgent"), None, None),
            "its own item first, urgent until its day"
        );
        let kinds: Vec<_> = queue.tasks.iter().map(|t| t.free).collect();
        assert_eq!(
            kinds,
            [None, Some(FreeKind::Planning), None],
            "a Docs label is planning work for big-pickle"
        );
        let light: Vec<bool> = queue.tasks.iter().map(|t| t.builds_nothing).collect();
        assert_eq!(light, [false, true, false], "a Docs label builds nothing");
        assert_eq!(
            queue.tasks[1].waits,
            ["his", "old", "a"],
            "a done wait is met, a dropped one is not, as Runner.finished/2"
        );
        assert_eq!(queue.tasks[1].reopened_ms, Some(1_794_173_212_543));
        assert_eq!(queue.tasks[0].reopened_ms, None);
        assert_eq!(
            queue.tasks[2].waits,
            ["his DNS steps", "a"],
            "each name of its words, a done one met"
        );
        assert!([5, 8].contains(&queue.seats), "{}", queue.seats);
        db.execute(
            "INSERT INTO project_settings VALUES ('site_perso', 'claude-boost', '2 1794100000')",
            [],
        )
        .expect("a boost");
        assert!([7, 10].contains(&read(&db).expect("the list").seats));
        db.execute("UPDATE project_settings SET value = '2x'", [])
            .expect("a malformed boost");
        assert!(
            [5, 8].contains(&read(&db).expect("the list").seats),
            "2x grants none"
        );
        let drain = "INSERT INTO project_settings VALUES ('*', 'drain', '2026-11-09T13:26:27Z')";
        db.execute(drain, []).expect("a drain");
        assert_eq!(
            read(&db).expect("the list").seats,
            0,
            "a drain starts nothing"
        );
        db.execute("DELETE FROM project_settings WHERE field = 'drain'", [])
            .expect("a resume");
        assert!(
            [5, 8].contains(&read(&db).expect("the list").seats),
            "a resume gives them back"
        );
        // Rust's own seats, whatever the drain: the older app's runner stays held, Rust runs its n
        db.execute(drain, []).expect("a drain again");
        db.execute(
            "INSERT INTO project_settings VALUES ('site_perso', 'rust-seats', '2')",
            [],
        )
        .expect("rust's seats");
        assert_eq!(read(&db).expect("the list").seats, 2, "rust's own seats");
        db.execute(
            "UPDATE project_settings SET value = 'x' WHERE field = 'rust-seats'",
            [],
        )
        .expect("a malformed count");
        assert_eq!(
            read(&db).expect("the list").seats,
            0,
            "a malformed count falls back to the drain"
        );
        db.execute(
            "DELETE FROM project_settings WHERE field IN ('rust-seats', 'drain')",
            [],
        )
        .expect("back");
        // /code-review of rust-own-queue: a list kept in data, blanks around « and », a since that is only a date
        let data = r#"{"blocked-by":["shut","a"],"since":"2026-11-08"}"#;
        task(&db, 9, "listed", "open", "rust", data, None);
        task(&db, 10, "spaced", "open", "rust", "{}", Some("x  and\ny"));
        let data = r#"{"since":"2026-11-08T21:26:52"}"#;
        task(&db, 11, "no-offset", "open", "rust", data, Some("a and a"));
        let queue = read(&db).expect("the list");
        assert_eq!(
            queue.tasks[3].waits,
            ["a"],
            "a list's names, a done one met"
        );
        assert_eq!(
            queue.tasks[3].reopened_ms, None,
            "a date alone is no reopen time"
        );
        assert_eq!(queue.tasks[4].waits, ["x", "y"]);
        assert_eq!(
            queue.tasks[5].reopened_ms, None,
            "no offset, as from_iso8601"
        );
    }
}
