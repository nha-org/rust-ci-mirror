//! Section 11 and 12 through the real binary: a compiled-in crate, a Steel script and an MCP pod each add an extractor
//! through flat dispatch, the last two by a file in <data>/extensions.d and with the binary byte for byte the same; and one
//! extension adds a search bar, an MCP tool and a rerank hook. Sample extensions: crates/ext/extensions.d.
use dash_core::Store;
use dash_meter::TestDir;
use dash_meter::budgets::{SERVER_IDLE, within};
use dash_meter::memory::TestBudget;
use dash_store as store;
use std::hash::Hasher;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ext/extensions.d")
        .join(name)
}

fn install(dir: &TestDir, names: &[&str]) {
    let folder = dir.0.join("extensions.d");
    std::fs::create_dir_all(&folder).expect("folder");
    for name in names {
        std::fs::copy(sample(name), folder.join(name)).expect(name);
    }
}

fn install_pod(dir: &TestDir) {
    install(dir, &[]);
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
fn a_steel_script_adds_an_extractor_with_no_rebuild() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-script");
    let before = fingerprint();
    assert_eq!(
        ingest(&dir, "text/x-shout", "quiet"),
        "quiet",
        "the default"
    );

    install(&dir, &["10-shout.scm"]);

    assert_eq!(ingest(&dir, "text/x-shout", "quiet"), "QUIET");
    assert_eq!(
        ingest(&dir, "text/plain", "quiet"),
        "quiet",
        "others keep theirs"
    );
    assert!(
        dash(&["extensions"], &dir).contains("extractor text/x-shout 10-shout.scm\n"),
        "what runs for a key is read"
    );
    let after = fingerprint();
    assert_eq!(before, after, "the binary is unchanged");
    println!("the binary, before and after: {after:?}");
    let left: Vec<_> = std::fs::read_dir(dir.0.join("extensions.d"))
        .expect("folder")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left, ["10-shout.scm"], "Steel wrote nothing beside it");
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
    install(&dir, &["10-shout.scm"]);
    let folder = dir.0.join("extensions.d");
    let write = |name: &str, text: &str| std::fs::write(folder.join(name), text).expect(name);
    write(
        "20-takes-the-same-key.scm",
        r#"(register-ui! "never" (hash "root" "a" "elements" (hash "a" (hash "type" "stack"))))
           (register-extractor! "text/x-shout" (lambda (s) "later"))"#,
    );
    write("30-broken.scm", "(define (unbalanced");
    write("40-broken.json", "not json");
    write(
        "45-bad-tree.scm",
        r#"(register-ui! "slot" (hash "root" "a" "elements" (hash "a" (hash "type" "iframe"))))"#,
    );
    write(
        "50-fine.scm",
        r#"(register-extractor! "text/x-fine" (lambda (s) "fine"))"#,
    );
    write("notes.txt", "neither a script nor a manifest");

    assert_eq!(
        ingest(&dir, "text/x-shout", "quiet"),
        "QUIET",
        "the first file keeps it"
    );
    assert_eq!(
        ingest(&dir, "text/x-fine", "x"),
        "fine",
        "the others still load"
    );
    let listing = dash(&["extensions"], &dir);
    assert!(
        !listing.contains("never"),
        "a refused file adds nothing: {listing}"
    );
    let refused: Vec<&str> = listing
        .lines()
        .filter(|l| l.starts_with("refused"))
        .collect();
    assert_eq!(refused.len(), 4, "{listing}");
    assert!(
        refused[0].contains("20-takes-the-same-key.scm")
            && refused[0].contains("taken by 10-shout.scm"),
        "{refused:?}"
    );
    assert!(
        refused[1].contains("30-broken.scm") && refused[2].contains("40-broken.json"),
        "{refused:?}"
    );
    assert!(
        refused[3].contains("45-bad-tree.scm") && refused[3].contains("iframe"),
        "a tree that cannot draw: {refused:?}"
    );
}

/// A script's own tests pass on a file a start would refuse, leaving the extension silently absent: the registry's and the
/// renderer's checks are held here, for every sample, so a new one needs no new Rust.
#[test]
fn every_sample_loads_as_a_start_would() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-samples");
    let samples = std::fs::read_dir(sample("")).expect("samples");
    let names: Vec<String> = samples
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!names.is_empty());
    install(&dir, &names.iter().map(String::as_str).collect::<Vec<_>>());
    let listing = dash(&["extensions"], &dir);
    assert!(!listing.contains("refused"), "{listing}");
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
    install(&dir, &["10-shout.scm"]);
    std::fs::write(dir.0.join("extensions.d/20-broken.scm"), "(define (x").expect("write");
    assert!(failing("text/x-broken", b"text").contains("nothing is stored"));
    let rows = store::open(&dir.0)
        .expect("open")
        .scan()
        .expect("scan")
        .rows;
    assert_eq!(rows, 0, "no fact landed");
    assert_eq!(
        ingest(&dir, "text/x-shout", "quiet"),
        "QUIET",
        "a key with its entry still runs"
    );
}

