//! The VPS runner's watchdog on /watchers (task rust-vps-runner-watchdog): `dash orch` runs a pass of dash_forge::watchdog
//! every `watchers::VPS.every_ms`, through `gh`, each run's workflow read at its own commit; `dash ci watch --data <dir>` runs
//! one.
//! Each run it ends is its wake on /watchers, kept until the next one.
//! The same pass records the CI queue row, each run judged by its jobs (dash_forge::queue), and /status/ci-queue draws it with
//! the watchdog's last cancels: Rust's port of Dashboard.CiQueue's /status row (task rust-ci-queue-port; the dialog's answer,
//! 2026-11-12: « Own URL, /status/ci-queue »). The older app's /status keeps its own row until that page is ported whole.
use crate::tasks::now_ms;
use crate::watchers;
use dash_core::stamp;
use dash_surface as surface;
use serde_json::json;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SLUG: &str = "nha-org/site_perso";
const ASK: Duration = Duration::from_secs(60);

/// A pass and its beat on /watchers, whatever it said; each run it ended its wake.
fn beating(dir: &Path) -> Result<String, String> {
    let ask = |words: &[&str]| dash_process::run("gh", words, None, ASK);
    let now = now_ms() / 1000;
    let ran = dash_forge::watchdog::pass(&ask, SLUG, now);
    let said = match &ran {
        Ok((line, _, _, _)) => line.clone(),
        Err(e) => format!("vps watchdog: {e}"),
    };
    // a run GitHub refused to end still holds the queue, and a running one unread may: NOT OK on /watchers
    let ok = ran.as_ref().is_ok_and(|(_, _, ok, _)| *ok);
    if let Err(e) = watchers::beat(dir, &watchers::VPS, now_ms(), ok, &said) {
        eprintln!("vps watchdog: the beat: {e}");
    }
    // a pass that did not list the runs is a red row at once, never the last read shown as current (rust/CLAUDE.md point 5:
    // a watcher that stops says so where it is read); the row wakes nobody, so a gh hiccup costs one red pass on the page
    let (calm, row) = match &ran {
        Ok((_, _, _, (calm, row))) => (*calm, row.clone()),
        Err(e) => (false, format!("the runs did not read: {e}")),
    };
    if let Err(e) = watchers::beat(dir, &watchers::CI_QUEUE, now_ms(), calm, &row) {
        eprintln!("vps watchdog: the queue row: {e}");
    }
    if let Ok((_, ended, _, _)) = &ran
        && !ended.is_empty()
        && let Err(e) = watchers::woke(dir, &watchers::VPS, now_ms(), &ended.join("; "))
    {
        eprintln!("vps watchdog: the wake: {e}");
    }
    ran.map(|(line, _, _, _)| line)
}

inventory::submit! { crate::usage::Command::usage("dash ci watch --data <dir>") }
pub fn once(dir: &Path, mut out: File) {
    match beating(dir) {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

inventory::submit! { crate::usage::Command::usage("dash ci queue --data <dir>") }
/// The CI queue row's text from `dir`'s last pass: /status/ci-queue's, for an agent with no port.
pub fn queue(dir: &Path, mut out: File) {
    let tree = queue_tree(
        &watchers::beats(dir),
        watchers::is_paused(dir, watchers::VPS.name),
        now_ms(),
    );
    match surface::text(&tree) {
        Ok(text) => write!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("ci queue: {e}");
            std::process::exit(1);
        }
    }
}

/// A pass every `watchers::VPS.every_ms` for as long as `dash orch` runs.
pub fn watch(dir: PathBuf) {
    let sleep = Duration::from_millis(watchers::VPS.every_ms as u64);
    watchers::run(dir, &watchers::VPS, sleep, beating);
}

/// A row older than this is stale: no pass read the forge since (Dashboard.CiQueue's `@fresh`).
const FRESH_MS: i64 = 30 * 60 * 1000;

/// The CI queue row's tree at `now_ms` from `beats`, the watchers table: the row the last pass recorded, red once stale, and
/// the VPS watchdog's last cancels, so what it ended shows where the queue is read. `paused`: his switch on /watchers holds the
/// watchdog, so a stale row says why.
pub(crate) fn queue_tree(
    beats: &Result<Vec<watchers::Beat>, String>,
    paused: bool,
    now_ms: i64,
) -> String {
    let find = |name: &str| match beats {
        Ok(rows) => Ok(rows.iter().find(|r| r.0 == name)),
        Err(e) => Err(e.clone()),
    };
    let (ok, queue, read) = match find(watchers::CI_QUEUE.name) {
        Err(e) => (false, format!("the row did not read: {e}"), "never".into()),
        Ok(Some((_, Some(at), _, said, _))) if now_ms - at > FRESH_MS => (
            false,
            format!(
                "stale: no read of the forge since {}{}; last read: {said}",
                stamp(*at),
                if paused {
                    ", the VPS watchdog being paused by his switch on /watchers"
                } else {
                    ""
                }
            ),
            stamp(*at),
        ),
        Ok(Some((_, Some(at), calm, said, _))) => (*calm, said.clone(), stamp(*at)),
        // before the first pass nothing is known: no outage, as the old row
        Ok(_) => (
            true,
            "no pass has read the forge yet".into(),
            "never".into(),
        ),
    };
    let cancels = match find(watchers::VPS.name) {
        Err(e) => format!("its record did not read: {e}"),
        Ok(Some((_, _, _, _, Some((what, at))))) => format!("{}: {what}", stamp(*at)),
        Ok(_) => "none yet".into(),
    };
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let lines = [
        (
            "state",
            format!("State: {}", if ok { "ok" } else { "NOT OK" }),
        ),
        ("queue", format!("Queue: {queue}")),
        ("cancels", format!("VPS watchdog's last cancels: {cancels}")),
        (
            "read",
            format!(
                "Read: {read}, by the VPS watchdog's pass every {} min",
                watchers::CI_QUEUE.every_ms / 60_000
            ),
        ),
        (
            "rule",
            format!(
                "Red: a run waiting past {} min, or no read for {} min; a run whose jobs ended or never came is left out",
                dash_forge::queue::LIMIT_MIN,
                FRESH_MS / 60_000
            ),
        ),
    ];
    let mut elements = serde_json::Map::new();
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "CI queue"}}),
    );
    let mut names = vec![];
    for (name, line) in lines {
        elements.insert(name.into(), text(line));
        names.push(name);
    }
    elements.insert(
        "row".into(),
        json!({"type": "section", "props": {"title": "CI queue"}, "children": names}),
    );
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": ["title", "row"]}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

