//! Each long-running dash process's footprint against its budget, read live (task rust-live-memory-budget; @nha, 2026-11-14:
//! « there is dash instance runing at 150MB. EDIT 180MB ! This is clearly bloat we were always at 50MB or way below 10MB
//! before »). Every process that reports (the binary's reporting.rs: serve, orch and its watchers, a task's run) keeps its
//! reading here each round, a file per pid under the data folder, and reads everyone's back: the page /status/memory, a line
//! a process, and one line for /watchers' process memory row, each red while any process is past its budget. The tests' budgets (dash-meter's budgets.rs) hold synthetic states;
//! these hold the running ones.
use dash_core::{Drawing, Page, Reads};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};

/// The UTC day this process's wake landed on (`round`): one past its budget all day reads the wake log once, once it
/// landed; a held or failed wake is tried each round until it lands.
static WOKE_DAY: AtomicI64 = AtomicI64::new(i64::MIN);
const DAY_MS: i64 = 86_400_000;

/// The server's budget; every other command's is the CLI's, `dash orch` included: it held 1.0 MB in rfc-rust-unified#baseline.
pub const SERVER_MB: f64 = 50.0;
pub const CLI_MB: f64 = 10.0;
/// Under the data folder: one file per process, its command, footprint and time.
pub const FOLDER: &str = "memory";
/// A reading older than this is of a process gone: a round is 30 s, so three missed rounds and more.
pub const STALE_MS: i64 = 2 * 60_000;

pub fn budget_mb(command: &str) -> f64 {
    match command {
        "serve" => SERVER_MB,
        _ => CLI_MB,
    }
}

/// One process's footprint, in MB, read at `at_ms`.
#[derive(Debug, PartialEq)]
pub struct Reading {
    pub command: String,
    pub pid: u32,
    pub mb: f64,
    pub at_ms: i64,
}

impl Reading {
    pub fn past(&self) -> bool {
        self.mb > budget_mb(&self.command)
    }

    pub fn line(&self) -> String {
        let past = if self.past() { ", past its budget" } else { "" };
        let (command, pid, mb) = (&self.command, self.pid, self.mb);
        format!(
            "{command} (pid {pid}) {mb:.1} of {} MB{past}",
            budget_mb(command)
        )
    }
}

/// Keeps `r` as its process's reading in `dir`, then reads every process's still fresh at `r.at_ms`, by command and pid; a
/// stale one's file is removed. A file that does not read (one being written) is left for the next round.
pub fn record(dir: &Path, r: &Reading) -> Result<Vec<Reading>, String> {
    let folder = dir.join(FOLDER);
    let said = |e: std::io::Error| format!("{}: {e}", folder.display());
    std::fs::create_dir_all(&folder).map_err(said)?;
    let text = format!("{}\t{}\t{}", r.command, r.mb, r.at_ms);
    // written aside then renamed, so a sibling's round never reads it half written; a `.tmp` name parses as no pid
    let (file, aside) = (
        folder.join(r.pid.to_string()),
        folder.join(format!("{}.tmp", r.pid)),
    );
    std::fs::write(&aside, text).map_err(said)?;
    std::fs::rename(&aside, &file).map_err(said)?;
    fresh(dir, r.at_ms, true)
}

/// Every process's reading still fresh at `now_ms`, by command and pid; with `prune`, a stale one's file removed. No folder
/// is no process yet.
fn fresh(dir: &Path, now_ms: i64, prune: bool) -> Result<Vec<Reading>, String> {
    let folder = dir.join(FOLDER);
    let entries = match std::fs::read_dir(&folder) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        read => read.map_err(|e| format!("{}: {e}", folder.display()))?,
    };
    let mut all = vec![];
    for entry in entries.flatten() {
        let Some(seen) = read(&entry.path()) else {
            continue;
        };
        match now_ms - seen.at_ms > STALE_MS {
            true if prune => drop(std::fs::remove_file(entry.path())),
            true => {}
            false => all.push(seen),
        }
    }
    all.sort_by(|a, b| (&a.command, a.pid).cmp(&(&b.command, b.pid)));
    Ok(all)
}

