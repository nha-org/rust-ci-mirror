//! The older app's tasks.edn rows for Rust, read from its SQLite file, read-only: no call to the app, so its stalls never hold a
//! pass (it answered no command for 120 s under disk load, 2026-11-09). Deleted once tasks are filed here.
//! ponytail: its schema is read as it stands (`tasks`, `links`, `project_settings`); a change there fails the sync, named,
//! and the store keeps the last list.
use dash_core::tasks::{Queue, Queued};
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
                              WHERE json_type(t.data, '$.\"blocked-by\"') = 'array'))
  FROM tasks t
  WHERE t.collection = 'tasks.edn' AND t.stage = 'open' AND t.owner = 'rust'
    AND NOT EXISTS (SELECT 1 FROM json_each(json_extract(t.data, '$.labels')) WHERE lower(value) = 'unplanned')
    AND trim(coalesce(json_extract(t.data, '$.later'), '')) = ''
  ORDER BY t.pos";

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
/// forecast loop writes (`claude-boost`, « n » or « n unix », 0 to 3).
const SEATS: &str = "SELECT CASE WHEN CAST(strftime('%H', 'now', 'localtime') AS INTEGER) >= 22
      OR CAST(strftime('%H', 'now', 'localtime') AS INTEGER) < 7 THEN 8 ELSE 5 END
  + coalesce((SELECT CASE WHEN n BETWEEN 0 AND 3 THEN n ELSE 0 END FROM
      (SELECT CASE WHEN trim(value) GLOB '[0-3]' OR trim(value) GLOB '[0-3] *' THEN CAST(substr(trim(value), 1, 1) AS INTEGER)
              ELSE 0 END AS n FROM project_settings
       WHERE project = 'site_perso' AND field = 'claude-boost')), 0)";

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

fn read(db: &Connection) -> rusqlite::Result<Queue> {
    let mut rows = db.prepare(TASKS)?;
    let rows = rows.query_map([], |r| {
        let links: Option<String> = r.get(3)?;
        let links = links.unwrap_or_default();
        let task = Queued {
            id: r.get(0)?,
            title: r.get(1)?,
            reopened_ms: r.get(2)?,
            waits: links.lines().map(String::from).collect(),
            // the older app keeps no layer: Rust's store holds it, by `dash task add --layer`
            layer: None,
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
        title TEXT, blocked_by TEXT, owner TEXT);
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
        let sql = "INSERT INTO tasks VALUES ('tasks.edn', ?1, ?2, ?3, ?4, ?2, ?5, ?6)";
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
            r#"{"since":"2026-11-08T21:26:52.543184Z"}"#,
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
        let queue = read(&db).expect("the list");
        let ids: Vec<&str> = queue.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "words"]);
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
