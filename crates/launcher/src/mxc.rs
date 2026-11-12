//! A Claude Code session isolated by MXC (rust-dash-claude): the pane runs dash, and dash starts the agent through mxc-sdk
//! (Seatbelt on macOS, Bubblewrap on Linux) under the policy `sandbox-apply launch claude` would give it under safehouse. That
//! policy is the approved copy's own, printed by its `sandbox-apply profile claude`, and handed to Seatbelt whole: no rule is
//! written here, so nothing here can widen it.
//! ponytail: macOS only, as the approved policy is Seatbelt's; Linux takes a Bubblewrap policy once one is approved.
use super::{host, quoted};
use mxc_sdk::v1::configs::SeatbeltConfig;
use mxc_sdk::v1::{
    self, ContainerRequest, Containment, MxcPtyProcess, MxcPtySize, SpawnWithPtyOptions, WaitResult,
};
use std::fs::File;
use std::io::{IsTerminal, Read, Write};
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// What safehouse passes a session by default, its sanitized environment (PWD aside: mxc sets it). safehouse is pinned by
/// hash in sandbox/jobs.conf, so this list moves only with a commit bumping that hash. The last five are what the claude job's
/// `--enable=herdr` adds (safehouse 0.12.0): without them herdr delegate and session.sh fail in the session (/code-review).
/// ponytail: copied by hand; `sandbox-apply job` printing the final names once a second job runs here (task rust-dash-claude-env).
const SAFEHOUSE_ENV: [&str; 49] = [
    "HOME",
    "PATH",
    "SHELL",
    "TMPDIR",
    "USER",
    "LOGNAME",
    "TERM",
    "COLORTERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "EDITOR",
    "VISUAL",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_IDENTIFICATION",
    "TZ",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "NODE_EXTRA_CA_CERTS",
    "NO_BROWSER",
    "PLAYWRIGHT_MCP_SANDBOX",
    "SSH_AUTH_SOCK",
    "SDKROOT",
    "HERDR_ENV",
    "HERDR_SOCKET_PATH",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
];

/// The line the pane runs: the live dash, built from main, as the approved Makefile is the line of a task's pane. It applies
/// the sandbox, so it runs outside one.
pub fn pane_line(dash: &str, data: &str) -> String {
    format!(
        "exec {} claude --in-pane --data {}",
        quoted(dash),
        quoted(data)
    )
}

/// `sandbox-apply <verb> <job>` of the approved copy, in `folder`: its output.
/// ponytail: unbounded, as the launch it mirrors is; a bound once a slow safehouse shows.
fn approved(home: &str, folder: &str, verb: &str, job: &str) -> Result<String, String> {
    let script = format!("{}/bin/sandbox-apply", host(home));
    let said = |why: String| format!("{script} {verb} {job}: {why}");
    let ran = Command::new(&script)
        .args([verb, job])
        .current_dir(folder)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| said(e.to_string()))?;
    if !ran.status.success() {
        return Err(said(String::from_utf8_lossy(&ran.stderr).trim().into()));
    }
    String::from_utf8(ran.stdout).map_err(|e| said(e.to_string()))
}

/// The approved copy's policy for `job`, for a session working in `folder`, the checkout its workdir grant names.
pub fn approved_profile(home: &str, folder: &str, job: &str) -> Result<String, String> {
    approved(home, folder, "profile", job)
}

/// A job as the approved copy's launch runs it: its command, the names its profile's flags pass (--env-pass), and the
/// account a claude start runs on, when one is named.
#[derive(Debug, Default, PartialEq)]
pub struct Job {
    pub command: String,
    pub env_pass: Vec<String>,
    pub account: Option<String>,
}

/// `sandbox-apply job`'s lines, the one parser of sandbox/jobs.conf being its own: each command word quoted for mxc's sh.
pub fn job_of(lines: &str) -> Result<Job, String> {
    let mut job = Job::default();
    let mut words = vec![];
    for line in lines.lines() {
        match line.split_once(' ') {
            Some(("command", word)) => words.push(quoted(word)),
            Some(("env-pass", name)) => job.env_pass.push(name.into()),
            Some(("account", folder)) => job.account = Some(folder.into()),
            _ => return Err(format!("sandbox-apply job: a line it never prints: {line}")),
        }
    }
    if words.is_empty() {
        return Err("sandbox-apply job: no command".into());
    }
    job.command = words.join(" ");
    Ok(job)
}

