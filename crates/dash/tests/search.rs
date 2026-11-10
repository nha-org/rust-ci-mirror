//! Task rust-search-port through the real binary: /search finds leads, tasks, notes and Flare's hits as FindLive did, each kind
//! a test on one fixture (the older app's SQLite, a notes folder, a stand-in Flare), and a browser's /search is Rust's page in
//! the menu whose own search field targets it.
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;

mod common;
use common::{Server, serve};

/// The older app's tables, as much of them as the search reads, and a lead, its template, two tasks and two notes.
fn older_app(repo: &Path) {
    std::fs::create_dir_all(repo.join("private/dashboard")).expect("folder");
    let db =
        dash_store::Connection::open(repo.join("private/dashboard/dashboard_dev.db")).expect("db");
    db.execute_batch(
        "CREATE TABLE leads (collection TEXT, pos INTEGER, key TEXT, id TEXT, title TEXT, company TEXT, via TEXT, stage TEXT,
           location TEXT, duration TEXT, data TEXT NOT NULL);
         CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, title TEXT, stage TEXT, why TEXT, data TEXT NOT NULL);
         INSERT INTO leads VALUES ('opportunities', 0, '_template.edn', 'slug', 'Originator template', 'Originator', NULL, 'lead',
           NULL, NULL, '{}');
         INSERT INTO leads VALUES ('opportunities', 1, '2026-11-originator.edn', 'originator-data', 'Data Engineer', 'Originator',
           'LinkedIn', 'applied', 'Paris', '6 months', '{\"stack\": [\"Clojure\", \"Kafka\"]}');
         INSERT INTO leads VALUES ('opportunities', 2, '2026-11-other.edn', 'other-lead', 'Platform Engineer', 'Acme', NULL,
           'lead', NULL, NULL, '{}');
         INSERT INTO tasks VALUES ('tasks.edn', 0, 'fast-free-ci', 'Fast and free CI', 'open', 'the runners cost too much', '{}');
         INSERT INTO tasks VALUES ('tasks.edn', 1, 'other-task', 'Something else', 'done', NULL, '{}');",
    )
    .expect("the older app's rows");
    let notes = repo.join("private/notes");
    std::fs::create_dir_all(&notes).expect("notes");
    std::fs::write(
        notes.join("fast-free-ci-rfc.md"),
        "# RFC fast free CI\n\nA runner on Hetzner costs less.\n",
    )
    .expect("note");
    std::fs::write(notes.join("other.md"), "# Other\n\nNothing.\n").expect("note");
}

/// A stand-in Flare on a port of its own, written where `FLARE_DATA_DIR` says: it answers every search with one file of the
/// checkout and one meeting, and sends each request's body back to the test.
fn flare(dir: &Path, repo: &Path) -> std::sync::mpsc::Receiver<String> {
    std::fs::create_dir_all(dir).expect("flare's folder");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("its address").port();
    std::fs::write(dir.join("mcp.port"), port.to_string()).expect("its port");
    let file = repo
        .join("private/notes/fast-free-ci-rfc.md")
        .display()
        .to_string();
    let (asked, bodies) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = vec![0u8; 64 * 1024];
            let n = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..n]).into_owned();
            let _ = asked.send(
                request
                    .split_once("\r\n\r\n")
                    .map_or("", |(_, b)| b)
                    .to_string(),
            );
            let json = serde_json::json!({"result": {"results": [
                {"path": file, "name": "fast-free-ci-rfc.md", "excerpt": "A runner on Hetzner", "type": "file"},
                {"title": "Call with Originator", "snippet": "they run\nKafka", "type": "note", "source_label": "meetings"}
            ]}})
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{json}",
                json.len()
            );
        }
    });
    bodies
}

struct Fixture {
    _dir: TestDir,
    _server: Server,
    port: u16,
    flare: std::sync::mpsc::Receiver<String>,
}

fn fixture(name: &str) -> Fixture {
    let dir = TestDir::new(name);
    let (repo, data, flare_dir) = (dir.0.join("repo"), dir.0.join("data"), dir.0.join("flare"));
    std::fs::create_dir_all(&data).expect("data");
    older_app(&repo);
    let flare = flare(&flare_dir, &repo);
    let db = dash_store::open(&data).expect("the store");
    db.add_fact("file", "ingested note", "Originator's call, ingested")
        .expect("a fact");
    drop(db);
    let (server, port) = serve(&data, |c: &mut Command| {
        c.env("DASH_REPO", &repo).env("FLARE_DATA_DIR", &flare_dir)
    });
    Fixture {
        _dir: dir,
        _server: server,
        port,
        flare,
    }
}

fn get(port: u16, path: &str, accept: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: {accept}\r\n\r\n"
    )
    .expect("write");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).expect("read");
    assert!(answer.starts_with("HTTP/1.1 200"), "{path}: {answer}");
    answer.split_once("\r\n\r\n").expect("a body").1.to_string()
}