/// /status/memory's tree at `now_ms`: its verdict, then a line a process, drawn by the design system in both forms.
pub fn tree(dir: &Path, now_ms: i64) -> String {
    let lines = match fresh(dir, now_ms, false) {
        Err(e) => vec![format!("NOT OK: the readings did not read: {e}")],
        Ok(all) if all.is_empty() => vec!["no process reported in the last 2 minutes".into()],
        Ok(all) => {
            let past = all.iter().filter(|r| r.past()).count();
            let verdict = match past {
                0 => format!("ok: {} processes, each inside its budget", all.len()),
                n => format!("NOT OK: {n} of {} processes past their budget", all.len()),
            };
            std::iter::once(verdict)
                .chain(all.iter().map(Reading::line))
                .collect()
        }
    };
    dash_core::lines(TITLE, &lines)
}

const TITLE: &str = "Process memory";

// One URL, both forms (rust/CLAUDE.md point 4), as /status/ci-queue and /status/front-door: the older app's /status keeps its
// own rows until that page is ported whole.
dash_core::submit! {
    Page { path: "/status/memory", step: "route_status_memory", reads: Reads::Nothing, redraw: true, serve: |a| {
        a.form(Some(Ok(Drawing::Tree { title: TITLE.into(), tree: tree(&a.data(), a.host.now_ms()) })))
    } }
}

fn read(file: &Path) -> Option<Reading> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut parts = text.split('\t');
    let command = parts.next()?.to_string();
    let (mb, at_ms) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    let pid = file.file_name()?.to_str()?.parse().ok()?;
    Some(Reading {
        command,
        pid,
        mb,
        at_ms,
    })
}

/// The row's line, every process's reading, and whether all are inside their budgets.
pub fn said(all: &[Reading]) -> (bool, String) {
    let lines: Vec<String> = all.iter().map(Reading::line).collect();
    (all.iter().all(|r| !r.past()), lines.join("; "))
}

/// One round of this process: its reading kept, the row's verdict and line, and while past its budget, its wake's key and
/// words handed to `land` (none while held) until one lands that UTC day: a held or failed one is tried at the next round
/// (/code-review), and what `land` said is returned.
pub fn round(
    dir: &Path,
    command: &str,
    mb: f64,
    now_ms: i64,
    land: impl FnOnce(String, String) -> Option<Result<(), String>>,
) -> (bool, String, Option<Result<(), String>>) {
    let r = Reading {
        command: command.into(),
        pid: std::process::id(),
        mb,
        at_ms: now_ms,
    };
    let (ok, line) = record(dir, &r)
        .map(|all| said(&all))
        .unwrap_or_else(|e| (false, e));
    let woke = wake(&r, &WOKE_DAY).and_then(|(key, words)| land(key, words));
    if let Some(Ok(())) = woke {
        WOKE_DAY.store(now_ms.div_euclid(DAY_MS), Ordering::Relaxed);
    }
    (ok, line, woke)
}

