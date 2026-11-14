//! A command line tool's call, bounded: a tool that hangs never holds a run. Every adapter on such a tool takes it.
//!
//! The tool starts in its own process group, and a kill ends the whole group: a wrapper's child (`sh -c`, a script) would
//! outlive the wrapper and hold a reader thread on its pipe (/code-review of PR 1162). Its own group is also outside a
//! terminal's foreground group, so a tool that asks on /dev/tty is stopped by SIGTTIN, never prompts, and fails by its
//! limit (task process-run-group-kill: chosen over setsid, which would need an unsafe pre_exec for an earlier failure).
//! The cost: a kill of dash's own group (a Ctrl-C, `bin/bounded`) no longer reaches the tool, which then runs to its own
//! end or dies writing to its closed pipe, the trade `ext::pod` makes too; while dash lives, the call's limit ends it.
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Now in Unix milliseconds: a clock set before 1970 reads 0.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Now in Unix seconds.
pub fn now_s() -> u64 {
    now_ms() as u64 / 1000
}

/// What a call holds of a tool's stdout, in bytes. Past it the tool is killed and the call fails naming it, under the memory cap.
pub const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// What a failure's reason keeps of stderr: its end, where a tool says what went wrong. The rest is read and dropped: a noisy tool
/// is not a failed one.
const REASON_LIMIT: usize = 64 * 1024;

/// Stdout; a failure names what it said on stderr, which the task's record then keeps.
pub fn run(
    bin: &str,
    args: &[&str],
    dir: Option<&Path>,
    limit: Duration,
) -> Result<String, String> {
    let (status, text, reason) = answer(bin, args, dir, &[], limit)?;
    if status.success() {
        return Ok(text);
    }
    let what = format!("{bin} {}", args.join(" "));
    Err(format!(
        "{what}: {status}: {} {}",
        reason.trim(),
        text.trim()
    ))
}

/// No answer from a tool. `started`: it ran, so it may have done its work (and spent) before it was killed or lost.
#[derive(Debug)]
pub struct NoAnswer {
    pub started: bool,
    pub why: String,
}

impl From<NoAnswer> for String {
    fn from(no: NoAnswer) -> String {
        no.why
    }
}

/// Its exit status, stdout and, on a failure, stderr's end, whatever the status: a tool that failed may still say what it did.
/// `env` is set in the tool's environment alone, never on its command line, so no failure's words name it.
pub fn answer(
    bin: &str,
    args: &[&str],
    dir: Option<&Path>,
    env: &[(&str, &str)],
    limit: Duration,
) -> Result<(std::process::ExitStatus, String, String), NoAnswer> {
    answer_fed(bin, args, dir, env, None, limit)
}

