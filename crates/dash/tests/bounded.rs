//! bin/bounded, the port of private/dashboard/test/scripts/bounded-test's default run (task bounded-test-under-load): the
//! script sits in a folder the no-elixir gate refuses any change to, so its terminal read is fixed here. The script's
//! `vite` half (node) stays there until scripts_test.exs goes, then the script goes whole.
use dash_meter::memory::TestBudget;
use dash_meter::{TestDir, monorepo};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// On a panic, kills the one pid written in its file (the shell script's EXIT trap): only a pid the test started, never
/// a pattern (.claude/hooks/no-pkill.py: a pkill and a kill fed by pgrep -f each stopped Claude sessions). A pass leaves
/// it alone: bin/bounded ended it, and its number may be another process's by then.
struct KillPid(PathBuf);

impl Drop for KillPid {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        if let Ok(pid) = std::fs::read_to_string(&self.0) {
            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        }
    }
}

fn bounded(top: &Path) -> PathBuf {
    top.join("bin/bounded")
}

fn code(cmd: &mut Command) -> i32 {
    cmd.stderr(Stdio::null())
        .status()
        .expect("spawn")
        .code()
        .expect("an exit code, no signal")
}

/// What pgrep finds started from `dir`: bin/bounded returns once its group is gone, so no wait before the look.
fn left(dir: &Path) -> String {
    let out = Command::new("pgrep")
        .args(["-lf", "--", &format!("{}/", dir.display())])
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A missing command exits 127, where the perl alarm idiom it replaces exits 0.
#[test]
fn a_missing_command_exits_127() {
    let _budget = TestBudget::new();
    let Some(top) = monorepo() else { return };
    let dir = TestDir::new("bounded-missing");
    let missing = dir.0.join("missing");
    assert_eq!(
        code(Command::new(bounded(&top)).arg("5").arg(&missing)),
        127
    );
    let perl = Command::new("perl")
        .args(["-e", "alarm shift; exec @ARGV", "5"])
        .arg(&missing)
        .stderr(Stdio::null())
        .status()
        .expect("perl");
    assert_eq!(
        perl.code(),
        Some(0),
        "the perl idiom no longer exits 0: this check's why is gone"
    );
}

/// The command's exit code passes through and the child it left in its group is ended; the limit exits 124.
#[test]
fn the_exit_code_passes_and_the_group_ends_with_its_command() {
    let _budget = TestBudget::new();
    let Some(top) = monorepo() else { return };
    let dir = TestDir::new("bounded-group");
    let sleeper = dir.0.join("sleeper");
    let _kill = KillPid(dir.0.join("sleeper.pid"));
    std::fs::write(&sleeper, "#!/bin/sh\necho $$ > \"$0.pid\"\nexec sleep 30\n").expect("sleeper");
    assert!(
        Command::new("chmod")
            .arg("+x")
            .arg(&sleeper)
            .status()
            .expect("chmod")
            .success()
    );
    let inner = format!("{} & exit 3", sleeper.display());
    assert_eq!(
        code(Command::new(bounded(&top)).args(["5", "sh", "-c", &inner])),
        3
    );
    let l = left(&dir.0);
    assert!(
        l.is_empty(),
        "a child left in the group outlived its command: {l}"
    );
    assert_eq!(
        code(Command::new(bounded(&top)).args(["1", "sleep", "10"])),
        124
    );
}

/// The terminal read needs BSD script's argument order, so it and its helpers build on macOS only: a helper left
/// outside this module is dead code on Linux, where the mirror's clippy refuses it (PR 1452's red, 2026-11-11).
#[cfg(target_os = "macos")]
mod terminal {
    use super::{TestBudget, bounded, monorepo};
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};

    /// script, killed and waited for if the test panics first: its pty's hangup then ends bin/bounded's group.
    struct Reap(std::process::Child);

    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Read `from` until `want` is in what was read, or its end: no timer, bin/bounded's limit ends script and so the read.
    /// The bytes stay whole until then: a character split across two reads decodes as one.
    fn read_until(from: &mut impl Read, out: &mut Vec<u8>, want: &str) -> bool {
        let mut buf = [0u8; 256];
        while !String::from_utf8_lossy(out).contains(want) {
            match from.read(&mut buf).expect("read script's output") {
                0 => return false,
                n => out.extend_from_slice(&buf[..n]),
            }
        }
        true
    }

    /// On a terminal (BSD script's pty) the group takes the foreground, so a read gets its line, not SIGTTIN until the limit.
    /// The line goes once the command prints « ready », inside the pty; script's stdin stays open until « got » is read back,
    /// never on a timer: its end is script's end of input, a ^D that beat the line at load 18 (PR 910) and at load 30 with
    /// the shell writer's fixed waits (PRs 964, 961, 963).
    #[test]
    fn on_a_terminal_the_command_reads_its_line() {
        let _budget = TestBudget::new();
        let Some(top) = monorepo() else { return };
        let mut script = Reap(
            Command::new("script")
                .args(["-q", "/dev/null"])
                .arg(bounded(&top))
                .args(["10", "sh", "-c", "echo ready; read x; echo \"got $x\""])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("script"),
        );
        let mut stdin = script.0.stdin.take().expect("stdin");
        let mut stdout = script.0.stdout.take().expect("stdout");
        let mut out = vec![];
        let ready = read_until(&mut stdout, &mut out, "ready");
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(
            ready,
            "script ended before the command printed « ready »: {text:?}"
        );
        stdin.write_all(b"hi\n").expect("write the line");
        read_until(&mut stdout, &mut out, "got");
        drop(stdin);
        let _ = stdout.read_to_end(&mut out);
        let status = script.0.wait().expect("wait script");
        let out = String::from_utf8_lossy(&out);
        assert!(
            status.success(),
            "a read on a terminal, script exited {status}: {out:?}"
        );
        assert!(
            out.contains("got hi"),
            "a read on a terminal never got its line: {out:?}"
        );
    }
}
