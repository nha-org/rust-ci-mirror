//! rust/CLAUDE.md rule 5: the old app's defects never return in Rust, a test each (task rust-old-defects). The slow request's
//! is in the binary's own tests (main.rs, `a_slow_request_leaves_the_ping_and_the_pages_answering_under_200_ms`), as it drives
//! the server's private loop; the stopped watcher's in comment_queue.rs, beside the queue's own.
use dash_meter::TestDir;
use dash_meter::dash_test;
use dash_store::oplog::Replica;
use dash_sync::Node;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Ops the rebuilt replica takes in: enough that applying them under one lock, the defect, holds a write past 1 s (1.55 s
/// measured with sync's page unbounded, 2026-11-09).
const LOG: usize = 120_000;

fn replica(dir: &Path, writer: &str) -> Replica {
    let db = dash_store::connect(&dir.join("tasks.sqlite")).expect("the file");
    Replica::new(db, writer).expect("its log")
}

/// A store at `<dir>/to` rebuilding from another's whole log through sync, then copying itself whole, on a thread: the flag
/// stays up until it ends, its join says how long it took. `<dir>/locks` holds the claims' locks.
fn rebuilding(dir: &Path) -> (Arc<AtomicBool>, std::thread::JoinHandle<Duration>) {
    let (from, to, locks) = (dir.join("from"), dir.join("to"), dir.join("locks"));
    for d in [&from, &to] {
        std::fs::create_dir_all(d).expect("folder");
        dash_store::tasks::open(d, &locks).expect("the schema");
    }
    // a chunk a write, so the test's peak stays inside its budget
    let mut source = replica(&from, "fly");
    for chunk in (0..LOG).collect::<Vec<_>>().chunks(10_000) {
        let writes: Vec<_> = (chunk.iter())
            .map(|i| dash_core::FieldWrite {
                table: "listed".into(),
                row: format!("t{i}"),
                field: "title".into(),
                value: dash_core::Value::Text(format!("task {i}")),
            })
            .collect();
        source.write(&writes).expect("the log");
    }
    drop(source);

    let rebuilding = Arc::new(AtomicBool::new(true));
    let rebuild = {
        let rebuilding = Arc::clone(&rebuilding);
        std::thread::spawn(move || {
            let start = Instant::now();
            let (mut a, mut b) = (
                Node::new(Box::new(replica(&from, "fly"))),
                Node::new(Box::new(replica(&to, "mac"))),
            );
            while !a.settled().expect("settled") {
                let push = a.on_timer().expect("a page").expect("a push");
                if let Some(ack) = b.on_msg(push).expect("applied") {
                    a.on_msg(ack).expect("acked");
                }
            }
            let copy = to.join("copy.sqlite");
            replica(&to, "scan").vacuum_into(&copy).expect("the copy");
            rebuilding.store(false, Ordering::Relaxed);
            start.elapsed()
        })
    };
    (rebuilding, rebuild)
}

/// No lock held across a rebuild or a scan (Elixir held its store lock 75 to 83 s on 2026-11-09): while a replica rebuilds
/// from another's whole log through sync, then copies itself whole, a write on the same file waits under 1 s each time.
#[dash_test]
fn a_write_waits_under_1_s_while_the_store_rebuilds() {
    let dir = TestDir::new("old-defects-rebuild");
    let (rebuilding, rebuild) = rebuilding(&dir.0);
    let mut writer =
        dash_store::tasks::open(&dir.0.join("to"), &dir.0.join("locks")).expect("the writer");
    let (mut during, mut slowest) = (0, Duration::ZERO);
    while rebuilding.load(Ordering::Relaxed) {
        let start = Instant::now();
        let pane = format!("pane-{during}");
        writer.agent(&pane, "/tmp", "running").expect("a write");
        slowest = slowest.max(start.elapsed());
        during += 1;
        std::thread::sleep(Duration::from_millis(20));
    }
    let took = rebuild.join().expect("the rebuild");
    eprintln!("rebuild {took:?}, {during} writes, the slowest {slowest:?}");
    assert!(during >= 3, "the writes ran during the rebuild: {during}");
    assert!(
        slowest < Duration::from_secs(1),
        "a write waited {slowest:?}"
    );
    let rows = writer.agents().expect("the writes landed").len();
    assert_eq!(rows, during);
}

/// One green PR nobody holds, no reviewer.
struct OneGreen;

impl dash_orch::review::Lane for OneGreen {
    fn open(&self) -> Result<Vec<dash_orch::review::Open>, String> {
        Ok(vec![dash_orch::review::Open {
            number: 1460,
            url: "https://github.com/nha-org/site_perso/pull/1460".into(),
            branch: "b".into(),
            created: "2026-11-12T10:00:00Z".into(),
        }])
    }
    fn green(&self, _: &dash_orch::review::Open) -> Result<bool, String> {
        Ok(true)
    }
    fn reviewed(&self) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
}

/// `dash review next` answers within 2 s while a whole-store write runs (task rust-review-next-port, done-when 3: the lane
/// waited over an hour on the older app's on 2026-11-12): its claim, then its touch of that claim, each time.
#[dash_test]
fn review_next_answers_within_2_s_while_the_store_rebuilds() {
    use dash_orch::review::{Lane, Next, next};
    let dir = TestDir::new("old-defects-review-next");
    let (rebuilding, rebuild) = rebuilding(&dir.0);
    let mut tasks =
        dash_store::tasks::open(&dir.0.join("to"), &dir.0.join("locks")).expect("the store");
    let (mut said, mut slowest) = (vec![], Duration::ZERO);
    while rebuilding.load(Ordering::Relaxed) {
        let start = Instant::now();
        said.push(next(&OneGreen, &mut tasks).expect("an answer"));
        slowest = slowest.max(start.elapsed());
        std::thread::sleep(Duration::from_millis(20));
    }
    let took = rebuild.join().expect("the rebuild");
    eprintln!(
        "rebuild {took:?}, {} answers, the slowest {slowest:?}",
        said.len()
    );
    assert!(
        said.len() >= 3,
        "the asks ran during the rebuild: {}",
        said.len()
    );
    assert!(
        slowest < Duration::from_secs(2),
        "review next took {slowest:?}"
    );
    let pr = OneGreen.open().expect("the PR").remove(0);
    assert_eq!(said[0], Next::Took(pr.clone()));
    assert!(
        said[1..].iter().all(|n| *n == Next::Held(pr.clone())),
        "{said:?}"
    );
}

/// A stand-in MCP server: logs each start, answers tools/list with `list.json` as it is at that moment, a call of `bad` with
/// an error of its own, and any other call.
fn stand_in(dir: &Path) -> String {
    format!(
        r#"echo start >> '{0}/starts'
answer() {{ printf '{{"jsonrpc":"2.0","id":%s,"result":%s}}\n' "$id" "$1"; }}
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"tools/list"'*) answer "{{\"tools\":$(cat '{0}/list.json')}}" ;;
    *'"name":"bad"'*) printf '{{"jsonrpc":"2.0","id":%s,"error":{{"code":-32602,"message":"bad arguments"}}}}\n' "$id" ;;
    *'"tools/call"'*) answer '{{"content":[{{"type":"text","text":"sent"}}]}}' ;;
    *'"initialize"'*) answer '{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"stand-in","version":"1"}}}}' ;;
  esac
