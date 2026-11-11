//! The older app's tasks.edn rows and leads for Rust, read from its SQLite file, read-only: no call to the app, so its stalls never hold a
//! pass (it answered no command for 120 s under disk load, 2026-11-09). Deleted once tasks are filed here.
//! ponytail: its schema is read as it stands (`tasks`, `links`, `project_settings`; for the chances `leads`, `events`,
//! `drafts`, `records` and their `data` keys); a change there fails the sync, named,
//! and the store keeps the last list.
use dash_core::tasks::{BUILDS_NOTHING, Comment, Demand, Filed, FreeKind, Queue, Queued, Rank};
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
    t.epic, ltrim(json_extract(t.data, '$.roadmap'), ':'),
    -- its demand (RFC 20h): its own word, else hard when its effort is high
    coalesce(ltrim(json_extract(t.data, '$.run.demand'), ':'),
      CASE WHEN ltrim(json_extract(t.data, '$.run.effort'), ':') = 'high' THEN 'hard' END)
  FROM tasks t
  WHERE t.collection = 'tasks.edn' AND t.stage = 'open' AND t.owner = 'rust'
    AND NOT EXISTS (SELECT 1 FROM json_each(json_extract(t.data, '$.labels')) WHERE lower(value) = 'unplanned')
    AND trim(coalesce(json_extract(t.data, '$.later'), '')) = ''
  ORDER BY t.pos";

/// The roadmap's items in its order (`Focus.roadmap/0`): each id, its horizon today, urgent through its `until` day, else its own
/// (urgent stored by hand reads as now), and what it covers, a line apart.
/// A roadmap item's horizon today, `Focus`'s reading: urgent through its `until` day, else its own, urgent stored by hand
/// reading as now. One fragment for ROADMAP and FOCUS, so the rank and the brief never read it two ways.
macro_rules! horizon {
    () => {
        "CASE WHEN json_extract(data, '$.until') >= date('now', 'localtime') THEN 'urgent'
      ELSE replace(ltrim(coalesce(json_extract(data, '$.horizon'), 'now'), ':'), 'urgent', 'now') END"
    };
}

const ROADMAP: &str = concat!(
    "SELECT id, ",
    horizon!(),
    ",
    (SELECT group_concat(value, char(10)) FROM json_each(data, '$.covers') WHERE json_type(data, '$.covers') = 'array')
  FROM records WHERE collection = 'focus-overall.edn' AND id IS NOT NULL ORDER BY pos"
);

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

/// A consistent copy of `db` at `to`, read while the older app writes it (task rust-elixir-reference: the frozen app on :7799
/// reads a copy, never the live file). `to` and its journal files are replaced.
pub fn snapshot(db: &Path, to: &Path) -> Result<(), String> {
    for end in ["", "-wal", "-shm"] {
        let mut file = to.as_os_str().to_owned();
        file.push(end);
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {e}", to.display()));
            }
            _ => {}
        }
    }
    let into = to
        .to_str()
        .ok_or_else(|| format!("{}: not UTF-8", to.display()))?;
    opened(db)?
        .execute("VACUUM INTO ?1", [into])
        .map(drop)
        .map_err(|e| format!("{} into {}: {e}", db.display(), to.display()))
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

/// Task `id`'s stage in `db` (« open », « done », « dropped », « new », « backlog ») and its `:run :prompt`, none when the list
/// has no such task: what an automation reads before filing its one task (crates/dash/src/automations.rs).
pub fn stage(db: &Path, id: &str) -> Result<Option<(String, Option<String>)>, String> {
    let sql = "SELECT coalesce(stage, 'open'), json_extract(data, '$.run.prompt') FROM tasks
               WHERE collection = 'tasks.edn' AND id = ?1";
    let read = opened(db)?.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?)));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read.map(Some).map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// Task `id`'s newest report in `db` (`dash task report`): its row id and its record, JSON as written.
