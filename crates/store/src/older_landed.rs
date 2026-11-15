//! What Rust's landed pass (dash_orch::landed, task rust-lane-landed-port) reads and writes of the older app's SQLite: its
//! claims, a claimed task's row and newest report, and a claim's release with the « ended » event `Runner.reaped/3` records.
use dash_core::tasks::{LANE, LandedRow, OlderClaim};
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
    // new or backlog is neither open nor closed: its claim stays the older app's, as a task the list does not hold
    // (/code-review medium of PR 1602)
    if !matches!(stage.as_str(), "open" | "done" | "dropped") {
        return Ok(None);
    }
    let sql = "SELECT record FROM task_reports WHERE task = ?1
                 AND (?2 IS NULL OR julianday(at) > julianday(?2)) ORDER BY id DESC LIMIT 1";
    let report: Option<String> = (conn.query_row(sql, params![id, since], |r| r.get(0)))
        .optional()
        .map_err(named(db))?;
    // a report that does not parse leaves work, as no report does: never read as ended (/code-review medium of PR 1602)
    let work_left = report.is_none_or(|r| {
        serde_json::from_str::<serde_json::Value>(&r).map_or(true, |r| !ends_work(&r))
    });
    Ok(Some(LandedRow {
        open: stage == "open",
        runs_here: matches!(owner.as_str(), "" | "claude" | "vibe"),
        review_of,
        work_left,
        past_prs: past_prs(history.as_deref()),
    }))
}

/// Whether a report ends its task's work, as `Reports.work_left?/1` reads the newest: neither blocked nor « not met ».
pub fn ends_work(report: &serde_json::Value) -> bool {
    report["kind"] != "blocked" && !NOT_MET.is_match(report["outcome"].as_str().unwrap_or_default())
}

/// The PR lane's claim released once the older app filed `report`, a report ending its run, while its pane still runs it
/// (task rust-lane-gone-claim-ladder): the older runner walks claims alone, so the pane's loss finds none to judge, where its
/// `run_ended/3` sent `PrLane.task()` to `Tracker.Files.done/4` and crashed whenever it beat Rust's minute pass. The line
/// said after the older app's answer, empty for another task's report, one that leaves work, or a lane with no claim.
/// ponytail: a lane session lost before it reports still races the older pass; the runner's retirement closes it.
pub fn lane_reported(db: &Path, task: &str, report: &serde_json::Value) -> String {
    let closed = |c: &OlderClaim| -> Result<Option<String>, String> {
        let pane = c.pane.as_deref().unwrap_or("none");
        Ok(released(db, c, "reported")?
            .then(|| format!("{LANE}'s report ends its run: claim on {pane} closed, its pane left to the PM check\n")))
    };
    if task != LANE || !ends_work(report) {
        return String::new();
    }
    let lane = claims(db).map(|c| c.into_iter().find(|c| c.task == LANE));
    match lane.and_then(|c| c.map_or(Ok(None), |c| closed(&c))) {
        Ok(said) => said.unwrap_or_default(),
        Err(e) => format!("{LANE}'s claim not closed: {e}\n"),
    }
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
    released(db, c, "reaped")
}

/// `release` with its event's `via`: « reported » for the lane's report, never read as a reap (`Runner.close_reported/1`
/// acts on « reaped »; /code-review medium of PR 1671).
fn released(db: &Path, c: &OlderClaim, via: &str) -> Result<bool, String> {
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
    let mut data = serde_json::json!({"via": via, "by": "rust-landed"});
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
             INSERT INTO tasks VALUES ('tasks.edn', 2, 'later', 'backlog', '{}', NULL);
             INSERT INTO tasks VALUES ('tasks.edn', 3, 'torn', 'open', '{}', NULL);
             INSERT INTO task_reports (task, at, record) VALUES ('a', '2026-11-01T00:00:00', '{\"outcome\":\"met\"}'),
               ('b', '2026-11-14T00:00:00', '{\"outcome\":\"not met: CI red\"}'),
               ('a', '2026-11-14T00:00:00', '{\"outcome\":\"met, every part\"}'),
               ('torn', '2026-11-14T00:00:00', '{\"outcome\":\"met');",
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
            row(&path, "later").expect("later"),
            None,
            "a backlog task is neither open nor closed: the older app's"
        );
        let torn = row(&path, "torn").expect("torn").expect("its row");
        assert!(torn.work_left, "a report that does not parse leaves work");
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

    #[dash_test]
    fn a_lane_report_ending_its_run_releases_its_live_claim_so_no_older_pass_judges_its_loss() {
        let dir = dash_meter::TestDir::new("older-lane-reported");
        let path = dir.0.join("older.db");
        let db = Connection::open(&path).expect("db");
        db.execute_batch(
            "CREATE TABLE task_claims (task TEXT PRIMARY KEY, project TEXT NOT NULL, pane TEXT, session TEXT, request TEXT,
               via TEXT NOT NULL, at TEXT NOT NULL);
             CREATE TABLE task_events (id INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT, status TEXT, at TEXT, data TEXT);
             INSERT INTO task_claims VALUES ('review-lane', 'site_perso', 'w20B:p1', 's', NULL, 'adopted',
               '2026-11-15T13:17:22.302447'), ('a', 'site_perso', 'w1:p1', NULL, NULL, 'pm', '2026-11-15T13:00:00');",
        )
        .expect("tables");
        let said = |task: &str, report: serde_json::Value| lane_reported(&path, task, &report);
        let met =
            serde_json::json!({"kind": "done", "outcome": "met: dash review next prints none"});
        for (task, report) in [
            (
                LANE,
                serde_json::json!({"kind": "done", "outcome": "not met: PR 1651 still red"}),
            ),
            (
                LANE,
                serde_json::json!({"kind": "blocked", "needs": "an approval"}),
            ),
            ("a", met.clone()),
        ] {
            assert_eq!(said(task, report), "");
        }
        assert_eq!(
            claims_left(&db),
            2,
            "a report leaving work, or another task's, keeps every claim"
        );
        assert_eq!(
            said(LANE, met.clone()),
            "review-lane's report ends its run: claim on w20B:p1 closed, its pane left to the PM check\n"
        );
        // the older runner's watch walks `Claims.all/0` alone: with no lane claim, its run_ended/3 never sends PrLane.task()
        // to Tracker.Files.done/4 when the pane goes
        let left: Vec<_> = claims(&path)
            .expect("claims")
            .into_iter()
            .map(|c| c.task)
            .collect();
        assert_eq!(
            left,
            ["a"],
            "the lane's claim is gone while its pane still runs"
        );
        let event: String = db
            .query_row(
                "SELECT task || ' ' || status || ' ' || data FROM task_events",
                [],
                |r| r.get(0),
            )
            .expect("its event");
        assert_eq!(
            event, r#"review-lane ended {"by":"rust-landed","pane":"w20B:p1","via":"reported"}"#,
            "« ended »: the older app's adopt/1 never claims it again; « reported », never a reap close_reported/1 acts on"
        );
        assert_eq!(
            said(LANE, met),
            "",
            "a second report finds no claim to close"
        );
    }

    fn claims_left(db: &Connection) -> i64 {
        db.query_row("SELECT count(*) FROM task_claims", [], |r| r.get(0))
            .expect("count")
    }
}
