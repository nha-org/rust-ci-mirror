//! The tool's own numbers, as statsd lines over UDP to whatever listens on loopback: the fewest dependencies there are, none,
//! and no key here (the listener holds its own). Tooling beside the rings: only the binary takes it, and the core never hears
//! of it (@nha, 2026-11-06: « integrate datadog in rust too. different app. so we don't grow accidentally. »).
//!
//! The switch is one file in the data folder, read before each send, so `dash telemetry on|off` also acts on what is running.
//! The one exception is `say_stop`, whose caller holds the address as last read: within 30 s while a reporting thread runs free,
//! else from the process's start (the binary's reporting.rs).
//! No file is off: a test's or CI's fresh folder sends nothing, and `make rust/serve` turns the live folder on once.
//!
//! The same switch gates spans: a `Span` with a name and a resource, its children, and its finish, sent as one v0.3 trace
//! by HTTP PUT to the local trace agent on a background thread with a bounded queue, so a caller never waits on it.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const SERVICE: &str = "dash-rust";
const SWITCH: &str = "telemetry";
/// The statsd port's convention.
const ENDPOINT: &str = "127.0.0.1:8125";
/// The trace agent's port and path's conventions.
const TRACE_ENDPOINT: &str = "127.0.0.1:8126";
const TRACE_PATH: &str = "/v0.3/traces";
/// How many finished traces wait for the sender: a full queue drops the newest, never the caller.
const TRACE_QUEUE: usize = 256;
const TRACE_CONNECT: Duration = Duration::from_secs(2);
const TRACE_ANSWER: Duration = Duration::from_secs(2);
const OFF: &str = "off";

/// `on` or `off`, kept in `dir` across restarts.
pub fn set(dir: &Path, on: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let value = if on { ENDPOINT } else { OFF };
    std::fs::write(dir.join(SWITCH), value)
}

/// Where the numbers go, or None while off. Only an address counts as on: a host name would look itself up at each send, on the
/// serial request loop, and any other text, `off` too, is off.
pub fn endpoint(dir: &Path) -> Option<SocketAddr> {
    let text = std::fs::read_to_string(dir.join(SWITCH)).ok()?;
    text.trim().parse().ok()
}

pub struct Statsd {
    dir: PathBuf,
    tags: String,
    socket: Option<UdpSocket>,
    stop_line: String,
    traces: Option<SyncSender<String>>,
}

impl Statsd {
    /// `command` tells one process's numbers from another's on the same service.
    pub fn new(dir: &Path, command: &str) -> Self {
        Self::with_traces(
            dir,
            command,
            TRACE_ENDPOINT.parse().expect("the trace endpoint"),
        )
    }

