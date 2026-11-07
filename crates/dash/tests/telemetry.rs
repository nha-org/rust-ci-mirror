//! The real binary's numbers, read off a stand-in for the statsd port: what `dash serve` says at its start and per request, and
//! the switch stopping it with no restart.
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn ping(port: u16) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(stream, "GET /ping HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
    let mut text = String::new();
    stream.read_to_string(&mut text).expect("read");
    assert!(text.ends_with("pong"), "{text}");
}

/// Every line that arrives until one quiet stretch.
fn drain(socket: &UdpSocket) -> Vec<String> {
    let mut lines = vec![];
    let mut buf = [0u8; 512];
    while let Ok(n) = socket.recv(&mut buf) {
        lines.push(String::from_utf8_lossy(&buf[..n]).into_owned());
    }
    lines
}

#[test]
fn serve_says_its_memory_start_and_steps_until_the_switch_is_off() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("telemetry");
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("timeout");
    let to = socket.local_addr().expect("address").to_string();
    std::fs::write(dir.0.join("telemetry"), to).expect("the switch, at this listener");

    let mut child = Command::new(DASH)
        .args(["serve", "--data"])
        .arg(&dir.0)
        .stdout(Stdio::piped())
        .spawn()
        .expect("boot");
    let stdout = child.stdout.take().expect("stdout");
    let _server = Server(child);
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("its port");
    let port: u16 = line
        .trim()
        .strip_prefix("port ")
        .expect("port")
        .parse()
        .expect("a number");

    ping(port);
    let lines = drain(&socket);
    for (part, kind) in [
        ("dash_rust.memory.heap_mb:", "|g|"),
        ("dash_rust.memory.heap_peak_mb:", "|h|"),
        ("dash_rust.memory.current_mb:", "|g|"),
        ("dash_rust.memory.peak_mb:", "|h|"),
        ("dash_rust.start_ms:", "|ms|"),
        ("step:open_store", "|ms|"),
        ("step:registry", "|ms|"),
        ("step:route_ping", "|ms|"),
    ] {
        assert!(
            lines.iter().any(|l| l.contains(part)
                && l.contains(kind)
                && l.contains("service:dash-rust,command:serve")),
            "{part}: {lines:?}"
        );
    }

    let off = Command::new(DASH)
        .args(["telemetry", "off", "--data"])
        .arg(&dir.0)
        .output()
        .expect("the switch");
    assert_eq!(String::from_utf8_lossy(&off.stdout), "telemetry: off\n");
    ping(port);
    assert_eq!(drain(&socket), Vec::<String>::new(), "off, with no restart");
}

/// A process's last words, after it held a 2 MB line in memory: a dead counter reads 0.0, and a heap this small rounds to 0.0
/// without that line.
#[test]
fn a_process_says_what_the_cap_counted_as_gauges() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("telemetry-heap");
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("timeout");
    let to = socket.local_addr().expect("address").to_string();
    std::fs::write(dir.0.join("telemetry"), to).expect("the switch, at this listener");

    let mut child = Command::new(DASH)
        .args(["mcp", "--data"])
        .arg(&dir.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("boot");
    let mut stdin = child.stdin.take().expect("stdin");
    let line = format!("{}\n", "x".repeat(2 * 1024 * 1024));
    stdin.write_all(line.as_bytes()).expect("the line");
    drop(stdin);
    let output = child.wait_with_output().expect("its end");
    assert!(output.status.success(), "{output:?}");

    let lines = drain(&socket);
    let last = |name: &str, kind: &str| -> f64 {
        let tags = format!("{kind}#service:dash-rust,command:mcp");
        let line = lines
            .iter()
            .rfind(|l| l.starts_with(name) && l.contains(&tags))
            .unwrap_or_else(|| panic!("{name}: {lines:?}"));
        let value = line[name.len()..].split('|').next().expect("a value");
        value.parse().expect("a number")
    };
    let current = last("dash_rust.memory.heap_mb:", "|g|");
    let peak = last("dash_rust.memory.heap_peak_mb:", "|h|");
    assert!(peak >= 2.0 && current < peak, "{current} {peak}: {lines:?}");
}

/// The command most likely to reach the cap says what it held, though it lives under 30 s.
#[test]
fn ingest_says_its_heap() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("telemetry-ingest");
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("timeout");
    let to = socket.local_addr().expect("address").to_string();
    std::fs::write(dir.0.join("telemetry"), to).expect("the switch, at this listener");
    let file = dir.0.join("input.txt");
    std::fs::write(&file, "plain").expect("input");

    let output = Command::new(DASH)
        .args(["ingest", "--data"])
        .arg(&dir.0)
        .args(["--mime", "text/plain"])
        .arg(&file)
        .output()
        .expect("boot");
    assert!(output.status.success(), "{output:?}");

    let lines = drain(&socket);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("dash_rust.memory.heap_peak_mb:")
                && l.contains("|h|#service:dash-rust,command:ingest")),
        "{lines:?}"
    );
}
