//! The held pushes (task held-push-recheck), the logic beside their rows (`crate::older`): each push the older app holds
//! for the morning taken by Rust and re-checked; `dash morning-report` releases them at the front door, and from 09:00
//! the « held pushes » watcher (dash's `watchers::HELD`) gives the rest back, re-checked, to the older app's own flush. Moved out of the dash binary, which only shrinks
//! (task rust-binary-ratchet).
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

/// One re-check of the held pushes, each left marked `to` (`older::drop_closed_held`): the keys it dropped, and how many
/// it marked.
pub fn recheck(store: &Store, to: &str) -> Result<(Vec<String>, usize), String> {
    match store {
        Some((db, approved)) => older::drop_closed_held(db, approved.as_deref(), to),
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
        true => (recheck(store, older::TAKEN)?.0, vec![]),
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
/// quiet hours those the morning run did not report given back, re-checked in the same transaction, to the older app's
/// `Phone.flush_held/1`, which sends them as one push on its own route (« morning », to /) and holds them through a booked
/// interview. Rust never sends them itself: the older app's `dash push` files a push as a reply on /help, urgent
/// (/code-review medium of PR 1559).
pub fn pass(store: &Store, now: &str) -> Result<String, String> {
    let flush = !quiet(now) && now.get(11..13).unwrap_or_default() >= "09";
    let to = match flush {
        true => "held",
        false => older::TAKEN,
    };
    let (dropped, marked) = recheck(store, to)?;
    let given = match (flush, marked) {
        (true, n) if n > 0 => format!("; {n} given back to the older app's 09:00 flush"),
        _ => String::new(),
    };
    Ok(match dropped.is_empty() {
        true => format!("no held push closed since{given}"),
        false => format!("dropped, closed since: {}{given}", dropped.join(", ")),
    })
}