/// The approved copy's `job`, for a session in `folder`.
pub fn approved_job(home: &str, folder: &str, job: &str) -> Result<Job, String> {
    job_of(&approved(home, folder, "job", job)?)
}

/// The environment of the session, as `launch claude` gives it: safehouse's names and the job's --env-pass names the pane has,
/// its session marker (`DASH_SESSION` when set and not empty, else the pane), its account and the two it exports. The pane's own
/// CLAUDE_CONFIG_DIR never passes: launch unsets it for the account it picks. Nor does CLAUDE_SUPERVISOR: no supervisor runs
/// under dash claude, so the session's hooks would signal the pane's parent's (/code-review).
pub fn environment(
    job: &Job,
    pane: &str,
    vars: impl Iterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let passed = |k: &str| SAFEHOUSE_ENV.contains(&k) || job.env_pass.iter().any(|n| n == k);
    let (mut marker, mut env) = (pane.to_string(), vec![]);
    for (k, v) in vars {
        match k.as_str() {
            "DASH_SESSION" if !v.is_empty() => marker = v,
            "DASH_SESSION" | "CLAUDE_CONFIG_DIR" | "CLAUDE_SUPERVISOR" => {}
            _ if passed(&k) => env.push((k, v)),
            _ => {}
        }
    }
    let account = job.account.as_deref().map(|a| ("CLAUDE_CONFIG_DIR", a));
    // As launch claude exports them: its marker, Codex told it is confined, no feedback prompt.
    let set = [
        ("DASH_SESSION", marker.as_str()),
        ("CODEX_SANDBOX_MODE", "danger-full-access"),
        ("CLAUDE_CODE_SEND_FEEDBACK", "0"),
    ];
    for (k, v) in set.into_iter().chain(account) {
        env.retain(|(name, _)| name != k);
        env.push((k.into(), v.into()));
    }
    env
}

/// `command` in `folder` under `profile` and nothing else: no filesystem, network or UI policy of mxc's own.
pub fn request(
    profile: &str,
    folder: &str,
    command: &str,
    env: Vec<(String, String)>,
) -> ContainerRequest {
    let mut seatbelt = SeatbeltConfig::default();
    seatbelt.profile_override = Some(profile.into());
    let mut request = ContainerRequest::new(command);
    request.containment = Containment::Seatbelt(seatbelt);
    request.working_directory = Some(folder.into());
    request.environment = Some(env);
    request
}

/// The terminal on fd 0, raw while the session runs, as it was after.
struct Raw(Option<libc::termios>);

impl Raw {
    fn on() -> Self {
        let mut was = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(0, &mut was) } != 0 {
            return Self(None);
        }
        let mut raw = was;
        unsafe { libc::cfmakeraw(&mut raw) };
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) };
        Self(Some(was))
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        if let Some(was) = self.0 {
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &was) };
        }
    }
}

fn size() -> MxcPtySize {
    let mut w = unsafe { std::mem::zeroed::<libc::winsize>() };
    if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut w) } != 0 || w.ws_row == 0 {
        return MxcPtySize::default();
    }
    MxcPtySize {
        rows: w.ws_row,
        cols: w.ws_col,
        ..MxcPtySize::default()
    }
}

