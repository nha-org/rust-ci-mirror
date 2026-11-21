//! Task rust-doctor-xprotect through the binary: `dash doctor` is registered from its plugin, refuses an argument it does
//! not know before any check, and writes where the handoff reads.
use dash_meter::{TestDir, dash_test};
use std::process::Command;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

#[dash_test]
fn dash_doctor_is_wired_and_records_where_the_handoff_reads() {
    assert_eq!(dash_doctor::RECORD, dash_store::older_handoff::DOCTOR);
    let repo = TestDir::new("doctor-wired");
    let out = Command::new(DASH)
        .env("DASH_REPO", &repo.0)
        .args(["doctor", "--sudoo"])
        .output()
        .expect("dash");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("usage: dash doctor [--sudo]"), "{err}");
    assert!(
        !repo.0.join(dash_doctor::RECORD).exists(),
        "a misuse checks nothing"
    );
    let help = Command::new(DASH).arg("help").output().expect("dash help");
    assert!(String::from_utf8_lossy(&help.stdout).contains("dash doctor [--sudo]"));
}