pub fn last_report(db: &Path, id: &str) -> Result<Option<(i64, String)>, String> {
    let sql = "SELECT id, record FROM task_reports WHERE task = ?1 ORDER BY id DESC LIMIT 1";
    let read = opened(db)?.query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?)));
    match read {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read.map(Some).map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// The list's labels, Linear's four groups in its order: type, area, agent, plan (rfc-dashboard 5.5; the older app's
/// `Dashboard.Labels.set/0`). A label is a kind of work, never a project: 12, shared by every project.
pub const LABELS: [&str; 12] = [
    "Bug",
    "Feature",
    "Improvement",
    "Research",
    "Docs",
    "Design",
    "Infra",
    "Security",
    "Perf",
    "Claude",
    "vibe",
    "Unplanned",
];

/// The agent's labels: the Linear sync sets them from who did the work, never his menu.
const AGENT_LABELS: [&str; 2] = ["Claude", "vibe"];

/// The labels he sets from an issue's menu (task rust-issue-sidebar-menus): the set but the agent's.
pub fn settable_labels() -> Vec<&'static str> {
    LABELS
        .into_iter()
        .filter(|l| !AGENT_LABELS.contains(l))
        .collect()
}

/// A task's labels as every view shows them, `Labels.of/2`'s: each of the set once, in the set's spelling (a stored « docs » is
/// Docs), any other word left off.
pub fn labels_of<'a>(held: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut shown: Vec<String> = vec![];
    for word in held {
        let word = word.trim().trim_start_matches(':');
        if let Some(l) = LABELS.iter().find(|l| l.eq_ignore_ascii_case(word))
            && !shown.iter().any(|s| s == l)
        {
            shown.push(l.to_string());
        }
    }
    shown
}

/// Who runs a task, Linear's assignee: the older app's `Tasks.owners/0`, its `:owner`; none is claude.
pub const OWNERS: [&str; 4] = ["claude", "nicolas", "vibe", "rust"];

/// What an issue page's sidebar sets of a task, as the older app holds it (task rust-issue-sidebar-menus): Linear's assignee
/// (`:owner`, none is claude), its labels in the set's spelling, its project (`:epic`), its cycle (a Monday) and the day it
/// reminds him (`:remind`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sidebar {
    pub owner: String,
    pub labels: Vec<String>,
    pub epic: Option<String>,
    pub cycle: Option<String>,
    pub remind: Option<String>,
}

/// Each tasks.edn row's sidebar; `cycle` is a column, the older app's `:remind` and labels sit in its data. A column's field is
/// read from its column alone: its data holds the model's marker there, U+E000 « col » U+E001 (`Model.Row`'s `@col`), and a
/// task with no cycle showed « col » (rust-issue-sidebar-menus' live check). A reminder that is no text is none: a number
/// there would fail every row's read.
const SIDEBAR: &str = "SELECT id, ltrim(coalesce(nullif(owner, ''), 'claude'), ':'),
    (SELECT group_concat(value, char(10)) FROM json_each(data, '$.labels') WHERE json_type(data, '$.labels') = 'array'),
    nullif(ltrim(coalesce(epic, ''), ':'), ''),
    nullif(cycle, ''),
    CASE json_type(data, '$.remind') WHEN 'text' THEN nullif(json_extract(data, '$.remind'), '') END
  FROM tasks WHERE collection = 'tasks.edn' AND id IS NOT NULL";

fn sidebar_row(r: &rusqlite::Row) -> rusqlite::Result<(String, Sidebar)> {
    let labels: Option<String> = r.get(2)?;
    let side = Sidebar {
        owner: r.get(1)?,
        labels: labels_of(labels.as_deref().unwrap_or("").lines()),
        epic: r.get(3)?,
        cycle: r.get(4)?,
        remind: r.get(5)?,
    };
    Ok((r.get(0)?, side))
}