#[test]
fn a_scripts_stdout_never_reaches_what_a_command_answers() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-noisy");
    install(&dir, &[]);
    std::fs::write(
        dir.0.join("extensions.d/10-noisy.scm"),
        r#"(displayln "printed at load")
           (register-extractor! "text/x-noisy" (lambda (s) (displayln "printed in a call") s))"#,
    )
    .expect("write");
    assert_eq!(ingest(&dir, "text/x-noisy", "text"), "text");
    assert!(!dash(&["extensions"], &dir).contains("printed"));
    let (_server, port) = boot(&dir); // It panics unless `port <n>` is the first line.
    assert_eq!(body(port, "/ping"), "pong");
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

fn titles(lines: &str) -> Vec<&str> {
    lines.lines().filter_map(|l| l.split('\t').nth(1)).collect()
}

#[test]
fn one_extension_adds_a_search_bar_and_a_rerank_hook_to_the_running_server() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-plugin-web");
    let db = store::open(&dir.0).expect("open");
    db.add_fact("file", "first", "rust in the body")
        .expect("add");
    db.add_fact("file", "second", "nothing").expect("add");
    db.add_fact("file", "a RUST title", "more").expect("add");
    drop(db);

    {
        let (_server, port) = boot(&dir);
        assert_eq!(
            titles(&body(port, "/search?q=rust")),
            ["first", "a RUST title"],
            "the store's order"
        );
        assert!(
            get(port, "/ui/search", "127.0.0.1").starts_with("HTTP/1.1 404"),
            "no bar yet"
        );
    }

    install(&dir, &["20-notes-search.scm"]);

    let (_server, port) = boot(&dir);
    let page = get(port, "/ui/search", "127.0.0.1");
    assert!(page.contains("Content-Type: text/html"), "{page}");
    assert!(
        page.contains(r#"<input type="search" name="q" placeholder="Search your notes">"#),
        "{page}"
    );
    assert!(
        page.contains(r#"action="/search""#),
        "the bar's form goes to the route below: {page}"
    );
    assert_eq!(
        titles(&body(port, "/search?q=rust")),
        ["a RUST title", "first"],
        "the hook moved it"
    );
    assert!(
        get(port, "/search?q=rust", "evil.example").starts_with("HTTP/1.1 403"),
        "a rebinding page"
    );
}

#[test]
fn the_same_extension_adds_an_mcp_tool_and_a_scripts_stdout_cannot_break_the_protocol() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-plugin-mcp");
    install(&dir, &["20-notes-search.scm"]);
    std::fs::write(
        dir.0.join("extensions.d/90-noisy.scm"),
        r#"(displayln "printed at load")
           (register-tool! "noisy" "Prints" (hash "type" "object")
             (lambda (arguments) (displayln "printed in a call") "quiet answer"))"#,
    )
    .expect("write");
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
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"query_terms","arguments":{"query":"Rust PODS"}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"noisy","arguments":{}}}"#,
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
    assert_eq!(lines.len(), 4, "only the protocol on stdout: {text}");
    assert!(lines.iter().all(|l| l.starts_with('{')), "{text}");
    assert!(lines[1].contains(r#""name":"query_terms""#), "{text}");
    assert!(lines[1].contains(r#""required":["query"]"#), "{text}");
    assert!(lines[2].contains(r#"[\"rust\",\"pods\"]"#), "{text}");
    assert!(lines[3].contains("quiet answer"), "{text}");
}

#[test]
fn the_extensions_stay_inside_the_idle_budget() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-memory");
    let footprint = |port| -> (f64, f64) {
        let answer = body(port, "/memory");
        let (current, peak) = answer.split_once(' ').expect("two figures");
        (
            current.parse().expect("current"),
            peak.parse().expect("peak"),
        )
    };
    let (none, _) = {
        let (_server, port) = boot(&dir);
        footprint(port)
    };
    install(&dir, &["10-shout.scm", "20-notes-search.scm"]);
    install_pod(&dir);
    let (_server, port) = boot(&dir);
    let (with, peak) = footprint(port);
    println!(
        "idle: {none:.1} MB with no extension, {with:.1} MB (peak {peak:.1}) with two scripts and a pod"
    );
    assert!(none < 10.0, "no script, no engine: {none:.1} MB");
    within(SERVER_IDLE, peak, with).expect("the scripts, at their peak");
}