    /// A report whose traces go to `to`: the same sender, for a test's listener.
    pub fn with_traces(dir: &Path, command: &str, to: SocketAddr) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").ok();
        // A full send buffer drops a number and never holds the caller.
        let socket = socket.filter(|s| s.set_nonblocking(true).is_ok());
        let tags = format!("service:{SERVICE},command:{command}");
        let (sender, incoming) = sync_channel::<String>(TRACE_QUEUE);
        // The one sender thread: a caller never waits on the agent, and a switch off drops what it holds.
        let folder = dir.to_path_buf();
        let traces = std::thread::Builder::new()
            .name("dash-traces".into())
            .spawn(move || send_traces(folder, to, incoming))
            .ok()
            .map(|_| sender);
        Self {
            dir: dir.to_path_buf(),
            stop_line: format!("dash_rust.memory.cap_stop:1|c|#{tags}"),
            tags,
            socket,
            traces,
        }
    }

    /// This report's folder's switch, read now.
    pub fn switch(&self) -> Option<SocketAddr> {
        endpoint(&self.dir)
    }

    /// Fire and forget: nothing listening costs the tool nothing and tells it nothing.
    // ponytail: one small file read a number, on the serial request loop too, where the next request's route_* holds the wait;
    // a cached switch if it ever shows there.
    fn send(&self, line: &str) {
        if let (Some(to), Some(socket)) = (self.switch(), &self.socket) {
            let _ = socket.send_to(line.as_bytes(), to);
        }
    }

    /// A stop at the memory cap, from inside the allocator: no allocation, no lock, no file, so the line was made in `new` and the
    /// caller holds the address.
    pub fn say_stop(&self, to: SocketAddr) {
        if let Some(socket) = &self.socket {
            let _ = socket.send_to(self.stop_line.as_bytes(), to);
        }
    }

    pub fn gauge(&self, name: &str, value: f64) {
        self.send(&format!("dash_rust.{name}:{value:.1}|g|#{}", self.tags));
    }

    /// For a number whose highest matters: a gauge keeps the last sent per tag set, so two processes of one command would hide
    /// each other's. The agent's `.max` series keeps the highest, of however many processes.
    pub fn highest(&self, name: &str, value: f64) {
        self.send(&format!("dash_rust.{name}:{value:.1}|h|#{}", self.tags));
    }

    /// The process's time so far, to its first useful answer.
    pub fn started(&self, since: Instant) {
        let ms = since.elapsed().as_secs_f64() * 1000.0;
        self.send(&format!("dash_rust.start_ms:{ms:.1}|ms|#{}", self.tags));
    }

    /// One answer of 400 and over, or a connection closed for want of room. `status` and `route` come from the tool's own lists, never from a client.
    pub fn refused(&self, status: &str, route: &str) {
        let tags = &self.tags;
        self.send(&format!(
            "dash_rust.http.refused:1|c|#{tags},status:{status},route:{route}"
        ));
    }

    /// One page or command at the front door, `asked` or `forwarded` to the older app (task rust-fallback-off). `kind` and
    /// `what` are tags the caller has made safe (`fallback::page_tag`, `command_tag`).
    pub fn front_door(&self, name: &str, kind: &str, what: &str) {
        let tags = &self.tags;
        self.send(&format!(
            "dash_rust.front_door.{name}:1|c|#{tags},kind:{kind},what:{what}"
        ));
    }

    pub fn step_took(&self, step: &str, took: Duration) {
        let ms = took.as_secs_f64() * 1000.0;
        let tags = &self.tags;
        self.send(&format!("dash_rust.step_ms:{ms:.1}|ms|#{tags},step:{step}"));
    }

    pub fn step<T>(&self, step: &str, work: impl FnOnce() -> T) -> T {
        let at = Instant::now();
        let value = work();
        self.step_took(step, at.elapsed());
        value
    }

    /// A root span, while the switch is on; a span that sends nothing while off. The caller names it, makes its children,
    /// and finishes it: a finished root sends its whole trace, its children first.
    pub fn span(&self, name: &str, resource: &str) -> Span {
        match (self.switch(), &self.traces) {
            (Some(_), Some(sender)) => Span::root(name, resource, sender.clone()),
            _ => Span::none(),
        }
    }
}

/// The one thread that puts finished traces: it reads the switch before each send, so `dash telemetry on|off` acts on
/// what is running, and a switch off drops what it holds.
fn send_traces(dir: PathBuf, to: SocketAddr, incoming: Receiver<String>) {
    while let Ok(json) = incoming.recv() {
        if endpoint(&dir).is_none() {
            continue;
        }
        put(&to, &json);
    }
}

/// One HTTP PUT of a finished trace, whole: a failure costs the trace, never the caller.
fn put(to: &SocketAddr, body: &str) {
    let Ok(mut stream) = TcpStream::connect_timeout(to, TRACE_CONNECT) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(TRACE_ANSWER));
    let _ = stream.set_read_timeout(Some(TRACE_ANSWER));
    let head = format!(
        "PUT {TRACE_PATH} HTTP/1.1\r\nHost: {to}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(head.as_bytes()).is_err() || stream.write_all(body.as_bytes()).is_err() {
        return;
    }
    // The agent's answer, read so its buffer is drained; a short read is no matter.
    let _ = stream.read(&mut [0u8; 256]);
}

/// One span of a trace: a name, a resource, its children, and its finish. A span is a handle; the trace's spans share one
/// buffer, and the root's finish sends them all.
#[derive(Clone)]
pub struct Span {
    inner: Option<Arc<SpanInner>>,
}

struct SpanInner {
    name: String,
    resource: String,
    trace_id: u64,
    span_id: u64,
    parent_id: Option<u64>,
    started: Instant,
    start_ns: u128,
    buffer: Arc<Mutex<TraceBuffer>>,
    sender: SyncSender<String>,
}

struct TraceBuffer {
    spans: Vec<SpanRecord>,
    sent: bool,
}

struct SpanRecord {
    name: String,
    resource: String,
    trace_id: u64,
    span_id: u64,
    parent_id: Option<u64>,
    start_ns: u128,
    duration_ns: u128,
}

