//! The older app's comments resolved by Rust (task rust-comments-resolve-port; @nha, 2026-11-10: « remember we move to rust. no
//! more elixir, so if something needs fixing port and then fix »): `Comments.resolve_merged/1` and `resolve_landed/1`, ported.
//! Its pass read every comment at once and crashed the PM loop (Exqlite out of memory, 2026-11-12 13:58); here the comments are
//! read `PAGE` rows at a time and only what a resolve needs is kept: one small summary per ask.
//!
//! A comment resolves when its task merged (its last event met or merged, the list's row done with no PR or a PR merged, or a PR
//! left for review merged, which records the merged event), unless he wrote it after that merge; one no task holds resolves once
//! the commit Claude's last answer names (« Fixed (<sha>): … ») is on main. Each resolve is one row's UPDATE, made only while the
//! row is still open, so his own resolve or the older app's pass in between is never written over, and the older app reads it
//! on its next model read (the `model_version` trigger). A PR's state is the older app's `pr_links` row: one not there is
//! unknown, and a comment waiting on it stays open, as `ForgeKeep`'s unkept PR did.
use crate::older::{COL, STAGE};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::time::Duration;

/// Comments read per query: a pass holds one page of rows, never the table.
pub const PAGE: i64 = 500;
const COMMENTS: &str = "live/comments.edn";

/// A comment resolved: its id and the reply it got.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub id: String,
    pub reply: String,
}

/// One open comment, as a resolve needs it.
struct Open {
    id: String,
    key: u64,
    task: Option<String>,
    at_ms: Option<i64>,
}

/// An ask, the open comment and its twins (the same words on the same passage the same day: `Comments.asked_once/1`), summed:
/// the task of its first open twin, else its first task; the date of its last reply and the commit that reply names as a fix.
#[derive(Default)]
struct Ask {
    base_task: Option<Option<String>>,
    first_task: Option<String>,
    last: Option<(String, Option<String>)>,
}

impl Ask {
    fn task(&self) -> Option<String> {
        self.base_task.clone().flatten().or(self.first_task.clone())
    }
}

/// The open comments and their asks, read a page at a time.
struct Read {
    open: Vec<Open>,
    asks: HashMap<u64, Ask>,
}

impl Read {
    /// Each open comment's task, its own else its ask's (`Comments.as_asked/1`).
    fn task_of(&self, c: &Open) -> Option<String> {
        c.task
            .clone()
            .or_else(|| self.asks.get(&c.key).and_then(Ask::task))
    }
}

fn opened(db: &Path, flags: OpenFlags) -> Result<Connection, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", db.display());
    let mut conn =
        Connection::open_with_flags(db, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(named)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(named)?;
    crate::timed::trace(&mut conn, db).map_err(named)?;
    Ok(conn)
}

fn sql(e: rusqlite::Error) -> String {
    e.to_string()
}

/// « ~:name » is the older app's keyword: its name.
fn plain(s: &str) -> &str {
    s.strip_prefix("~:").unwrap_or(s)
}

/// A field from its column, else from the row's data when held there, never the marker: `COL` for a field its column holds,
/// `STAGE` for `status`. A nil field is a NULL column under the marker too, so the marker never reads as a value
/// (/code-review: a plain « col » written by the first cut showed as the resolved date).
fn field(col: Option<String>, data: &Json, key: &str) -> Option<String> {
    col.or_else(|| {
        data.get(key)
            .and_then(Json::as_str)
            .filter(|v| *v != COL && *v != STAGE)
            .map(str::to_string)
    })
}

fn blank_none(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.is_empty())
}

/// Spaces folded, ends trimmed, lower case: two posts of the same words are one ask (`Comments.plain_words/1`).
fn plain_words(s: Option<&str>) -> Option<String> {
    s.map(|s| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    })
}

/// The commit `text` names as a fix: « Done (cf642552): … » or « Fixed (…) », 7 to 40 hex digits (`Comments.fix_commit/1`).
pub fn fix_commit(text: &str) -> Option<String> {
    let t = text.trim_start();
    let word = ["done", "fixed"]
        .into_iter()
        .find(|w| t.get(..w.len()).is_some_and(|h| h.eq_ignore_ascii_case(w)))?;
    let rest = t[word.len()..].trim_start().strip_prefix('(')?;
    let sha: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    let fits = (7..=40).contains(&sha.len()) && rest[sha.len()..].starts_with(')');
    fits.then_some(sha)
}

/// An ISO time with its zone in Unix milliseconds, as SQLite reads it; none for a day alone or a time with no zone.
const MS: &str =
    "CASE WHEN {t} GLOB '????-??-??T*' AND ({t} GLOB '*Z' OR {t} GLOB '*[+-][0-9][0-9]:[0-9][0-9]')
    THEN CAST(round((julianday({t}) - 2440587.5) * 86400000) AS INTEGER) END";

fn ms_of(t: &str) -> String {
    MS.replace("{t}", t)
}

