//! `dash tui`, the real binary, with no terminal (task rust-tui-app).
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::process::{Command, Stdio};

/// Done-when 3 of rust-tui-app: with its input and output no terminal, `dash tui` says so and exits 1, never ratatui's panic,
/// before it opens the store.
#[dash_test]
fn without_a_terminal_dash_tui_says_so_and_exits_1() {
    let dir = TestDir::new("tui-no-terminal");
    let ran = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["tui", "--data"])
        .arg(&dir.0)
        .env("DASH_REPO", dir.0.join("repo"))
        .stdin(Stdio::null())
        .output()
        .expect("dash tui");
    let said = String::from_utf8_lossy(&ran.stderr);
    assert_eq!(ran.status.code(), Some(1), "{said}");
    assert!(said.contains("dash tui: no terminal here"), "{said}");
    assert!(!said.contains("panicked"), "{said}");
    assert!(ran.stdout.is_empty(), "nothing drawn");
}

/// Done-when 1 of rust-tui-app: `private/dashboard/bin/dash tui` runs this binary in his terminal, never the app's server,
/// which has none: bin/dash routes `tui` here when the binary holds the words it greps for.
#[dash_test]
fn bin_dash_hands_tui_to_this_binary() {
    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let wrapper =
        std::fs::read_to_string(top.join("private/dashboard/bin/dash")).expect("bin/dash");
    let known = "dash tui [--data <dir>]";
    assert!(
        wrapper.contains(&format!("\"tui \"*) known='{known}'")),
        "bin/dash does not route tui to Rust"
    );
    // the wrapper's own check, its grep: reading the binary in here would pass this test's memory budget
    let grep = Command::new("grep")
        .args(["-aqF", known, env!("CARGO_BIN_EXE_dash")])
        .env("LC_ALL", "C")
        .status()
        .expect("grep");
    assert!(
        grep.success(),
        "the binary lacks « {known} », so bin/dash would send tui to the app"
    );
}