/// `answer`, `stdin` written to the tool's input then closed: a text kept off its command line, where a tool may read a
/// leading `-` as a flag (/code-review of rust-task-page-edits: the description « --dry-run is broken » was refused).
pub fn answer_fed(
    bin: &str,
    args: &[&str],
    dir: Option<&Path>,
    env: &[(&str, &str)],
    stdin: Option<&str>,
    limit: Duration,
) -> Result<(std::process::ExitStatus, String, String), NoAnswer> {
    let what = format!("{bin} {}", args.join(" "));
    let ran = |why: String| NoAnswer { started: true, why };
    let failed = |e: std::io::Error| ran(format!("{what}: {e}"));
    let past = || ran(format!("{what}: its output is past {OUTPUT_LIMIT} bytes"));
    let mut command = Command::new(bin);
    command
        .args(args)
        .envs(env.iter().copied())
        .stdin(match stdin {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(|e| NoAnswer {
        started: false,
        why: format!("{what}: {e}"),
    })?;
    // on its own thread, so a tool that answers before it reads never blocks the wait; dropped there, the input closes
    if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = input.write_all(text.as_bytes());
        });
    }
    let out = drain(child.stdout.take().expect("piped"));
    let err = drain_reason(child.stderr.take().expect("piped"));
    // one deadline for the whole call: the waits after the leader's exit take what is left, never a limit each (a second,
    // at least, for a tool that ended at the deadline to have its pipes read)
    let deadline = Instant::now() + limit;
    let left = || {
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_secs(1))
    };
    let mut text = None;
    let succeeded = loop {
        text = text.or_else(|| out.try_recv().ok());
        if let Some(None) = &text {
            end(&mut child);
            return Err(past());
        }
        match exited(&child) {
            Ok(Some(succeeded)) => break succeeded,
            Ok(None) => {}
            // ECHILD: another reaper took the leader, so its group id may be a stranger's now: no kill (/code-review)
            Err(e) => return Err(failed(e)),
        }
        if Instant::now() > deadline {
            end(&mut child);
            return Err(ran(format!("{what}: no answer in {limit:?}")));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // The leader is still unreaped, so its group id stays its own: each failure below ends what it left behind (a process
    // holding the pipes) and no stranger's group. Bounded too.
    let text = match text.map(Ok).unwrap_or_else(|| out.recv_timeout(left())) {
        Ok(Some(text)) => text,
        Ok(None) => {
            end(&mut child);
            return Err(past());
        }
        Err(_) => {
            end(&mut child);
            return Err(ran(format!("{what}: its output never closed")));
        }
    };
    let reason = match succeeded {
        true => String::new(),
        false => err.recv_timeout(left()).unwrap_or_else(|_| {
            end(&mut child);
            String::new()
        }),
    };
    let status = child.wait().map_err(failed)?;
    Ok((status, text, reason))
}

/// Whether the tool ended, and with success, its leader left unreaped (WNOWAIT) so its group id cannot pass to another.
fn exited(child: &std::process::Child) -> std::io::Result<Option<bool>> {
    // SAFETY: a zeroed siginfo_t is valid, and waitid writes only into it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    if unsafe { libc::waitid(libc::P_PID, child.id(), &mut info, flags) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if info.si_signo == 0 {
        return Ok(None);
    }
    // SAFETY: waitid filled a SIGCHLD siginfo, whose status field is the one set.
    let status = unsafe { info.si_status() };
    Ok(Some(info.si_code == libc::CLD_EXITED && status == 0))
}

/// The tool's whole group, then its leader reaped: a wrapper's child dies with it.
fn end(child: &mut std::process::Child) {
    // SAFETY: killpg takes no pointer; the group is the tool's own (process_group(0)) and its leader is not yet reaped,
    // so the id names no other group. ESRCH, a group already gone, needs nothing.
    unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
    let _ = child.wait();
}

/// A pipe read beside the wait: an answer past its size would block the tool and read as a hang. The text, or none past
/// `OUTPUT_LIMIT`: the limit counts the text, since a stray byte's replacement is up to three times its size, and never smaller.
fn drain(pipe: impl Read + Send + 'static) -> Receiver<Option<String>> {
    let (send, receive) = channel();
    std::thread::spawn(move || {
        let mut bytes = vec![];
        let _ = pipe.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes);
        let text = text_of(bytes);
        let _ = send.send((text.len() as u64 <= OUTPUT_LIMIT).then_some(text));
    });
    receive
}

fn drain_reason(mut pipe: impl Read + Send + 'static) -> Receiver<String> {
    let (send, receive) = channel();
    std::thread::spawn(move || {
        let (mut tail, mut chunk) = (vec![], [0; 8192]);
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => tail.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
            if tail.len() > 2 * REASON_LIMIT {
                tail.drain(..tail.len() - REASON_LIMIT);
            }
        }
        let _ = send.send(text_of(
            tail.split_off(tail.len().saturating_sub(REASON_LIMIT)),
        ));
    });
    receive
}

/// No copy of valid text: at the limit a second one is 16 MB for nothing.
fn text_of(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