/// Runs `request` on this terminal, through mxc's own terminal: keys in, screen out, the pane's size followed. Its exit code.
/// A closed pane ends the session: its keys reach their end, so the session is killed and its end still recorded.
/// ponytail: a SIGKILL of dash leaves the row « running »; a liveness look on the page if one is ever left.
pub fn in_terminal(request: ContainerRequest) -> Result<i32, String> {
    let options = SpawnWithPtyOptions {
        size: size(),
        ..Default::default()
    };
    let child = v1::spawn_with_pty(request, options).map_err(|e| e.to_string())?;
    let streams = child
        .try_clone_reader()
        .and_then(|r| Ok((r, child.take_writer()?)));
    let (mut screen, mut keys) = streams.map_err(|e| {
        // A kill that failed is never waited on: the session could run on with no terminal to end it.
        if child.kill().is_ok() {
            let _ = child.wait();
        }
        e.to_string()
    })?;
    // The pane's hang-up would end dash before the session's end is written.
    unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
    let raw = Raw::on();
    // Only a terminal's end is a closed pane: a stdin that was never one ends at once.
    let pane = std::io::stdin().is_terminal();
    let gone = Arc::new(AtomicBool::new(false));
    let keys_gone = Arc::clone(&gone);
    // Blocked on a key once the session ends: the process exits under it.
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = std::io::stdin().read(&mut buf) {
            if keys
                .write_all(&buf[..n])
                .and_then(|_| keys.flush())
                .is_err()
            {
                return;
            }
        }
        keys_gone.store(pane, Ordering::Release);
    });
    // Unbuffered: stdout's LineWriter would hold a redraw that ends with no newline.
    let shown = std::thread::spawn(move || {
        let mut out = ManuallyDrop::new(unsafe { File::from_raw_fd(1) });
        std::io::copy(&mut screen, &mut *out)
    });
    let outcome = watched(&child, &gone);
    // A descendant left holding the terminal never holds this one open.
    if let Some(closer) = child.stdout_closer() {
        closer.close();
    }
    let _ = shown.join();
    drop(raw);
    match outcome.map_err(|e| e.to_string())? {
        WaitResult::Exited(code) => Ok(code),
        WaitResult::TimedOut => Ok(124),
    }
}

/// The session waited on, its terminal kept the pane's size, and killed once the pane is gone.
fn watched(child: &MxcPtyProcess, gone: &AtomicBool) -> std::io::Result<WaitResult> {
    let ended = AtomicBool::new(false);
    std::thread::scope(|s| {
        // ponytail: a look every half second, no SIGWINCH handler; one if a resize lags.
        s.spawn(|| {
            let mut was = size();
            while !ended.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(500));
                // A kill that failed is tried again next look: the session never runs on with no pane (/code-review).
                if gone.load(Ordering::Acquire) && child.kill().is_ok() {
                    return;
                }
                let now = size();
                if now != was && child.resize(now).is_ok() {
                    was = now;
                }
            }
        });
        let outcome = child.wait();
        ended.store(true, Ordering::Release);
        outcome
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    #[dash_test]
    fn the_pane_runs_the_live_dash_on_its_data() {
        assert_eq!(
            pane_line("/r/private/live/rust/dash", "/d's"),
            r"exec '/r/private/live/rust/dash' claude --in-pane --data '/d'\''s'"
        );
    }

    #[dash_test]
    fn the_session_gets_the_approved_profile_and_no_policy_of_mxc_s_own() {
        let request = request("(version 1)(deny default)", "/w", "claude", vec![]);
        let mut seatbelt = SeatbeltConfig::default();
        seatbelt.profile_override = Some("(version 1)(deny default)".into());
        assert_eq!(request.containment, Containment::Seatbelt(seatbelt));
        assert!(request.filesystem.is_none() && request.network.is_none() && request.ui.is_none());
        assert_eq!(
            (
                request.command.as_str(),
                request.working_directory.as_deref()
            ),
            ("claude", Some("/w"))
        );
        assert_eq!(
            request.timeout_ms, None,
            "a session runs as long as he works in it"
        );
    }

    #[dash_test]
    fn the_session_s_environment_is_safehouse_s_names_the_job_s_and_its_marker() {
        let env_of = |job: &Job, pane: &[(&str, &str)]| {
            let vars = pane.iter().map(|(k, v)| (k.to_string(), v.to_string()));
            let env = environment(job, "p1", vars);
            env.into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
        };
        let job = Job {
            env_pass: vec![
                "CLAUDE_CONFIG_DIR".into(),
                "HERDR_DELEGATE".into(),
                "CLAUDE_SUPERVISOR".into(),
            ],
            ..Job::default()
        };
        let pane = [
            ("HOME", "/h"),
            ("CLAUDE_SUPERVISOR", "4242"),
            ("AWS_SECRET_ACCESS_KEY", "x"),
            ("HERDR_DELEGATE", "1"),
            ("HERDR_PANE_ID", "3"),
            ("CLAUDE_CONFIG_DIR", "/pane-s"),
            ("CODEX_SANDBOX_MODE", "y"),
        ];
        assert_eq!(
            env_of(&job, &pane),
            [
                "HOME=/h",
                "HERDR_DELEGATE=1",
                "HERDR_PANE_ID=3",
                "DASH_SESSION=p1",
                "CODEX_SANDBOX_MODE=danger-full-access",
                "CLAUDE_CODE_SEND_FEEDBACK=0"
            ],
            "the pane's account never passes"
        );
        let picked = Job {
            account: Some("/accounts/second".into()),
            ..Job::default()
        };
        let env = env_of(&picked, &[("DASH_SESSION", "rust-x")]);
        assert!(
            env.contains(&"DASH_SESSION=rust-x".into()),
            "a launch's own marker is kept: {env:?}"
        );
        assert!(
            env.contains(&"CLAUDE_CONFIG_DIR=/accounts/second".into()),
            "{env:?}"
        );
        let env = env_of(&job, &[("DASH_SESSION", "")]);
        assert!(
            env.contains(&"DASH_SESSION=p1".into()),
            "an empty one reads unset: {env:?}"
        );
    }

    #[dash_test]
    fn the_job_is_sandbox_apply_s_lines_each_word_quoted() {
        let lines = "command claude\ncommand two words\nenv-pass A\nenv-pass B\naccount /a b\n";
        let job = job_of(lines).expect("job");
        let want = Job {
            command: "'claude' 'two words'".into(),
            env_pass: vec!["A".into(), "B".into()],
            account: Some("/a b".into()),
        };
        assert_eq!(job, want);
        assert!(job_of("env-pass A\n").is_err(), "no command");
        assert!(
            job_of("command x\nhealth=y\n").is_err(),
            "a line it never prints"
        );
    }
}