// One URL, both forms (rust/CLAUDE.md point 4): a browser gets HTML, an agent's curl the text.
inventory::submit! {
    crate::pages::Page { path: "/status/ci-queue", step: "route_status_ci_queue", reads: crate::pages::Reads::Nothing, redraw: true, serve: |a| {
        a.form(Some(crate::pages::serve_from_dir().map(|dir| surface::Drawing::Tree {
            title: "CI queue".into(),
            tree: queue_tree(&watchers::beats(dir), watchers::is_paused(dir, watchers::VPS.name), now_ms()),
        })))
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    /// Done-when 1 and 3 of rust-ci-queue-port: the row's text form from the table a pass writes, the watchdog's cancel in it;
    /// red past 30 minutes of wait, and red once no pass has read for 30 minutes, naming his pause when it is one.
    #[test]
    fn the_queue_row_s_text_names_the_wait_and_the_watchdog_s_cancels() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("ci-queue-row");
        let shown = |paused, now| {
            surface::text(&queue_tree(&watchers::beats(&dir.0), paused, now)).expect("its text")
        };
        let text = |now| shown(false, now);
        let first = text(0);
        assert!(
            first.contains("State: ok") && first.contains("Queue: no pass has read the forge yet"),
            "{first}"
        );
        assert!(
            first.contains("VPS watchdog's last cancels: none yet"),
            "{first}"
        );
        let at = 1_794_428_220_000; // 2026-11-11T20:17:00Z
        let (calm, row) = dash_forge::queue::row(
            &[("explainer-gate".into(), at / 1000 - 40 * 60)],
            &[],
            at / 1000,
        );
        watchers::beat(&dir.0, &watchers::CI_QUEUE, at, calm, &row).expect("the row");
        let cancel =
            "run 43095022385 (explainer-gate.yml gate) cancelled: 5 min past its 6-minute limit";
        watchers::woke(&dir.0, &watchers::VPS, at, cancel).expect("the cancel");
        let red = text(at + 60_000);
        assert!(red.contains("State: NOT OK"), "{red}");
        assert!(
            red.contains(
                "Queue: 1 queued, oldest waiting 40 min: explainer-gate 1 (oldest 40 min)"
            ),
            "{red}"
        );
        assert!(
            red.contains(&format!(
                "VPS watchdog's last cancels: 2026-11-11 20:17 UTC: {cancel}"
            )),
            "{red}"
        );
        assert!(
            red.contains("Read: 2026-11-11 20:17 UTC, by the VPS watchdog's pass every 2 min"),
            "{red}"
        );
        let (calm, row) =
            dash_forge::queue::row(&[], &["run 43095721996 (explainer-gate)".into()], at / 1000);
        watchers::beat(&dir.0, &watchers::CI_QUEUE, at, calm, &row).expect("the row");
        let calm = text(at + 60_000);
        assert!(
            calm.contains("State: ok") && calm.contains("run 43095721996 (explainer-gate)"),
            "{calm}"
        );
        let stale = text(at + 31 * 60_000);
        assert!(
            stale.contains("State: NOT OK")
                && stale.contains("Queue: stale: no read of the forge since 2026-11-11 20:17 UTC;"),
            "{stale}"
        );
        // his switch on /watchers holds the watchdog: the stale row says so (/code-review)
        let paused = shown(true, at + 31 * 60_000);
        assert!(
            paused.contains("since 2026-11-11 20:17 UTC, the VPS watchdog being paused by his switch on /watchers;"),
            "{paused}"
        );
    }
}
