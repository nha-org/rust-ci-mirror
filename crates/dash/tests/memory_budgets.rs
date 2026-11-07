//! Section 4 held by a test: writes a fixture the size of dash's store, boots the binary on it, has it read the whole
//! store, fails past the budget it can reach, and prints the idle and loaded footprints (.config/nextest.toml shows this
//! test's output on a pass).
use dash_meter::TestDir;
use dash_meter::budgets::{BUDGETS, Check, SERVER_IDLE, within};
use dash_meter::cap::{CAP_MB, STOP_CODE};
use dash_meter::memory::TestBudget;
use dash_store as store;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const FIXTURE_ROWS: u64 = 200_000;

/// `dash ingest` reads the whole file into memory: a real path past the cap, with no test hook in the binary. The file is sparse
/// and the read stops before touching it, so this costs no disk and no memory.
#[test]
fn the_binary_stops_with_a_named_line_past_its_cap() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-cap");
    let file = dir.0.join("past-the-cap");
    std::fs::File::create(&file)
        .expect("a file")
        .set_len((CAP_MB as u64 + 50) * 1024 * 1024)
        .expect("its length");
    let output = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["ingest", "--data"])
        .arg(&dir.0)
        .args(["--mime", "text/plain"])
        .arg(&file)
        .output()
        .expect("boot");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("past the cap of {CAP_MB} MB, stopping")),
        "{output:?}"
    );
    assert_eq!(output.status.code(), Some(STOP_CODE), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

fn ingest(dir: &TestDir, key: &str, file: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dash"));
    command
        .args(["ingest", "--data"])
        .arg(&dir.0)
        .args(["--mime", key])
        .arg(file)
        .stdout(Stdio::null());
    command
}

/// A stop whose stderr pipe is already full still ends the process: a parent that reads nothing must not get a hung server.
#[test]
fn the_binary_stops_with_its_stderr_full() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-cap-full");
    let file = dir.0.join("past-the-cap");
    std::fs::File::create(&file)
        .expect("a file")
        .set_len((CAP_MB as u64 + 50) * 1024 * 1024)
        .expect("its length");
    let (reader, mut writer) = std::io::pipe().expect("a pipe");
    let stderr = writer.try_clone().expect("its write end");
    let filler = std::thread::spawn(move || {
        let _ = writer.write_all(&vec![b'x'; 1024 * 1024]);
    });
    std::thread::sleep(Duration::from_millis(200));
    let mut child = ingest(&dir, "text/plain", &file)
        .stderr(stderr)
        .spawn()
        .expect("boot");
    let end = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("its state") {
            break Some(status);
        }
        if std::time::Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(reader);
    filler.join().expect("the filler");
    assert_eq!(status.and_then(|s| s.code()), Some(STOP_CODE), "{status:?}");
}

/// A pod that ignores its stdin's end does not outlive a stop. It answers `initialize`, then stays. The file's NUL bytes are valid
/// text: its read and its copy into the call fit under the cap, the next copy passes it, with the pod running by then.
#[test]
fn a_stop_kills_a_running_pods_group() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("memory-cap-pod");
    let folder = dir.0.join("extensions.d");
    std::fs::create_dir_all(&folder).expect("folder");
    let program = r#"IFS= read -r line; echo $$ > pid; printf '{"jsonrpc":"2.0","id":1,"result":{}}\n'; while :; do sleep 1; done"#;
    // A Rust string's debug form is a JSON string here: only quotes and a backslash to escape.
    let manifest = format!(
        r#"{{"command": ["sh", "-c", {program:?}], "extractors": {{"text/x-stay": "stay"}}}}"#
    );
    std::fs::write(folder.join("30-stay.json"), manifest).expect("the manifest");
    let file = dir.0.join("under-the-cap");
    std::fs::File::create(&file)
        .expect("a file")
        .set_len((CAP_MB as u64 / 2 - 10) * 1024 * 1024)
        .expect("its length");

    let output = ingest(&dir, "text/x-stay", &file).output().expect("boot");
    assert_eq!(output.status.code(), Some(STOP_CODE), "{output:?}");

    let pid: i32 = std::fs::read_to_string(folder.join("pid"))
        .expect("the pod started")
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
