//! Section 4 held by a test: writes a fixture the size of dash's store, boots the binary on it, has it read the whole
//! store, fails past the budget it can reach, and prints the idle and loaded footprints (.config/nextest.toml shows this
//! test's output on a pass).
use dash_ingest::FILE_LIMIT;
use dash_meter::budgets::{BUDGETS, Check, SERVER_IDLE, within};
use dash_meter::cap::{CAP_MB, STOP_CODE};
use dash_meter::memory::TestBudget;
use dash_meter::{TestDir, drain};
use dash_store as store;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

const FIXTURE_ROWS: u64 = 200_000;

/// Its end, or None with the process killed: an allocation on the stop path parks it for good, past the cap's one-stop guard. A
/// stop takes under a second; the limit sits under nextest's 30 s, so a parked process gets this verdict and dies with it.
fn ended(child: &mut Child) -> Option<ExitStatus> {
    let end = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().expect("its state") {
            return Some(status);
        }
        if std::time::Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A stand-in listener, with the folder's switch pointing at it.
fn listening(dir: &TestDir) -> UdpSocket {
    let listener = UdpSocket::bind("127.0.0.1:0").expect("a stand-in listener");
    listener
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("its timeout");
    let to = listener.local_addr().expect("its address").to_string();
    std::fs::write(dir.0.join("telemetry"), to).expect("the switch");
    listener
}

/// A file of NUL bytes, sparse, so it costs no disk, at `dash ingest`'s limit, and a pod to extract it that answers `initialize`,
/// then stays. Each NUL is six bytes as JSON: the file's read fits under the cap, the call to the pod passes it, with the pod
/// running by then. A real path past the cap, with no test hook in the binary.
fn file_for_a_pod_past_the_cap(dir: &TestDir) -> std::path::PathBuf {
    let folder = dir.0.join("extensions.d");
    std::fs::create_dir_all(&folder).expect("folder");
    let program = r#"IFS= read -r line; echo $$ > pid; printf '{"jsonrpc":"2.0","id":1,"result":{}}\n'; while :; do sleep 1; done"#;
    // A Rust string's debug form is a JSON string here: only quotes and a backslash to escape.
    let manifest = format!(
        r#"{{"command": ["sh", "-c", {program:?}], "extractors": {{"text/x-stay": "stay"}}}}"#
    );
    std::fs::write(folder.join("30-stay.json"), manifest).expect("the manifest");
    let file = dir.0.join("at-the-limit");
    sparse(&file, FILE_LIMIT);
    file
}

fn sparse(file: &std::path::Path, bytes: u64) {
    std::fs::File::create(file)
        .expect("a file")
        .set_len(bytes)
        .expect("its length");
}

#[test]
fn ingest_refuses_a_file_past_its_limit_naming_the_file() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ingest-limit");
    let file = dir.0.join("past-the-limit");
    sparse(&file, FILE_LIMIT + 1);
    let output = ingest(&dir, "text/plain", &file).output().expect("boot");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(file.to_str().expect("a path")) && stderr.contains("past"),
        "{output:?}"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "refused, no stop: {output:?}"
    );

    // At the limit it is stored: the limit sits under the cap.
    sparse(&file, FILE_LIMIT);
    let output = ingest(&dir, "text/plain", &file).output().expect("boot");
    assert!(output.status.success(), "{output:?}");
}

/// Each step of the stop runs with the count past the cap, where an allocation parks the process: the UDP line and the exit code
/// together say none did.
#[test]
fn the_binary_stops_with_a_named_line_past_its_cap() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-cap");
    let file = file_for_a_pod_past_the_cap(&dir);
    let listener = listening(&dir);
    let stderr_file = dir.0.join("stderr");
    let stdout_file = dir.0.join("stdout");

    let mut child = ingest(&dir, "text/x-stay", &file)
        .stdout(std::fs::File::create(&stdout_file).expect("a file"))
        .stderr(std::fs::File::create(&stderr_file).expect("a file"))
        .spawn()
        .expect("boot");
    let status = ended(&mut child);

    let stderr = std::fs::read_to_string(&stderr_file).expect("its stderr");
    assert_eq!(
        status.and_then(|s| s.code()),
        Some(STOP_CODE),
        "{status:?}: {stderr}"
    );
    assert!(
        stderr.contains(&format!("past the cap of {CAP_MB} MB, stopping")),
        "{stderr}"
    );
    assert_eq!(std::fs::read(&stdout_file).expect("its stdout"), b"");
    // The memory thread's gauges come first, in any number.
    let lines = drain(&listener);
    let stop = "dash_rust.memory.cap_stop:1|c|#service:dash-rust,command:ingest";
    assert!(lines.iter().any(|l| l == stop), "{lines:?}");
}

