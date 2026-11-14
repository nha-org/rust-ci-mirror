//! A command line tool's call, bounded: a tool that hangs never holds a run. Every adapter on such a tool takes it.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

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
        .stderr(Stdio::piped());
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
    let start = Instant::now();
    let mut text = None;
    let status = loop {
        text = text.or_else(|| out.try_recv().ok());
        if let Some(None) = &text {
            let _ = child.kill();
            let _ = child.wait();
            return Err(past());
        }
        if let Some(status) = child.try_wait().map_err(failed)? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ran(format!("{what}: no answer in {limit:?}")));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Bounded too: a process it left behind may hold the pipe open.
    let text = match text {
        Some(text) => text,
        None => out
            .recv_timeout(limit)
            .map_err(|_| ran(format!("{what}: its output never closed")))?,
    };
    let text = text.ok_or_else(past)?;
    if status.success() {
        return Ok((status, text, String::new()));
    }
    let reason = err.recv_timeout(limit).unwrap_or_default();
    Ok((status, text, reason))
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