impl Span {
    fn root(name: &str, resource: &str, sender: SyncSender<String>) -> Self {
        Self {
            inner: Some(Arc::new(SpanInner {
                name: name.to_string(),
                resource: resource.to_string(),
                trace_id: next_id(),
                span_id: next_id(),
                parent_id: None,
                started: Instant::now(),
                start_ns: now_ns(),
                buffer: Arc::new(Mutex::new(TraceBuffer {
                    spans: Vec::new(),
                    sent: false,
                })),
                sender,
            })),
        }
    }

    fn none() -> Self {
        Self { inner: None }
    }

    /// A child of this span: the same trace, its parent this span's id.
    pub fn child(&self, name: &str, resource: &str) -> Span {
        match &self.inner {
            None => Span::none(),
            Some(inner) => Span {
                inner: Some(Arc::new(SpanInner {
                    name: name.to_string(),
                    resource: resource.to_string(),
                    trace_id: inner.trace_id,
                    span_id: next_id(),
                    parent_id: Some(inner.span_id),
                    started: Instant::now(),
                    start_ns: now_ns(),
                    buffer: Arc::clone(&inner.buffer),
                    sender: inner.sender.clone(),
                })),
            },
        }
    }

    /// The span's end: a child adds itself to its trace's buffer; the root sends the whole trace, never blocking (a full
    /// queue drops it).
    pub fn finish(self) {
        let Some(inner) = self.inner else {
            return;
        };
        let record = SpanRecord {
            name: inner.name.clone(),
            resource: inner.resource.clone(),
            trace_id: inner.trace_id,
            span_id: inner.span_id,
            parent_id: inner.parent_id,
            start_ns: inner.start_ns,
            duration_ns: inner.started.elapsed().as_nanos(),
        };
        let mut buffer = inner
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buffer.sent {
            return;
        }
        buffer.spans.push(record);
        if inner.parent_id.is_none() {
            buffer.sent = true;
            let json = trace_json(&buffer.spans);
            let _ = inner.sender.try_send(json);
        }
    }
}

/// The v0.3 wire form: an array of traces, each an array of spans.
fn trace_json(spans: &[SpanRecord]) -> String {
    let mut json = String::from("[[");
    for (at, span) in spans.iter().enumerate() {
        if at > 0 {
            json.push(',');
        }
        json.push_str("{\"service\":");
        json.push_str(&quoted(SERVICE));
        json.push_str(",\"name\":");
        json.push_str(&quoted(&span.name));
        json.push_str(",\"resource\":");
        json.push_str(&quoted(&span.resource));
        json.push_str(",\"trace_id\":");
        json.push_str(&span.trace_id.to_string());
        json.push_str(",\"span_id\":");
        json.push_str(&span.span_id.to_string());
        json.push_str(",\"parent_id\":");
        match span.parent_id {
            Some(parent) => json.push_str(&parent.to_string()),
            None => json.push('0'),
        }
        json.push_str(",\"start\":");
        json.push_str(&span.start_ns.to_string());
        json.push_str(",\"duration\":");
        json.push_str(&span.duration_ns.to_string());
        json.push_str(",\"error\":0,\"meta\":{},\"metrics\":{},\"type\":\"web\"}");
    }
    json.push_str("]]");
    json
}

/// A JSON string, escaped: a name or resource is the tool's own, but a resource may hold a path's any byte.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A trace's and each span's id: the process's time, then a counter, so no two spans of one process share one.
fn next_id() -> u64 {
    static IDS: LazyLock<AtomicU64> = LazyLock::new(|| AtomicU64::new(now_ns() as u64 | 1));
    IDS.fetch_add(1, Ordering::Relaxed)
}

/// This process's time in nanoseconds, for a span's start.
fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos())
}

thread_local! {
    /// The span this thread is under: a page render and a store read make their children of it.
    static CURRENT: std::cell::RefCell<Option<Span>> = const { std::cell::RefCell::new(None) };
}

/// The span this thread is under, if any.
pub fn current() -> Option<Span> {
    CURRENT.with(|current| current.borrow().clone())
}

