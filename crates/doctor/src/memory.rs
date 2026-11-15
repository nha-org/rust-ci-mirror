//! Each long-running dash process's footprint against its budget, read live (task rust-live-memory-budget; @nha, 2026-11-14:
//! « there is dash instance runing at 150MB. EDIT 180MB ! This is clearly bloat we were always at 50MB or way below 10MB
//! before »). Every process that reports (the binary's reporting.rs: serve, orch and its watchers, a task's run) keeps its
//! reading here each round, a file per pid under the data folder, and reads everyone's back: the page /status/memory, a line
//! a process, and one line for /watchers' process memory row, each red while any process is past its budget. The tests' budgets (dash-meter's budgets.rs) hold synthetic states;
//! these hold the running ones.
use dash_core::{Drawing, Page, Reads};
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this process's wake was handed out: once a process, so one past its budget for days never reads the wake log.
static WOKE: AtomicBool = AtomicBool::new(false);

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
    let mut elements = serde_json::Map::new();
    for (i, line) in lines.iter().enumerate() {
        elements.insert(
            format!("l{i}"),
            json!({"type": "text", "props": {"text": line}}),
        );
    }
    let names: Vec<String> = (0..lines.len()).map(|i| format!("l{i}")).collect();
    let page = json!({"type": "section", "props": {"title": TITLE}, "children": names});
    elements.insert("page".into(), page);
    json!({"root": "page", "elements": elements}).to_string()
}

const TITLE: &str = "Process memory";

// One URL, both forms (rust/CLAUDE.md point 4), as /status/ci-queue and /status/front-door: the older app's /status keeps its
// own rows until that page is ported whole.
inventory::submit! {
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

/// One round of this process: its reading kept, the row's verdict and line, and at its first round past its budget, its
/// wake's key and words: once a process.
pub fn round(
    dir: &Path,
    command: &str,
    mb: f64,
    now_ms: i64,
) -> (bool, String, Option<(String, String)>) {
    let r = Reading {
        command: command.into(),
        pid: std::process::id(),
        mb,
        at_ms: now_ms,
    };
    let (ok, line) = record(dir, &r)
        .map(|all| said(&all))
        .unwrap_or_else(|e| (false, e));
    // its first past round only, keyed by that round's time too, so a recycled pid wakes again
    let first = r.past() && !WOKE.swap(true, Ordering::Relaxed);
    let wake = first.then(|| (format!("memory-{command}-{}-{now_ms}", r.pid), r.line()));
    (ok, line, wake)
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
}