/// Every task's sidebar in `db`, by id: what the board's reader keeps for the issue pages, one read a minute.
pub fn sidebars(db: &Path) -> Result<std::collections::BTreeMap<String, Sidebar>, String> {
    let read = |db: &Connection| -> rusqlite::Result<_> {
        db.prepare(SIDEBAR)?.query_map([], sidebar_row)?.collect()
    };
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// Task `id`'s sidebar in `db`, none when the list has no such task: what a menu's write starts from, read as it stands.
pub fn sidebar(db: &Path, id: &str) -> Result<Option<Sidebar>, String> {
    let sql = format!("{SIDEBAR} AND id = ?1");
    match opened(db)?.query_row(&sql, [id], sidebar_row) {
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        read => read
            .map(|(_, s)| Some(s))
            .map_err(|e| format!("{}: {e}", db.display())),
    }
}

/// The projects a task may join, the epics tasks.edn names at any stage (`Write.property("epic")` takes no other), in
/// order of name: the issue page's « Add to project » menu.
pub fn epics(db: &Path) -> Result<Vec<String>, String> {
    let sql = "SELECT ltrim(epic, ':') AS e FROM tasks WHERE collection = 'tasks.edn' AND trim(coalesce(epic, '')) NOT IN ('', ':')
               GROUP BY e ORDER BY e";
    let read = |db: &Connection| -> rusqlite::Result<Vec<String>> {
        db.prepare(sql)?.query_map([], |r| r.get(0))?.collect()
    };
    read(&opened(db)?).map_err(|e| format!("{}: {e}", db.display()))
}

/// A task's words, as `Launch.argv/2` read them: its run's kind before its own, plan-first on the row or its run.
const FILED: &str = "SELECT coalesce(title, ''), coalesce(why, ''), coalesce(done_when, ''),
    coalesce(json_extract(data, '$.run.prompt'), ''),
    ltrim(coalesce(json_extract(data, '$.run.kind'), json_extract(data, '$.kind'), ''), ':'), coalesce(epic, ''),
    -- a path absent is NULL, and NULL OR false is NULL: none is false
    coalesce(plan_first = 1 OR json_extract(data, '$.\"plan-first\"') = 1 OR json_extract(data, '$.run.\"plan-first\"') = 1, 0),
    CASE WHEN CAST(json_extract(data, '$.run.turns') AS INTEGER) > 0 THEN CAST(json_extract(data, '$.run.turns') AS INTEGER) END
  FROM tasks WHERE collection = 'tasks.edn' AND id = ?1";
/// His comments still waiting on Claude, in his order, as `Comments.untreated_comments/0` reads them: not resolved, and no
/// Claude reply after his last word. A reviewer's run (review-<id>) gets those on the task it reviews too (`Watch.owners/1`).
const COMMENTS: &str =
    "SELECT coalesce(id, ''), coalesce(file, ''), coalesce(quote, ''), coalesce(text, '')
  FROM comments WHERE (task = ?1 OR 'review-' || task = ?1) AND coalesce(stage, '') <> 'resolved'
    AND coalesce(json_extract(data, '$.replies[#-1].by'), '') <> 'claude'
  ORDER BY pos";
/// Its Linear key, as `dash linear sync` linked it: a row without a Linear address is none (`Dashboard.Linear.links/0`).
const LINEAR: &str = "SELECT json_extract(data, '$.identifier') FROM records
  WHERE collection = 'linear.edn' AND id = ?1 AND json_extract(data, '$.url') LIKE 'https://linear.app/%'";
/// The overall focuses, `Focus.overall/0`: the roadmap's Urgent items, then its Now ones, horizons read as ROADMAP reads them.
const FOCUS: &str = concat!(
    "SELECT id, coalesce(title, id) FROM (SELECT id, title, pos, ",
    horizon!(),
    " AS h FROM records WHERE collection = 'focus-overall.edn' AND id IS NOT NULL)
  WHERE h IN ('urgent', 'now') ORDER BY h = 'now', pos"
);

/// What task `id`'s brief is written from, none when the list has no such task (task brief-tests-prepush-skip: Rust writes
/// the brief, never `dash task brief`).
pub fn brief(db: &Path, id: &str) -> Result<Option<Filed>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let row = db_.query_row(FILED, [id], |r| {
        Ok(Filed {
            id: id.to_string(),
            title: r.get(0)?,
            why: r.get(1)?,
            done_when: r.get(2)?,
            prompt: r.get(3)?,
            kind: r.get(4)?,
            epic: r.get(5)?,
            plan_first: r.get(6)?,
            turns: r.get(7)?,
            ..Filed::default()
        })
    });
    let mut filed = match row {
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        row => row.map_err(named)?,
    };
    let mut q = db_.prepare(COMMENTS).map_err(named)?;
    let comments = q.query_map([id], |r| {
        let (id, file, quote, text) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
        Ok(Comment {
            id,
            file,
            quote,
            text,
        })
    });
    filed.comments = comments.and_then(Iterator::collect).map_err(named)?;
    let linear = db_.query_row(LINEAR, [id], |r| r.get::<_, Option<String>>(0));
    filed.linear = match linear {
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        key => key.map_err(named)?.filter(|k| !k.is_empty()),
    };
    let mut q = db_.prepare(FOCUS).map_err(named)?;
    let focus = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)));
    filed.focus = focus.and_then(Iterator::collect).map_err(named)?;
    Ok(Some(filed))
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

