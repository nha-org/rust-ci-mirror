//! Section 11 and 12 through the real binary: a compiled-in crate and an MCP pod each add an extractor through flat dispatch,
//! the pod by a file in <data>/extensions.d and with the binary byte for byte the same.
use dash_core::Store;
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use dash_store as store;
use std::hash::Hasher;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

fn install_pod(dir: &TestDir) {
    std::fs::create_dir_all(dir.0.join("extensions.d")).expect("folder");
    let manifest = dir.0.join("extensions.d/30-rot13.json");
    std::fs::write(manifest, dash_ext::pod::sample_manifest()).expect("the manifest");
}

fn dash(args: &[&str], dir: &TestDir) -> String {
    let output = Command::new(DASH)
        .arg(args[0])
        .arg("--data")
        .arg(&dir.0)
        .args(&args[1..])
        .output()
        .expect("dash");
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).expect("utf-8")
}

fn ingest(dir: &TestDir, key: &str, text: &str) -> String {
    let file = dir.0.join("input.txt");
    std::fs::write(&file, text).expect("input");
    let path = file.to_str().expect("a path");
    dash(&["ingest", "--mime", key, path], dir)
        .trim_end()
        .to_string()
}

/// The binary's size, modified time and a hash of every byte, read in pieces: a debug build is past a test's budget.
fn fingerprint() -> (u64, std::time::SystemTime, u64) {
    let mut file = std::fs::File::open(DASH).expect("the binary");
    let meta = file.metadata().expect("metadata");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut piece = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut piece).expect("read");
        if n == 0 {
            break;
        }
        hasher.write(&piece[..n]);
    }
    (meta.len(), meta.modified().expect("mtime"), hasher.finish())
}

#[test]
fn a_compiled_in_crate_adds_its_extractor_at_link_time() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-crate");
    assert_eq!(
        dash(&["extensions"], &dir),
        "extractor * crate dash-ingest\n\
         extractor text/markdown crate dash-ingest\n\
         extractor text/plain crate dash-ingest\n"
    );
    assert_eq!(ingest(&dir, "text/plain", "plain"), "plain");
    assert_eq!(ingest(&dir, "image/png", "by default"), "by default");
}

#[test]
fn an_mcp_pod_adds_an_extractor_with_no_rebuild() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-pod");
    let before = fingerprint();
    assert_eq!(
        ingest(&dir, "text/x-rot13", "hello"),
        "hello",
        "the default"
    );

    install_pod(&dir);

    assert_eq!(ingest(&dir, "text/x-rot13", "hello"), "uryyb");
    assert!(
        dash(&["extensions"], &dir).contains("extractor text/x-rot13 30-rot13.json\n"),
        "what runs for a key is read"
    );
    let after = fingerprint();
    assert_eq!(before, after, "the binary is unchanged");
    println!("the binary, before and after: {after:?}");
}

#[test]
fn files_load_in_name_order_and_a_bad_or_later_file_is_refused_alone() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-order");
    install_pod(&dir);
    let folder = dir.0.join("extensions.d");
    let write = |name: &str, text: &str| std::fs::write(folder.join(name), text).expect(name);
    write("20-broken.json", "not json");
    write(
        "50-takes-the-same-key.json",
        &dash_ext::pod::sample_manifest(),
    );
    write("notes.txt", "not a manifest");

    assert_eq!(
        ingest(&dir, "text/x-rot13", "hello"),
        "uryyb",
        "a file after a broken one still loads, and the first file keeps its key"
    );
    let listing = dash(&["extensions"], &dir);
    let refused: Vec<&str> = listing
        .lines()
        .filter(|l| l.starts_with("refused"))
        .collect();
    assert_eq!(refused.len(), 2, "{listing}");
    assert!(refused[0].contains("20-broken.json"), "{refused:?}");
    assert!(
        refused[1].contains("50-takes-the-same-key.json")
            && refused[1].contains("taken by 30-rot13.json"),
        "{refused:?}"
    );
}

