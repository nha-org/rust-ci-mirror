//! The held pushes (tasks held-push-recheck, rust-held-push-release-port), the logic beside their rows (`crate::older`):
//! each push the older app holds for the morning taken by Rust and re-checked; `dash morning-report` releases them at the
//! front door, and from 09:00 the « held pushes » watcher (dash's `watchers::HELD`) releases the rest and sends them as one
//! push, so the older app's `Phone.release_held` and `flush_held` find none. Moved out of the dash binary, which only
//! shrinks (task rust-binary-ratchet).
use crate::older::{self, Held};
use std::path::{Path, PathBuf};

/// The older app's store and the approved commit it reads.
pub type Store = Option<(PathBuf, Option<String>)>;

/// The older app's store at `db` and the approved commit (`.claude/run/approved` under `repo`), none when unread; and
/// whether the store is here at all: none (retired, a fresh Mac) holds nothing, never a red beat (/code-review medium of
/// held-push-recheck).
pub fn store(db: &Path, repo: &Path) -> Store {
    let approved = std::fs::read_to_string(repo.join(".claude/run/approved")).ok();
    db.exists().then(|| (db.to_path_buf(), approved))
}

/// One re-check of the held pushes, each left taken (`older::drop_closed_held`): the keys it dropped, and how many it
/// took.
pub fn recheck(store: &Store) -> Result<(Vec<String>, usize), String> {
    match store {
        Some((db, approved)) => older::drop_closed_held(db, approved.as_deref()),
        None => Ok((vec![], 0)),
    }
}

/// The release at `now`, its re-check in the same transaction: the keys dropped, then the pushes released.
fn release(store: &Store, now: &str) -> Result<(Vec<String>, Vec<Held>), String> {
    match store {
        Some((db, approved)) => older::release_held(db, approved.as_deref(), now),
        None => Ok((vec![], vec![])),
    }
}

/// Whether `now` (`YYYY-MM-DDTHH:MM`, his local time) is in the quiet hours, 22:30 to 08:00: no release then
/// (`Phone.quiet_hours?/1`; @nha, 2026-10-22: « Continue independently during the night. Do not ask me things as I won't
/// be there. »).
pub fn quiet(now: &str) -> bool {
    let hm = now.get(11..16).unwrap_or_default();
    !("08:00".."22:30").contains(&hm)
}

/// The morning report's lines, `Phone.report_lines/1`: each held push with its link, the first 10 and how many more, since
/// a push carries 1000 characters.
pub fn report_lines(held: &[Held]) -> Vec<String> {
    let link = |u: &str| match u.starts_with("http") {
        true => u.to_string(),
        false => format!("http://100.72.158.115:7788{u}"),
    };
    let mut lines: Vec<String> = (held.iter().take(10))
        .map(|h| format!("- {} {}", h.text, link(&h.url)))
        .collect();
    if held.len() > 10 {
        lines.push(format!("- and {} more on the bell", held.len() - 10));
    }
    lines
}

/// `dash morning-report` at `now`: the held pushes released, each re-checked in the release's transaction, as lines;
/// nothing released in the quiet hours, so a run at 23:00 never reports the night before the morning. The keys dropped
/// go to stderr, the front door's log.
pub fn morning_report(store: &Store, now: &str) -> Result<String, String> {
    let (dropped, held) = match quiet(now) {
        true => (recheck(store)?.0, vec![]),
        false => release(store, now)?,
    };
    if !dropped.is_empty() {
        eprintln!(
            "morning-report: dropped held pushes closed since: {}",
            dropped.join(", ")
        );
    }
    Ok(report_lines(&held)
        .iter()
        .map(|l| format!("{l}\n"))
        .collect())
}