/// What this thread is under; `None` clears it.
pub fn set_current(span: Option<Span>) {
    CURRENT.with(|current| *current.borrow_mut() = span);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    fn listener() -> UdpSocket {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(200)))
            .expect("timeout");
        socket
    }

    fn line(socket: &UdpSocket) -> Option<String> {
        let mut buf = [0u8; 512];
        let n = socket.recv(&mut buf).ok()?;
        Some(String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    #[test]
    fn each_kind_is_a_line_under_the_service() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("statsd-lines");
        let socket = listener();
        let to = socket.local_addr().expect("address").to_string();
        std::fs::write(dir.0.join(SWITCH), &to).expect("the switch");
        let report = Statsd::new(&dir.0, "serve");
        report.gauge("memory.current_mb", 12.34);
        assert_eq!(
            line(&socket).expect("a gauge"),
            "dash_rust.memory.current_mb:12.3|g|#service:dash-rust,command:serve"
        );
        report.highest("memory.peak_mb", 150.0);
        assert_eq!(
            line(&socket).expect("a highest"),
            "dash_rust.memory.peak_mb:150.0|h|#service:dash-rust,command:serve"
        );
        report.step_took("open_store", Duration::from_millis(5));
        assert_eq!(
            line(&socket).expect("a step"),
            "dash_rust.step_ms:5.0|ms|#service:dash-rust,command:serve,step:open_store"
        );
        assert_eq!(
            report.step("pick", || 7),
            7,
            "a step answers its work's value"
        );
        assert!(line(&socket).expect("its time").contains("step:pick"));
        report.refused("404", "route_other");
        assert_eq!(
            line(&socket).expect("a refusal"),
            "dash_rust.http.refused:1|c|#service:dash-rust,command:serve,status:404,route:route_other"
        );
        report.say_stop(to.parse().expect("an address"));
        assert_eq!(
            line(&socket).expect("a stop"),
            "dash_rust.memory.cap_stop:1|c|#service:dash-rust,command:serve"
        );
        report.started(Instant::now());
        assert!(
            line(&socket)
                .expect("the start")
                .starts_with("dash_rust.start_ms:")
        );
    }

    #[test]
    fn no_file_or_off_sends_nothing_and_acts_on_a_running_process() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("statsd-switch");
        let socket = listener();
        let to = socket.local_addr().expect("address").to_string();
        let report = Statsd::new(&dir.0, "serve");
        assert_eq!(endpoint(&dir.0), None, "a fresh folder is off");
        report.gauge("a", 1.0);
        assert_eq!(line(&socket), None);

        std::fs::write(dir.0.join(SWITCH), &to).expect("on");
        report.gauge("a", 1.0);
        assert!(line(&socket).is_some(), "on, with no restart");

        set(&dir.0, false).expect("off");
        assert_eq!(endpoint(&dir.0), None);
        report.gauge("a", 1.0);
        assert_eq!(line(&socket), None, "off, with no restart");

        set(&dir.0, true).expect("on");
        assert_eq!(endpoint(&dir.0), ENDPOINT.parse().ok());

        for not_an_address in ["on", "localhost:8125", ""] {
            std::fs::write(dir.0.join(SWITCH), not_an_address).expect("the text");
            assert_eq!(endpoint(&dir.0), None, "{not_an_address}");
        }
    }

    #[test]
    fn a_span_with_a_child_arrives_as_a_valid_v0_3_trace() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("statsd-spans");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let to = listener.local_addr().expect("address");
        std::fs::write(dir.0.join(SWITCH), to.to_string()).expect("the switch");
        let report = Statsd::with_traces(&dir.0, "serve", to);
        let root = report.span("http.request", "/tasks");
        let child = root.child("store.read", "scan");
        child.finish();
        root.finish();

        let (mut stream, _) = listener.accept().expect("the trace agent");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let mut reader = std::io::BufReader::new(stream.try_clone().expect("a clone"));
        let mut head = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).expect("a line");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = rest.trim().parse().expect("a length");
            }
            head.push_str(&line);
        }
        assert!(head.starts_with("PUT /v0.3/traces HTTP/1.1\r\n"), "{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{head}"
        );
        let mut body = vec![0u8; length];
        std::io::Read::read_exact(&mut reader, &mut body).expect("the body");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("the answer");

        let trace: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        let spans = trace[0].as_array().expect("one trace");
        assert_eq!(spans.len(), 2, "{trace}");
        let find = |name: &str| {
            spans
                .iter()
                .find(|span| span["name"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("{name}: {trace}"))
        };
        let root = find("http.request");
        let child = find("store.read");
        assert_eq!(root["service"], "dash-rust");
        assert_eq!(root["resource"], "/tasks");
        assert_eq!(child["resource"], "scan");
        assert_eq!(root["parent_id"].as_u64(), Some(0), "{trace}");
        assert_eq!(child["parent_id"], root["span_id"], "{trace}");
        assert_eq!(root["trace_id"], child["trace_id"], "{trace}");
        for span in [root, child] {
            assert!(
                span["start"].as_u64().is_some_and(|start| start > 0),
                "{trace}"
            );
            assert!(
                span["duration"].as_u64().is_some_and(|took| took > 0),
                "{trace}"
            );
        }
    }
}
