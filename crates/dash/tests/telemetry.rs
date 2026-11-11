//! The real binary's numbers, read off a stand-in for the statsd port: what `dash serve` says at its start and per request, and
//! the switch stopping it with no restart.
use dash_meter::memory::TestBudget;
use dash_meter::{TestDir, drain};
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, TcpStream, UdpSocket};
use std::process::{Command, Stdio};
use std::time::Duration;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

mod common;
use common::serve;

fn ping(port: u16) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(stream, "GET /ping HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
    let mut text = String::new();
    stream.read_to_string(&mut text).expect("read");
    assert!(text.ends_with("pong"), "{text}");
}

fn sent(port: u16, bytes: &[u8]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.write_all(bytes).expect("write");
    stream.shutdown(Shutdown::Write).expect("done writing");
    let mut text = String::new();
    stream.read_to_string(&mut text).expect("read");
    text
}

#[test]
fn each_answer_of_400_and_over_is_counted_by_status_and_route_and_logged() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("telemetry-refused");
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("timeout");
    let to = socket.local_addr().expect("address").to_string();
    std::fs::write(dir.0.join("telemetry"), to).expect("the switch, at this listener");

    let (mut server, port) = serve(&dir.0, |c| c.stderr(Stdio::piped()));
    let stderr = server.0.stderr.take().expect("stderr");

    ping(port);
    let favicon = sent(
        port,
        b"GET /favicon.ico HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert!(favicon.starts_with("HTTP/1.1 200"), "{favicon}");
    for (bytes, status, route) in [
        (&b"\xff\r\n\r\n"[..], "400", "route_other"),
        // Cut short after its first line: still its route.
        (b"GET /ping HTTP/1.1\r\n", "400", "route_ping"),
        (
            b"GET /ping HTTP/1.1\r\nHost: evil.example\r\n\r\n",
            "403",
            "route_ping",
        ),
        (
            b"GET /secret/path?q=private HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "404",
            "route_other",
        ),
    ] {
        let text = sent(port, bytes);
        assert!(text.starts_with(&format!("HTTP/1.1 {status}")), "{text}");
        let lines: Vec<String> = drain(&socket)
            .into_iter()
            .filter(|l| l.starts_with("dash_rust.http.refused"))
            .collect();
        assert_eq!(
            lines,
            [format!(
                "dash_rust.http.refused:1|c|#service:dash-rust,command:serve,status:{status},route:{route}"
            )],
        );
    }
    // A client that closes before a byte is no answer, so no count.
    drop(TcpStream::connect(("127.0.0.1", port)).expect("connect"));
    assert!(
        !drain(&socket)
            .iter()
            .any(|l| l.starts_with("dash_rust.http.refused")),
    );

    drop(server);
    let mut log = String::new();
    BufReader::new(stderr)
        .read_to_string(&mut log)
        .expect("the log");
    let said: Vec<&str> = log.lines().filter(|l| l.starts_with("answered")).collect();
    assert_eq!(
        said,
        [
            "answered 400 on route_other",
            "answered 400 on route_ping",
            "answered 403 on route_ping",
            "answered 404 on route_other",
        ],
        "a 200 says nothing, nor a silent close, and a client's path stays out: {log}"
    );
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

    let (_server, port) = serve(&dir.0, |c| c.stderr(Stdio::inherit()));

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