/// Every comment, `PAGE` rows at a time, summed into the open ones and their asks.
fn read(conn: &Connection) -> Result<Read, String> {
    let q = format!(
        "SELECT pos, id, file, date, quote, path, text, task, stage, at, {}, data FROM comments
         WHERE collection = ?1 AND pos > ?2 ORDER BY pos LIMIT ?3",
        ms_of("at")
    );
    let mut stmt = conn.prepare(&q).map_err(sql)?;
    let mut out = Read {
        open: vec![],
        asks: HashMap::new(),
    };
    let mut after = i64::MIN;
    loop {
        let mut rows = stmt.query(params![COMMENTS, after, PAGE]).map_err(sql)?;
        let mut n = 0;
        while let Some(r) = rows.next().map_err(sql)? {
            n += 1;
            after = r.get(0).map_err(sql)?;
            let data: Json =
                serde_json::from_str(&r.get::<_, String>(11).map_err(sql)?).unwrap_or_default();
            let col = |i: usize, key: &str| -> Result<Option<String>, String> {
                Ok(field(r.get(i).map_err(sql)?, &data, key))
            };
            let Some(id) = col(1, "id")? else { continue };
            let task = col(7, "task")?;
            let mut h = DefaultHasher::new();
            (
                col(2, "file")?,
                col(3, "date")?,
                plain_words(blank_none(col(4, "quote")?).as_deref()),
                blank_none(col(5, "path")?),
                plain_words(Some(col(6, "text")?.unwrap_or_default().as_str())),
            )
                .hash(&mut h);
            let key = h.finish();
            let open = col(8, "status")?.is_none_or(|s| plain(&s) == "open");
            let ask = out.asks.entry(key).or_default();
            if open && ask.base_task.is_none() {
                ask.base_task = Some(task.clone());
            }
            if ask.first_task.is_none() {
                ask.first_task.clone_from(&task);
            }
            // the ask's replies sorted by date, the last one kept: a later reply of an equal date wins, as a stable sort
            for reply in data
                .get("replies")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                let s = |k: &str| reply.get(k).and_then(Json::as_str).map(plain);
                let date = s("date").unwrap_or_default().to_string();
                if ask.last.as_ref().is_none_or(|(d, _)| date >= *d) {
                    let fix = (s("by").unwrap_or("claude") == "claude")
                        .then(|| fix_commit(s("text").unwrap_or_default()))
                        .flatten();
                    ask.last = Some((date, fix));
                }
            }
            if open {
                out.open.push(Open {
                    id,
                    key,
                    task,
                    at_ms: r.get(10).map_err(sql)?,
                });
            }
        }
        if n < PAGE {
            return Ok(out);
        }
    }
}

/// A task's last event: its status, its time in milliseconds and the PR its data names.
struct Event {
    status: String,
    at_ms: Option<i64>,
    pr: Option<String>,
}

/// A task's row of the list: its stage, whether it waits for his review, its PR and project.
#[derive(Default)]
struct Listed {
    stage: Option<String>,
    review: bool,
    pr: Option<String>,
    project: Option<String>,
}

fn last_events(conn: &Connection) -> Result<HashMap<String, Event>, String> {
    let q = format!(
        "SELECT task, status, {}, CASE WHEN json_valid(data) THEN json_extract(data, '$.pr') END FROM task_events
         WHERE id IN (SELECT max(id) FROM task_events GROUP BY task)",
        ms_of("(at || CASE WHEN at GLOB '*Z' THEN '' ELSE 'Z' END)")
    );
    let mut stmt = conn.prepare(&q).map_err(sql)?;
    let rows = stmt
        .query_map([], |r| {
            let pr: Option<rusqlite::types::Value> = r.get(3)?;
            let pr = match pr {
                Some(rusqlite::types::Value::Text(t)) => Some(t),
                Some(rusqlite::types::Value::Integer(n)) => Some(n.to_string()),
                _ => None,
            };
            let e = Event {
                status: r.get(1)?,
                at_ms: r.get(2)?,
                pr,
            };
            Ok((r.get::<_, String>(0)?, e))
        })
        .map_err(sql)?;
    rows.collect::<Result<_, _>>().map_err(sql)
}

fn listed(conn: &Connection) -> Result<HashMap<String, Listed>, String> {
    let q = "SELECT id, stage, coalesce(review, json_extract(data, '$.review') IN (1, 'true')), pr,
               json_extract(data, '$.run.project')
             FROM tasks WHERE collection = 'tasks.edn' AND id IS NOT NULL";
    let mut stmt = conn.prepare(q).map_err(sql)?;
    let rows = stmt
        .query_map([], |r| {
            let t = Listed {
                stage: r.get(1)?,
                review: r.get::<_, Option<i64>>(2)? == Some(1),
                pr: r.get(3)?,
                project: r.get(4)?,
            };
            Ok((r.get::<_, String>(0)?, t))
        })
        .map_err(sql)?;
    rows.collect::<Result<_, _>>().map_err(sql)
}

