//! Section 9's table on this Mac: each renderer's headless run, then its real window, three times each, one JSON line a
//! run. A window draws nothing on a locked or sleeping Mac, so --wait holds until it can, and a run during which the
//! display went away is killed and its line never printed, as is a report that shows no full, painted run
//! (meter::window_refusal). Exit 1 after 3 failed runs in a row.
//!
//!   cargo build --release -p slint-list -p blitz-list -p ui-spike-common && target/release/ui-measure --wait
//!
//! --page measures section 9's Dioxus page alone: its list three times, then its behaviours three times, windows only.
//!   cargo build --release -p dioxus-page -p ui-spike-common && target/release/ui-measure --wait --page
//! --design measures the design system page, its script three times, then the plain page's: styled beside plain, the same day.
//!   cargo build --release -p dioxus-page -p ui-spike-common && target/release/ui-measure --wait --design
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};
use ui_spike_common::meter::{HIDDEN_EXIT, display_refusal, drive_refusal, window_refusal};

const RUNS: usize = 3;
const LIMIT: Duration = Duration::from_secs(300);
const BINS: [&str; 2] = ["slint-list", "blitz-list"];
/// --wait holds this many 30 s turns, 8 hours, on a window macOS never shows: a measure must not loop for ever.
const HOLDS: usize = 960;

#[derive(PartialEq)]
enum Outcome {
    Counted,
    Hidden,
    Failed,
}

#[derive(Clone, Copy)]
enum Kind {
    Headless,
    Window,
    PageList,
    PageDrive,
    DesignDrive,
}

/// Runs one measure to its end; its JSON line reaches stdout only when the run counts.
fn run(bin: &str, kind: Kind) -> Outcome {
    let headless = matches!(kind, Kind::Headless);
    let path = std::env::current_exe()
        .expect("own path")
        .with_file_name(bin);
    // caffeinate holds the display awake for the run: frames stop when it sleeps.
    let mut cmd = Command::new("caffeinate");
    cmd.args(["-d", "-u"]).arg(&path).stdout(Stdio::piped());
    match kind {
        Kind::Headless => drop(cmd.arg("--no-window")),
        Kind::Window => {}
        Kind::PageList => drop(cmd.arg("--measure")),
        Kind::PageDrive => drop(cmd.arg("--drive")),
        Kind::DesignDrive => drop(cmd.args(["--design", "--drive"])),
    }
    let mut child = cmd.spawn().expect("caffeinate");
    let start = Instant::now();
    loop {
        if child.try_wait().expect("wait").is_some() {
            let out = child.wait_with_output().expect("output");
            if out.status.code() == Some(HIDDEN_EXIT) {
                eprintln!("{bin}: run discarded: it refused, its window cannot be shown");
                return Outcome::Hidden;
            }
            if !out.status.success() {
                eprintln!("{bin}: run discarded: {}", out.status);
                return Outcome::Failed;
            }
            let report = String::from_utf8_lossy(&out.stdout);
            let refusal = match kind {
                Kind::Headless => None,
                Kind::Window | Kind::PageList => window_refusal(&report),
                Kind::PageDrive | Kind::DesignDrive => drive_refusal(&report),
            };
            if let Some(why) = refusal {
                eprintln!("{bin}: run discarded: {why}: {report}");
                return Outcome::Failed;
            }
            print!("{report}");
            return Outcome::Counted;
        }
        let gone = if headless { None } else { display_refusal() };
        if gone.is_some() || start.elapsed() > LIMIT {
            child.kill().expect("kill");
            let _ = child.wait();
            eprintln!(
                "{bin}: run discarded: {}",
                gone.unwrap_or("no end in the time limit")
            );
            return Outcome::Failed;
        }
        sleep(Duration::from_millis(200));
    }
}

/// Three counted runs, or exit 1 after 3 failures in a row: a measure that always fails must not loop for ever.
fn three_runs(bin: &str, kind: Kind, wait: bool) {
    let headless = matches!(kind, Kind::Headless);
    let (mut done, mut failed, mut holds) = (0, 0, 0);
    while done < RUNS {
        if !headless && let Some(why) = display_refusal() {
            if !wait {
                eprintln!("refused: {why}; --wait holds until the Mac is unlocked");
                std::process::exit(3);
            }
            sleep(Duration::from_secs(30));
            continue;
        }
        let outcome = run(bin, kind);
        if outcome == Outcome::Counted {
            done += 1;
            failed = 0;
        } else if outcome == Outcome::Hidden {
            holds += 1;
            if !wait || holds == HOLDS {
                eprintln!("{bin}: its window was never shown");
                std::process::exit(HIDDEN_EXIT);
            }
            sleep(Duration::from_secs(30));
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
    if std::env::args().any(|a| a == "--design") {
        three_runs("dioxus-page", Kind::DesignDrive, wait);
        three_runs("dioxus-page", Kind::PageDrive, wait);
        return;
    }
    if std::env::args().any(|a| a == "--page") {
        three_runs("dioxus-page", Kind::PageList, wait);
        three_runs("dioxus-page", Kind::PageDrive, wait);
        return;
    }
    for kind in [Kind::Headless, Kind::Window] {
        for bin in BINS {
            three_runs(bin, kind, wait);
        }
    }
}
