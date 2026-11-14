//! The LinkedIn gate (Dashboard.Linkedin's `load`, `check`, `limited` and `hello`), answered from the older app's own tables
//! (`linkedin_loads`, `linkedin_state`) and the hook's cooldown file, so both apps count the same loads (task
//! rust-linkedin-gate-port: the older app's check took 5.07 s on 2026-11-13, past the hook's 5 s; @nha, 2026-11-10:
//! « remember we move to rust. no more elixir, so if something needs fixing port and then fix »). A check only reads, so
//! no write of the older app ever holds it; a load reads and writes in one IMMEDIATE transaction, so two loads asked at once
//! cannot both pass the gap, as the older app's one process held them, and across processes too.
//! ponytail: the older app's schema as it stands and its caps copied; its cache ops (`cached`, `cache`, `forget`) and its
//! summary stay there until their readers move.
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

/// {per hour, per day, per week} page loads, rolling windows (Dashboard.Linkedin's `@caps`). A reply where the other person
/// wrote has its own budget, so cold sends never delay an answer (@nha, 2026-10-22: « We should not have the same budget for
/// all maybe if it delays replies »).
const CAPS: [(&str, [usize; 3]); 4] = [
    ("write", [3, 8, 40]),
    ("reply", [6, 25, 120]),
    ("profile", [8, 30, 150]),
    ("read", [12, 50, 250]),
];
/// Every class together.
const ALL: [usize; 3] = [20, 80, 400];
/// Seconds between two loads of any class, and between two writes.
const GAP: f64 = 20.0;
const WRITE_GAP: f64 = 60.0;
/// LinkedIn answered with a limit: nothing loads for a day.
const COOL: f64 = 86_400.0;
/// A session re-check is skipped this long after a load passed.
const FRESH: f64 = 1800.0;
const WEEK: f64 = 7.0 * 86_400.0;
/// easy_apply's one page is the job page it submits on: a write.
const WRITE: [&str; 4] = [
    "connect_with_person",
    "send_message",
    "message_job_poster",
    "easy_apply",
];
const PROFILE: [&str; 7] = [
    "get_person_profile",
    "get_my_profile",
    "get_company_profile",
    "get_company_employees",
    "get_company_posts",
    "get_sidebar_profiles",
    "search_people",
];
const NO_HELLO: &str = "LinkedIn gate: the LinkedIn server does not ask the gate yet (no hello), so its page loads would go uncounted. Bump private/dashboard/priv/commands/mcp-linkedin.rev to a fork commit with core/gate.py.";

/// The gate's answer to one load.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Ok,
    /// the gap: ask again in this many seconds
    Wait(i64),
    Refuse(String),
    /// a session re-check a recent load makes useless
    Skip,
}

impl Answer {
    fn json(self) -> Value {
        match self {
            Answer::Ok => json!({"allow": true}),
            Answer::Skip => json!({"allow": false, "skip": true}),
            Answer::Wait(s) => json!({"allow": false, "wait": s}),
            Answer::Refuse(why) => json!({"allow": false, "reason": why}),
        }
    }
}

/// The class a load counts under: `source` is `tool:<name>`, `revalidate`, `login` and so on; a write tool's profile or job
/// page is the page a person opens before writing, counted as what it is.
pub fn class(source: &str, url: Option<&str>) -> &'static str {
    let tool = source.strip_prefix("tool:");
    if tool == Some("easy_apply") {
        return "write";
    }
    if let (Some(t), Some(url)) = (tool, url)
        && (t == "reply_in_thread" || WRITE.contains(&t))
    {
        if url.contains("linkedin.com/in/") {
            return "profile";
        }
        if url.contains("linkedin.com/jobs/") {
            return "read";
        }
    }
    match tool {
        Some("reply_in_thread") => "reply",
        Some(t) if WRITE.contains(&t) => "write",
        Some(t) if PROFILE.contains(&t) => "profile",
        _ => "read",
    }
}

fn caps(class: &str) -> [usize; 3] {
    CAPS.iter()
        .find(|(c, _)| *c == class)
        .map_or(ALL, |(_, c)| *c)
}

/// The first cap `n` more loads would pass, said.
fn over(name: &str, times: &[f64], n: usize, now: f64, caps: [usize; 3]) -> Option<String> {
    let spans = [(3600.0, "an hour"), (86_400.0, "a day"), (WEEK, "a week")];
    caps.iter().zip(spans).find_map(|(cap, (span, label))| {
        let used = times.iter().filter(|t| now - **t < span).count();
        (used + n > *cap).then(|| {
            format!(
                "LinkedIn gate: {cap} {name} page loads {label}, {used} used. Wait; do not work around it."
            )
        })
    })
}