/// A command that sends no number says its stop all the same: `main` arms it by `armed`. The manifest's read passes the cap.
#[test]
fn a_command_with_no_numbers_says_its_stop() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-cap-extensions");
    let folder = dir.0.join("extensions.d");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::File::create(folder.join("30-huge.json"))
        .expect("a file")
        .set_len((CAP_MB as u64 + 50) * 1024 * 1024)
        .expect("its length");
    let listener = listening(&dir);
    let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["extensions", "--data"])
        .arg(&dir.0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("boot");
    let status = ended(&mut child);
    assert_eq!(status.and_then(|s| s.code()), Some(STOP_CODE), "{status:?}");
    assert_eq!(
        drain(&listener),
        ["dash_rust.memory.cap_stop:1|c|#service:dash-rust,command:extensions"]
    );
}

fn ingest(dir: &TestDir, key: &str, file: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dash"));
    command
        .args(["ingest", "--data"])
        .arg(&dir.0)
        .args(["--mime", key])
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    command
}

/// A stop whose stderr pipe is already full still ends the process: a parent that reads nothing must not get a hung server.
#[test]
fn the_binary_stops_with_its_stderr_full() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-cap-full");
    let file = file_for_a_pod_past_the_cap(&dir);
    let (reader, mut writer) = std::io::pipe().expect("a pipe");
    let stderr = writer.try_clone().expect("its write end");
    let filler = std::thread::spawn(move || {
        let _ = writer.write_all(&vec![b'x'; 1024 * 1024]);
    });
    std::thread::sleep(Duration::from_millis(200));
    let mut child = ingest(&dir, "text/x-stay", &file)
        .stderr(stderr)
        .spawn()
        .expect("boot");
    let status = ended(&mut child);
    drop(reader);
    filler.join().expect("the filler");
    assert_eq!(status.and_then(|s| s.code()), Some(STOP_CODE), "{status:?}");
}

/// A pod that ignores its stdin's end does not outlive a stop.
#[test]
fn a_stop_kills_a_running_pods_group() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-cap-pod");
    let file = file_for_a_pod_past_the_cap(&dir);

    let stderr_file = dir.0.join("stderr");
    let mut child = ingest(&dir, "text/x-stay", &file)
        .stderr(std::fs::File::create(&stderr_file).expect("a file"))
        .spawn()
        .expect("boot");
    let status = ended(&mut child);
    let stderr = std::fs::read_to_string(&stderr_file).expect("its stderr");

    let pid: i32 = std::fs::read_to_string(dir.0.join("extensions.d/pid"))
        .unwrap_or_else(|e| panic!("the pod never started, {e}: {status:?}: {stderr}"))
        .trim()
        .parse()
        .expect("a pid");
    let end = std::time::Instant::now() + Duration::from_secs(5);
    let gone = loop {
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive || std::time::Instant::now() > end {
            break !alive;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !gone {
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    assert_eq!(
        status.and_then(|s| s.code()),
        Some(STOP_CODE),
        "{status:?}: {stderr}"
    );
    assert!(gone, "the pod {pid} outlived the stop");
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn call(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(stream, "{request} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").expect("write");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).expect("read");
    let (head, body) = answer.split_once("\r\n\r\n").expect("an HTTP answer");
    assert!(head.starts_with("HTTP/1.1 200"), "{request}: {answer}");
    body.to_string()
}

fn footprint(port: u16) -> (f64, f64) {
    let body = call(port, "GET /memory");
    let (current, peak) = body.split_once(' ').expect("two figures");
    (
        current.parse().expect("current"),
        peak.parse().expect("peak"),
    )
}

#[test]
fn the_binary_stays_inside_its_budgets() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-budgets");
    let mut db = store::open(&dir.0).expect("open");
    store::write_fixture(&mut db, FIXTURE_ROWS).expect("the fixture");
    drop(db);

    let mut child = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["serve", "--data"])
        .arg(&dir.0)
        .stdout(Stdio::piped())
        .spawn()
        .expect("boot");
    let stdout = child.stdout.take().expect("stdout");
    let server = Server(child);
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("its port");
    let port: u16 = line
        .trim()
        .strip_prefix("port ")
        .expect("port <n>")
        .parse()
        .expect("a port");

    assert_eq!(call(port, "GET /ping"), "pong");
    std::thread::sleep(Duration::from_millis(200));
    let (idle, _) = footprint(port);
    let scanned = call(port, "GET /scan");
    let (loaded, peak) = footprint(port);
    drop(server);
    println!(
        "idle: {idle:.1} MB; loaded {FIXTURE_ROWS} rows ({scanned}): {loaded:.1} MB (peak {peak:.1})"
    );

    assert!(
        scanned.starts_with(&format!("{FIXTURE_ROWS} ")),
        "{scanned}"
    );
    // The peak covers the idle and loaded readings: it can only be at or above them.
    within(SERVER_IDLE, peak, idle).expect("the fixture read, at its peak");
    for b in BUDGETS.iter().filter(|b| b.check != Check::Now) {
        println!("not checked yet: {} ({}), {:?}", b.state, b.budget, b.check);
    }
}