/// A lead or a task as /search lists them (crates/dash/src/search.rs, FindLive's port, task rust-search-port): its kind, id,
/// title, the line under it and the words beside those it is found by, as `Dashboard.Search`'s rows were.
#[derive(Debug, Clone, PartialEq)]
pub struct Searchable {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub line: String,
    pub words: String,
}

/// The leads in their order (a key starting with _ is their template), then the tasks of the list in theirs. A lead by its
/// company, where it came from, its stage, place, length and stack; a task by its id, stage and why.
const SEARCHABLE: &str = "SELECT kind, id, title, line, words FROM (
    SELECT 'lead' AS kind, 0 AS n, pos, id, coalesce(nullif(title, ''), id) AS title,
      concat_ws(' · ', nullif(company, ''), nullif(via, ''), nullif(stage, '')) AS line,
      concat_ws(' ', location, duration, (SELECT group_concat(value, ' ') FROM json_each(data, '$.stack')
                                          WHERE json_type(data, '$.stack') = 'array')) AS words
    FROM leads WHERE collection = 'opportunities' AND id IS NOT NULL AND coalesce(key, '') NOT LIKE '\\_%' ESCAPE '\\'
    UNION ALL
    SELECT 'task', 1, pos, id, coalesce(nullif(title, ''), id), concat_ws(' · ', id, stage), coalesce(why, '')
    FROM tasks WHERE collection = 'tasks.edn' AND id IS NOT NULL)
  ORDER BY n, pos";

