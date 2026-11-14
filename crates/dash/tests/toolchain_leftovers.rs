//! private/dashboard/bin/toolchain's leftovers (task ci-standins-reaped): a test beam killed with KILL skips its on_exit,
//! and leftovers_test's loop.sh ran 37 h past it, freework_standin.py 47 h. The toolchain stays the run's parent and,
//! once it ends, stops what it left: a process in a run folder tmp/run-<pid>-* of a dead beam, and an orphan whose
//! command line names test/support/. Here a stand-in mix starts one of each, then kills itself with KILL. The test
//! folders sit where the no-elixir gate refuses any change, so the test is here (as bounded.rs).
use dash_meter::dash_test;
use dash_meter::{TestDir, monorepo};
use std::process::Command;
use std::time::{Duration, Instant};

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// On a failure, kills the pids the stand-in mix wrote: only pids this test started, never a pattern (dash hook no-pkill).
struct KillPids(std::path::PathBuf);

impl Drop for KillPids {
    fn drop(&mut self) {
        for pid in std::fs::read_to_string(&self.0)
            .unwrap_or_default()
            .split_whitespace()
        {
            let _ = Command::new("kill").args(["-9", pid]).status();
        }
    }
}

fn version(toolchain: &str, key: &str) -> String {
    toolchain
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .and_then(|v| v.split_whitespace().next())
        .unwrap_or_else(|| panic!("Found: no {key}= in bin/toolchain"))
        .to_string()
}

#[dash_test]
fn a_test_beam_killed_leaves_nothing_running() {
    let Some(top) = monorepo() else { return };
    let dir = TestDir::new("toolchain-leftovers");
    // as given, never canonicalized: macOS's temp folder is reached through a symlink (/var to /private/var), and a
    // process's folder reads as the physical path
    let t = dir.0.clone();
    let src = std::fs::read_to_string(top.join("private/dashboard/bin/toolchain"))
        .expect("bin/toolchain");
    // the install every checkout shares, under HOME, faked by its stamp as test-lock-test does
    let vers = format!(
        "otp-{}-elixir-{}-hex-{}-rebar-{}",
        version(&src, "OTP"),
        version(&src, "ELIXIR"),
        version(&src, "HEX"),
        version(&src, "REBAR")
    );
    let tc = t.join("home/.claude/run/toolchain").join(&vers);
    let app = t.join("app");
    for d in [
        tc.join("otp/bin"),
        app.join("bin"),
        app.join("test/dashboard"),
    ] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    std::fs::write(tc.join(format!("installed-{vers}")), "").expect("stamp");
    std::fs::write(app.join("test/dashboard/a_test.exs"), "").expect("a test file");
    let toolchain = app.join("bin/toolchain");
    std::fs::write(&toolchain, &src).expect("the copy");
    let pids = t.join("pids");
    // the stand-in mix, a beam: one process in its run folder (loop.sh's), one port child naming test/support/
    // (freework_standin.py's), each a single perl off the run's pipes (a stand-in left holding them would hang the
    // test, not fail it), then KILL of itself; with STANDIN_WAIT it waits instead, for a TERM to the toolchain, and
    // ends it with exit 0 as a beam does (init:stop); it writes its INT disposition, which an async run would ignore
    let mix = tc.join("otp/bin/mix");
    std::fs::write(
        &mix,
        format!(
            "#!/usr/bin/env bash\n\
             mkdir -p tmp/run-$$-1/w\n\
             (cd tmp/run-$$-1/w && exec perl -e 'sleep 120') > /dev/null 2>&1 & echo $! >> '{p}'\n\
             perl -e 'sleep 120' \"$PWD/test/support/standin\" > /dev/null 2>&1 & echo $! >> '{p}'\n\
             perl -e 'print $SIG{{INT}} || \"DEFAULT\"' > '{p}.int'\n\
             sleep 0.5\n\
             [ -z \"${{STANDIN_WAIT:-}}\" ] || exec perl -e '$SIG{{TERM}} = sub {{ exit 0 }}; sleep 60'\n\
             kill -KILL $$\n",
            p = pids.display()
        ),
    )
    .expect("mix");
    for f in [&toolchain, &mix] {
        Command::new("chmod")
            .arg("755")
            .arg(f)
            .status()
            .expect("chmod");
    }
    let _kill = KillPids(pids.clone());

    // a test file runs at once; CI's suite runs inside mix precommit (make dashboard/check), whose first mix call dies
    // here; a TERM to the toolchain itself (a hand, a timeout that signals its child) reaches the run, as exec did
    let file = &["test", "test/dashboard/a_test.exs"][..];
    for (args, term) in [(file, false), (&["precommit"][..], false), (file, true)] {
        let _ = std::fs::remove_file(&pids);
        let mut cmd = Command::new(&toolchain);
        cmd.arg("mix")
            .args(args)
            .current_dir(&app)
            .env("HOME", t.join("home"))
            .env_remove("CI")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if term {
            cmd.env("STANDIN_WAIT", "1");
        }
        let child = cmd.spawn().expect("bin/toolchain");
        if term {
            let end = Instant::now() + Duration::from_secs(20);
            while std::fs::read_to_string(&pids).map_or(0, |s| s.split_whitespace().count()) < 2
                && Instant::now() < end
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            std::thread::sleep(Duration::from_millis(700));
            Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status()
                .expect("kill");
        }
        let out = child.wait_with_output().expect("bin/toolchain");
        let err = String::from_utf8_lossy(&out.stderr);
        let started: Vec<String> = std::fs::read_to_string(&pids)
            .expect("the stand-ins' pids")
            .split_whitespace()
            .map(String::from)
            .collect();
        assert_eq!(
            started.len(),
            2,
            "Found: mix {args:?}: the stand-in mix started {started:?}; {err}"
        );
        let end = Instant::now() + Duration::from_secs(5);
        while started.iter().any(|p| alive(p)) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        let left: Vec<&String> = started.iter().filter(|p| alive(p)).collect();
        assert!(
            left.is_empty(),
            "Found: mix {args:?}: {left:?} still running after the run's end; toolchain said: {err}"
        );
        let int = std::fs::read_to_string(t.join("pids.int")).unwrap_or_default();
        assert_eq!(
            int, "DEFAULT",
            "Found: mix {args:?}: ctrl-c would not end the run"
        );
        assert!(
            err.contains("toolchain: stopping what a dead test beam left running:"),
            "Found: mix {args:?}: no stop line: {err}"
        );
    }
}
