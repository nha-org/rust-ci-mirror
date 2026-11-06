//! Section 9's table on this Mac: each renderer's headless run, then its real window, three times each, one JSON line a
//! run. A window draws nothing on a locked or sleeping Mac, so --wait holds until it can, and a run during which the
//! display went away is killed and its line never printed, as is a report that shows no full, painted run
//! (meter::window_refusal). Exit 1 after 3 failed runs in a row.
//!
//!   cargo build --release -p slint-list -p blitz-list -p ui-spike-common && target/release/ui-measure --wait
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};
use ui_spike_common::meter::{display_refusal, window_refusal};

const RUNS: usize = 3;
const LIMIT: Duration = Duration::from_secs(300);
const BINS: [&str; 2] = ["slint-list", "blitz-list"];

/// Runs one measure to its end; its JSON line reaches stdout only when the run counts.
fn run(bin: &str, headless: bool) -> bool {
    let path = std::env::current_exe()
        .expect("own path")
        .with_file_name(bin);
    // caffeinate holds the display awake for the run: frames stop when it sleeps.
    let mut cmd = Command::new("caffeinate");
    cmd.args(["-d", "-u"]).arg(&path).stdout(Stdio::piped());
    if headless {
        cmd.arg("--no-window");
    }
    let mut child = cmd.spawn().expect("caffeinate");
    let start = Instant::now();
    loop {
        if child.try_wait().expect("wait").is_some() {
            let out = child.wait_with_output().expect("output");
            if !out.status.success() {
                eprintln!("{bin}: run discarded: {}", out.status);
                return false;
            }
            let report = String::from_utf8_lossy(&out.stdout);
            if !headless && let Some(why) = window_refusal(&report) {
                eprintln!("{bin}: run discarded: {why}: {report}");
                return false;
            }
            print!("{report}");
            return true;
        }
        let gone = if headless { None } else { display_refusal() };
        if gone.is_some() || start.elapsed() > LIMIT {
            child.kill().expect("kill");
            let _ = child.wait();
            eprintln!(
                "{bin}: run discarded: {}",
                gone.unwrap_or("no end in the time limit")
            );
            return false;
        }
        sleep(Duration::from_millis(200));
    }
}

/// Three counted runs, or exit 1 after 3 failures in a row: a measure that always fails must not loop for ever.
fn three_runs(bin: &str, headless: bool, wait: bool) {
    let (mut done, mut failed) = (0, 0);
    while done < RUNS {
        if !headless && let Some(why) = display_refusal() {
            if !wait {
                eprintln!("refused: {why}; --wait holds until the Mac is unlocked");
                std::process::exit(3);
            }
            sleep(Duration::from_secs(30));
            continue;
        }
        if run(bin, headless) {
            done += 1;
            failed = 0;
        } else if wait && !headless && display_refusal().is_some() {
            // Killed by a lock or a sleep mid-run: --wait holds, the run is not a failure.
            continue;
        } else {
            failed += 1;
            if failed == 3 {
                eprintln!("{bin}: 3 failed runs in a row");
                std::process::exit(1);
            }
        }
    }
}

fn main() {
    let wait = std::env::args().any(|a| a == "--wait");
    for headless in [true, false] {
        for bin in BINS {
            three_runs(bin, headless, wait);
        }
    }
}