/// Every lead and task in `db`, the older app's database.
pub fn searchable(db: &Path) -> Result<Vec<Searchable>, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let db_ = opened(db)?;
    let mut q = db_.prepare(SEARCHABLE).map_err(named)?;
    let rows = q.query_map([], |r| {
        Ok(Searchable {
            kind: r.get(0)?,
            id: r.get(1)?,
            title: r.get(2)?,
            line: r.get(3)?,
            words: r.get(4)?,
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

fn roadmap(db: &Connection) -> rusqlite::Result<Vec<Item>> {
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
    items.collect()
}

/// The tasks of the list named in ?1, a line apart, at any stage: each id, whether the older app closed it (`done` or `dropped`:
/// it left the board) and its roadmap horizon (urgent, now, next, later) as the plan finds it, none on no item. What /comments
/// draws a filed comment's card from (task comments-board-pill).
const PLACES: &str = "SELECT t.id, coalesce(t.stage, 'open') IN ('done', 'dropped'),
    (SELECT group_concat(value, char(10)) FROM json_each(t.data, '$.labels') WHERE json_type(t.data, '$.labels') = 'array'),
    t.epic, ltrim(json_extract(t.data, '$.roadmap'), ':')
  FROM tasks t WHERE t.collection = 'tasks.edn'
    AND instr(char(10) || ?1 || char(10), char(10) || t.id || char(10)) > 0";

/// A task's place in the list: closed, and its roadmap horizon.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub closed: bool,
    pub horizon: Option<String>,
}

fn places_in(db: &Connection, ids: &[String]) -> rusqlite::Result<Vec<(String, Place)>> {
    let items = roadmap(db)?;
    let mut rows = db.prepare(PLACES)?;
    let rows = rows.query_map([ids.join("\n")], |r| {
        let id: String = r.get(0)?;
        let labels: Option<String> = r.get(2)?;
        let labels: Vec<String> = labels
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        let (epic, own): (Option<String>, Option<String>) = (r.get(3)?, r.get(4)?);
        let horizon = item_of(&items, &id, epic.as_deref(), &labels, own.as_deref()).map(|i| i.1);
        let closed = r.get(1)?;
        Ok((id, Place { closed, horizon }))
    })?;
    rows.collect()
}

/// The place of each task of `ids` in `db`, the older app's database; a task it does not hold has none.
pub fn places(db: &Path, ids: &[String]) -> Result<Vec<(String, Place)>, String> {
    places_in(&opened(db)?, ids).map_err(|e| format!("{}: {e}", db.display()))
}

fn read(db: &Connection) -> rusqlite::Result<Queue> {
    let items = roadmap(db)?;
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
            demand: Demand::of(r.get::<_, Option<String>>(10)?.as_deref()),
            // Rust's store resolves it at each read of its queue
            route: None,
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
        title TEXT, blocked_by TEXT, owner TEXT, priority INTEGER, epic TEXT, cycle TEXT);
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

    /// Task rust-issue-sidebar-menus, done-when 1: the label list his menu offers is the set but the agent's, and a task's
    /// labels read in the set's spelling, once, any other word left off.
    #[test]
    fn the_label_list_is_the_set_s_and_a_task_s_labels_read_in_its_spelling() {
        let _budget = TestBudget::new();
        assert_eq!(
            settable_labels(),
            [
                "Bug",
                "Feature",
                "Improvement",
                "Research",
                "Docs",
                "Design",
                "Infra",
                "Security",
                "Perf",
                "Unplanned"
            ]
        );
        assert_eq!(
            labels_of(["docs", ":bug", "Docs", "site_perso", "claude"]),
            ["Docs", "Bug", "Claude"]
        );
    }

    /// Task rust-issue-sidebar-menus, done-when 1: each task's sidebar is read off its row, an owner none being claude, its
    /// labels in the set's spelling, its cycle from the column; the epic list is every stage's epics, once, by name.
    #[test]
    fn a_task_s_sidebar_and_the_epic_list_are_read_off_the_list() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("older-sidebar");
        let file = dir.0.join("older.db");
        let db = Connection::open(&file).expect("db");
        db.execute_batch(OLDER).expect("tables");
        task(
            &db,
            1,
            "a",
            "open",
            "rust",
            r#"{"labels":["docs","site_perso"],"remind":"2026-11-13"}"#,
            None,
        );
        // b's reminder is no text: none, every other row still read
        task(&db, 2, "b", "done", "", r#"{"remind":1}"#, None);
        // c's cycle was cleared: the model leaves its column's marker in its data, its column empty
        task(
            &db,
            3,
            "c",
            "open",
            "nicolas",
            "{\"cycle\":\"\u{E000}col\u{E001}\"}",
            None,
        );
        db.execute(
            "UPDATE tasks SET epic = 'rust-poc', cycle = '2026-11-09' WHERE id = 'a'",
            [],
        )
        .expect("a's epic");
        db.execute("UPDATE tasks SET epic = ':crm' WHERE id = 'b'", [])
            .expect("b's epic");
        db.execute("UPDATE tasks SET epic = 'rust-poc' WHERE id = 'c'", [])
            .expect("c's epic");
        let a = Sidebar {
            owner: "rust".into(),
            labels: vec!["Docs".into()],
            epic: Some("rust-poc".into()),
            cycle: Some("2026-11-09".into()),
            remind: Some("2026-11-13".into()),
        };
        assert_eq!(sidebar(&file, "a"), Ok(Some(a.clone())));
        assert_eq!(sidebar(&file, "z"), Ok(None));
        let all = sidebars(&file).expect("every sidebar");
        assert_eq!(all.get("a"), Some(&a));
        assert_eq!(all["b"].owner, "claude", "none is claude");
        assert_eq!(all["b"].epic.as_deref(), Some("crm"));
        assert_eq!(all["c"].cycle, None, "the column's marker is no cycle");
        assert_eq!(all["b"].remind, None, "a number is no reminder");
        assert_eq!(epics(&file), Ok(vec!["crm".to_string(), "rust-poc".into()]));
    }

    /// Task rust-elixir-reference: the copy the frozen app reads holds the rows, and a second copy replaces the first.
    #[test]
    fn a_snapshot_copies_the_database_and_replaces_the_last_one() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("snapshot");
        let (db, to) = (dir.0.join("live.db"), dir.0.join("copy.db"));
        let live = Connection::open(&db).expect("db");
        live.execute_batch(OLDER).expect("tables");
        task(&live, 1, "a", "open", "rust", "{}", None);
        snapshot(&db, &to).expect("the first copy");
        task(&live, 2, "b", "open", "rust", "{}", None);
        snapshot(&db, &to).expect("a copy over the first");
        let copy = Connection::open(&to).expect("the copy");
        let n: i64 = copy
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .expect("its rows");
        assert_eq!(n, 2);
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
        // RFC 20h's demand: an effort high is hard, a task's own word wins over its effort (task rust-pools-fable, done-when 2)
        let high = "UPDATE tasks SET data = json_set(data, '$.run', json('{\"effort\":\":high\"}')) WHERE id = 'a'";
        db.execute(high, []).expect("a's effort");
        let own = "UPDATE tasks SET data = json_set(data, '$.run', json('{\"effort\":\"high\",\"demand\":\"normal\"}'))
          WHERE id = 'words'";
        db.execute(own, []).expect("words' demand");
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
        let demands: Vec<Demand> = queue.tasks.iter().map(|t| t.demand).collect();
        assert_eq!(demands, [Demand::Hard, Demand::Normal, Demand::Normal]);
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

    #[test]
    fn a_task_s_stage_and_newest_report_read_none_when_absent() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("older-stage");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(OLDER).expect("schema");
        let reports = "CREATE TABLE task_reports (id INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT NOT NULL, at TEXT NOT NULL, record TEXT NOT NULL);";
        db.execute_batch(reports).expect("reports");
        task(
            &db,
            0,
            "morning-run",
            "done",
            "",
            r#"{"run":{"prompt":"Morning run"}}"#,
            None,
        );
        task(&db, 1, "bare", "open", "", "{}", None);
        assert_eq!(
            stage(&path, "morning-run").unwrap(),
            Some(("done".into(), Some("Morning run".into())))
        );
        assert_eq!(stage(&path, "bare").unwrap(), Some(("open".into(), None)));
        assert_eq!(stage(&path, "gone").unwrap(), None);
        assert_eq!(last_report(&path, "morning-run").unwrap(), None);
        for r in ["{\"n\":1}", "{\"n\":2}"] {
            db.execute(
                "INSERT INTO task_reports (task, at, record) VALUES ('morning-run', 'now', ?1)",
                [r],
            )
            .expect("report");
        }
        assert_eq!(
            last_report(&path, "morning-run").unwrap(),
            Some((2, "{\"n\":2}".into()))
        );
    }

    /// Task comments-board-pill: a stand-in list's tasks named, at any owner and stage, each closed once done or dropped, its
    /// horizon its roadmap item's (its own, else a cover, else its epic), none on no item; a task not named is not read.
    #[test]
    fn every_task_s_place_is_its_stage_and_its_item_s_horizon() {
        let _budget = TestBudget::new();
        let db = Connection::open_in_memory().expect("db");
        db.execute_batch(OLDER).expect("tables");
        task(
            &db,
            0,
            "shipped",
            "done",
            "claude",
            r#"{"roadmap":":trip"}"#,
            None,
        );
        task(&db, 1, "filed", "open", "rust", "{}", None);
        task(&db, 2, "loose", "dropped", "claude", "{}", None);
        task(&db, 3, "unnamed", "open", "claude", "{}", None);
        db.execute("UPDATE tasks SET epic = 'rust' WHERE id = 'filed'", [])
            .expect("its epic");
        let item = "INSERT INTO records VALUES ('focus-overall.edn', ?1, ?2, ?3)";
        for (pos, id, data) in [
            (0, "rust-poc", r#"{"covers":["rust"]}"#),
            (1, "trip", r#"{"horizon":":later"}"#),
        ] {
            db.execute(item, rusqlite::params![pos, id, data])
                .expect("an item");
        }
        let place = |closed: bool, horizon: Option<&str>| Place {
            closed,
            horizon: horizon.map(String::from),
        };
        let named = ["shipped", "filed", "loose", "gone"].map(String::from);
        let mut got = places_in(&db, &named).expect("the places");
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            [
                ("filed".to_string(), place(false, Some("now"))),
                ("loose".to_string(), place(true, None)),
                ("shipped".to_string(), place(true, Some("later"))),
            ]
        );
    }

    /// Done-when 1 of brief-tests-prepush-skip: a brief is written from the task's row, his comments waiting on it, its Linear key
    /// and his Urgent then Now focuses, read here, never asked of the older app.
    #[test]
    fn a_brief_reads_its_row_comments_linear_key_and_focuses() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("older-brief");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT,
               why TEXT, done_when TEXT, epic TEXT, plan_first INTEGER);
             CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, title TEXT, data TEXT NOT NULL);
             CREATE TABLE comments (pos INTEGER, id TEXT, stage TEXT, task TEXT, file TEXT, quote TEXT, text TEXT,
               data TEXT NOT NULL DEFAULT '{}');
             INSERT INTO tasks VALUES ('tasks.edn', 0, 'a', 'open',
               '{\"run\":{\"prompt\":\"Do it\",\"kind\":\"writing\",\"turns\":12,\"plan-first\":true}}',
               'A', 'Why', '1. It holds', 'speed', NULL);
             INSERT INTO tasks VALUES ('tasks.edn', 1, 'b', 'open', '{\"kind\":\"data\"}', 'B', NULL, NULL, NULL, 0);
             INSERT INTO comments VALUES (1, 'c2', 'open', 'a', '/p', NULL, 'second', '{}');
             INSERT INTO comments VALUES (0, 'c1', 'open', 'a', '/p', 'here', 'first', '{}');
             INSERT INTO comments VALUES (2, 'c3', 'resolved', 'a', '/p', NULL, 'done', '{}');
             INSERT INTO comments VALUES (3, 'c4', 'open', 'a', '/p', NULL, 'answered',
               '{\"replies\":[{\"by\":\"claude\",\"text\":\"Done\"}]}');
             INSERT INTO comments VALUES (4, 'c5', 'open', 'a', '/p', NULL, 'again',
               '{\"replies\":[{\"by\":\"claude\",\"text\":\"Done\"},{\"by\":\"nha\",\"text\":\"No\"}]}');
             INSERT INTO tasks VALUES ('tasks.edn', 2, 'review-a', 'open', '{}', 'R', NULL, NULL, NULL, 0);
             INSERT INTO records VALUES ('linear.edn', 0, 'a', NULL,
               '{\"identifier\":\"AAA-1\",\"url\":\"https://linear.app/x/issue/AAA-1\"}');
             INSERT INTO records VALUES ('linear.edn', 1, 'b', NULL, '{\"identifier\":\"AAA-2\",\"url\":\"hand-made\"}');
             INSERT INTO records VALUES ('focus-overall.edn', 0, 'now-one', 'Now one', '{}');
             INSERT INTO records VALUES ('focus-overall.edn', 1, 'later', 'Later', '{\"horizon\":\"~:later\"}');
             INSERT INTO records VALUES ('focus-overall.edn', 2, 'fire', 'Fire', '{\"until\":\"2999-01-01\"}');",
        )
        .expect("tables");
        let a = brief(&path, "a").unwrap().expect("a");
        let comment = |id: &str, quote: &str, text: &str| Comment {
            id: id.into(),
            file: "/p".into(),
            quote: quote.into(),
            text: text.into(),
        };
        let focus = vec![
            ("fire".into(), "Fire".into()),
            ("now-one".into(), "Now one".into()),
        ];
        let want = Filed {
            id: "a".into(),
            title: "A".into(),
            why: "Why".into(),
            done_when: "1. It holds".into(),
            prompt: "Do it".into(),
            kind: "writing".into(),
            epic: "speed".into(),
            plan_first: true,
            turns: Some(12),
            linear: Some("AAA-1".into()),
            // c3 resolved and c4 answered last by Claude are not waiting; c5, answered again by him, is
            comments: vec![
                comment("c1", "here", "first"),
                comment("c2", "", "second"),
                comment("c5", "", "again"),
            ],
            focus: focus.clone(),
        };
        assert_eq!(a, want);
        let b = brief(&path, "b").unwrap().expect("b");
        assert_eq!(
            (b.kind.as_str(), b.plan_first, b.linear, b.turns),
            ("data", false, None, None)
        );
        assert!(b.comments.is_empty() && b.focus == focus);
        assert_eq!(brief(&path, "gone").unwrap(), None);
        // a reviewer's run reads the comments on the task it reviews (`Watch.owners/1`)
        assert_eq!(
            brief(&path, "review-a")
                .unwrap()
                .expect("review-a")
                .comments,
            want.comments
        );
    }
}
