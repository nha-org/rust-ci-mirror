//! What Rust's landed pass (dash_orch::landed, task rust-lane-landed-port) reads and writes of the older app's SQLite: its
//! claims, a claimed task's row and newest report, and a claim's release with the « ended » event `Runner.reaped/3` records.
use dash_core::tasks::{LandedRow, OlderClaim};
use regex::Regex;
use rusqlite::{OptionalExtension, params};
use std::path::Path;
use std::sync::LazyLock;

/// A report saying its work goes on: `Reports.work_left?/1`'s outcome pattern.
static NOT_MET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\W*(not[\s-]+(yet[\s-]+)?|partly\s+)met\b").expect("a valid pattern")
});

fn named(db: &Path) -> impl Fn(rusqlite::Error) -> String + '_ {
    move |e| format!("{}: {e}", db.display())
}

/// Every claim of the older app's PM loop, its age read by SQLite (the older app writes UTC).
pub fn claims(db: &Path) -> Result<Vec<OlderClaim>, String> {
    let conn = crate::older::opened(db)?;
    let sql = "SELECT task, project, pane, at, CAST((julianday('now') - julianday(at)) * 86400 AS INTEGER)
               FROM task_claims";
    let mut read = conn.prepare(sql).map_err(named(db))?;
    let rows = read.query_map([], |r| {
        Ok(OlderClaim {
            task: r.get(0)?,
            project: r.get(1)?,
            pane: r.get(2)?,
            at: r.get(3)?,
            // an `at` SQLite cannot read is old: its pane is asked, never skipped for good
            age_s: r.get::<_, Option<i64>>(4)?.unwrap_or(i64::MAX),
        })
    });
    rows.map_err(named(db))?
        .collect::<Result<_, _>>()
        .map_err(named(db))
}

/// Task `id`'s row in site_perso's list, none when it holds none: its newest report since its last reopen (`:since`)
/// judges `work_left`, as `Reports.work_left?/1`; a `:since` SQLite cannot read finds no report, so work is left.
pub fn row(db: &Path, id: &str) -> Result<Option<LandedRow>, String> {
    let conn = crate::older::opened(db)?;
    let sql = "SELECT coalesce(stage, 'open'), ltrim(coalesce(owner, ''), ':'),
                 coalesce(json_extract(data, '$.\"review-of\"'), '') <> '', json_extract(data, '$.since'),
                 json_extract(data, '$.history')
               FROM tasks WHERE collection = 'tasks.edn' AND id = ?1";
    let read = conn.query_row(sql, [id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    });
    let Some((stage, owner, review_of, since, history)) = read.optional().map_err(named(db))?
    else {
        return Ok(None);
    };
    let sql = "SELECT record FROM task_reports WHERE task = ?1
                 AND (?2 IS NULL OR julianday(at) > julianday(?2)) ORDER BY id DESC LIMIT 1";
    let report: Option<String> = (conn.query_row(sql, params![id, since], |r| r.get(0)))
        .optional()
        .map_err(named(db))?;
    let work_left = report.is_none_or(|r| {
        let r: serde_json::Value = serde_json::from_str(&r).unwrap_or_default();
        r["kind"] == "blocked" || NOT_MET.is_match(r["outcome"].as_str().unwrap_or_default())
    });
    Ok(Some(LandedRow {
        open: stage == "open",
        runs_here: matches!(owner.as_str(), "" | "claude" | "vibe"),
        review_of,
        work_left,
        past_prs: past_prs(history.as_deref()),
    }))
}

/// The PRs a task's `:history` names, each period's: `Tasks.past_prs/1`, its URLs alone, never the words beside one in a
/// `:pr` (« … ready for review », which the older app read as PRs; /code-review medium of PR 1602).
fn past_prs(history: Option<&str>) -> Vec<String> {
    let history: serde_json::Value =
        serde_json::from_str(history.unwrap_or("[]")).unwrap_or_default();
    let mut prs = vec![];
    for h in history.as_array().into_iter().flatten() {
        let listed = |k: &str| h[k].as_array().cloned().unwrap_or_default();
        let said = h["pr"].as_str().unwrap_or_default().split_whitespace();
        let urls = said.map(str::to_string).chain(
            (listed("prs").into_iter().chain(listed("branch-prs")))
                .filter_map(|v| v.as_str().map(str::to_string)),
        );
        for url in urls.filter(|u| u.contains("/pull/")) {
            if !prs.contains(&url) {
                prs.push(url);
            }
        }
    }
    prs
}

