//! A command line tool's call, bounded: a tool that hangs never holds a run. Every adapter on such a tool takes it.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// Stdout; a failure names what it said on stderr, which the task's record then keeps.
pub fn run(
    bin: &str,
    args: &[&str],
    dir: Option<&Path>,
    limit: Duration,
) -> Result<String, String> {
    let what = format!("{bin} {}", args.join(" "));
    let failed = |e: std::io::Error| format!("{what}: {e}");
    let mut command = Command::new(bin);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(failed)?;
    let out = drain(child.stdout.take().expect("piped"));
    let err = drain(child.stderr.take().expect("piped"));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(failed)? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{what}: no answer in {limit:?}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Bounded too: a process it left behind may hold the pipe open.
    let text = out.recv_timeout(limit);
    let text = text.map_err(|_| format!("{what}: its output never closed"))?;
    if status.success() {
        return Ok(text);
    }
    let reason = err.recv_timeout(limit).unwrap_or_default();
    Err(format!(
        "{what}: {status}: {} {}",
        reason.trim(),
        text.trim()
    ))
}

/// A pipe read beside the wait: an answer past its size would block the tool and read as a hang.
fn drain(mut pipe: impl Read + Send + 'static) -> Receiver<String> {
    let (send, receive) = channel();
    std::thread::spawn(move || {
        let mut bytes = vec![];
        let _ = pipe.read_to_end(&mut bytes);
        let _ = send.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receive
}