/// The page's text for an agent: its parts, each a line per result, title, line and link a tab apart.
fn text(port: u16, q: &str) -> String {
    get(port, &format!("/search?q={q}"), "*/*")
}

/// The lines of one part of the text, from its title to the next blank line.
fn part(text: &str, title: &str) -> Vec<String> {
    let at = text
        .find(&format!("\n{title}\n"))
        .unwrap_or_else(|| panic!("no part {title}: {text}"));
    let rest = &text[at + title.len() + 2..];
    rest.split("\n\n")
        .next()
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

#[test]
fn search_finds_a_lead_by_its_company_and_stack_but_never_the_template() {
    let _budget = TestBudget::new();
    let f = fixture("search-leads");
    let page = text(f.port, "originator+kafka");
    assert_eq!(
        part(&page, "Leads"),
        ["Data Engineer\tOriginator · LinkedIn · applied\t/o/originator-data"]
    );
    // accents and case aside, as FindLive
    assert_eq!(
        part(&text(f.port, "ACMÉ"), "Leads"),
        ["Platform Engineer\tAcme · lead\t/o/other-lead"]
    );
}

#[test]
fn search_finds_a_task_by_its_id_title_or_why() {
    let _budget = TestBudget::new();
    let f = fixture("search-tasks");
    let line = "Fast and free CI\tfast-free-ci · open\t/tasks/fast-free-ci";
    assert_eq!(part(&text(f.port, "fast-free-ci"), "Tasks"), [line]);
    assert_eq!(part(&text(f.port, "runners+cost"), "Tasks"), [line]);
}

#[test]
fn search_finds_a_note_by_its_heading_or_words_at_its_page() {
    let _budget = TestBudget::new();
    let f = fixture("search-notes");
    let line =
        "RFC fast free CI\tA runner on Hetzner costs less.\t/v/private/notes/fast-free-ci-rfc.md";
    assert_eq!(part(&text(f.port, "hetzner"), "Notes"), [line]);
    let page = text(f.port, "RFC%20fast%20free%20CI");
    assert_eq!(
        part(&page, "Notes"),
        [
            "RFC fast free CI\tprivate/notes/fast-free-ci-rfc.md\t/v/private/notes/fast-free-ci-rfc.md"
        ]
    );
}

#[test]
fn search_lists_flare_s_hits_a_checkout_file_linked_and_says_when_flare_is_down() {
    let _budget = TestBudget::new();
    let f = fixture("search-flare");
    let page = text(f.port, "originator");
    assert_eq!(
        part(&page, "Flare"),
        [
            "fast-free-ci-rfc.md\tA runner on Hetzner\t/v/private/notes/fast-free-ci-rfc.md",
            "Call with Originator\tthey run Kafka",
        ]
    );
    let asked: serde_json::Value =
        serde_json::from_str(&f.flare.recv().expect("Flare was asked")).expect("json");
    assert_eq!(
        asked,
        serde_json::json!({"query": "originator", "limit": 8})
    );
    // the store's own facts too, `dash ingest`'s
    assert_eq!(
        part(&page, "In the store"),
        ["ingested note\tOriginator's call, ingested"]
    );
    // one letter asks nobody
    assert_eq!(text(f.port, "o"), "Search\nSearch: o\n");

    // Flare down: the rest still answers, its part says why
    let dir = TestDir::new("search-flare-down");
    let repo = dir.0.join("repo");
    older_app(&repo);
    let (_server, port) = serve(&dir.0, |c: &mut Command| {
        c.env("DASH_REPO", &repo)
            .env("FLARE_DATA_DIR", dir.0.join("none"))
    });
    let page = text(port, "originator");
    assert_eq!(part(&page, "Leads").len(), 1, "{page}");
    let flare = part(&page, "Flare");
    assert!(
        flare[0].starts_with("Flare did not answer: Flare is not running"),
        "{flare:?}"
    );
}

/// Done-when 2: a browser's /search is Rust's page, in the menu, and the menu's own search field targets /search.
#[test]
fn a_browser_s_search_is_rust_s_page_and_the_menu_s_field_targets_it() {
    let _budget = TestBudget::new();
    let f = fixture("search-browser");
    let html = get(
        f.port,
        "/search?q=originator",
        "text/html,application/xhtml+xml",
    );
    assert!(html.contains("Served by Rust (dash serve)"), "{html}");
    // the menu's field on every other page, its markup the surface crate's test (forms.rs); /search's own holds the words
    let inbox = get(f.port, "/inbox", "text/html,application/xhtml+xml");
    assert!(
        inbox.contains(r#"id="dash-search""#),
        "the menu's field: {inbox}"
    );
    assert!(html.contains(r#"value="originator""#), "{html}");
    assert!(html.contains(r#"href="/o/originator-data""#), "{html}");
}