/// A PR as the older app keeps it: its state (« open », « merged », « closed ») and when it merged.
struct Kept {
    state: String,
    merged_at: Option<String>,
}

/// The kept PRs, by repository and number.
fn kept_prs(conn: &Connection) -> Result<HashMap<(String, u64), Kept>, String> {
    let mut stmt = conn
        .prepare("SELECT url, state, merged_at FROM pr_links")
        .map_err(sql)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Kept {
                    state: r.get::<_, String>(1)?.to_lowercase(),
                    merged_at: r.get(2)?,
                },
            ))
        })
        .map_err(sql)?;
    let mut kept = HashMap::new();
    for row in rows {
        let (url, k) = row.map_err(sql)?;
        if let Some(key) = pr_key(&url) {
            kept.insert(key, k);
        }
    }
    Ok(kept)
}

/// A segment of a link: no slash, no space.
fn segment(s: &str) -> &str {
    let end = s
        .find(|c: char| c == '/' || c.is_whitespace())
        .unwrap_or(s.len());
    &s[..end]
}

/// Each « https://github.com/<owner>/<repo>/pull/<n> » in `s`: the link, the repository and the number.
fn pull_links(s: &str) -> Vec<(String, String, u64)> {
    const AT: &str = "https://github.com/";
    let mut out = vec![];
    for (i, _) in s.match_indices(AT) {
        let rest = &s[i + AT.len()..];
        let owner = segment(rest);
        let Some(rest) = rest[owner.len()..].strip_prefix('/') else {
            continue;
        };
        let repo = segment(rest);
        let Some(rest) = rest[repo.len()..].strip_prefix("/pull/") else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if owner.is_empty() || repo.is_empty() || digits.is_empty() {
            continue;
        }
        let Ok(n) = digits.parse() else { continue };
        out.push((
            format!("{AT}{owner}/{repo}/pull/{digits}"),
            repo.to_string(),
            n,
        ));
    }
    out
}

/// A PR link's repository and number (`Comments.pr_key/1`): the owner is left out, so an old « nha/ » link finds its PR.
fn pr_key(url: &str) -> Option<(String, u64)> {
    let (_, repo, n) = pull_links(url).into_iter().next()?;
    Some((repo, n))
}

/// The PRs a PR field names, as full links (`PM.pr_links/2`): its links, then each « #12 » as a PR of `project`'s repository
/// under `owner`; a field that is a number alone is that « #n ».
pub fn pr_links(pr: &str, project: &str, owner: &str) -> Vec<String> {
    let t = pr.trim();
    let pr = match !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
        true => format!("#{t}"),
        false => pr.to_string(),
    };
    let mut links: Vec<String> = pull_links(&pr).into_iter().map(|(l, _, _)| l).collect();
    let chars: Vec<char> = pr.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let after_word =
            i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '/');
        if *c != '#' || after_word {
            continue;
        }
        let digits: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let next = chars.get(i + 1 + digits.len());
        let bounded = next.is_none_or(|c| !(c.is_alphanumeric() || *c == '_'));
        if !digits.is_empty() && bounded {
            links.push(format!(
                "https://github.com/{owner}/{project}/pull/{digits}"
            ));
        }
    }
    let mut seen = BTreeSet::new();
    links.retain(|l| seen.insert(l.clone()));
    links
}

/// Whether a PR field's PRs merged: merged once none is open and one merged; a closed one counts neither way; one not kept is
/// unknown, so its comments wait (`Comments.merged_by?/2`).
#[derive(Debug, PartialEq)]
enum Merged {
    Yes,
    No,
    Unknown,
}

fn merged(links: &[String], kept: &HashMap<(String, u64), Kept>) -> Merged {
    let mut merged = Merged::No;
    for url in links {
        match pr_key(url)
            .and_then(|k| kept.get(&k))
            .map(|k| k.state.as_str())
        {
            Some("merged") => merged = Merged::Yes,
            Some("closed") => {}
            Some(_) => return Merged::No,
            None => return Merged::Unknown,
        }
    }
    merged
}

/// What a resolve by merge decided for one task: its PR field, and when it merged when an event dates it.
struct Settled {
    pr: String,
    merged_ms: Option<i64>,
}

