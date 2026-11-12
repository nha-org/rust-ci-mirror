//! The VPS runner's watchdog on /watchers (task rust-vps-runner-watchdog): `dash orch` runs a pass of dash_forge::watchdog
//! every `watchers::VPS.every_ms`, through `gh`, each run's workflow read at its own commit; `dash ci watch --data <dir>` runs
//! one.
//! Each run it ends is its wake on /watchers, kept until the next one. /status, the older app's, stays as it is until its CI
//! queue row is ported (task rust-status-ci-queue; the dialog's answer, 2026-11-12: « Both: /watchers now, port tasked »).
use crate::tasks::now_ms;
use crate::watchers;
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
        Ok((line, _, _)) => line.clone(),
        Err(e) => format!("vps watchdog: {e}"),
    };
    // a run GitHub refused to end still holds the queue, and a running one unread may: NOT OK on /watchers
    let ok = ran.as_ref().is_ok_and(|(_, _, ok)| *ok);
    if let Err(e) = watchers::beat(dir, &watchers::VPS, now_ms(), ok, &said) {
        eprintln!("vps watchdog: the beat: {e}");
    }
    if let Ok((_, ended, _)) = &ran
        && !ended.is_empty()
        && let Err(e) = watchers::woke(dir, &watchers::VPS, now_ms(), &ended.join("; "))
    {
        eprintln!("vps watchdog: the wake: {e}");
    }
    ran.map(|(line, _, _)| line)
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

/// A pass every `watchers::VPS.every_ms` for as long as `dash orch` runs.
pub fn watch(dir: PathBuf) {
    let sleep = Duration::from_millis(watchers::VPS.every_ms as u64);
    watchers::run(dir, &watchers::VPS, sleep, beating);
}