/// The claim released only as it was read, with the « ended » event `Runner.reaped/3` records: false when it moved since
/// (a restart's new pane, a close).
pub fn release(db: &Path, c: &OlderClaim) -> Result<bool, String> {
    let mut conn = crate::older::writable(db)?;
    let tx = conn.transaction().map_err(named(db))?;
    let sql = "DELETE FROM task_claims WHERE task = ?1 AND pane IS ?2 AND at = ?3";
    if tx
        .execute(sql, params![c.task, c.pane, c.at])
        .map_err(named(db))?
        == 0
    {
        return Ok(false);
    }
    // as `Events.record/4` writes it: no nil value, the instant to the microsecond
    let mut data = serde_json::json!({"via": "reaped", "by": "rust-landed"});
    if let Some(pane) = &c.pane {
        data["pane"] = pane.as_str().into();
    }
    let data = data.to_string();
    let sql = "INSERT INTO task_events (task, status, at, data)
               VALUES (?1, 'ended', strftime('%Y-%m-%dT%H:%M:%f', 'now') || '000', ?2)";
    tx.execute(sql, params![c.task, data]).map_err(named(db))?;
    tx.commit().map_err(named(db))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use rusqlite::Connection;

    #[dash_test]
    fn a_claim_reads_with_its_row_and_report_and_releases_only_as_read() {
        let dir = dash_meter::TestDir::new("older-landed");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, owner TEXT);
             CREATE TABLE task_claims (task TEXT PRIMARY KEY, project TEXT NOT NULL, pane TEXT, session TEXT, request TEXT,
               via TEXT NOT NULL, at TEXT NOT NULL);
             CREATE TABLE task_reports (id INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, at TEXT, record TEXT);
             CREATE TABLE task_events (id INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, status TEXT, at TEXT, data TEXT);
             INSERT INTO task_claims VALUES ('review-lane', 'site_perso', NULL, NULL, NULL, 'pm', '2026-11-12T16:46:10.280667');
             INSERT INTO tasks VALUES ('tasks.edn', 0, 'a', NULL,
               '{\"since\":\"2026-11-02T19:09:44.700748Z\",\"history\":[{\"pr\":\"https://g/pull/1 ready for review\",
                 \"branch-prs\":[\"https://g/pull/1\",\"https://g/pull/2\"]}]}', ':claude');
             INSERT INTO tasks VALUES ('tasks.edn', 1, 'b', 'open', '{\"review-of\":\"a\"}', ':rust');
             INSERT INTO task_reports (task, at, record) VALUES ('a', '2026-11-01T00:00:00', '{\"outcome\":\"met\"}'),
               ('b', '2026-11-14T00:00:00', '{\"outcome\":\"not met: CI red\"}'),
               ('a', '2026-11-14T00:00:00', '{\"outcome\":\"met, every part\"}');",
        )
        .expect("tables");
        let claims = claims(&path).expect("claims");
        assert_eq!(claims.len(), 1);
        assert!(claims[0].age_s > 86_400, "{:?}", claims[0]);
        let a = row(&path, "a").expect("a").expect("its row");
        let ended = LandedRow {
            open: true,
            runs_here: true,
            past_prs: vec!["https://g/pull/1".into(), "https://g/pull/2".into()],
            ..LandedRow::default()
        };
        assert_eq!(a, ended, "its report since its reopen ends its work");
        let b = row(&path, "b").expect("b").expect("its row");
        assert_eq!((b.runs_here, b.review_of, b.work_left), (false, true, true));
        assert_eq!(
            row(&path, "review-lane").expect("lane"),
            None,
            "the lane is no list row"
        );
        let moved = OlderClaim {
            pane: Some("w1:p1".into()),
            ..claims[0].clone()
        };
        assert!(
            !release(&path, &moved).expect("a release"),
            "a claim moved since is kept"
        );
        assert!(release(&path, &claims[0]).expect("a release"));
        assert_eq!(claims_left(&db), 0);
        let event: String = db
            .query_row("SELECT status || ' ' || data FROM task_events", [], |r| {
                r.get(0)
            })
            .expect("its event");
        assert_eq!(event, r#"ended {"by":"rust-landed","via":"reaped"}"#);
    }

    fn claims_left(db: &Connection) -> i64 {
        db.query_row("SELECT count(*) FROM task_claims", [], |r| r.get(0))
            .expect("count")
    }
}