/// The merged event a PR left for review records, dated by its PRs' last merge (`merged_at`, each with its zone), else now;
/// the older app keeps an event's time in UTC with no zone. Not recorded when the task's last event is already met or merged:
/// the older app's own pass, which runs until task rust-older-jobs-stop, may have recorded it since the read (/code-review).
fn record_merged(
    conn: &Connection,
    task: &str,
    pr: &str,
    merged_at: &[String],
    now_ms: i64,
) -> Result<Option<i64>, String> {
    let now = dash_core::iso(now_ms.div_euclid(1000));
    // the latest instant, not the greatest text: « 12:00+02:00 » is before « 11:00Z »
    let at: String = conn
        .query_row(
            "SELECT coalesce((SELECT strftime('%Y-%m-%dT%H:%M:%S', value) FROM json_each(?1)
               WHERE julianday(value) IS NOT NULL ORDER BY julianday(value) DESC LIMIT 1),
               strftime('%Y-%m-%dT%H:%M:%S', ?2))",
            params![serde_json::json!(merged_at).to_string(), now],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let data = serde_json::json!({ "pr": pr }).to_string();
    conn.execute(
        "INSERT INTO task_events (task, status, at, data) SELECT ?1, 'merged', ?2, ?3
         WHERE coalesce((SELECT status FROM task_events WHERE task = ?1 ORDER BY id DESC LIMIT 1), '')
           NOT IN ('met', 'merged')",
        params![task, at, data],
    )
    .map_err(sql)?;
    // the merge's instant: this event's, or the one recorded since the read
    let q = format!(
        "SELECT {} FROM task_events WHERE task = ?1 ORDER BY id DESC LIMIT 1",
        ms_of("(at || CASE WHEN at GLOB '*Z' THEN '' ELSE 'Z' END)")
    );
    conn.query_row(&q, [task], |r| r.get(0)).map_err(sql)
}

/// The reply that says how it merged, its closed PRs left out (`Comments.merged_reply/3`).
fn merged_reply(task: &str, pr: &str, owner: &str, kept: &HashMap<(String, u64), Kept>) -> String {
    let links: Vec<String> = pr_links(pr, "site_perso", owner)
        .into_iter()
        .filter(|l| {
            !pr_key(l)
                .and_then(|k| kept.get(&k))
                .is_some_and(|k| k.state == "closed")
        })
        .collect();
    match links.is_empty() {
        true => format!("Its task {task} is done (no PR to link)."),
        false => format!("Merged: {}", links.join(", ")),
    }
}

/// Comment `id` resolved with Claude's `reply`, while it is still open: false when it was resolved meanwhile.
fn resolve_one(conn: &Connection, id: &str, reply: &str, today: &str) -> Result<bool, String> {
    let n = conn
        .execute(
            "UPDATE comments SET stage = 'resolved', resolved = ?3,
               data = json_set(data, '$.replies',
                 json_insert(CASE WHEN json_type(data, '$.replies') = 'array' THEN json_extract(data, '$.replies')
                   ELSE '[]' END, '$[#]', json_object('by', 'claude', 'text', ?2, 'date', ?3)),
                 '$.resolved', ?5)
             WHERE collection = ?4 AND id = ?1 AND coalesce(stage, 'open') = 'open'",
            params![id, reply, today, COMMENTS, COL],
        )
        .map_err(sql)?;
    Ok(n > 0)
}

/// What one pass resolved: by its task's merge, by its fix landing, each a read of its own that may fail alone.
pub struct Resolutions {
    pub merged: Result<Vec<Resolved>, String>,
    pub landed: Result<Vec<Resolved>, String>,
}