done
"#,
        dir.display()
    )
}

/// One client session through `dash mcp-proxy`: these requests (`bad`: a call of the tool the server refuses), then the end
/// of its input; each answer whole.
fn session(dir: &Path, requests: &[&str]) -> Vec<serde_json::Value> {
    use std::io::Write;
    let mut proxy = std::process::Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["mcp-proxy", "--cache"])
        .arg(dir.join("cache.json"))
        .args(["--", "sh", "-c", &stand_in(dir)])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("dash mcp-proxy");
    let mut input = proxy.stdin.take().expect("its input");
    for (id, method) in requests.iter().enumerate() {
        let (method, params) = match *method {
            "tools/call" => (*method, r#"{"name":"send","arguments":{}}"#),
            "bad" => ("tools/call", r#"{"name":"bad","arguments":{}}"#),
            other => (other, "{}"),
        };
        let line = match method {
            "not json" => "{not json".to_string(),
            _ => format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#),
        };
        writeln!(input, "{line}").expect("a request");
    }
    drop(input);
    let output = proxy.wait_with_output().expect("its answers");
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).expect("utf-8");
    text.lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("JSON"))
        .collect()
}

fn names(answer: &serde_json::Value) -> Vec<String> {
    let tools = answer["result"]["tools"].as_array().expect("a list");
    tools
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// A tool list is never empty while its server refills (the bb proxy's LinkedIn list read [] after a rev bump,
/// 2026-11-08): with the server answering [] mid-refill, `dash mcp-proxy` serves its last list, before and after the
/// server starts, and keeps it in its cache; a full new list replaces it.
#[dash_test]
fn a_refilling_mcp_cache_serves_its_last_tool_list() {
    let dir = TestDir::new("old-defects-mcp");
    let list = |tools: &str| std::fs::write(dir.0.join("list.json"), tools).expect("the list");
    let starts = || {
        std::fs::read_to_string(dir.0.join("starts"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    let cache = || std::fs::read_to_string(dir.0.join("cache.json")).expect("the cache");
    list(
        r#"[{"name":"send","inputSchema":{"type":"object"}},{"name":"read","inputSchema":{"type":"object"}}]"#,
    );

    // no cache yet: the server starts and fills it
    let first = session(&dir.0, &["initialize", "tools/list"]);
    assert_eq!(names(&first[1]), ["send", "read"]);
    assert_eq!(starts(), 1);

    // the rev bump: the server answers [] while it refills
    list("[]");
    let during = session(
        &dir.0,
        &["initialize", "tools/list", "tools/call", "tools/list"],
    );
    assert_eq!(
        names(&during[1]),
        ["send", "read"],
        "served before any start"
    );
    assert_eq!(
        during[2]["result"]["content"][0]["text"], "sent",
        "the call reaches the server"
    );
    assert_eq!(
        names(&during[3]),
        ["send", "read"],
        "after its empty refill too"
    );
    assert_eq!(starts(), 2, "started for the call only");
    assert!(cache().contains(r#""name":"read""#), "{}", cache());

    // refilled: its new list replaces the last at its next start
    list(r#"[{"name":"send","inputSchema":{"type":"object"}}]"#);
    let after = session(&dir.0, &["tools/call", "tools/list"]);
    assert_eq!(names(&after[1]), ["send"]);
    assert!(!cache().contains(r#""name":"read""#), "{}", cache());

    // the server's own error reaches the client as it sent it, and the server stays: the next call needs no start
    let refused = session(&dir.0, &["bad", "tools/call"]);
    assert_eq!(refused[0]["error"]["code"], -32602, "{}", refused[0]);
    assert_eq!(refused[0]["id"], 0);
    assert_eq!(refused[1]["result"]["content"][0]["text"], "sent");
    assert_eq!(starts(), 4, "one start for the session");

    // a line that does not read is answered, never left waiting
    let unread = session(&dir.0, &["not json"]);
    assert_eq!(unread[0]["error"]["code"], -32700, "{}", unread[0]);
}
