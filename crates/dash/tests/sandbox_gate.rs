//! `dash sandbox gate <dir>`, the real binary (task rust-merge-gate-port): bin/dash hands it the words, its verdict's exit code
//! is the reviewer's step, and a misuse names its usage. The cases themselves are dash-gate's tests/merge_gate.rs.
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::process::Command;

/// git looks no higher than `ceiling`: a TMPDIR inside a checkout leaves the test's folder outside any repository (/code-review)
fn gate(args: &[&str], ceiling: &std::path::Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["sandbox", "gate"])
        .args(args)
        .env("GIT_CEILING_DIRECTORIES", ceiling)
        .output()
        .expect("dash sandbox gate");
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    (
        out.status.code().unwrap_or(-1),
        text(&out.stdout),
        text(&out.stderr),
    )
}

#[test]
fn dash_sandbox_gate_answers_and_names_its_usage_without_a_folder() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("sandbox-gate-cli");
    let (code, out, err) = gate(&[], &dir.0);
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    assert!(err.starts_with("usage: dash sandbox gate"), "{err}");

    let plain = dir.0.to_str().expect("utf-8");
    let (code, out, err) = gate(&[plain], &dir.0);
    assert_eq!((code, out.as_str()), (1, ""), "{err}");
    assert!(
        err.starts_with("merge gate: cannot judge: git rev-parse"),
        "{err}"
    );

    // bin/dash hands the words to this binary only when it holds the usage bin/dash greps for
    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let wrapper =
        std::fs::read_to_string(top.join("private/dashboard/bin/dash")).expect("bin/dash");
    assert!(
        wrapper.contains(&format!(
            "\"sandbox gate\") known='{}'",
            dash_gate::merge::USAGE
        )),
        "bin/dash does not route sandbox gate to Rust"
    );
}
