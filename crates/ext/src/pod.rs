//! A pod: any program speaking MCP over stdio (section 12, babashka's pods with MCP as the protocol). Its manifest
//! `*.json` names the program and which of its tools extracts which key:
//! `{"command": ["program", "argument"], "extractors": {"text/x-rot13": "rot13"}}`. It runs in the extension folder, starts
//! on its first call and stops with the app or past a call's limit.
// ponytail: a pod adds extractors only, as text, and binary input waits on passing a path. Tools, ui and hooks have no provider
// yet: the first plugin that needs one adds its field to the core's `Compiled`, or to this manifest, and a `ui` brings back the
// check at load, so a tree that cannot draw refuses its file. A killed dash leaves a pod to end on its stdin's EOF.
use dash_core::registry::{Contribution, Extract};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Lines a pod may have waiting: past this it blocks on its stdout, so one that talks forever holds no growing queue. With
/// `LINE_LIMIT`, it bounds what a pod can make us hold, between calls too: a test holds two pods under the memory cap.
const LINES_WAITING: usize = 2;
/// One message's bytes. A longer one ends the pod's stream, and its call fails.
const LINE_LIMIT: u64 = 8 * 1024 * 1024;

/// A pod in plain sh, to show that a pod takes no Rust, in a manifest so that no shell script is tracked (CLAUDE.md,
/// Coordination 5). One JSON message a line. A JSON escape in its text becomes a space, so its answer stays valid JSON;
/// its text stops at a quote.
const SAMPLE_PROGRAM: &str = r#"
answer() { printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$1"; }
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"tools/call"'*)
      text=$(printf '%s' "$line" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p' | sed 's/\\./ /g' | tr 'a-zA-Z' 'n-za-mN-ZA-M')
      answer "{\"content\":[{\"type\":\"text\",\"text\":\"$text\"}]}"
      ;;
    *'"initialize"'*)
      answer '{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"rot13","version":"1"}}'
      ;;
  esac
done
"#;

/// The manifest of a sample pod that serves `text/x-rot13`, for the tests.
pub fn sample_manifest() -> String {
    json!({"command": ["sh", "-c", SAMPLE_PROGRAM], "extractors": {"text/x-rot13": "rot13"}})
        .to_string()
}

/// The result of request `id`, its error as text.
fn result_of(
    lines: &Receiver<String>,
    id: u64,
    method: &str,
    deadline: Instant,
) -> Result<Value, String> {
    let message = answer_of(lines, id, method, deadline)?;
    if let Some(error) = message.get("error") {
        return Err(format!("{method}: {error}"));
    }
    Ok(message["result"].clone())
}

/// The whole answer to request `id`, its result or its error. The deadline is checked before each line too: a queued line
/// returns at once even with no time left, so a server that keeps talking would never run into it.
fn answer_of(
    lines: &Receiver<String>,
    id: u64,
    method: &str,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("no answer to {method} by the limit"));
        }
        let line = lines
            .recv_timeout(left)
            .map_err(|e| format!("no answer to {method}: {e}"))?;
        let message: Value =
            serde_json::from_str(&line).map_err(|_| format!("not JSON on its stdout: {line}"))?;
        // A notification, or a request of its own we do not serve: its ids are its own, so one can equal ours.
        if message["id"] != json!(id) || message.get("method").is_some() {
            continue;
        }
        return Ok(message);
    }
}

/// A pod's stdout into `send`, one message a line, until it ends, stops being text or sends a line past `limit`. The last
/// is sent on as a line that is not JSON, so the call in progress fails saying why.
fn read_lines(stdout: impl Read, limit: u64, send: &SyncSender<String>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = vec![];
        let read = (&mut reader).take(limit).read_until(b'\n', &mut line);
        if !matches!(read, Ok(n) if n > 0) {
            return;
        }
        if line.len() as u64 == limit && line.last() != Some(&b'\n') {
            let _ = send.send(format!("a line past {limit} bytes"));
            return;
        }
        let Ok(line) = String::from_utf8(line) else {
            return;
        };
        if send.send(line).is_err() {
            return;
        }
    }
}

/// Running pods' process groups, for a stop at the memory cap, where no Drop runs.
/// ponytail: 16 slots; past them a pod is left to its stdin's end, as on any crash.
static GROUPS: [AtomicI32; 16] = [const { AtomicI32::new(0) }; 16];

fn remember(group: i32) {
    for slot in &GROUPS {
        if slot.compare_exchange(0, group, Relaxed, Relaxed).is_ok() {
            return;
        }
    }
}