#[test]
fn ingest_stores_nothing_when_the_default_would_guess() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-ingest-refuses");
    let failing = |key: &str, input: &[u8]| {
        let file = dir.0.join("input.bin");
        std::fs::write(&file, input).expect("input");
        let output = Command::new(DASH)
            .args(["ingest", "--data"])
            .arg(&dir.0)
            .args(["--mime", key])
            .arg(&file)
            .output()
            .expect("dash");
        assert!(!output.status.success(), "{key}: {output:?}");
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    assert!(failing("image/png", b"\xff\xd8").contains("not text"));
    install_pod(&dir);
    std::fs::write(dir.0.join("extensions.d/20-broken.json"), "not json").expect("write");
    assert!(failing("text/x-broken", b"text").contains("nothing is stored"));
    let rows = store::open(&dir.0)
        .expect("open")
        .scan()
        .expect("scan")
        .rows;
    assert_eq!(rows, 0, "no fact landed");
    assert_eq!(
        ingest(&dir, "text/x-rot13", "hello"),
        "uryyb",
        "a key with its entry still runs"
    );
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn boot(dir: &TestDir) -> (Server, u16) {
    let mut child = Command::new(DASH)
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
    let port = line
        .trim()
        .strip_prefix("port ")
        .expect("port <n>")
        .parse()
        .expect("a port");
    (server, port)
}

fn get(port: u16, path: &str, host: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").expect("write");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).expect("read");
    answer
}

fn body(port: u16, path: &str) -> String {
    let answer = get(port, path, "127.0.0.1");
    assert!(answer.starts_with("HTTP/1.1 200"), "{path}: {answer}");
    answer.split_once("\r\n\r\n").expect("a body").1.to_string()
}

/// A page of this server: plain text by its header, and no tag of its own in it.
fn text_page(port: u16, path: &str) -> String {
    let answer = get(port, path, "127.0.0.1");
    let (head, page) = answer.split_once("\r\n\r\n").expect("a body");
    assert!(head.starts_with("HTTP/1.1 200"), "{path}: {answer}");
    let kind = "Content-Type: text/plain; charset=utf-8";
    assert!(head.contains(kind), "{path}: {answer}");
    assert_eq!(
        dash_gate::html::html_tags(page),
        Vec::<String>::new(),
        "{path}"
    );
    page.to_string()
}

/// A stale build fails here: the page served is the page of the code now. crates/gate's a_new_edge_is_on_the_page_after_the_next_build
/// holds the rebuild itself.
#[test]
fn deps_serves_the_graph_of_the_code_it_was_built_from() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-deps");
    let (_server, port) = boot(&dir);
    let page = text_page(port, "/deps");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(page, dash_gate::graph::page(&root));
    assert!(page.contains("\nstore\tcore"), "a crate's edge: {page}");
    assert!(page.contains("\nstore\nThis\tDepends on\n"), "{page}");
    assert!(page.contains("\ncrate::tasks\t"), "a module: {page}");
    assert!(page.contains("\nOne module each: "), "{page}");
}

#[test]
fn search_answers_in_the_store_s_order_with_no_extension_and_refuses_a_rebinding_page() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-search");
    let db = store::open(&dir.0).expect("open");
    db.add_fact("file", "first", "rust in the body")
        .expect("add");
    db.add_fact("file", "second", "nothing").expect("add");
    db.add_fact("file", "a RUST title", "more").expect("add");
    drop(db);

    let (_server, port) = boot(&dir);
    let answer = body(port, "/search?q=rust");
    let titles: Vec<&str> = answer
        .lines()
        .filter_map(|l| l.split('\t').nth(1))
        .collect();
    assert_eq!(titles, ["first", "a RUST title"]);
    assert!(get(port, "/ui/search", "127.0.0.1").starts_with("HTTP/1.1 404"));
    assert!(
        get(port, "/search?q=rust", "evil.example").starts_with("HTTP/1.1 403"),
        "a rebinding page"
    );
}

#[test]
fn serve_on_a_port_lists_the_tasks_and_refuses_any_other_host() {
    use dash_core::tasks::{Outcome, Run, TaskStore};
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-port");
    let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("open");
    tasks.claim("a-task").expect("claim");
    let outcome = Outcome::Merged("https://github.com/o/r/pull/7".into());
    let (pane, session) = (None, None);
    let run = Run {
        pane,
        session,
        outcome,
    };
    tasks.record("a-task", &run).expect("record");
    drop(tasks);

    let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let want = free.local_addr().expect("address").port();
    drop(free);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["serve", "--data"])
        .arg(&dir.0)
        .args(["--port", &want.to_string()])
        // Nothing reads an app here: the queue's thread fails fast, and the page says so.
        .env("DASH_CLI", "/nonexistent/dash")
        .env("DASH_WORKTREES", &dir.0)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("port line");
    let _server = Server(child);
    assert_eq!(line.trim(), format!("port {want}"));

    assert!(
        text_page(want, "/deps").contains("\nTasks: /\n"),
        "/ is served here, so /deps names it"
    );
    let page = text_page(want, "/");
    assert!(page.contains("\na-task\tfinished\t1\t"), "{page}");
    assert!(page.contains("\thttps://github.com/o/r/pull/7\t"), "{page}");
    assert!(page.contains("\nDependencies: /deps\n"), "{page}");
    assert!(
        page.ends_with(&format!(
            "\nBuilt from {}.\n",
            option_env!("DASH_COMMIT").unwrap_or("a local build")
        )),
        "make rust/deploy reads it: {page}"
    );
    for host in ["localhost", "localhost:1"] {
        assert!(
            get(want, "/ping", host).starts_with("HTTP/1.1 200"),
            "{host}"
        );
    }
    // Loopback takes loopback's names only: even his tailnet's address is refused on it.
    for host in ["evil.example", "100.72.158.115", "100.72.158.115:7799", ""] {
        for path in ["/", "/search?q=x"] {
            assert!(
                get(want, path, host).starts_with("HTTP/1.1 403"),
                "{host} {path}"
            );
        }
    }
}

/// What `dash mcp` writes. It cannot catch `claim_stdout` going: nothing in this command writes to fd 1. What that guard
/// holds today is `dash orch`'s children, who inherit it.
#[test]
fn mcp_writes_only_the_protocol_on_stdout() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-mcp");
    let mut child = Command::new(DASH)
        .args(["mcp", "--data"])
        .arg(&dir.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("boot");
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    ];
    let mut stdin = child.stdin.take().expect("stdin");
    writeln!(stdin, "{}", input.join("\n")).expect("write");
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("it ends at its input's end");
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).expect("utf-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "only the protocol on stdout: {text}");
    assert!(lines.iter().all(|l| l.starts_with('{')), "{text}");
}