/// One pass over `db`, the older app's database: one connection, one paged read of its comments, then `resolve_merged`, then
/// `resolve_landed` (a comment one resolved, the other never takes: no task holds a landed one). `owner` is the GitHub owner
/// a « #12 » is under, `today` the day a reply is dated, `now_ms` the time of an event no merge dates, `landed` whether a
/// commit is on main.
pub fn resolve(
    db: &Path,
    owner: &str,
    today: &str,
    now_ms: i64,
    landed: &dyn Fn(&str) -> bool,
) -> Result<Resolutions, String> {
    let conn = opened(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let read = read(&conn)?;
    Ok(Resolutions {
        merged: resolve_merged(&conn, &read, owner, today, now_ms),
        landed: resolve_landed(&conn, &read, today, landed),
    })
}

/// `Comments.resolve_merged/1`: each open comment whose task merged, resolved with the merge's PR as Claude's reply, unless
/// he wrote it after the merge. Every task in review is asked, comments or not, so its merged event is recorded.
fn resolve_merged(
    conn: &Connection,
    read: &Read,
    owner: &str,
    today: &str,
    now_ms: i64,
) -> Result<Vec<Resolved>, String> {
    let events = last_events(conn)?;
    let listed = listed(conn)?;
    let kept = kept_prs(conn)?;
    let none = Listed::default();
    let done = |s: &Option<String>| s.as_deref().map(plain) == Some("done");
    // the tasks asked: those of open comments, then each in review (an in-review event, or done for review not merged)
    let mut tasks: BTreeMap<String, ()> = BTreeMap::new();
    for c in &read.open {
        if let Some(t) = read.task_of(c) {
            tasks.insert(t, ());
        }
    }
    for (t, e) in &events {
        if e.status == "in-review" {
            tasks.insert(t.clone(), ());
        }
    }
    for (t, l) in &listed {
        let last = events.get(t).map(|e| e.status.as_str());
        if done(&l.stage) && l.review && !matches!(last, Some("met" | "merged")) {
            tasks.insert(t.clone(), ());
        }
    }
    let mut settled: HashMap<String, Settled> = HashMap::new();
    for task in tasks.keys() {
        let e = events.get(task);
        let t = listed.get(task).unwrap_or(&none);
        let project = t.project.as_deref().unwrap_or("site_perso");
        let field = e.and_then(|e| e.pr.clone()).or(t.pr.clone());
        let links = field
            .as_deref()
            .map(|f| pr_links(f, project, owner))
            .unwrap_or_default();
        // its PR field as full links, or as it is when it names none
        let pr = match links.is_empty() {
            true => field,
            false => Some(links.join(" ")),
        };
        let status = e.map(|e| e.status.as_str());
        let merged_ms = e
            .filter(|e| matches!(e.status.as_str(), "met" | "merged"))
            .and_then(|e| e.at_ms);
        let decided = if matches!(status, Some("met" | "merged")) {
            Some(Settled {
                pr: pr.unwrap_or_default(),
                merged_ms,
            })
        } else if status == Some("in-review") || t.review {
            match pr {
                Some(pr) if merged(&links, &kept) == Merged::Yes => {
                    let dates: Vec<String> = links
                        .iter()
                        .filter_map(|l| pr_key(l).and_then(|k| kept.get(&k)))
                        .filter_map(|k| k.merged_at.clone())
                        .collect();
                    let merged_ms = record_merged(conn, task, &pr, &dates, now_ms)?;
                    Some(Settled { pr, merged_ms })
                }
                _ => None,
            }
        } else if done(&t.stage) {
            // a task done with no PR resolves; one with a PR once it merged, never when it closed unmerged
            match links.is_empty() || merged(&links, &kept) == Merged::Yes {
                true => Some(Settled {
                    pr: pr.unwrap_or_default(),
                    merged_ms: None,
                }),
                false => None,
            }
        } else {
            None
        };
        if let Some(s) = decided {
            settled.insert(task.clone(), s);
        }
    }
    let mut out = vec![];
    for c in &read.open {
        let Some(task) = read.task_of(c) else {
            continue;
        };
        let Some(s) = settled.get(&task) else {
            continue;
        };
        // a comment written after its task merged is new work, not part of that merge
        if c.at_ms
            .zip(s.merged_ms)
            .is_some_and(|(at, merged)| at > merged)
        {
            continue;
        }
        let reply = merged_reply(&task, &s.pr, owner, &kept);
        if resolve_one(conn, &c.id, &reply, today)? {
            out.push(Resolved {
                id: c.id.clone(),
                reply,
            });
        }
    }
    Ok(out)
}

/// `Comments.resolve_landed/1`: each open comment no task holds whose last reply is Claude's answer naming its fix's commit,
/// once `landed` says that commit is on main.
fn resolve_landed(
    conn: &Connection,
    read: &Read,
    today: &str,
    landed: &dyn Fn(&str) -> bool,
) -> Result<Vec<Resolved>, String> {
    let mut out = vec![];
    for c in &read.open {
        if read.task_of(c).is_some() {
            continue;
        }
        let fix = read
            .asks
            .get(&c.key)
            .and_then(|a| a.last.as_ref())
            .and_then(|(_, f)| f.clone());
        let Some(sha) = fix.filter(|sha| landed(sha)) else {
            continue;
        };
        let reply = format!("On main and running: {sha}.");
        if resolve_one(conn, &c.id, &reply, today)? {
            out.push(Resolved {
                id: c.id.clone(),
                reply,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use dash_meter::memory::{TestBudget, memory};
    use serde_json::json;

    /// The older app's tables as a resolve reads them: the columns it names, nothing else.
    fn older(dir: &Path) -> (std::path::PathBuf, Connection) {
        let db = dir.join("dashboard_dev.db");
        let conn = Connection::open(&db).expect("open");
        conn.execute_batch(
            "CREATE TABLE comments (collection TEXT, pos INTEGER, key TEXT, id TEXT, name TEXT, stage TEXT, data TEXT NOT NULL,
               file TEXT, text TEXT, date TEXT, quote TEXT, path TEXT, resolved TEXT, task TEXT, at TEXT,
               PRIMARY KEY (collection, pos));
             CREATE TABLE task_events (id INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT NOT NULL, status TEXT NOT NULL,
               at TEXT NOT NULL, data TEXT DEFAULT '{}' NOT NULL);
             CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, pr TEXT, review INTEGER,
               PRIMARY KEY (collection, pos));
             CREATE TABLE pr_links (url TEXT PRIMARY KEY, project TEXT NOT NULL, state TEXT NOT NULL, merged_at TEXT);
             PRAGMA journal_mode = WAL;",
        )
        .expect("schema");
        (db, conn)
    }

    struct C<'a> {
        id: &'a str,
        task: Option<&'a str>,
        text: &'a str,
        at: &'a str,
        open: bool,
        replies: Json,
    }

    fn comment(conn: &Connection, pos: i64, c: C) {
        // as the older app keeps it: each field under the marker, a nil task too (its column NULL)
        let data = json!({"id": COL, "file": COL, "text": COL, "date": COL, "at": COL, "status": STAGE,
                          "task": COL, "replies": c.replies});
        conn.execute(
            "INSERT INTO comments (collection, pos, id, stage, data, file, text, date, task, at)
             VALUES ('live/comments.edn', ?1, ?2, ?3, ?4, '/design', ?5, '2026-11-01', ?6, ?7)",
            params![
                pos,
                c.id,
                if c.open { "open" } else { "resolved" },
                data.to_string(),
                c.text,
                c.task,
                c.at
            ],
        )
        .expect("comment");
    }

    fn event(conn: &Connection, task: &str, status: &str, at: &str, data: Json) {
        conn.execute(
            "INSERT INTO task_events (task, status, at, data) VALUES (?1, ?2, ?3, ?4)",
            params![task, status, at, data.to_string()],
        )
        .expect("event");
    }

    fn task(conn: &Connection, pos: i64, id: &str, stage: &str, pr: Option<&str>, review: bool) {
        let data = json!({"id": id, "status": "stage", "run": {"project": "site_perso"}});
        conn.execute(
            "INSERT INTO tasks (collection, pos, id, stage, data, pr, review) VALUES ('tasks.edn', ?1, ?2, ?3, ?4, ?5, ?6)",
            params![pos, id, stage, data.to_string(), pr, review],
        )
        .expect("task");
    }

    fn pr(conn: &Connection, n: u64, state: &str, merged_at: Option<&str>) {
        conn.execute(
            "INSERT INTO pr_links (url, project, state, merged_at) VALUES (?1, 'site_perso', ?2, ?3)",
            params![format!("https://github.com/nha-org/site_perso/pull/{n}"), state, merged_at],
        )
        .expect("pr");
    }

    /// Each comment's stage and its last reply's words.
    fn state(conn: &Connection, id: &str) -> (String, String) {
        conn.query_row(
            "SELECT stage, coalesce(json_extract(data, '$.replies[#-1].text'), '') FROM comments WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("state")
    }

    const LINK: &str = "https://github.com/nha-org/site_perso/pull/";

    /// Done-when 1, merged: `Comments.resolve_merged/1`'s cases, each read off the store after the pass.
    #[dash_test]
    fn a_comment_resolves_once_its_task_merged_never_one_written_after_the_merge() {
        let dir = dash_meter::TestDir::new("resolve-merged");
        let (db, conn) = older(&dir.0);
        let none = || json!([]);
        // a: met at 10:00, its PR in the event
        event(&conn, "a", "started", "2026-11-01T08:00:00", json!({}));
        event(
            &conn,
            "a",
            "met",
            "2026-11-01T10:00:00.123456",
            json!({"pr": format!("{LINK}1")}),
        );
        comment(
            &conn,
            1,
            C {
                id: "c1",
                task: Some("a"),
                text: "Fix  the Menu",
                at: "2026-11-01T09:00:00Z",
                open: true,
                replies: none(),
            },
        );
        // its twin, the same words spaced and cased otherwise, no task of its own: resolved with the ask's
        comment(
            &conn,
            2,
            C {
                id: "c1-twin",
                task: None,
                text: "fix the menu ",
                at: "2026-11-01T09:00:01Z",
                open: true,
                replies: none(),
            },
        );
        // written after the merge: new work, it stays open
        comment(
            &conn,
            3,
            C {
                id: "c2",
                task: Some("a"),
                text: "and the footer",
                at: "2026-11-01T11:00:00Z",
                open: true,
                replies: none(),
            },
        );
        // b: done for review, its « #5 » merged at 12:00 by the kept PRs: the merged event is recorded at that time
        task(&conn, 1, "b", "done", Some("#5 ready for review"), true);
        pr(&conn, 5, "merged", Some("2026-11-01T12:00:00Z"));
        comment(
            &conn,
            4,
            C {
                id: "c3",
                task: Some("b"),
                text: "b's ask",
                at: "2026-11-01T11:30:00Z",
                open: true,
                replies: none(),
            },
        );
        // c: in review, its PR not kept: unknown, the comment waits
        event(
            &conn,
            "c",
            "in-review",
            "2026-11-01T09:00:00",
            json!({"pr": format!("{LINK}6")}),
        );
        comment(
            &conn,
            5,
            C {
                id: "c4",
                task: Some("c"),
                text: "c's ask",
                at: "2026-11-01T08:00:00Z",
                open: true,
                replies: none(),
            },
        );
        // d: done with no PR: resolved, said so
        task(&conn, 2, "d", "done", None, false);
        comment(
            &conn,
            6,
            C {
                id: "c5",
                task: Some("d"),
                text: "d's ask",
                at: "2026-11-01T08:00:00Z",
                open: true,
                replies: none(),
            },
        );
        // f: done, its PR closed unmerged: never resolved
        task(&conn, 3, "f", "done", Some(&format!("{LINK}7")), false);
        pr(&conn, 7, "closed", None);
        comment(
            &conn,
            7,
            C {
                id: "c6",
                task: Some("f"),
                text: "f's ask",
                at: "2026-11-01T08:00:00Z",
                open: true,
                replies: none(),
            },
        );
        // e: in review with no comment, its two PRs merged: its merged event is recorded all the same, at the later merge in
        // time, never the greater text (« 14:00+02:00 » is 12:00 UTC, before « 13:00Z »)
        event(
            &conn,
            "e",
            "in-review",
            "2026-11-01T09:00:00",
            json!({"pr": format!("{LINK}8 {LINK}9")}),
        );
        pr(&conn, 8, "merged", Some("2026-11-01T13:00:00Z"));
        pr(&conn, 9, "merged", Some("2026-11-01T14:00:00+02:00"));
        // already resolved by him: untouched
        comment(
            &conn,
            8,
            C {
                id: "c7",
                task: Some("a"),
                text: "his own",
                at: "2026-11-01T08:00:00Z",
                open: false,
                replies: none(),
            },
        );

        let got = (resolve(&db, "nha-org", "2026-11-02", 0, &|_| false)
            .expect("resolve")
            .merged)
            .expect("merged");
        let ids: Vec<&str> = got.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["c1", "c1-twin", "c3", "c5"], "{got:?}");
        assert_eq!(
            state(&conn, "c1"),
            ("resolved".into(), format!("Merged: {LINK}1"))
        );
        assert_eq!(
            state(&conn, "c1-twin"),
            ("resolved".into(), format!("Merged: {LINK}1"))
        );
        assert_eq!(
            state(&conn, "c3"),
            ("resolved".into(), format!("Merged: {LINK}5"))
        );
        assert_eq!(
            state(&conn, "c5"),
            (
                "resolved".into(),
                "Its task d is done (no PR to link).".into()
            )
        );
        for open in ["c2", "c4", "c6"] {
            assert_eq!(state(&conn, open).0, "open", "{open}");
        }
        assert_eq!(state(&conn, "c7"), ("resolved".into(), String::new()));
        let resolved: String = conn
            .query_row("SELECT resolved || ' ' || json_extract(data, '$.resolved') FROM comments WHERE id = 'c1'", [], |r| r.get(0))
            .expect("resolved");
        assert_eq!(resolved, format!("2026-11-02 {COL}"));
        let merged: Vec<(String, String, String)> = conn
            .prepare("SELECT task, at, json_extract(data, '$.pr') FROM task_events WHERE status = 'merged' ORDER BY id")
            .and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect())
            .expect("events");
        let want = |t: &str, at: &str, n: u8| (t.to_string(), at.to_string(), format!("{LINK}{n}"));
        assert_eq!(
            merged,
            [
                want("b", "2026-11-01T12:00:00", 5),
                (
                    "e".into(),
                    "2026-11-01T13:00:00".into(),
                    format!("{LINK}8 {LINK}9")
                )
            ]
        );
        // the older app's pass recorded it since the read: no second merged event
        // its instant is the event already there, b's merge at 12:00 UTC
        assert_eq!(
            record_merged(&conn, "b", "x", &[], 0),
            Ok(Some(1793534400000))
        );
        let events: i64 = conn
            .query_row(
                "SELECT count(*) FROM task_events WHERE status = 'merged'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(events, 2);
        // a second pass finds nothing more: c3 was written before b's merge, the others wait or are new work
        let again = resolve(&db, "nha-org", "2026-11-02", 0, &|_| false).expect("again");
        assert_eq!(again.merged, Ok(vec![]));
        println!("resolved {ids:?}; merged events {merged:?}");
    }

    /// Done-when 1, landed: `Comments.resolve_landed/1`'s cases.
    #[dash_test]
    fn a_comment_no_task_holds_resolves_once_the_fix_its_answer_names_is_on_main() {
        let dir = dash_meter::TestDir::new("resolve-landed");
        let (db, conn) = older(&dir.0);
        let said = |by: &str, text: &str, date: &str| json!({"by": by, "text": text, "date": date});
        let fixed = |sha: &str| {
            json!([said(
                "claude",
                &format!("Fixed ({sha}): the menu"),
                "2026-11-01"
            )])
        };
        comment(
            &conn,
            1,
            C {
                id: "on-main",
                task: None,
                text: "one",
                at: "",
                open: true,
                replies: fixed("abc1234"),
            },
        );
        comment(
            &conn,
            2,
            C {
                id: "not-yet",
                task: None,
                text: "two",
                at: "",
                open: true,
                replies: fixed("def5678"),
            },
        );
        // a task holds it: the merge resolves it, not the commit
        comment(
            &conn,
            3,
            C {
                id: "tasked",
                task: Some("x"),
                text: "three",
                at: "",
                open: true,
                replies: fixed("abc1234"),
            },
        );
        // he answered after the fix: his word is last
        let his = json!([
            said("claude", "Done (abc1234): it", "2026-11-01"),
            said("nicolas", "still broken", "2026-11-02")
        ]);
        comment(
            &conn,
            4,
            C {
                id: "his-last",
                task: None,
                text: "four",
                at: "",
                open: true,
                replies: his,
            },
        );
        // the twin of a comment Claude answered: the ask's last reply is that answer
        comment(
            &conn,
            5,
            C {
                id: "answered",
                task: None,
                text: "five",
                at: "",
                open: false,
                replies: json!([said("claude", "done (ABC1234) yes", "2026-11-01")]),
            },
        );
        comment(
            &conn,
            6,
            C {
                id: "its-twin",
                task: None,
                text: "Five",
                at: "",
                open: true,
                replies: json!([]),
            },
        );
        // a short sha is no fix
        comment(
            &conn,
            7,
            C {
                id: "short",
                task: None,
                text: "seven",
                at: "",
                open: true,
                replies: fixed("abc12"),
            },
        );

        let landed = |sha: &str| sha.eq_ignore_ascii_case("abc1234");
        let got = (resolve(&db, "nha-org", "2026-11-03", 0, &landed)
            .expect("resolve")
            .landed)
            .expect("landed");
        let ids: Vec<&str> = got.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["on-main", "its-twin"], "{got:?}");
        assert_eq!(
            state(&conn, "on-main"),
            ("resolved".into(), "On main and running: abc1234.".into())
        );
        assert_eq!(
            state(&conn, "its-twin"),
            ("resolved".into(), "On main and running: ABC1234.".into())
        );
        for open in ["not-yet", "tasked", "his-last", "short"] {
            assert_eq!(state(&conn, open).0, "open", "{open}");
        }
        assert_eq!(
            fix_commit("  Fixed (dad5b1c5): the menu"),
            Some("dad5b1c5".into())
        );
        assert_eq!(fix_commit("Fixed dad5b1c5"), None);
        assert_eq!(fix_commit("Done (dad5b1c5 and more)"), None);
        println!("resolved {ids:?}");
    }

    /// Comments in the peak test, and each one's replies: about 6 KB a comment, 60 MB in all, past the 50 MB budget had
    /// they been read at once as `Comments.all/0` read them.
    const MANY: i64 = 10_000;
    const REPLY: usize = 1_000;

    /// Done-when 2: both resolves over 10,000 comments read a page at a time, inside the test's 50 MB budget; its peak printed.
    #[dash_test(TestBudget::of(50.0))]
    fn ten_thousand_comments_resolve_a_page_at_a_time_inside_the_budget() {
        let dir = dash_meter::TestDir::new("resolve-many");
        let (db, conn) = older(&dir.0);
        let words = "w".repeat(REPLY);
        event(
            &conn,
            "merged-task",
            "met",
            "2026-11-01T10:00:00",
            json!({"pr": format!("{LINK}1")}),
        );
        conn.execute_batch("BEGIN").expect("begin");
        let mut bytes = 0;
        for i in 0..MANY {
            let replies: Vec<Json> = (0..6).map(|r| json!({"by": "claude", "text": format!("{r} {words}"), "date": "2026-11-01"})).collect();
            let task = (i % 100 == 0).then_some("merged-task");
            let mut replies = json!(replies);
            if i % 100 == 50 {
                replies.as_array_mut().expect("list").push(
                    json!({"by": "claude", "text": "Fixed (abc1234): it", "date": "2026-11-02"}),
                );
            }
            bytes += replies.to_string().len();
            let text = format!("comment {i}");
            comment(
                &conn,
                i,
                C {
                    id: &format!("c{i}"),
                    task,
                    text: &text,
                    at: "2026-11-01T09:00:00Z",
                    open: true,
                    replies,
                },
            );
        }
        conn.execute_batch("COMMIT").expect("commit");
        let start = memory();
        let both =
            resolve(&db, "nha-org", "2026-11-02", 0, &|sha| sha == "abc1234").expect("resolve");
        let (merged, landed) = (both.merged.expect("merged"), both.landed.expect("landed"));
        let peak = memory().peak_mb - start.current_mb;
        assert_eq!((merged.len(), landed.len()), (100, 100));
        let open: i64 = conn
            .query_row(
                "SELECT count(*) FROM comments WHERE stage = 'open'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(open, MANY - 200);
        println!(
            "{MANY} comments ({:.1} MB of replies) read {PAGE} at a time: 200 resolved, peak {peak:.1} MB above the start of the pass, budget 50 MB",
            bytes as f64 / 1_048_576.0
        );
    }
}