/// That group is gone: its pid may come back as someone else's.
fn forget(group: i32) {
    for slot in &GROUPS {
        if slot.compare_exchange(group, 0, Relaxed, Relaxed).is_ok() {
            return;
        }
    }
}

/// Runs inside the allocator: no allocation, no lock.
pub fn kill_running() {
    for slot in &GROUPS {
        let group = slot.load(Relaxed);
        if group != 0 {
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
    }
}

struct Pod {
    command: Vec<String>,
    folder: PathBuf,
    limit: Duration,
    running: RefCell<Option<Running>>,
}

/// A running MCP server over stdio, past its handshake: a pod's, or `dash mcp-proxy`'s server.
pub struct Running {
    child: Child,
    to_pod: Sender<String>,
    lines: Receiver<String>,
    next_id: u64,
}

impl Drop for Running {
    /// The whole group: a pod's own children would outlive it and hold its stdout open.
    fn drop(&mut self) {
        forget(self.child.id() as i32);
        unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

impl Running {
    /// `command` started in `folder`, and its initialize answered by `deadline`.
    pub fn start(command: &[String], folder: &Path, deadline: Instant) -> Result<Self, String> {
        let program = command.first().ok_or("no command")?;
        let mut child = Command::new(program)
            .args(&command[1..])
            .current_dir(folder)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {program}: {e}"))?;
        remember(child.id() as i32);
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        // A thread each way, so a pod that answers nothing or stops reading runs into the deadline and not a blocked read
        // or write. Each ends at the pod's end, which the kill in Drop brings.
        let (to_pod, outgoing) = channel::<String>();
        std::thread::spawn(move || {
            for line in outgoing {
                if writeln!(stdin, "{line}")
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let (send, lines) = sync_channel(LINES_WAITING);
        std::thread::spawn(move || read_lines(stdout, LINE_LIMIT, &send));
        let mut running = Self {
            child,
            to_pod,
            lines,
            next_id: 1,
        };
        running.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "dash", "version": env!("CARGO_PKG_VERSION")},
            }),
            deadline,
        )?;
        running.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(running)
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        self.to_pod
            .send(message.to_string())
            .map_err(|_| "the server stopped reading".to_string())
    }

    /// Its input closed, then `grace` to end on its own, as a server cleans up at its input's end (dash-scratch removes its
    /// copy: task worktree-surfaces); the kill of Drop after that, for its group and a server that keeps going.
    pub fn close(mut self, grace: Duration) {
        drop(std::mem::replace(&mut self.to_pod, channel().0));
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// `method`'s result, or its error as text.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        result_of(&self.lines, id, method, deadline)
    }

    /// `method`'s whole answer, an error the server sent included: `Err` only when no answer came (`dash mcp-proxy` passes
    /// a server's error on as it is, and restarts it only then).
    pub fn ask(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        answer_of(&self.lines, id, method, deadline)
    }
}

impl Pod {
    /// A failure stops the program, so the next call starts a clean one and no late answer is read as its answer.
    fn call_tool(&self, tool: &str, arguments: Value) -> Result<String, String> {
        let deadline = Instant::now() + self.limit;
        let mut running = self.running.borrow_mut();
        let answer = (|| {
            if running.is_none() {
                *running = Some(Running::start(&self.command, &self.folder, deadline)?);
            }
            let params = json!({"name": tool, "arguments": arguments});
            let live = running.as_mut().ok_or("not running")?;
            live.request("tools/call", params, deadline)
        })();
        if answer.is_err() {
            *running = None;
        }
        let answer = answer?;
        let text: Vec<&str> = answer["content"]
            .as_array()
            .ok_or_else(|| format!("{tool} answered no content: {answer}"))?
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect();
        if answer["isError"] == json!(true) {
            return Err(format!("{tool}: {}", text.join("\n")));
        }
        Ok(text.join("\n"))
    }
}

struct PodExtract {
    pod: Rc<Pod>,
    key: String,
    tool: String,
}

impl Extract for PodExtract {
    fn extract(&self, input: &[u8]) -> Result<String, String> {
        let text = std::str::from_utf8(input).map_err(|_| "a pod extracts text only")?;
        self.pod
            .call_tool(&self.tool, json!({"key": self.key, "text": text}))
    }
}

pub(crate) fn load(path: &Path, limit: Duration) -> Result<Contribution, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let manifest: Value =
        serde_json::from_str(&source).map_err(|e| format!("not a pod's manifest: {e}"))?;
    let command: Vec<String> = manifest["command"]
        .as_array()
        .and_then(|words| {
            words
                .iter()
                .map(|w| w.as_str().map(String::from))
                .collect::<Option<Vec<String>>>()
        })
        .filter(|words| !words.is_empty())
        .ok_or(r#"a manifest names its program: "command": ["program", "argument"]"#)?;
    let extractors = manifest["extractors"]
        .as_object()
        .ok_or(r#"a manifest names its keys: "extractors": {"key": "tool"}"#)?;
    let ui = match manifest.get("ui") {
        None => None,
        Some(value) => Some(
            value
                .as_object()
                .ok_or(r#"a manifest names its trees: "ui": {"key": "{...}"}"#)?,
        ),
    };
    let pod = Rc::new(Pod {
        command,
        folder: path.with_file_name(""),
        limit,
        running: RefCell::new(None),
    });
    let mut c = Contribution::default();
    for (key, tool) in extractors {
        let tool = tool
            .as_str()
            .ok_or(format!("{key}: its tool is not a string"))?;
        c.extractors.push((
            key.clone(),
            Box::new(PodExtract {
                pod: pod.clone(),
                key: key.clone(),
                tool: tool.to_string(),
            }),
        ));
    }
    if let Some(ui) = ui {
        for (key, tree) in ui {
            let tree = tree
                .as_str()
                .ok_or(format!("{key}: its tree is not a string"))?;
            c.ui.push((key.clone(), tree.to_string().into_boxed_str()));
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::cap::CAP_MB;
    use dash_meter::dash_test;

    #[dash_test]
    fn a_forgotten_group_leaves_its_slot() {
        remember(1234);
        assert_eq!(GROUPS[0].load(Relaxed), 1234);
        forget(1234);
        assert_eq!(GROUPS[0].load(Relaxed), 0);
    }

    const LIMIT: Duration = Duration::from_secs(10);

    /// Lines waiting, one blocked in send and one being read, each up to twice its bytes as a Vec doubles, then an answer's parsed
    /// value and joined text. Two pods at their limits stay under the cap, with room for the rest of the process.
    #[dash_test]
    fn two_pods_at_their_limits_fit_under_the_memory_cap() {
        let lines = LINES_WAITING as u64 + 2;
        let one_pod = lines * 2 * LINE_LIMIT + 2 * LINE_LIMIT;
        assert!(2 * one_pod < CAP_MB as u64 * 1024 * 1024, "{one_pod}");
    }

    /// Answers `initialize`, then each tools/call with `answer`.
    fn answering(answer: &str) -> String {
        format!(
            r#"while read -r line; do
                id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
                case "$line" in
                  *'"tools/call"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{answer}}}\n' "$id" ;;
                  *'"initialize"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id" ;;
                esac
              done"#
        )
    }

    fn pod(dir: &TestDir, program: &str, limit: Duration) -> Box<dyn Extract> {
        let path = dir.0.join("p.json");
        let manifest = json!({"command": ["sh", "-c", program], "extractors": {"k": "t"}});
        std::fs::write(&path, manifest.to_string()).expect("write");
        let mut c = load(&path, limit).expect("load");
        c.extractors.remove(0).1
    }

    #[dash_test]
    fn a_tool_error_is_an_error_and_the_text_parts_are_the_answer() {
        let dir = TestDir::new("pod-answers");
        let parts =
            answering(r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}"#);
        assert_eq!(pod(&dir, &parts, LIMIT).extract(b"x").unwrap(), "a\nb");
        let failed = answering(r#"{"isError":true,"content":[{"type":"text","text":"no"}]}"#);
        assert_eq!(
            pod(&dir, &failed, LIMIT).extract(b"x").unwrap_err(),
            "t: no"
        );
        assert!(
            pod(&dir, &parts, LIMIT).extract(b"\xff").is_err(),
            "text only"
        );
    }

    #[dash_test]
    fn a_silent_pod_fails_at_the_limit_with_its_children_stopped_and_the_next_call_starts_a_new_one()
     {
        let dir = TestDir::new("pod-silent");
        let started = dir.0.join("started");
        let program = format!("sleep 67 & echo $! >> '{}'; wait", started.display());
        let silent = pod(&dir, &program, Duration::from_millis(300));
        let begin = Instant::now();
        let e = silent.extract(b"x").expect_err("at the limit");
        assert!(e.contains("no answer"), "{e}");
        assert!(
            begin.elapsed() < Duration::from_secs(5),
            "{:?}",
            begin.elapsed()
        );
        silent.extract(b"x").expect_err("again");
        let starts = std::fs::read_to_string(&started).expect("it ran");
        assert_eq!(starts.lines().count(), 2, "one program per failed call");
        for child in starts.lines() {
            let child: i32 = child.parse().expect("a pid");
            let alive = || unsafe { libc::kill(child, 0) } == 0;
            let deadline = Instant::now() + Duration::from_secs(3); // Till init reaps the orphan.
            while alive() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!alive(), "the pod's child {child} outlived it");
        }
    }

    #[dash_test]
    fn a_queue_of_lines_cannot_outrun_the_deadline() {
        let (send, lines) = channel();
        send.send("{}".to_string()).expect("queued");
        let e = result_of(&lines, 1, "m", Instant::now()).expect_err("past it");
        assert!(e.contains("by the limit"), "{e}");
        assert!(lines.try_recv().is_ok(), "the line was not read first");
    }

    #[dash_test]
    fn a_request_from_the_pod_with_our_id_is_not_our_answer() {
        let (send, lines) = channel();
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"roots/list"}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{"ours":true}}"#,
        ] {
            send.send(line.to_string()).expect("queued");
        }
        let deadline = Instant::now() + LIMIT;
        let answer = result_of(&lines, 1, "m", deadline).expect("our answer");
        assert_eq!(answer, json!({"ours": true}));
    }

    #[dash_test]
    fn a_line_past_the_limit_ends_the_stream_and_its_call_fails_saying_why() {
        let (send, lines) = sync_channel(LINES_WAITING);
        read_lines(&b"{}\n"[..], 16, &send);
        assert_eq!(lines.try_recv().unwrap(), "{}\n", "a short line passes");
        read_lines(std::io::repeat(b'x'), 16, &send); // It returns: a stream with no newline holds no growing line.
        drop(send);
        let e = result_of(&lines, 1, "m", Instant::now() + LIMIT).expect_err("too long");
        assert!(e.contains("past 16 bytes"), "{e}");
    }

    #[dash_test]
    fn a_pod_that_stops_reading_fails_at_the_limit() {
        let dir = TestDir::new("pod-rude");
        // Past a pipe's 64 KB, so a write to it would block for good.
        let reads_one_line =
            r#"read -r line; printf '{"jsonrpc":"2.0","id":1,"result":{}}\n'; sleep 67"#;
        let begin = Instant::now();
        let e = pod(&dir, reads_one_line, Duration::from_millis(400))
            .extract(&vec![b'a'; 1024 * 1024])
            .expect_err("stops reading");
        assert!(e.contains("no answer to tools/call"), "{e}");
        assert!(
            begin.elapsed() < Duration::from_secs(5),
            "{:?}",
            begin.elapsed()
        );
    }

    #[dash_test]
    fn the_sample_pod_answers_valid_json_for_a_file_with_escapes() {
        let dir = TestDir::new("pod-sample");
        let path = dir.0.join("p.json");
        std::fs::write(&path, sample_manifest()).expect("write");
        let c = load(&path, LIMIT).expect("load");
        let rot13 = &c.extractors[0].1;
        assert_eq!(rot13.extract(b"hello").unwrap(), "uryyb");
        assert_eq!(rot13.extract(b"hello\nworld\n").unwrap(), "uryyb jbeyq ");
    }

    #[dash_test]
    fn a_pod_that_exits_or_cannot_start_fails_its_call() {
        let dir = TestDir::new("pod-exit");
        let e = pod(&dir, "exit 0", LIMIT).extract(b"x").expect_err("gone");
        assert!(
            e.contains("initialize") || e.contains("stopped reading"),
            "{e}"
        );
        let path = dir.0.join("none.json");
        let manifest = json!({"command": ["/no/such/program"], "extractors": {"k": "t"}});
        std::fs::write(&path, manifest.to_string()).expect("write");
        let c = load(&path, LIMIT).expect("a lazy pod loads");
        let e = c.extractors[0].1.extract(b"x").expect_err("no program");
        assert!(e.contains("/no/such/program"), "{e}");
    }

    #[dash_test]
    fn a_bad_manifest_is_refused_at_load() {
        let dir = TestDir::new("pod-manifest");
        let path = dir.0.join("p.json");
        for (text, says) in [
            ("not json", "manifest"),
            (r#"{"extractors": {}}"#, "command"),
            (r#"{"command": [], "extractors": {}}"#, "command"),
            (r#"{"command": ["x", 5, "y"], "extractors": {}}"#, "command"),
            (r#"{"command": ["x"]}"#, "extractors"),
            (
                r#"{"command": ["x"], "extractors": {"k": 1}}"#,
                "not a string",
            ),
        ] {
            std::fs::write(&path, text).expect("write");
            let e = load(&path, LIMIT).err().expect(text);
            assert!(e.contains(says), "{text}: {e}");
        }
    }
}