/// `loads`: (unix seconds, class) of the last week; `cooling`: a refusal or none. The gap is skipped for a dry run, since the
/// fork waits it out page by page.
pub fn decide(
    loads: &[(f64, String)],
    cooling: Option<&str>,
    class: &str,
    n: usize,
    now: f64,
    gap: bool,
) -> Answer {
    if let Some(why) = cooling {
        return Answer::Refuse(why.into());
    }
    let all: Vec<f64> = loads.iter().map(|(t, _)| *t).collect();
    let mine: Vec<f64> = loads
        .iter()
        .filter(|(_, c)| c == class)
        .map(|(t, _)| *t)
        .collect();
    if let Some(why) =
        over(class, &mine, n, now, caps(class)).or_else(|| over("LinkedIn", &all, n, now, ALL))
    {
        return Answer::Refuse(why);
    }
    let left = |ts: &mut dyn Iterator<Item = f64>, gap: f64| {
        ts.reduce(f64::max)
            .map_or(0, |last| (gap - (now - last)).ceil() as i64)
    };
    let writing = class == "write" || class == "reply";
    let wait = match gap {
        false => 0,
        true => left(&mut all.iter().copied(), GAP).max(match writing {
            true => left(
                &mut loads
                    .iter()
                    .filter(|(_, c)| c == "write" || c == "reply")
                    .map(|(t, _)| *t),
                WRITE_GAP,
            ),
            false => 0,
        }),
    };
    match wait > 0 {
        true => Answer::Wait(wait),
        false => Answer::Ok,
    }
}

/// The hook's dry run: a write's first page is the profile or job ad a person opens before writing, checked under its
/// class too, so a send the profile cap would stop midway is refused before it starts.
fn dry_run(
    loads: &[(f64, String)],
    cooling: Option<&str>,
    tool: &str,
    pages: usize,
    now: f64,
) -> Answer {
    let class = class(&format!("tool:{tool}"), None);
    let answer = decide(loads, cooling, class, pages, now, false);
    let first = match tool {
        "message_job_poster" | "reply_in_thread" => "read",
        _ => "profile",
    };
    match answer {
        Answer::Ok if (class == "write" || class == "reply") && tool != "easy_apply" => {
            decide(loads, cooling, first, 1, now, false)
        }
        answer => answer,
    }
}

/// The store's or the file's cooldown in force, the later ending, said as a refusal.
fn cooling(db: &Connection, file: &Path, now: f64) -> rusqlite::Result<Option<String>> {
    let file = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let until = |c: &Value| c["until"].as_f64().filter(|u| *u > now);
    let Some((until, c)) = [state(db, "cooldown")?, file]
        .into_iter()
        .flatten()
        .filter_map(|c| until(&c).map(|u| (u, c)))
        .max_by(|a, b| a.0.total_cmp(&b.0))
    else {
        return Ok(None);
    };
    let why = text(&c["why"]);
    Ok(Some(format!(
        "LinkedIn cooldown until {}: {why}. No LinkedIn page of any kind until then; do not retry, do not work around it.",
        dash_core::stamp(until.trunc() as i64 * 1000)
    )))
}

/// A value as Elixir's `to_string` writes it: a string as is, null empty (a cooldown or `limited` with no why).
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn state(db: &Connection, key: &str) -> rusqlite::Result<Option<Value>> {
    let text: Option<String> = db
        .query_row(
            "SELECT value FROM linkedin_state WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
}

fn put(db: &Connection, key: &str, value: &Value) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO linkedin_state (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )
    .map(drop)
}

fn loads(db: &Connection, now: f64) -> rusqlite::Result<Vec<(f64, String)>> {
    let mut q = db.prepare("SELECT at, class FROM linkedin_loads WHERE at > ?1")?;
    q.query_map([now - WEEK], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Whether a tool page or a session re-check loaded in the last half hour (a login page proves nothing).
fn fresh(db: &Connection, now: f64) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM linkedin_loads WHERE at > ?1 AND (source = 'revalidate' OR source LIKE 'tool:%'))",
        [now - FRESH],
        |r| r.get(0),
    )
}

