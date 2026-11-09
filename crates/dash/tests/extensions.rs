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

/// What a browser sends for a page.
const BROWSER: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

fn get_as(port: u16, path: &str, accept: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: {accept}\r\n\r\n"
    )
    .expect("write");
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
    tasks.add("b-task", &["a-task".to_string()]).expect("add");
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
        // nor the live repo's list: a sync of it landing between two reads made them differ (rust-task-page-linear's run)
        .env("DASH_REPO", &dir.0)
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
        text_page(want, "/deps").contains("\nTasks: /tasks\n"),
        "/tasks is served here, so /deps names it"
    );
    // Done-when 4 of rust-nav-board: Home, what waits on him first, then his daily, then what runs.
    let page = text_page(want, "/");
    assert!(page.starts_with("Home\n\nWaiting on you\n"), "{page}");
    let at = |part: &str| {
        page.find(part)
            .unwrap_or_else(|| panic!("{part} in {page}"))
    };
    assert!(
        at("\nWaiting on you\n") < at("\nToday's daily\n")
            && at("\nToday's daily\n") < at("\nWhat runs\n"),
        "{page}"
    );
    assert!(page.contains("\na-task\tfinished\t1\t"), "{page}");
    assert!(page.contains("\thttps://github.com/o/r/pull/7\t"), "{page}");
    assert_eq!(page.matches("Next: ").count(), 1, "written once: {page}");
    assert!(page.contains("\nTasks: /tasks\n"), "{page}");
    assert!(page.contains("\nCode map: /deps\n"), "{page}");
    // Done-when 2: /tasks opens as the board, every status a column; the list and the graph are its other views.
    let tasks = text_page(want, "/tasks");
    assert!(
        tasks.starts_with("Tasks\nViews: Board /tasks (shown), List /tasks?layout=list, Graph /tasks?layout=graph\n"),
        "{tasks}"
    );
    assert!(
        tasks.contains("\nIn Review (0)\n"),
        "an empty column stands: {tasks}"
    );
    assert!(tasks.contains("\nBacklog (1)\nb-task\t"), "{tasks}");
    let list = text_page(want, "/tasks?layout=list");
    assert!(list.contains("List /tasks?layout=list (shown)"), "{list}");
    assert!(
        !list.contains("In Review (0)"),
        "the list stacks only what holds a task: {list}"
    );
    // Done-when 3 of rust-dioxus-only-app: one URL. A browser's Accept gets the Dioxus page, any other the text, ?view= overrides.
    for board in [
        get_as(want, "/tasks", BROWSER),
        get(want, "/tasks?view=html", "127.0.0.1"),
    ] {
        assert!(board.starts_with("HTTP/1.1 200"), "{board}");
        assert!(board.contains("Content-Type: text/html"), "{board}");
        assert!(board.contains("--color-accent:"), "its theme: {board}");
        assert!(board.contains(r#"class="board""#), "{board}");
        assert!(board.contains(r#"data-task="b-task""#), "{board}");
        assert!(board.contains(r#"href="/tasks/b-task""#), "{board}");
        assert!(
            board.contains(r#"href="/tasks?layout=list">List"#),
            "{board}"
        );
    }
    let listed = get_as(want, "/tasks?layout=list", BROWSER);
    assert!(
        listed.contains(r#"data-task="b-task""#) && !listed.contains(r#"class="board""#),
        "{listed}"
    );
    let asked = get_as(want, "/?view=text", BROWSER);
    assert!(asked.contains("Content-Type: text/plain"), "{asked}");
    assert!(asked.contains("\nTasks: /tasks\n"), "{asked}");
    // the window's tree is gone with its fetch (rust-window-in-process): ?view=tree is any other view now
    let tree = get(want, "/tasks?view=tree", "127.0.0.1");
    assert!(tree.contains("Content-Type: text/plain"), "{tree}");
    // a note's age may tick between two reads, so digits are left out of a compare
    let unticked = |t: &str| t.replace(|c: char| c.is_ascii_digit(), "#");
    // any other view leaves it to the Accept, as main's wants_html says for every page
    let other = get(want, "/?view=pdf", "127.0.0.1");
    assert!(other.contains("Content-Type: text/plain"), "{other}");
    // a browser's pages: a client with no Accept gets their text (rust/CLAUDE.md rule 4, rust-design-page)
    let opened = get_as(want, "/tasks/b-task", BROWSER);
    assert!(opened.starts_with("HTTP/1.1 200"), "{opened}");
    assert!(opened.contains("Last run"), "{opened}");
    // No app here to say it is none: its words are not known, and its id reads decoded.
    let unknown = get_as(want, "/tasks/no%20such", BROWSER);
    assert!(unknown.starts_with("HTTP/1.1 200"), "{unknown}");
    assert!(
        unknown.contains(r#"class="li-issue-title">no such"#),
        "{unknown}"
    );
    // the older app's link to a task opens its page here, either form of it (task rust-task-page-linear)
    for old in [
        "/projects/site_perso?issue=b-task",
        "/projects/site_perso/issue/b-task",
    ] {
        let routed = get(want, old, "127.0.0.1");
        assert!(
            routed.starts_with("HTTP/1.1 303 See Other\r\nLocation: /tasks/b-task\r\n"),
            "{routed}"
        );
    }
    let other = get(want, "/projects/klar?issue=b-task", "127.0.0.1");
    assert!(
        other.starts_with("HTTP/1.1 404"),
        "another project's link is the older app's: {other}"
    );
    let text = get(want, "/tasks/b-task", "127.0.0.1");
    assert!(
        text.contains("Content-Type: text/plain")
            && text.contains("Last run")
            && !text.contains("class="),
        "{text}"
    );
    // Done-when 3: /graph is the board's dependency view at its own URL, and both answer the text form.
    for at in ["/graph", "/tasks?layout=graph"] {
        let graph = get_as(want, at, BROWSER);
        assert!(graph.starts_with("HTTP/1.1 200"), "{at}: {graph}");
        assert!(graph.contains("text/html"), "{at}: {graph}");
        assert!(graph.contains("--color-accent:"), "its theme: {graph}");
        assert!(graph.contains(r#"class="task-graph""#), "{graph}");
        assert!(
            graph.contains(r#"data-from="a-task" data-to="b-task""#),
            "{graph}"
        );
        assert!(
            graph.contains(r#"href="/tasks?layout=graph" aria-current="page">Graph"#),
            "{graph}"
        );
        let text = get(want, at, "127.0.0.1");
        assert!(
            text.contains("Content-Type: text/plain") && !text.contains("class="),
            "{at}: {text}"
        );
        // the graph tree's own text (task rust-page-forms), never its HTML's words
        assert!(
            text.contains("\nb-task\twaiting\t") && text.contains("\twaits on a-task\n"),
            "{at}: {text}"
        );
    }
    assert_eq!(
        unticked(&text_page(want, "/graph")),
        unticked(&text_page(want, "/tasks?layout=graph"))
    );
    // ... and /deps, the code map: its text the page as built, its HTML that text with the menu.
    let map = get_as(want, "/deps", BROWSER);
    assert!(
        map.contains("Content-Type: text/html") && map.contains("Rust dependencies"),
        "{map}"
    );
    // Done-when 1: every page carries the one menu, its foot's toggle switching human and agent by the URL.
    for at in [
        "/",
        "/tasks",
        "/tasks?layout=list",
        "/graph",
        "/deps",
        "/design",
        "/tasks/b-task",
    ] {
        let human = get_as(want, at, BROWSER);
        assert_eq!(
            human.matches(r#"id="dash-menu""#).count(),
            1,
            "{at}: {human}"
        );
        for entry in ["Home", "Tasks", "Mail", "Tools", "Design", "Code"] {
            assert!(human.contains(&format!(">{entry}<")), "{at}: {entry}");
        }
        let agent = dash_surface::forms::href(
            at.split('?').next().unwrap_or(at),
            at.split_once('?').map_or("", |q| q.1),
            "text",
        );
        assert!(
            human.contains(&format!(
                r#"href="{}" target="_top">Agent<"#,
                agent.replace('&', "&amp;")
            )),
            "{at}: {human}"
        );
        let read = get_as(want, &agent, BROWSER);
        assert!(
            read.contains("Content-Type: text/plain") && !read.contains(r#"id="dash-menu""#),
            "{agent}: {read}"
        );
    }
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
    for host in ["evil.example", "100.72.158.115", "100.72.158.115:7788", ""] {
        for path in ["/", "/search?q=x"] {
            assert!(
                get(want, path, host).starts_with("HTTP/1.1 403"),
                "{host} {path}"
            );
        }
    }
}

#[test]
fn mcp_answers_a_json_rpc_error_to_a_line_past_its_limit() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-mcp-long");
    let mut child = Command::new(DASH)
        .args(["mcp", "--data"])
        .arg(&dir.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("boot");
    let limit = dash_surface::LINE_LIMIT as usize;
    // At the limit, in the costliest shape measured, each a request that goes on past the parse, under the cap.
    let item = format!("{}0{},", r#"{"":"#.repeat(120), "}".repeat(120));
    let at_it = |before: &str, after: &str| {
        let items = item.repeat((limit - before.len() - after.len()) / item.len());
        let line = format!("{before}{}{after}", items.trim_end_matches(','));
        assert!(line.len() <= limit && line.len() + 2 * item.len() > limit);
        line
    };
    let nested_id = at_it(r#"{"jsonrpc":"2.0","method":"ping","id":["#, "]}");
    let nested_arguments = at_it(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"none","arguments":["#,
        "]}}",
    );
    let past_it = "x".repeat(limit + 1);
    let mut stdin = child.stdin.take().expect("stdin");
    writeln!(stdin, "{nested_id}\n{nested_arguments}\n{past_it}").expect("write");
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"ping"}}"#).expect("write");
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("it ends at its input's end");
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).expect("utf-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines[0].contains("an id is a string"), "{text}");
    assert!(lines[1].contains(r#""id":1"#), "{text}");
    assert!(lines[1].contains("no tool none"), "{text}");
    for says in [r#""id":null"#, "-32600", "a line past"] {
        assert!(lines[2].contains(says), "{says}: {text}");
    }
    assert!(lines[3].contains(r#""id":2"#), "still served: {text}");
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

/// Done-when 2 of rust-dioxus-only-app: behind the front door, what this server does not serve reaches the older app whole (its
/// method, its Host, its body), and the older app's own paths go there even though this server has one of the same name.
#[test]
fn the_front_door_hands_the_older_app_what_it_does_not_serve() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-front");
    // The older app: each request's bytes as it got them, sent back as its answer's body.
    let older = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let older_port = older.local_addr().expect("address").port();
    std::thread::spawn(move || {
        for stream in older.incoming() {
            let mut stream = stream.expect("a connection");
            // what arrives within a moment: a head and its body come in two writes
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(200)))
                .expect("timeout");
            let mut body = vec![];
            let _ = stream.read_to_end(&mut body);
            // one path is a page, as the older app's are: a whole document, the request its words
            let page = body.starts_with(b"GET /older-page");
            if page {
                let said = String::from_utf8_lossy(&body).into_owned();
                let tree = serde_json::json!({"root": "p", "elements": {"p": {"type": "text", "props": {"text": said}}}});
                body = dash_surface::screen::page("Older", &tree.to_string())
                    .expect("a page")
                    .into_bytes();
            }
            let kind = if page {
                "Content-Type: text/html; charset=utf-8\r\n"
            } else {
                ""
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\n{kind}Content-Length: {}\r\n\r\n",
                body.len()
            )
            .expect("head");
            stream.write_all(&body).expect("body");
        }
    });
    let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let want = free.local_addr().expect("address").port();
    drop(free);
    let mut child = Command::new(DASH)
        .args(["serve", "--data"])
        .arg(&dir.0)
        .args([
            "--port",
            &want.to_string(),
            "--older",
            &older_port.to_string(),
        ])
        .env("DASH_CLI", "/nonexistent/dash")
        .env("DASH_WORKTREES", &dir.0)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("port line");
    let _server = Server(child);

    let mut stream = TcpStream::connect(("127.0.0.1", want)).expect("connect");
    write!(stream, "POST /comment HTTP/1.1\r\nHost: 127.0.0.1:7788\r\nConnection: keep-alive\r\nContent-Length: 5\r\n\r\nhello").expect("write");
    let mut posted = String::new();
    stream.read_to_string(&mut posted).expect("read");
    assert!(posted.starts_with("HTTP/1.1 200"), "{posted}");
    assert!(
        posted.contains("POST /comment HTTP/1.1\r\nHost: 127.0.0.1:7788\r\n"),
        "{posted}"
    );
    assert!(
        posted.contains("Connection: close\r\n\r\nhello"),
        "its next request is ours again: {posted}"
    );
    assert!(!posted.contains("keep-alive"), "{posted}");
    for (path, host) in [
        ("/search?q=x", "127.0.0.1"),
        ("/ping", "localhost"),
        ("/agents?app=older", "localhost"),
        ("/projects/site_perso", "127.0.0.1"),
        ("/", "mac.tail.ts.net"),
    ] {
        let answer = get(want, path, host);
        assert!(
            answer.contains(&format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n")),
            "{path} {host}: {answer}"
        );
    }
    let ours = get_as(want, "/", BROWSER);
    assert!(ours.contains(r#"id="title">Home"#), "{ours}");
    // The older app's pages carry the menu too (rust-nav-board; the orchestrator's answer, 2026-11-08: « Older pages too »):
    // asked as a browser asks, with no view and no compression, the menu first in what comes back.
    let mut stream = TcpStream::connect(("127.0.0.1", want)).expect("connect");
    write!(stream, "GET /older-page?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: {BROWSER}\r\nAccept-Encoding: gzip, br\r\n\r\n").expect("write");
    let mut page = String::new();
    stream.read_to_string(&mut page).expect("read");
    assert!(
        page.starts_with("HTTP/1.1 200 OK\r\n") && page.contains("Content-Type: text/html"),
        "{page}"
    );
    assert_eq!(page.matches(r#"id="dash-menu""#).count(), 1, "{page}");
    assert!(
        page.contains(r#"href="/older-page?x=1&amp;view=text" target="_top">Agent<"#),
        "{page}"
    );
    assert!(
        page.contains("GET /older-page?x=1 HTTP/1.1\r\n") && page.contains("Accept: text/html\r\n"),
        "{page}"
    );
    assert!(!page.contains("gzip"), "asked uncompressed: {page}");
    let (head, body) = page.split_once("\r\n\r\n").expect("a body");
    assert!(
        head.contains(&format!("Content-Length: {}\r\n", body.len())),
        "{head}"
    );
    // a frame inside an older page (its previews) stays bare; the page's own view= values reach it
    let mut stream = TcpStream::connect(("127.0.0.1", want)).expect("connect");
    write!(stream, "GET /older-page?view=xml HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: {BROWSER}\r\nSec-Fetch-Dest: iframe\r\n\r\n").expect("write");
    let mut framed = String::new();
    stream.read_to_string(&mut framed).expect("read");
    assert!(
        framed.contains("GET /older-page?view=xml HTTP/1.1")
            && !framed.contains(r#"id="dash-menu""#),
        "{framed}"
    );
    let words = get(want, "/older-page?view=text", "127.0.0.1");
    assert!(
        words.contains("Content-Type: text/plain") && !words.contains(r#"id="dash-menu""#),
        "{words}"
    );
    assert!(
        words.contains("GET /older-page HTTP/1.1 Host: 127.0.0.1"),
        "its view stays here: {words}"
    );
    let both = get(want, "/older-page?view=both", "127.0.0.1");
    assert!(
        both.contains(r#"src="/older-page?view=html""#)
            && both.contains(r#"src="/older-page?view=text""#),
        "{both}"
    );
    // His words name /tasks the board (rust-nav-board): it is ours now, the older app's projects keep their own boards.
    let board = get_as(want, "/tasks", BROWSER);
    assert!(
        board.contains(r#"class="board""#) && !board.contains("GET /tasks HTTP/1.1"),
        "{board}"
    );
}

/// Done-when 3 of rust-window-in-process: `dash serve` serves the web's pages, sidebar and style, with no window running, from
/// a dash built with no window at all (Linux CI's, this test's by default); the window's own start with no server is
/// main.rs' the_window_draws_the_web_s_page_in_its_own_process.
#[test]
fn dash_serve_serves_the_pages_with_no_window_running_or_built() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("serve-no-window");
    if !cfg!(feature = "window") {
        let ui = Command::new(DASH)
            .args(["ui", "--data"])
            .arg(&dir.0)
            .env("DASH_CLI", "/nonexistent/dash")
            .output()
            .expect("dash ui");
        let said = String::from_utf8_lossy(&ui.stderr);
        assert_eq!(ui.status.code(), Some(2), "{said}");
        assert!(said.contains("built without its window"), "{said}");
    }
    // port 0: the port it bound is read back, none freed for another test to take meanwhile
    let mut child = Command::new(DASH)
        .args(["serve", "--data"])
        .arg(&dir.0)
        .args(["--port", "0"])
        .env("DASH_CLI", "/nonexistent/dash")
        .env("DASH_WORKTREES", &dir.0)
        .env("DASH_REPO", &dir.0)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("port line");
    let _server = Server(child);
    let port: u16 = (line.trim().strip_prefix("port ").expect("port <n>"))
        .parse()
        .expect("a port");
    for page in ["/tasks", "/tasks?layout=list", "/design"] {
        let answer = get_as(port, page, BROWSER);
        assert!(answer.starts_with("HTTP/1.1 200"), "{page}: {answer}");
        assert!(answer.contains(r#"id="dash-nav""#), "{page}: the sidebar");
        assert!(
            answer.contains(dash_surface::components::CSS),
            "{page}: the design system's style"
        );
    }
}