/// A file reading « probe » in `dir`, by its real path: what the seatbelt and bubblewrap checks read.
#[cfg(test)]
fn probe(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).expect("probe dir");
    let file = dir.join("probe");
    std::fs::write(&file, "probe").expect("probe");
    file.canonicalize().expect("real path")
}

/// Done-when 2: a path safehouse's policy grants reads under mxc as under sandbox-exec, and one it denies is denied by both.
/// The policy is safehouse's own for a scratch workdir; the session's is the approved copy's, through the same `request`.
/// Ignored, as Seatbelt refuses a second sandbox inside one (a session, CI has no Mac): from his terminal on the Mac,
/// `cargo nextest run -p dash-launcher --run-ignored only seatbelt`. Run inside a sandbox, each fails saying so.
#[cfg(all(test, target_os = "macos"))]
mod seatbelt {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    /// Safehouse's policy for `workdir`.
    fn safehouse_policy(workdir: &std::path::Path) -> String {
        let nested = Command::new("sandbox-exec")
            .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
            .status();
        assert!(
            nested.is_ok_and(|s| s.success()),
            "inside a sandbox: Seatbelt refuses a second one, so this proves nothing here"
        );
        let out = Command::new("safehouse")
            .arg("--workdir")
            .arg(workdir)
            .arg("--stdout")
            .output()
            .expect("safehouse installed (make claude/setup)");
        assert!(out.status.success(), "safehouse --stdout failed");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `cat <file>` under the policy: (through mxc, through sandbox-exec), each true when it read the file.
    fn read_both(policy: &str, workdir: &std::path::Path, file: &std::path::Path) -> (bool, bool) {
        let folder = workdir.to_str().expect("utf-8");
        let env = vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), folder.into()),
        ];
        let line = format!("/bin/cat {}", quoted(file.to_str().expect("utf-8")));
        let mxc =
            v1::run(request(policy, folder, &line, env), Default::default()).expect("mxc runs");
        let mxc = mxc.outcome == WaitResult::Exited(0) && mxc.stdout == b"probe";
        let policy_file = workdir.join("policy.sb");
        std::fs::write(&policy_file, policy).expect("policy file");
        let exec = Command::new("sandbox-exec")
            .arg("-f")
            .arg(&policy_file)
            .arg("/bin/cat")
            .arg(file)
            .output();
        let exec = exec.expect("sandbox-exec runs");
        (mxc, exec.status.success() && exec.stdout == b"probe")
    }

    #[dash_test]
    #[ignore = "on the Mac outside any sandbox"]
    fn a_path_safehouse_grants_reads_in_the_session() {
        let work = TestDir::new("mxc-granted");
        let workdir = work.0.canonicalize().expect("real path");
        let policy = safehouse_policy(&workdir);
        assert_eq!(
            read_both(&policy, &workdir, &probe(&workdir.join("in"))),
            (true, true)
        );
    }

    #[dash_test]
    #[ignore = "on the Mac outside any sandbox"]
    fn a_path_safehouse_denies_is_denied_in_the_session() {
        let work = TestDir::new("mxc-denied");
        let workdir = work.0.canonicalize().expect("real path");
        let policy = safehouse_policy(&workdir);
        // Under HOME, outside the workdir and every grant: temporary folders are granted, so the probe sits in neither.
        let home = std::env::var("HOME").expect("HOME");
        let outside =
            std::path::Path::new(&home).join(format!(".dash-mxc-probe-{}", std::process::id()));
        // Removed however the check ends: a failed run leaves nothing in HOME.
        let _gone = TestDir(outside.clone());
        let file = probe(&outside);
        assert_eq!(read_both(&policy, &workdir, &file), (false, false));
    }
}