/// A past reading's wake until its process's wake of the UTC day landed (`woke_day`, set by `round`), keyed by command
/// and day, never pid: the log lands a key once, so a restart's new pid adds nothing to a day's wake (45 of 59 items on
/// 2026-11-19 13:35 were one overrun re-woken by restarts; task memory-wake-per-day). One key a day, closed or not: the
/// row stays red.
fn wake(r: &Reading, woke_day: &AtomicI64) -> Option<(String, String)> {
    let day = r.at_ms.div_euclid(DAY_MS);
    let first = r.past() && woke_day.load(Ordering::Relaxed) != day;
    first.then(|| {
        (
            format!("memory-{}-{}", r.command, dash_core::date(day)),
            r.line(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    fn reading(command: &str, pid: u32, mb: f64, at_ms: i64) -> Reading {
        Reading {
            command: command.into(),
            pid,
            mb,
            at_ms,
        }
    }

    /// Done-when 1: each process's reading reaches the row beside the others', a gone one's dropped; done-when 2: one past
    /// its budget (50 MB the server, 10 MB the CLI) turns the row red, and only while it is past.
    #[dash_test]
    fn each_process_s_reading_shows_and_one_past_its_budget_turns_the_row_red() {
        let dir = TestDir::new("memory-readings");
        record(&dir.0, &reading("orch", 7, 9.5, 2)).expect("orch");
        record(&dir.0, &reading("run", 9, 3.0, 0)).expect("a gone run");
        let all = record(&dir.0, &reading("serve", 8, 49.9, STALE_MS + 1)).expect("serve");
        let (ok, line) = said(&all);
        println!("{ok}: {line}");
        assert!(ok, "{line}");
        assert_eq!(all.len(), 2, "the run's reading is stale: {all:?}");
        assert_eq!(
            line,
            "orch (pid 7) 9.5 of 10 MB; serve (pid 8) 49.9 of 50 MB"
        );
        assert!(!dir.0.join(FOLDER).join("9").exists(), "its file removed");

        let all = record(&dir.0, &reading("serve", 8, 50.1, STALE_MS + 2)).expect("serve");
        let (ok, line) = said(&all);
        println!("{ok}: {line}");
        assert!(!ok);
        assert!(
            line.ends_with("serve (pid 8) 50.1 of 50 MB, past its budget"),
            "{line}"
        );
        let all = record(&dir.0, &reading("orch", 7, 10.1, STALE_MS + 3)).expect("orch");
        assert_eq!(all.iter().filter(|r| r.past()).count(), 2, "{all:?}");
        // /status/memory draws the same readings, its verdict first
        let page = tree(&dir.0, STALE_MS + 3);
        println!("/status/memory: {page}");
        assert!(
            page.contains("NOT OK: 2 of 2 processes past their budget"),
            "{page}"
        );
        assert!(
            page.contains("orch (pid 7) 10.1 of 10 MB, past its budget"),
            "{page}"
        );
        let none = TestDir::new("memory-none");
        assert!(tree(&none.0, 0).contains("no process reported in the last 2 minutes"));
    }

    /// Task memory-wake-per-day, done-when 1: two pids of one command past budget on one UTC day land one wake, the next
    /// day one more. The wake log lands a key once (dash_comments::watch::land), so the landed keys stand for its rows.
    #[dash_test]
    fn two_pids_of_one_command_past_budget_wake_once_a_day() {
        let day = 20_776 * DAY_MS; // 2026-11-19, 00:00 UTC
        let (old, new) = (AtomicI64::new(i64::MIN), AtomicI64::new(i64::MIN));
        let mut landed = std::collections::BTreeSet::new();
        // the old pid's rounds past budget, then the restart's new pid's, all on one day
        for (pid, woke, at) in [(7, &old, 13), (7, &old, 14), (8, &new, 15), (8, &new, 16)] {
            let r = reading("orch", pid, 10.5, day + at * 3_600_000);
            if let Some((key, _)) = wake(&r, woke) {
                landed.insert(key);
                woke.store(20_776, Ordering::Relaxed); // `woke`, on this test's own day
            }
        }
        println!("day one: {landed:?}");
        assert_eq!(Vec::from_iter(&landed), ["memory-orch-2026-11-19"]);
        // the next UTC day, the same pid past again: one more
        let r = reading("orch", 8, 10.5, day + DAY_MS + 60_000);
        let (key, words) = wake(&r, &new).expect("the next day's wake");
        assert!(
            wake(&r, &new).is_some(),
            "not landed yet: the next round tries again"
        );
        landed.insert(key);
        new.store(20_777, Ordering::Relaxed);
        println!("day two: {landed:?}");
        assert_eq!(
            Vec::from_iter(&landed),
            ["memory-orch-2026-11-19", "memory-orch-2026-11-20"]
        );
        assert_eq!(words, "orch (pid 8) 10.5 of 10 MB, past its budget");
        assert!(wake(&r, &new).is_none(), "its next round, the same day");
        // inside its budget, no wake
        assert!(wake(&reading("orch", 9, 9.5, day), &AtomicI64::new(i64::MIN)).is_none());
    }

    /// /code-review of memory-wake-per-day: the day is marked only once its wake landed, so a held (his switch) or failed
    /// land is tried again at the next round, and a landed one is not.
    #[dash_test]
    fn a_held_or_failed_wake_is_tried_again_and_a_landed_one_is_not() {
        let dir = TestDir::new("memory-wake-land");
        let mut asked = 0;
        let mut once = |said: Option<Result<(), String>>| {
            round(&dir.0, "orch", 10.5, 60_000, |key, _| {
                assert_eq!(key, "memory-orch-1970-01-01");
                asked += 1;
                said
            })
            .2
        };
        assert_eq!(once(None), None, "held");
        assert_eq!(once(Some(Err("busy".into()))), Some(Err("busy".into())));
        assert_eq!(once(Some(Ok(()))), Some(Ok(())), "landed");
        assert_eq!(once(Some(Ok(()))), None, "the same day: not asked");
        assert_eq!(asked, 3);
    }
}
