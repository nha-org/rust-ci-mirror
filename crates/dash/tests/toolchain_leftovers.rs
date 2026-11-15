//! private/dashboard/bin/toolchain's leftovers (task ci-standins-reaped): a test beam killed with KILL skips its on_exit,
//! and leftovers_test's loop.sh ran 37 h past it, freework_standin.py 47 h. The toolchain stays the run's parent and,
//! once it ends, stops what it left: a process in a run folder tmp/run-<pid>-* of a dead beam, and an orphan whose
//! command line names test/support/. Here a stand-in mix starts one of each, then kills itself with KILL. The test
//! folders sit where the no-elixir gate refuses any change, so the test is here (as bounded.rs).
use dash_meter::dash_test;
use dash_meter::{TestDir, monorepo};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{Duration, Instant};

fn alive(pid: &str) -> bool {
    // SAFETY: signal 0 sends nothing, it asks whether the pid is there
    pid.parse()
        .is_ok_and(|p: libc::pid_t| unsafe { libc::kill(p, 0) } == 0)
}

/// On a failure, kills the pids the stand-in mix wrote: only pids this test started, never a pattern (dash hook no-pkill).
/// A pass leaves them alone, as bounded.rs's KillPid: the toolchain ended them, and their numbers may be others' by then.
struct KillPids(std::path::PathBuf);

impl Drop for KillPids {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        for pid in std::fs::read_to_string(&self.0)
            .unwrap_or_default()
            .split_whitespace()
        {
            if let Ok(p) = pid.parse::<libc::pid_t>() {
                // SAFETY: a pid this test's stand-in mix started, killed on a failure only
                unsafe { libc::kill(p, libc::SIGKILL) };
            }
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
    let int = t.join("pids.int");
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
             sleep 0.5\n\
             [ -z \"${{STANDIN_WAIT:-}}\" ] || exec perl -e '$SIG{{TERM}} = sub {{ exit 0 }}; open my $f, \">\", \"{p}.int\"; print $f $SIG{{INT}} || \"DEFAULT\"; close $f; sleep 60'\n\
             perl -e 'print $SIG{{INT}} || \"DEFAULT\"' > '{p}.int'\n\
             kill -KILL $$\n",
            p = pids.display()
        ),
    )
    .expect("mix");
    for f in [&toolchain, &mix] {
        std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let _kill = KillPids(pids.clone());

    // a test file runs at once; CI's suite runs inside mix precommit (make dashboard/check), whose first mix call dies
    // here, as does the suite's own (mix test, under the lock); a TERM to the toolchain itself (a hand, a timeout that
    // signals its child) reaches the run, as exec did; an INT to its process group (Ctrl-C) stops the run too: a
    // background command of a non-interactive bash ignores INT unless the toolchain resets it (/code-review)
    let file = &["test", "test/dashboard/a_test.exs"][..];
    for (args, signal) in [
        (file, None),
        (&["precommit"][..], None),
        (&["test"][..], None),
        (file, Some("TERM")),
        (file, Some("INT")),
    ] {
        let _ = std::fs::remove_file(&pids);
        // each case writes its own: an earlier case's DEFAULT must not pass a later one (/code-review of PR 1621)
        let _ = std::fs::remove_file(&int);
        let mut cmd = Command::new(&toolchain);
        cmd.arg("mix")
            .args(args)
            .current_dir(&app)
            .env("HOME", t.join("home"))
            .env_remove("CI")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if signal.is_some() {
            // its own process group, so that the INT below is Ctrl-C's without reaching the test runner
            cmd.env("STANDIN_WAIT", "1").process_group(0);
        }
        let child = cmd.spawn().expect("bin/toolchain");
        if let Some(signal) = signal {
            let end = Instant::now() + Duration::from_secs(20);
            // pids.int is written by the waiting perl, the beam's stand-in, once its TERM handler is set: a signal before
            // it would reach the mix's bash, which dies of it on its own
            while std::fs::read_to_string(&int).map_or(true, |s| s.is_empty())
                && Instant::now() < end
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            let pid = child.id() as libc::pid_t;
            // SAFETY: the pid, and the process group, of the child this test just started
            match signal {
                "INT" => unsafe { libc::killpg(pid, libc::SIGINT) },
                _ => unsafe { libc::kill(pid, libc::SIGTERM) },
            };
        }
        let sent = Instant::now();
        let out = child.wait_with_output().expect("bin/toolchain");
        assert!(
            signal.is_none() || sent.elapsed() < Duration::from_secs(10),
            "Found: mix {args:?}: {signal:?} did not stop the run, which ended by itself after {:?}",
            sent.elapsed()
        );
        let err = String::from_utf8_lossy(&out.stderr);
        // a TERM passed on is a stop, though the stand-in, as a beam, answers it with 0 (/code-review of PR 1621)
        if signal == Some("TERM") {
            assert_eq!(
                out.status.code(),
                Some(143),
                "Found: mix {args:?}: TERM's exit; {err}"
            );
        }
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
        let disposition = std::fs::read_to_string(&int).unwrap_or_default();
        assert_eq!(
            disposition, "DEFAULT",
            "Found: mix {args:?}: ctrl-c would not end the run"
        );
        assert!(
            err.contains("toolchain: stopping what a dead test beam left running:"),
            "Found: mix {args:?}: no stop line: {err}"
        );
    }
}