/// Done-when 2 on Linux CI, through mxc's Bubblewrap backend (@nha, 2026-11-07: « Prove denied and allowed paths in Linux CI
/// with Bubblewrap; the Mac proof is the first live dash claude run after his tap »): a session granted its workdir only, as
/// safehouse grants it, reads a file there, and a file in a sibling folder outside every grant is denied. Linux has no approved
/// profile yet, so this grant is the test's own; the Mac's approved one is checked live. CI installs bwrap (ci.yml).
#[cfg(all(test, target_os = "linux"))]
mod bubblewrap {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;
    use mxc_sdk::v1::policy::FilesystemPolicy;

    /// `cat <file>` from a session granted `workdir` read-write and nothing else: true when it read the file.
    fn reads(workdir: &std::path::Path, file: &std::path::Path) -> bool {
        let folder = workdir.to_str().expect("utf-8");
        let line = format!("cat {}", quoted(file.to_str().expect("utf-8")));
        let env = vec![("PATH".into(), "/usr/bin:/bin".into())];
        let mut request = ContainerRequest::new(line);
        request.containment = Containment::Bubblewrap;
        request.working_directory = Some(folder.into());
        request.environment = Some(env);
        request.filesystem = Some(FilesystemPolicy {
            readwrite_paths: vec![folder.into()],
            ..Default::default()
        });
        let ran =
            v1::run(request, Default::default()).expect("mxc runs bwrap (ci.yml installs it)");
        ran.outcome == WaitResult::Exited(0) && ran.stdout == b"probe"
    }

    #[dash_test]
    fn a_path_the_session_is_granted_reads_under_bubblewrap() {
        let work = TestDir::new("bwrap-granted");
        let workdir = work.0.canonicalize().expect("real path");
        assert!(reads(&workdir, &probe(&workdir.join("in"))));
    }

    #[dash_test]
    fn a_path_outside_every_grant_is_denied_under_bubblewrap() {
        let (work, outside) = (TestDir::new("bwrap-work"), TestDir::new("bwrap-outside"));
        let workdir = work.0.canonicalize().expect("real path");
        let file = probe(&outside.0);
        assert!(file.exists(), "the file is there on the host");
        assert!(!reads(&workdir, &file));
    }
}
