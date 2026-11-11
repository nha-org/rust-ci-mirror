//! The Hc backend against a real herdr-core: open, run, read, get, keys, kill, round trip. Skipped, and saying so, when no hc
//! answers (`dash_terminal::hc_bin`: `HC_BIN`, else ~/.local/bin/hc): nextest prints its output on a pass too (.config/nextest.toml).
use dash_core::tasks::Terminal;
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use dash_terminal::Hc;
use std::io::Write;
use std::time::{Duration, Instant};

#[test]
fn hc_round_trip_live() {
    let _budget = TestBudget::new();
    let bin = dash_terminal::hc_bin();
    let version = dash_process::run(&bin, &["--version"], None, Duration::from_secs(5));
    let Ok(version) = version else {
        // Straight to stderr: libtest's capture keeps eprintln! from a passing test.
        let _ = writeln!(
            std::io::stderr(),
            "skipped: hc_round_trip_live, hc is not installed ({bin})"
        );
        return;
    };
    let _ = writeln!(std::io::stderr(), "live: {}", version.trim());
    let dir = TestDir::new("hc-live");
    let hc = Hc::from_env();
    let t = hc
        .open(dir.0.to_str().expect("path"), "dash-hc-live")
        .expect("open");
    // Opened with no parent, it would outlive a failed assertion: killed at the end whatever happens.
    struct Kill<'a>(&'a Hc, &'a str);
    impl Drop for Kill<'_> {
        fn drop(&mut self) {
            let _ = self.0.close(self.1);
        }
    }
    let _kill = Kill(&hc, &t);
    hc.run(&t, "echo hc-live-$((40+2))").expect("run");
    let begin = Instant::now();
    let screen = loop {
        let screen = hc.read(&t).expect("read");
        if screen.contains("hc-live-42\n") || begin.elapsed() > Duration::from_secs(10) {
            break screen;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(screen.contains("hc-live-42\n"), "{screen}");
    // Its shell is in front again: no agent there.
    assert_eq!(hc.agent(&t).expect("get"), None);
    hc.sessions().expect("ls");
    hc.key(&t, "Enter").expect("keys");
    hc.close(&t).expect("kill");
    assert_eq!(hc.agent(&t).expect("get"), None, "ended");
}
