//! rust/CLAUDE.md rule 5: the old app's defects never return in Rust, a test each (task rust-old-defects). The slow request's
//! is in the binary's own tests (main.rs, `a_slow_request_leaves_the_ping_and_the_pages_answering_under_200_ms`), as it drives
//! the server's private loop; the stopped watcher's in comment_queue.rs, beside the queue's own.
use dash_meter::TestDir;
use dash_meter::dash_test;
use dash_store::oplog::Replica;
use dash_sync::{Msg, Node};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

/// Ops the rebuilt replica takes in: enough pages of sync's 64 that a write waiting behind all of them shows (applying them
/// under one lock, the defect, held a write 1.55 s with sync's page unbounded, 2026-11-09).
const LOG: usize = 120_000;

fn replica(dir: &Path, writer: &str) -> Replica {
    let db = dash_store::connect(&dir.join("tasks.sqlite")).expect("the file");
    Replica::new(db, writer).expect("its log")
}

/// A rebuild's progress in ops: those begun, counted before each page applies, and those applied, after it commits. It
/// starts once the caller passes `go`, its store open, so a slow open never reads as a late first call.
struct Progress {
    go: Barrier,
    begun: AtomicUsize,
    applied: AtomicUsize,
}

/// A store at `<dir>/to` rebuilding from another's whole log through sync, then copying itself whole, on a thread:
/// `Progress` follows it, its join says how long it took and how many ops it applied. `<dir>/locks` holds the claims' locks.
fn rebuilding(dir: &Path) -> (Arc<Progress>, std::thread::JoinHandle<(Duration, usize)>) {
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

    let progress = Arc::new(Progress {
        go: Barrier::new(2),
        begun: AtomicUsize::new(0),
        applied: AtomicUsize::new(0),
    });
    let rebuild = {
        let progress = Arc::clone(&progress);
        std::thread::spawn(move || {
            progress.go.wait();
            let start = Instant::now();
            let (mut a, mut b) = (
                Node::new(Box::new(replica(&from, "fly"))),
                Node::new(Box::new(replica(&to, "mac"))),
            );
            while !a.settled().expect("settled") {
                let push = a.on_timer().expect("a page").expect("a push");
                let Msg::Push(ops) = &push else {
                    unreachable!("a timer pushes")
                };
                let ops = ops.len();
                progress.begun.fetch_add(ops, Ordering::SeqCst);
                if let Some(ack) = b.on_msg(push).expect("applied") {
                    a.on_msg(ack).expect("acked");
                }
                progress.applied.fetch_add(ops, Ordering::SeqCst);
            }
            let copy = to.join("copy.sqlite");
            replica(&to, "scan").vacuum_into(&copy).expect("the copy");
            (start.elapsed(), progress.applied.load(Ordering::SeqCst))
        })
    };
    (progress, rebuild)
}

/// The rebuild's ops `call` may have waited on: from those applied when it started to those begun when it ended, so a lock
/// across n ops reads at least n, whatever sync's page. Counted in ops, not seconds, as a shared runner slows both threads
/// alike: a 1 s bound there failed with a healthy write path (1.07 and 1.15 s slowest, rebuild 8 to 11 s, mirror runs
/// 43214529491 and 43276140188, task rust-store-rebuild-write-flake). The copy holds no op, 91 ms of a 2.5 s rebuild on the
/// Mac: a lock across it alone shows in neither this count nor that bound, and VACUUM INTO under WAL takes no write lock.
fn ops_during<T>(progress: &Progress, call: impl FnOnce() -> T) -> (T, Range<usize>) {
    let applied = progress.applied.load(Ordering::SeqCst);
    let out = call();
    (out, applied..progress.begun.load(Ordering::SeqCst))
}

/// The calls ran during the rebuild, the first in its first half, and each waited under half its ops: with a lock across
/// the whole rebuild, that first call would wait all the ops left, more than half (sync's page unbounded, the defect's
/// measured form, reads 120,000 of 120,000).
fn never_waited_out(waits: &[Range<usize>], total: usize) {
    assert!(
        waits.len() >= 3,
        "the calls ran during the rebuild: {}",
        waits.len()
    );
    let first = waits[0].start;
    assert!(
        first < total / 2,
        "the first call started at op {first} of {total}"
    );
    let most = waits.iter().map(Range::len).max().unwrap_or(0);
    assert!(most < total / 2, "a call waited {most} of {total} ops");
}

/// No lock held across a rebuild (Elixir held its store lock 75 to 83 s on 2026-11-09): while a replica rebuilds from
/// another's whole log through sync, then copies itself whole, a write on the same file never waits it out
/// (`ops_during`; 1 s of wall clock until task rust-store-rebuild-write-flake).
#[dash_test]
fn a_write_never_waits_out_the_rebuild() {
    let dir = TestDir::new("old-defects-rebuild");
    let (progress, rebuild) = rebuilding(&dir.0);
    let mut writer =
        dash_store::tasks::open(&dir.0.join("to"), &dir.0.join("locks")).expect("the writer");
    let mut waits = vec![];
    progress.go.wait();
    while !rebuild.is_finished() {
        let pane = format!("pane-{}", waits.len());
        let ((), waited) = ops_during(&progress, || {
            writer.agent(&pane, "/tmp", "running").expect("a write")
        });
        waits.push(waited);
        std::thread::sleep(Duration::from_millis(20));
    }
    let (took, total) = rebuild.join().expect("the rebuild");
    let longest = waits.iter().map(Range::len).max().unwrap_or(0);
    eprintln!(
        "rebuild {took:?} in {total} ops, {} writes, the longest wait {longest} ops",
        waits.len()
    );
    never_waited_out(&waits, total);
    writer
        .agent("after", "/tmp", "running")
        .expect("a write after it");
    let rows = writer.agents().expect("the writes landed").len();
    assert_eq!(rows, waits.len() + 1);
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

/// `dash review next` answers while a whole-store write runs, never waiting it out (task rust-review-next-port, done-when 3:
/// the lane waited over an hour on the older app's on 2026-11-12; `ops_during`, 2 s of wall clock until task
/// rust-store-rebuild-write-flake): its claim, then its touch of that claim, each time.
#[dash_test]
fn review_next_never_waits_out_the_rebuild() {
    use dash_orch::review::{Lane, Next, next};
    let dir = TestDir::new("old-defects-review-next");
    let (progress, rebuild) = rebuilding(&dir.0);
    let mut tasks =
        dash_store::tasks::open(&dir.0.join("to"), &dir.0.join("locks")).expect("the store");
    let (mut said, mut waits) = (vec![], vec![]);
    progress.go.wait();
    while !rebuild.is_finished() {
        let (answer, waited) = ops_during(&progress, || {
            next(&OneGreen, &mut tasks).expect("an answer")
        });
        said.push(answer);
        waits.push(waited);
        std::thread::sleep(Duration::from_millis(20));
    }
    let (took, total) = rebuild.join().expect("the rebuild");
    let longest = waits.iter().map(Range::len).max().unwrap_or(0);
    eprintln!(
        "rebuild {took:?} in {total} ops, {} answers, the longest wait {longest} ops",
        said.len()
    );
    never_waited_out(&waits, total);
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