/// `db`, read-only for a check, else read-write. A lock is waited 3 s at most: the fork gives a load up at 5 s (gate.py's
/// `_ask`), so a load committed past it would count a page that never loaded and hold the next one a gap for nothing
/// (/code-review of PR 1553); a store still busy at 3 s is a refusal, said.
fn open(db: &Path, write: bool) -> rusqlite::Result<Connection> {
    let flags = match write {
        true => OpenFlags::SQLITE_OPEN_READ_WRITE,
        false => OpenFlags::SQLITE_OPEN_READ_ONLY,
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(db, flags)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    crate::timed::trace(&mut conn, db)?;
    Ok(conn)
}

/// The gate's JSON answer to `op` with the request `body`, as `POST /dashboard/linkedin/<op>` answered in the older app:
/// `load` and `check` `allow` (or `wait`, `skip`, `reason`), `limited` and `hello` `ok`. `db` is the older app's database,
/// `cooldown` the hook's file. A store that does not answer is a refusal, said.
pub fn answer(db: &Path, cooldown: &Path, op: &str, body: &Value, now: f64) -> Value {
    let pages = body["pages"].as_u64().filter(|n| *n > 0);
    let ran = match (op, body["tool"].as_str(), body["source"].as_str(), pages) {
        ("check", Some(tool), _, Some(n)) => (|| -> rusqlite::Result<Value> {
            let conn = open(db, false)?;
            Ok(match state(&conn, "hello")? {
                None => Answer::Refuse(NO_HELLO.into()),
                Some(_) => {
                    let cooling = cooling(&conn, cooldown, now)?;
                    dry_run(
                        &loads(&conn, now)?,
                        cooling.as_deref(),
                        tool,
                        n as usize,
                        now,
                    )
                }
            }
            .json())
        })(),
        ("load", _, Some(source), _) => (|| -> rusqlite::Result<Value> {
            let url = body["url"].as_str();
            let class = class(source, url);
            let mut conn = open(db, true)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let cooling = cooling(&tx, cooldown, now)?;
            let answer = match source == "revalidate" && cooling.is_none() && fresh(&tx, now)? {
                true => Answer::Skip,
                false => decide(&loads(&tx, now)?, cooling.as_deref(), class, 1, now, true),
            };
            if answer == Answer::Ok {
                let url: Option<String> = url.map(|u| u.chars().take(500).collect());
                tx.execute(
                    "INSERT INTO linkedin_loads (at, class, source, url) VALUES (?1, ?2, ?3, ?4)",
                    params![now, class, source, url],
                )?;
            }
            tx.commit()?;
            Ok(answer.json())
        })(),
        // any why, null included, as the older app's `%{"why" => why}` matched
        ("limited", _, _, _) if body.get("why").is_some() => (|| -> rusqlite::Result<Value> {
            let why: String = text(&body["why"]).chars().take(300).collect();
            put(
                &open(db, true)?,
                "cooldown",
                &json!({"until": now + COOL, "since": now, "why": why}),
            )?;
            Ok(json!({"ok": true}))
        })(),
        ("hello", _, _, _) if body["version"].is_string() => (|| -> rusqlite::Result<Value> {
            put(
                &open(db, true)?,
                "hello",
                &json!({"version": body["version"], "at": now}),
            )?;
            Ok(json!({"ok": true}))
        })(),
        _ => {
            return json!({"allow": false, "ok": false, "reason": format!("linkedin {op}: unknown or missing arguments")});
        }
    };
    ran.unwrap_or_else(|e: rusqlite::Error| {
        json!({"allow": false, "ok": false, "reason": format!("LinkedIn gate: the store did not answer ({}: {e}), so the call is refused.", db.display())})
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// The older app's tables, as its migration made them, in a scratch database.
    fn older(dir: &TestDir) -> std::path::PathBuf {
        let path = dir.0.join("dashboard_dev.db");
        Connection::open(&path)
            .and_then(|c| {
                c.execute_batch(
                    "PRAGMA journal_mode = WAL;
                     CREATE TABLE linkedin_loads (id INTEGER PRIMARY KEY AUTOINCREMENT, at REAL NOT NULL, class TEXT NOT NULL,
                       source TEXT NOT NULL, url TEXT);
                     CREATE INDEX linkedin_loads_at_index ON linkedin_loads (at);
                     CREATE TABLE linkedin_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
                )
            })
            .expect("the older app's tables");
        path
    }

    const NOW: f64 = 1_794_412_800.0;

    fn ask(db: &Path, file: &Path, op: &str, body: Value) -> Value {
        answer(db, file, op, &body, NOW)
    }

    /// Done-when 1, the ask: a load passes, is counted in the older app's table, and the next one waits the gap; a write waits
    /// its own; a session re-check after a tool page is skipped.
    #[dash_test]
    fn a_load_is_counted_in_the_older_store_and_the_next_waits_the_gap() {
        let dir = TestDir::new("linkedin-load");
        let (db, file) = (older(&dir), dir.0.join("cooldown.json"));
        let load = |source: &str, at: f64| {
            answer(
                &db,
                &file,
                "load",
                &json!({"source": source, "url": "https://www.linkedin.com/feed/"}),
                at,
            )
        };
        assert_eq!(load("tool:get_inbox", NOW), json!({"allow": true}));
        let rows: Vec<(f64, String, String)> = Connection::open(&db)
            .and_then(|c| {
                c.prepare("SELECT at, class, source FROM linkedin_loads")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect()
            })
            .expect("rows");
        assert_eq!(rows, [(NOW, "read".into(), "tool:get_inbox".into())]);
        assert_eq!(
            load("tool:get_inbox", NOW + 5.0),
            json!({"allow": false, "wait": 15})
        );
        assert_eq!(
            load("revalidate", NOW + 25.0),
            json!({"allow": false, "skip": true})
        );
        assert_eq!(
            load("tool:send_message", NOW + 25.0),
            json!({"allow": true})
        );
        assert_eq!(
            load("tool:send_message", NOW + 50.0),
            json!({"allow": false, "wait": 35})
        );
        assert_eq!(
            ask(&db, &file, "load", json!({"url": "x"})),
            json!({"allow": false, "ok": false, "reason": "linkedin load: unknown or missing arguments"})
        );
    }

    /// Done-when 1, the check: refused with no hello, refused past a cap (its message the older app's), passed under it, never
    /// counted, and a write is checked under its first page's class too.
    #[dash_test]
    fn a_check_reads_the_caps_from_the_store_and_counts_nothing() {
        let dir = TestDir::new("linkedin-check");
        let (db, file) = (older(&dir), dir.0.join("cooldown.json"));
        let check = |tool: &str, pages: u64| {
            ask(&db, &file, "check", json!({"tool": tool, "pages": pages}))
        };
        assert_eq!(
            check("get_inbox", 1),
            json!({"allow": false, "reason": NO_HELLO})
        );
        assert_eq!(
            ask(&db, &file, "hello", json!({"version": "abc"})),
            json!({"ok": true})
        );
        assert_eq!(check("get_inbox", 1), json!({"allow": true}));
        let conn = Connection::open(&db).expect("db");
        for i in 0..10 {
            conn.execute(
                "INSERT INTO linkedin_loads (at, class, source) VALUES (?1, 'read', 'tool:search_jobs')",
                [NOW - 100.0 - f64::from(i)],
            )
            .expect("a load");
        }
        assert_eq!(check("search_jobs", 2), json!({"allow": true}));
        assert_eq!(
            check("search_jobs", 3),
            json!({"allow": false, "reason": "LinkedIn gate: 12 read page loads an hour, 10 used. Wait; do not work around it."})
        );
        let count: i64 = conn
            .query_row("SELECT count(*) FROM linkedin_loads", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 10, "a check counts nothing");
        for i in 0..8 {
            conn.execute(
                "INSERT INTO linkedin_loads (at, class, source) VALUES (?1, 'profile', 'tool:get_person_profile')",
                [NOW - 200.0 - f64::from(i)],
            )
            .expect("a profile load");
        }
        // a send opens the profile first: refused on the profile cap though no write was made
        assert_eq!(
            check("send_message", 1)["reason"],
            json!(
                "LinkedIn gate: 8 profile page loads an hour, 8 used. Wait; do not work around it."
            )
        );
        assert_eq!(
            ask(
                &db,
                &file,
                "check",
                json!({"tool": "get_inbox", "pages": 0})
            ),
            json!({"allow": false, "ok": false, "reason": "linkedin check: unknown or missing arguments"})
        );
    }

    /// Done-when 1, the budgets: every class's hour, day and week caps and the cap on all classes, at their edges.
    #[dash_test]
    fn each_budget_refuses_at_its_cap_and_not_before() {
        for (class, caps) in CAPS {
            for (cap, span) in caps.iter().zip([3600.0, 86_400.0, WEEK]) {
                // spread over the window, outside any smaller one
                let loads: Vec<(f64, String)> = (0..*cap)
                    .map(|i| (NOW - span + 1.0 + i as f64, class.to_string()))
                    .collect();
                assert_eq!(
                    decide(&loads[1..], None, class, 1, NOW, false),
                    Answer::Ok,
                    "{class} {span}"
                );
                assert!(
                    matches!(decide(&loads, None, class, 1, NOW, false), Answer::Refuse(ref w) if w.contains(&format!("{cap} {class} page loads"))),
                    "{class} {span}"
                );
            }
        }
        let mixed: Vec<(f64, String)> = (0..20)
            .map(|i| {
                (
                    NOW - 100.0 - f64::from(i),
                    ["read", "profile", "reply"][i as usize % 3].into(),
                )
            })
            .collect();
        assert_eq!(
            decide(&mixed, None, "write", 1, NOW, false),
            Answer::Refuse("LinkedIn gate: 20 LinkedIn page loads an hour, 20 used. Wait; do not work around it.".into())
        );
        assert_eq!(
            class(
                "tool:send_message",
                Some("https://www.linkedin.com/in/ana/")
            ),
            "profile"
        );
        assert_eq!(
            class(
                "tool:message_job_poster",
                Some("https://www.linkedin.com/jobs/view/1")
            ),
            "read"
        );
        assert_eq!(
            class(
                "tool:easy_apply",
                Some("https://www.linkedin.com/jobs/view/1")
            ),
            "write"
        );
        assert_eq!(class("tool:reply_in_thread", None), "reply");
        assert_eq!(class("tool:search_people", None), "profile");
        assert_eq!(class("login", None), "read");
    }

    /// Done-when 1, the cooldown: `limited` stops every load and check for a day, read from the store; the hook's file holds
    /// one too when the store has none, and the later end wins.
    #[dash_test]
    fn limited_cools_every_load_for_a_day_and_the_file_s_cooldown_holds_too() {
        let dir = TestDir::new("linkedin-cool");
        let (db, file) = (older(&dir), dir.0.join("cooldown.json"));
        ask(&db, &file, "hello", json!({"version": "abc"}));
        assert_eq!(
            ask(
                &db,
                &file,
                "limited",
                json!({"why": "get_inbox answered « rate limit »"})
            ),
            json!({"ok": true})
        );
        let said = "LinkedIn cooldown until 2026-11-12 16:00 UTC: get_inbox answered « rate limit ». No LinkedIn page of any kind until then; do not retry, do not work around it.";
        assert_eq!(
            ask(
                &db,
                &file,
                "check",
                json!({"tool": "get_inbox", "pages": 1})
            ),
            json!({"allow": false, "reason": said})
        );
        let load = |at: f64| answer(&db, &file, "load", &json!({"source": "revalidate"}), at);
        assert_eq!(load(NOW + 3600.0), json!({"allow": false, "reason": said}));
        assert_eq!(load(NOW + COOL + 1.0), json!({"allow": true}));
        std::fs::write(
            &file,
            json!({"until": NOW + 2.0 * COOL, "since": NOW, "why": "by hand"}).to_string(),
        )
        .expect("file");
        let later = load(NOW + COOL + 30.0);
        assert!(
            later["reason"].as_str().is_some_and(
                |r| r.starts_with("LinkedIn cooldown until 2026-11-13 16:00 UTC: by hand.")
            ),
            "{later}"
        );
        // a null why still cools, said as nothing, as the older app's to_string(nil) (/code-review)
        assert_eq!(
            answer(
                &db,
                &file,
                "limited",
                &json!({"why": null}),
                NOW + 3.0 * COOL
            ),
            json!({"ok": true})
        );
        let none = load(NOW + 3.0 * COOL + 1.0);
        assert!(
            none["reason"]
                .as_str()
                .is_some_and(|r| r
                    .starts_with("LinkedIn cooldown until 2026-11-15 16:00 UTC: . No LinkedIn")),
            "{none}"
        );
    }

    /// No database there: a refusal that names it, never a pass.
    #[dash_test]
    fn a_store_that_does_not_answer_refuses() {
        let dir = TestDir::new("linkedin-none");
        let none = dir.0.join("none.db");
        let said = ask(
            &none,
            &dir.0.join("c.json"),
            "check",
            json!({"tool": "get_inbox", "pages": 1}),
        );
        assert_eq!(said["allow"], json!(false), "{said}");
        assert!(
            said["reason"]
                .as_str()
                .is_some_and(|r| r.starts_with("LinkedIn gate: the store did not answer")),
            "{said}"
        );
    }
}