/// One pass of the « held pushes » watcher at `now`: each held push taken and re-checked, and from 09:00 outside the
/// quiet hours those the morning run did not report released, re-checked in the release's transaction and held through a
/// booked interview, then sent as one « Held overnight: » push through `tell` (`Phone.flush_held/1` ported, task
/// rust-held-push-release-port). Nothing goes back to the older app, whose flush then finds none. A push that failed
/// (the older app down, an error, its binary gone) puts them back for the next pass; one with no answer in its time
/// (`dash_process`'s « no answer in »: it may have gone out) leaves them reported, on the bell, so a retry never sends
/// twice (/code-review medium: the older app's « not pushed » never comes, its notify answers :ok or :seen). ponytail:
/// `tell` is the older app's `dash push`, a reply on /help (its row listed there as Claude's answer, one more on the
/// bell), Rust's one route to his phone until `Dashboard.Phone`'s sender is ported (task web-push-device); `dash orch`
/// killed during the send loses that one push, its rows reported; a lasting failure retries each pass.
pub fn pass(
    store: &Store,
    now: &str,
    tell: &dyn Fn(&str) -> Result<(), String>,
) -> Result<String, String> {
    let flush = !quiet(now) && now.get(11..13).unwrap_or_default() >= "09";
    let (dropped, held) = match flush {
        true => release(store, now)?,
        false => (recheck(store)?.0, vec![]),
    };
    if !held.is_empty() {
        let text = ["Held overnight:".to_string()]
            .into_iter()
            .chain(report_lines(&held))
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = tell(&text) {
            let keys: Vec<String> = held.into_iter().map(|h| h.key).collect();
            // the error names the command, its words the push's own text: a held push saying « no answer in » is none
            if e.replace(&text, "").contains("no answer in") {
                return Err(format!(
                    "the 09:00 push may not have gone out, its {} pushes left on the bell: {e}",
                    keys.len()
                ));
            }
            let back = store
                .as_ref()
                .map_or(Ok(()), |(db, _)| older::unrelease(db, &keys));
            return Err(format!(
                "the 09:00 push failed, its {} pushes {}: {e}",
                keys.len(),
                back.map_or_else(|b| format!("not held again ({b})"), |_| "held again".into())
            ));
        }
    }
    let sent = match held.len() {
        0 => String::new(),
        n => format!("; {n} sent at 09:00"),
    };
    Ok(match dropped.is_empty() {
        true => format!("no held push closed since{sent}"),
        false => format!("dropped, closed since: {}{sent}", dropped.join(", ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};
    use rusqlite::Connection;

    /// A 09:00 push that failed puts its pushes back for the next pass, its error echoing the push's words (« no answer
    /// in » in a held push's text included); one that may have gone out (no answer in its time) leaves them reported, so
    /// a retry never sends them twice.
    #[dash_test]
    fn a_failed_09_00_push_keeps_its_pushes_for_the_next_pass() {
        let dir = TestDir::new("older-held-failed");
        let db = dir.0.join("older.db");
        Connection::open(&db)
            .expect("db")
            .execute_batch(
                "CREATE TABLE push_log (key TEXT PRIMARY KEY, task TEXT NOT NULL, text TEXT NOT NULL, url TEXT NOT NULL,
                   outcome TEXT NOT NULL, at TEXT NOT NULL);
                 CREATE TABLE tasks (collection TEXT, id TEXT, stage TEXT, done TEXT, dropped TEXT, data TEXT, project TEXT);
                 CREATE TABLE pr_links (url TEXT PRIMARY KEY, state TEXT NOT NULL, merged_at TEXT);
                 INSERT INTO push_log VALUES ('a', 'x', 'mail: no answer in a week', '/', 'held', '2026-10-28T22:00:00'),
                   ('b', 'x', 't', '/', 'held', '2026-10-28T22:00:00');",
            )
            .expect("tables");
        let store: Store = Some((db.clone(), None));
        let outcomes = || -> Vec<String> {
            (Connection::open(&db).expect("db"))
                .prepare("SELECT outcome FROM push_log ORDER BY key")
                .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
                .expect("rows")
        };
        let said = pass(&store, "2026-10-29T09:00", &|t| {
            Err(format!("dash push {t}: exit status: 1: connection refused"))
        });
        assert!(said.is_err_and(|e| e.contains("2 pushes held again")));
        assert_eq!(outcomes(), [older::TAKEN, older::TAKEN]);
        let said = pass(&store, "2026-10-29T09:01", &|t| {
            Err(format!("dash push {t}: no answer in 150s"))
        });
        assert!(said.is_err_and(|e| e.contains("2 pushes left on the bell")));
        assert_eq!(outcomes(), ["reported", "reported"]);
    }
}
