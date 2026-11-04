//! Section 4 held by a test: writes a fixture the size of dash's store, boots the binary on it, has it read the whole
//! store, fails past the budget it can reach, and prints the idle and loaded footprints (.config/nextest.toml shows this
//! test's output on a pass).
use dash::TestDir;
use dash::budgets::{BUDGETS, Check, SERVER_IDLE, within};
use dash::memory::TestBudget;
use dash::store;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const FIXTURE_ROWS: u64 = 200_000;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn call(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(stream, "{request} HTTP/1.1\r\nHost: x\r\n\r\n").expect("write");
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
