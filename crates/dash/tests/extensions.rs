//! Section 11 and 12 through the real binary: a compiled-in crate and an MCP pod each add an extractor through flat dispatch,
//! the pod by a file in <data>/extensions.d and with the binary byte for byte the same.
use dash_core::Store;
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use dash_store as store;
use std::hash::Hasher;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

mod common;
use common::{Server, serve};

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
         extractor text/plain crate dash-ingest\n\
         slot today/10-drafts drafts\n\
         slot today/20-mail mail\n"
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

/// Task notion-page-menu-export: Export PDF's link answers the Notion document printed to PDF by headless Chrome, a file to
/// save, whose text is the document's lines alone (poppler's pdftotext reads it; none fails the test, never skips it).
#[test]
fn the_notion_document_downloads_as_its_printed_pdf() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-notion-pdf");
    let (_server, port) = serve(&dir.0, |c| c);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(
        stream,
        // a query after it is the same file (/code-review)
        "GET {}?v=2 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
        dash_surface::design::DOCUMENT_PDF
    )
    .expect("ask");
    let mut answer = vec![];
    stream.read_to_end(&mut answer).expect("read");
    let at = answer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a head")
        + 4;
    let head = String::from_utf8_lossy(&answer[..at]);
    assert!(
        head.starts_with("HTTP/1.1 200")
            && head.contains("Content-Type: application/pdf")
            && head.contains("attachment; filename=\"rendering-plan.pdf\""),
        "{head}{}",
        String::from_utf8_lossy(&answer[at..])
    );
    let pdf = dir.0.join("rendering-plan.pdf");
    std::fs::write(&pdf, &answer[at..]).expect("the file");
    let text = std::process::Command::new("pdftotext")
        .arg(&pdf)
        .arg("-")
        .output()
        .expect("pdftotext: install poppler");
    let lines: Vec<String> = String::from_utf8_lossy(&text.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "\u{c}")
        .map(String::from)
        .collect();
    assert_eq!(
        lines,
        [
            "Rendering plan",
            "2026:",
            "One tree, two renderers",
            "the window and the web draw the same components",
            "theme.css generated from design.json, shared by both",
            "each behaviour is checked against the live document",
            "Memory",
            "Frame time",
            "Fonts",
            "one face a window",
            "matches on family name, then falls back",
            "Selection",
            "Open questions",
            "Does a slow font load hold the first frame?",
            "One theme file for print too?",
        ]
    );
}

/// Task dash-settings-backup, the download: /settings lists what a backup holds, a store's tables and rows, and its zip
/// downloads as a file that counts back to the same rows.
#[test]
fn settings_downloads_every_store_as_sql_in_one_zip() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-settings");
    let (_server, port) = serve(&dir.0, |c| c);
    let page = text_page(port, "/settings");
    assert!(
        page.contains("Download the backup (zip) /settings/backup.zip"),
        "{page}"
    );
    assert!(page.contains("Store\tTable\tRows"), "{page}");
    // Done-when 1 of settings-upload-styled: the served /settings draws Read a backup as the File upload's drop zone, every
    // file field of it a zone's (the comment box's too). Its attributes alone, no tag: the zone's markup is dash_surface's
    // to test (screen.rs's an_upload_is_the_file_upload_its_submit_inside, notion_browser.rs in Chrome).
    assert!(
        page.contains("Read a backup: POST /settings/restore"),
        "{page}"
    );
    let html = dash_surface::markup(&get_as(port, "/settings", BROWSER));
    let zone = r#"class="dropzone-input" type="file" name="file" accept=".zip" required=true"#;
    assert!(
        html.contains(zone) && html.contains("Drop a .zip file here, or choose one"),
        "{html}"
    );
    let fields = html.matches(r#"type="file""#).count();
    assert_eq!(
        html.matches(r#"class="dropzone-input" type="file""#)
            .count(),
        fields,
        "{html}"
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(
        stream,
        "GET /settings/backup.zip HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    )
    .expect("ask");
    let mut answer = vec![];
    stream.read_to_end(&mut answer).expect("read");
    let at = answer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a head")
        + 4;
    let head = String::from_utf8_lossy(&answer[..at]);
    assert!(
        head.contains("Content-Type: application/zip")
            && head.contains("attachment; filename=\"dash-backup-"),
        "{head}"
    );
    let counts = store::backup::counts(&answer[at..]).expect("a zip");
    assert!(!counts.is_empty(), "the served folder's stores");
    for (store, table, n) in counts {
        assert!(
            page.contains(&format!("{store}\t{table}\t{n}")),
            "{store} {table} {n} in {page}"
        );
    }
    // Done-when 1 of rust-table-sortable: Rows' head link (?sort=Rows, then ?sort=Rows&dir=desc, its HTML proven in
    // dash_surface's screen tests) sorts by it, again reverses it, the text form ordered and saying so
    let rows = |page: &str| -> Vec<u64> {
        let table = page
            .lines()
            .skip_while(|l| !l.starts_with("Store\tTable\tRows"));
        let cells = table
            .skip(1)
            .map_while(|l| l.split('\t').nth(2).map(str::to_string));
        cells.map(|n| n.parse().expect("a count")).collect()
    };
    let up = text_page(port, "/settings?sort=Rows");
    let down = text_page(port, "/settings?sort=Rows&dir=desc");
    assert!(up.contains("; sorted by Rows, ascending."), "{up}");
    assert!(down.contains("; sorted by Rows, descending."), "{down}");
    let (up, mut down) = (rows(&up), rows(&down));
    assert!(up.is_sorted() && !up.is_empty(), "{up:?}");
    down.reverse();
    assert_eq!(up.len(), down.len());
    assert!(down.is_sorted(), "{down:?}");
}

/// Task dash-settings-backup, the reimport: an uploaded backup shows each table's rows in it and now, and is put back only
/// on the confirm; a post from another page is refused.
#[test]
fn a_backup_reads_back_with_its_counts_and_is_put_back_only_on_confirm() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-restore");
    let (_server, port) = serve(&dir.0, |c| c);
    let _ = text_page(port, "/settings");
    let raw = |request: Vec<u8>| {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.write_all(&request).expect("ask");
        let mut answer = vec![];
        stream.read_to_end(&mut answer).expect("read");
        answer
    };
    let zip = raw(
        format!("GET /settings/backup.zip HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").into_bytes(),
    );
    let zip = &zip[zip
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a head")
        + 4..];
    let post = |path: &str, kind: &str, body: &[u8], origin: bool| {
        let origin = if origin {
            format!("Origin: http://127.0.0.1:{port}\r\n")
        } else {
            String::new()
        };
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{origin}Accept: text/plain\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        String::from_utf8_lossy(&raw([head.as_bytes(), body].concat())).into_owned()
    };
    let mut form =
        b"--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.zip\"\r\n\r\n"
            .to_vec();
    form.extend_from_slice(zip);
    form.extend_from_slice(b"\r\n--b--\r\n");
    let kind = "multipart/form-data; boundary=b";
    assert!(
        post("/settings/restore", kind, &form, false).contains("403 Forbidden"),
        "another page's post"
    );
    let read = post("/settings/restore", kind, &form, true);
    assert!(
        read.contains("In the backup") && read.contains("Replace the stores with this backup"),
        "{read}"
    );
    assert!(
        dir.0.join("backup-to-restore.zip").exists(),
        "kept aside until he confirms"
    );
    let done = post(
        "/settings/restore/confirm",
        "application/x-www-form-urlencoded",
        b"",
        true,
    );
    assert!(
        done.contains("The backup is put back.") && done.contains("tables replaced"),
        "{done}"
    );
    assert!(
        !dir.0.join("backup-to-restore.zip").exists(),
        "nothing left waiting"
    );
}

/// /deps draws the code map in the served data folder, the one `dash code map` writes there; none yet says how to make it.
#[test]
fn deps_serves_the_code_map_of_its_data_folder() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-deps");
    let (_server, port) = serve(&dir.0, |c| c);
    assert!(text_page(port, "/deps").starts_with("No code map in this data folder yet"));
    let map = r#"{"sha": "abc", "at": "2026-11-09T20:00:00Z", "lines": 3, "tokens": 20, "target": 360768, "crates": [{"name": "core",
        "layer": "Core", "what": "The core.", "code": 2, "tests": 1, "uses": [], "modules": []}]}"#;
    std::fs::write(dir.0.join("codemap.json"), map).expect("write");
    let page = text_page(port, "/deps");
    assert!(
        page.starts_with("20 tokens of Rust against the 360,768 token target"),
        "{page}"
    );
    assert!(
        page.contains("Core model\tcore\t2\t1\t\t/deps?layer=Core"),
        "{page}"
    );
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

    // no older app, no Flare: the store's own part alone (task rust-search-port moved /search to its page)
    let (_server, port) = serve(&dir.0, |c| {
        c.env("DASH_REPO", dir.0.join("none"))
            .env("FLARE_DATA_DIR", dir.0.join("none"))
    });
    let answer = body(port, "/search?q=rust");
    let part = answer.split("\nIn the store\n").nth(1).expect("its part");
    let titles: Vec<&str> = part
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split('\t').next())
        .collect();
    assert_eq!(titles, ["first", "a RUST title"]);
    assert!(get(port, "/ui/search", "127.0.0.1").starts_with("HTTP/1.1 404"));
    assert!(
        get(port, "/search?q=rust", "evil.example").starts_with("HTTP/1.1 403"),
        "a rebinding page"
    );
}

/// Done-when 1 of rust-inbox-search-place: /inbox draws the design system's search field in one place, under its title and
/// before its parts; it queries /search, which draws its results on /inbox, the field in the same place holding the words.
/// Read from the page's tree (`?view=data`) and its text, as the gate asks of a dash test.
#[test]
fn the_inbox_s_search_field_queries_search_and_its_results_land_on_the_inbox() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-inbox-search");
    let db = store::open(&dir.0).expect("open");
    db.add_fact("file", "first", "rust in the body")
        .expect("add");
    db.add_fact("file", "second", "nothing").expect("add");
    drop(db);
    let (_server, port) = serve(&dir.0, |c| {
        c.env("DASH_REPO", dir.0.join("none"))
            .env("FLARE_DATA_DIR", dir.0.join("none"))
    });
    let tree = |path: &str| -> serde_json::Value {
        serde_json::from_str(&body(port, path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    };
    let order = |t: &serde_json::Value| -> Vec<String> {
        let page = &t["elements"][t["root"].as_str().expect("a root")];
        let kids = page["children"].as_array().expect("children");
        kids.iter()
            .map(|c| c.as_str().expect("an id").to_string())
            .collect()
    };

    let inbox = tree("/inbox?view=data");
    assert_eq!(order(&inbox), ["title", "search", "waits", "daily"]);
    let field = &inbox["elements"]["search"];
    assert_eq!(field["type"], "search-bar");
    assert_eq!(
        field["props"]["hidden"],
        serde_json::json!([["in", "inbox"]])
    );

    let found = tree("/search?in=inbox&q=rust&view=data");
    assert_eq!(
        order(&found),
        ["title", "search", "found", "waits", "daily"]
    );
    assert_eq!(
        found["elements"]["search"]["props"]["value"], "rust",
        "the field keeps the words"
    );
    assert_eq!(found["elements"]["found"]["props"]["title"], "Results");
    assert_eq!(
        found["elements"]["found-list"]["children"],
        serde_json::json!(["found-0"])
    );
    let card = &found["elements"]["found-0"]["props"];
    assert_eq!(
        (&card["title"], &card["line"]),
        (
            &serde_json::json!("first"),
            &serde_json::json!("rust in the body")
        )
    );

    let none = tree("/search?in=inbox&q=zzz&view=data");
    assert_eq!(
        none["elements"]["found-none"]["props"]["text"],
        "Nothing found for « zzz »."
    );
    // no words: the plain inbox, its field in the same place
    assert_eq!(
        order(&tree("/search?in=inbox&q=&view=data")),
        ["title", "search", "waits", "daily"]
    );
    // the same page for an agent, its text drawn from the same tree (rust/CLAUDE.md rule 4), the words searched in it
    let text = text_page(port, "/search?in=inbox&q=rust");
    let at = |what: &str| {
        text.find(what)
            .unwrap_or_else(|| panic!("{what} in {text}"))
    };
    assert!(
        at("Search: rust") < at("Results") && at("Results") < at("first"),
        "{text}"
    );
    // any other /search is the search page (task rust-search-port)
    assert!(body(port, "/search?q=zzz").starts_with("Search\nSearch: zzz\n"));
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
    let (_server, port) = serve(&dir.0, |c| {
        c.args(["--port", &want.to_string()])
            // Nothing reads an app here: the queue's thread fails fast, and the page says so.
            .env("DASH_CLI", "/nonexistent/dash")
            .env("DASH_WORKTREES", &dir.0)
            // nor the live repo's list: a sync of it landing between two reads made them differ (rust-task-page-linear's run)
            .env("DASH_REPO", &dir.0)
    });
    assert_eq!(port, want);

    assert!(
        text_page(want, "/deps").contains("code map"),
        "/deps is served here"
    );
    // Done-when 1 of rust-home-prompt: Home is the compass's prompt and three suggestions, nothing else but the menu every
    // page's text ends with (task rust-menu-text).
    let home = text_page(want, "/");
    let home = home
        .strip_suffix(&format!("\n{}", dash_surface::forms::menu_text()))
        .unwrap_or_else(|| panic!("the menu last: {home}"));
    assert!(
        home.starts_with("\nAsk: POST /compass, his words as text\n") && home.lines().count() == 5,
        "{home}"
    );
    // Done-when 2 and 3 of rust-home-prompt: what Home showed is /inbox's and /tasks', beside /plan and /insights.
    let inbox = text_page(want, "/inbox");
    let at = |part: &str| {
        inbox
            .find(part)
            .unwrap_or_else(|| panic!("{part} in {inbox}"))
    };
    assert!(
        inbox.starts_with("Inbox\n\nWaiting on you\n")
            && at("\nWaiting on you\n") < at("\nToday's daily\n"),
        "{inbox}"
    );
    assert!(text_page(want, "/plan").starts_with("Plan\n\nThe roadmap\n"));
    assert!(
        text_page(want, "/insights").starts_with("Insights\n\nHis feedback and the work done\n")
    );
    let page = text_page(want, "/tasks");
    assert!(page.contains("\nWhat runs\n"), "{page}");
    assert!(page.contains("\na-task\tfinished\t1\t"), "{page}");
    assert!(page.contains("\thttps://github.com/o/r/pull/7\t"), "{page}");
    assert_eq!(page.matches("Next: ").count(), 1, "written once: {page}");
    assert!(page.contains("\nTasks: /tasks\n"), "{page}");
    assert!(page.contains("\nCode map: /deps\n"), "{page}");
    // Done-when 2: /tasks opens as the board, every status a column; the list and the graph are its other views.
    let tasks = text_page(want, "/tasks");
    assert!(
        tasks.starts_with("Tasks\nNew issue: POST /tasks/new title, description\nViews: Board /tasks (shown), List /tasks?layout=list, Graph /tasks?layout=graph\n"),
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
        let board = dash_surface::markup(&board);
        assert!(board.contains(r#"class="board""#), "{board}");
        assert!(board.contains(r#"data-task="b-task""#), "{board}");
        assert!(board.contains(r#"href="/tasks/b-task""#), "{board}");
        assert!(
            // the views' toggle: each option its icon then its name (task rust-tasks-views-toggle)
            board.contains(r#"class="toggle""#) && board.contains(r#"href="/tasks?layout=list""#),
            "{board}"
        );
    }
    let listed = dash_surface::markup(&get_as(want, "/tasks?layout=list", BROWSER));
    assert!(
        listed.contains(r#"data-task="b-task""#) && !listed.contains(r#"class="board""#),
        "{listed}"
    );
    let asked = get_as(want, "/?view=text", BROWSER);
    assert!(asked.contains("Content-Type: text/plain"), "{asked}");
    assert!(asked.contains("\nAsk: POST /compass"), "{asked}");
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
    let unknown = dash_surface::markup(&get_as(want, "/tasks/no%20such", BROWSER));
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
        let graph = dash_surface::markup(&graph);
        assert!(graph.contains(r#"class="task-graph""#), "{graph}");
        // a-task merged: its wait is met, so the graph leaves it out (task rust-graph-queue-edges)
        assert!(
            graph.contains(r#"data-task="b-task""#) && !graph.contains(r#"data-task="a-task""#),
            "{graph}"
        );
        assert!(
            graph.contains(r#"href="/tasks?layout=graph" aria-current="page""#),
            "{graph}"
        );
        let text = get(want, at, "127.0.0.1");
        assert!(
            text.contains("Content-Type: text/plain") && !text.contains("class="),
            "{at}: {text}"
        );
        // the graph tree's own text (task rust-page-forms), never its HTML's words
        assert!(
            text.contains("\nb-task\twaiting\tnot known") && !text.contains("waits on a-task"),
            "{at}: {text}"
        );
    }
    assert_eq!(
        unticked(&text_page(want, "/graph")),
        unticked(&text_page(want, "/tasks?layout=graph"))
    );
    // ... and /deps, the code map: its HTML the same words with the menu.
    let map = get_as(want, "/deps", BROWSER);
    assert!(
        map.contains("Content-Type: text/html")
            && map.contains("No code map in this data folder yet"),
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
        let human = dash_surface::markup(&get_as(want, at, BROWSER));
        assert_eq!(
            human.matches(r#"id="dash-menu""#).count(),
            1,
            "{at}: {human}"
        );
        for entry in [
            "Inbox", "Tasks", "Plan", "Insights", "Mail", "Tools", "Design", "Code",
        ] {
            assert!(human.contains(&format!(">{entry}<")), "{at}: {entry}");
        }
        let agent = dash_surface::forms::href(
            at.split('?').next().unwrap_or(at),
            at.split_once('?').map_or("", |q| q.1),
            "text",
        );
        // the menu draws no view toggle (task rust-menu-no-view-toggle); the text view stays one URL away
        assert!(!human.contains("view-toggle"), "{at}: {human}");
        let read = get_as(want, &agent, BROWSER);
        assert!(
            read.contains("Content-Type: text/plain") && !read.contains(r#"id="dash-menu""#),
            "{agent}: {read}"
        );
    }
    assert!(
        page.ends_with(&format!(
            "\nBuilt from {}.\n\n{}",
            option_env!("DASH_COMMIT").unwrap_or("a local build"),
            dash_surface::forms::menu_text()
        )),
        "/tasks ends with it: {page}"
    );
    // make rust/serves and forge's deploy read the line alone (task rust-home-prompt)
    assert_eq!(
        text_page(want, "/built"),
        format!(
            "Built from {}.\n",
            option_env!("DASH_COMMIT").unwrap_or("a local build")
        )
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
    let (_server, bound) = serve(&dir.0, |c| {
        c.args(["--port", "0", "--older", &older_port.to_string()])
            .env("DASH_CLI", "/nonexistent/dash")
            .env("DASH_WORKTREES", &dir.0)
    });

    let mut stream = TcpStream::connect(("127.0.0.1", bound)).expect("connect");
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
    // done-when 2 of rust-search-port: the front door keeps a browser's /search, FindLive's path, the older app never asked
    let search = get_as(bound, "/search?q=x", BROWSER);
    assert!(
        search.starts_with("HTTP/1.1 200") && search.contains("Served by Rust (dash serve)"),
        "{search}"
    );
    assert!(!search.contains("GET /search"), "handed over: {search}");
    for (path, host) in [
        ("/ping", "localhost"),
        ("/agents?app=older", "localhost"),
        ("/projects/site_perso", "127.0.0.1"),
        ("/", "mac.tail.ts.net"),
    ] {
        let answer = get(bound, path, host);
        assert!(
            answer.contains(&format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n")),
            "{path} {host}: {answer}"
        );
    }
    let ours = dash_surface::markup(&get_as(bound, "/", BROWSER));
    assert!(
        ours.contains(r#"class="form" id="ask" method="post" action="/compass""#),
        "{ours}"
    );
    // The older app's pages carry the menu too (rust-nav-board; the orchestrator's answer, 2026-11-08: « Older pages too »):
    // asked as a browser asks, with no view and no compression, the menu first in what comes back.
    let mut stream = TcpStream::connect(("127.0.0.1", bound)).expect("connect");
    write!(stream, "GET /older-page?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: {BROWSER}\r\nAccept-Encoding: gzip, br\r\n\r\n").expect("write");
    let mut page = String::new();
    stream.read_to_string(&mut page).expect("read");
    assert!(
        page.starts_with("HTTP/1.1 200 OK\r\n") && page.contains("Content-Type: text/html"),
        "{page}"
    );
    let shown = dash_surface::markup(&page);
    assert_eq!(shown.matches(r#"id="dash-menu""#).count(), 1, "{page}");
    assert!(!shown.contains("view-toggle"), "{page}");
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
    let mut stream = TcpStream::connect(("127.0.0.1", bound)).expect("connect");
    write!(stream, "GET /older-page?view=xml HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: {BROWSER}\r\nSec-Fetch-Dest: iframe\r\n\r\n").expect("write");
    let mut framed = String::new();
    stream.read_to_string(&mut framed).expect("read");
    assert!(
        framed.contains("GET /older-page?view=xml HTTP/1.1")
            && !dash_surface::markup(&framed).contains(r#"id="dash-menu""#),
        "{framed}"
    );
    let words = get(bound, "/older-page?view=text", "127.0.0.1");
    assert!(
        words.contains("Content-Type: text/plain") && !words.contains(r#"id="dash-menu""#),
        "{words}"
    );
    assert!(
        words.contains("GET /older-page HTTP/1.1 Host: 127.0.0.1"),
        "its view stays here: {words}"
    );
    // an older page has no tree: its data view is its words, the view ours and never passed on
    let data = get(bound, "/older-page?view=data", "127.0.0.1");
    assert!(
        data.contains("Content-Type: text/plain") && data.contains("GET /older-page HTTP/1.1"),
        "{data}"
    );
    // His words name /tasks the board (rust-nav-board): it is ours now, the older app's projects keep their own boards.
    let board = dash_surface::markup(&get_as(bound, "/tasks", BROWSER));
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
    let (_server, port) = serve(&dir.0, |c| {
        c.args(["--port", "0"])
            .env("DASH_CLI", "/nonexistent/dash")
            .env("DASH_WORKTREES", &dir.0)
            .env("DASH_REPO", &dir.0)
    });
    for page in ["/tasks", "/tasks?layout=list", "/design"] {
        let answer = get_as(port, page, BROWSER);
        assert!(answer.starts_with("HTTP/1.1 200"), "{page}: {answer}");
        assert!(
            dash_surface::markup(&answer).contains(r#"id="dash-nav""#),
            "{page}: the sidebar"
        );
        assert!(
            answer.contains(dash_surface::component::css()),
            "{page}: the design system's style"
        );
    }
}

/// `dash ui --check --stop-front-door` boots nothing out it cannot load back: run where launchctl bootstrap fails (a sandbox, a
/// task's or CI's, or no plist), it refuses before stopping anything, :7788 as it was and no request for the host written
/// (the orchestrator's run of 2026-11-09 booted rust-serve out from its sandbox and left :7788 dark).
#[test]
fn stop_front_door_refuses_before_stopping_what_it_cannot_load_back() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("front-door");
    // :7788 itself is not compared before and after: launchd restarts the host's front door at any moment (KeepAlive); « so
    // nothing was stopped » is said only on the return before any launchctl call
    let check = Command::new(DASH)
        .args([
            "ui",
            "--check",
            "/nonexistent/window.png",
            "--stop-front-door",
        ])
        .env("HOME", &dir.0)
        .env("DASH_REPO", &dir.0)
        .output()
        .expect("dash ui --check --stop-front-door");
    let said = String::from_utf8_lossy(&check.stderr);
    assert_eq!(check.status.code(), Some(1), "{said}");
    assert!(said.contains("so nothing was stopped"), "{said}");
    assert!(!dir.0.join(".claude/run/launch.request").exists(), "{said}");
}

/// `dash ui --check` proves done-when 1 only with nothing on :7788: while something listens there it refuses, before any
/// window opens. On Linux CI the test makes something listen; on his Mac it never takes the port (launchd's front door could
/// be between restarts) and runs only while the front door holds it.
#[test]
fn ui_check_refuses_while_something_listens_on_7788() {
    let _budget = TestBudget::new();
    let _held = cfg!(target_os = "linux")
        .then(|| std::net::TcpListener::bind(("127.0.0.1", 7788)).ok())
        .flatten();
    if TcpStream::connect(("127.0.0.1", 7788)).is_err() {
        eprintln!("nothing listens on :7788 here: a run would open a window, so this one stops");
        return;
    }
    let check = Command::new(DASH)
        .args(["ui", "--check", "/nonexistent/window.png"])
        .output()
        .expect("dash ui --check");
    let said = String::from_utf8_lossy(&check.stderr);
    assert_eq!(check.status.code(), Some(1), "{said}");
    assert!(said.contains("listens on :7788: stop it first"), "{said}");
}

/// Done-when 3 of rust-daily-today: `dash outbox` is answered by the front door from the older app's export, as the older CLI
/// answered (x-exit, x-stdout-bytes); any other command, or one not from bin/dash on loopback, goes on whole to the older app.
/// Asked with curl as bin/dash asks: a body past 1 KB, so curl waits on « 100 Continue » first.
#[test]
fn the_front_door_answers_dash_outbox_and_hands_on_every_other_command() {
    use std::os::unix::fs::PermissionsExt;
    let _budget = TestBudget::new();
    let dir = TestDir::new("front-outbox");
    let export = serde_json::json!([
        {"id": "go-mail", "ready": true, "by-claude": true, "channel": "email", "approved-on": "2026-11-01",
         "to": "Ana, a@x.io", "banned": []},
        {"id": "sent-one", "ready": false, "status": "sent"},
    ]);
    std::fs::write(dir.0.join("export.json"), export.to_string()).expect("export");
    let cli = dir.0.join("older-dash");
    let script = format!(
        "#!/bin/sh\n[ \"$1 $2\" = 'model drafts' ] && exec cat '{}/export.json'\nexit 9\n",
        dir.0.display()
    );
    std::fs::write(&cli, script).expect("the stand-in");
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let older = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let older_port = older.local_addr().expect("address").port();
    std::thread::spawn(move || {
        for stream in older.incoming() {
            let mut stream = stream.expect("a connection");
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(300)))
                .expect("timeout");
            let mut body = vec![];
            let _ = stream.read_to_end(&mut body);
            let head = format!(
                "HTTP/1.1 200 OK\r\nx-exit: 0\r\nx-stdout-bytes: {0}\r\nContent-Length: {0}\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    let (_server, bound) = serve(&dir.0, |c| {
        c.args(["--port", "0", "--older", &older_port.to_string()])
            .env("DASH_CLI", &cli)
            .env("DASH_WORKTREES", &dir.0)
    });
    let env = format!("env[]=BIG={}", "x".repeat(2048));
    let asked = |word: &str, header: &str| {
        let out = Command::new("curl")
            .args([
                "-sS",
                "-D",
                "-",
                "-H",
                header,
                "--data-urlencode",
                "cwd=/tmp",
            ])
            .args([
                "--data-urlencode",
                &format!("args[]={word}"),
                "--data-urlencode",
                &env,
            ])
            .arg(format!("http://127.0.0.1:{bound}/dashboard/cli"))
            .output()
            .expect("curl");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // the first request warms the server up; the second is timed: curl waits a second on « 100 Continue » unless it comes
    let _ = asked("outbox", "x-dashboard-cli: 1");
    let begun = std::time::Instant::now();
    let answer = asked("outbox", "x-dashboard-cli: 1");
    let line = "go-mail  email  approuvé 2026-11-01  -> Claude l'envoie : private/dashboard/bin/dash mail send go-mail (envoie, journalise, marque envoyé)  Ana, a@x.io\n";
    assert!(answer.contains("x-exit: 0\r\n"), "{answer}");
    assert!(
        answer.contains(&format!("x-stdout-bytes: {}\r\n", line.len())) && answer.ends_with(line),
        "{answer}"
    );
    assert!(
        begun.elapsed().as_millis() < 900,
        "no wait on 100 Continue: {:?}",
        begun.elapsed()
    );
    for (word, header) in [("ping", "x-dashboard-cli: 1"), ("outbox", "x-other: 1")] {
        let handed = asked(word, header);
        assert!(
            handed.contains("POST /dashboard/cli HTTP/1.1"),
            "{word}: {handed}"
        );
        assert!(
            handed.contains(&format!("args[]={word}")) && handed.ends_with(&"x".repeat(100)),
            "its whole body: {handed}"
        );
        assert!(!handed.to_ascii_lowercase().contains("expect:"), "{handed}");
    }
}

/// Done-when 1 and 2 of rust-task-names-layer, through the front door as bin/dash asks: a Rust task's add with no layer is
/// refused there, the older app never asked; one naming a layer reaches the older app without `--layer`, its body's length
/// told again, and Rust's store keeps the layer for the picker; `task update <id> --layer` alone is answered here.
#[test]
fn the_front_door_refuses_a_rust_task_with_no_layer_and_keeps_the_one_it_names() {
    use dash_core::tasks::{Queue, Queued, TaskStore};
    let _budget = TestBudget::new();
    let dir = TestDir::new("front-layer");
    // DASH_REPO is this folder, with no RFC: the design system's names and the models are the layers
    let older = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let older_port = older.local_addr().expect("address").port();
    std::thread::spawn(move || {
        for stream in older.incoming() {
            let mut stream = stream.expect("a connection");
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(300)))
                .expect("timeout");
            let mut body = vec![];
            let _ = stream.read_to_end(&mut body);
            let head = format!(
                "HTTP/1.1 200 OK\r\nx-exit: 0\r\nx-stdout-bytes: {0}\r\nContent-Length: {0}\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
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
        .env("DASH_REPO", &dir.0)
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
    let asked = |words: &[&str]| {
        let mut curl = Command::new("curl");
        curl.args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"]);
        curl.args(["--data-urlencode", "cwd=/tmp"]);
        for w in words {
            curl.args(["--data-urlencode", &format!("args[]={w}")]);
        }
        let out = curl
            .arg(format!("http://127.0.0.1:{want}/dashboard/cli"))
            .output()
            .expect("curl");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let none = asked(&[
        "task",
        "add",
        "rust-x",
        "--title",
        "The timeline of a lead",
        "--owner",
        "rust",
    ]);
    assert!(none.contains("x-exit: 1\r\n"), "{none}");
    assert!(
        none.contains("rust-x: refused, a Rust task names the layer it changes")
            && none.contains("closest to its title: design:timeline"),
        "{none}"
    );
    assert!(
        !none.contains("POST /dashboard/cli"),
        "the older app never asked: {none}"
    );
    let wrong = asked(&["task", "add", "rust-x", "--layer", "design:timelin"]);
    assert!(wrong.contains("closest: design:timeline"), "{wrong}");
    let kept = asked(&[
        "task",
        "add",
        "rust-x",
        "--epic",
        "rust-poc",
        "--layer",
        "design:timeline",
        "--title",
        "T",
    ]);
    assert!(kept.contains("POST /dashboard/cli"), "{kept}");
    assert!(
        kept.contains("args[]=rust-x&args[]=--epic&args[]=rust-poc&args[]=--title&args[]=T"),
        "its words without --layer: {kept}"
    );

    let sent = kept.split("\r\n\r\n").last().expect("the older app's echo");
    let told = kept
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .and_then(|n| n.trim().parse::<usize>().ok());
    assert!(told.is_some_and(|n| n > 0), "{kept}");
    // an epic and no wait: the warning after the older app's answer (task rust-graph-why-these, done-when 5)
    let (sent, warned) = sent.split_once("rust-x: warning").expect("warned: {sent}");
    assert!(
        warned.starts_with(", it joins epic rust-poc and waits on nothing"),
        "{warned}"
    );
    // its layer's load before the warning: the list not synced yet, it says so and files it (task rust-add-layer-load)
    let (sent, unread) = sent
        .split_once("rust-x: its layer's")
        .expect("its load: {sent}");
    assert!(
        unread.starts_with(" open tasks could not be read: no sync of the list yet"),
        "{unread}"
    );
    assert!(
        !sent.contains("layer"),
        "the words it filed name no layer: {sent}"
    );
    // a layer and no owner: filed `--owner rust`, so the Rust picker sees it (task rust-layer-means-owner)
    assert!(
        sent.ends_with("args[]=T&args%5B%5D=--owner&args%5B%5D=rust"),
        "the whole shorter body, owned by rust: {sent}"
    );
    let updated = asked(&["task", "update", "rust-y", "--layer", "model:task"]);
    assert!(updated.contains("x-exit: 0\r\n"), "{updated}");
    assert!(
        updated.ends_with("rust-y: Rust's store keeps its layer, model:task\n"),
        "{updated}"
    );
    let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("store");
    let listed = |id: &str| Queued {
        id: id.into(),
        ..Queued::default()
    };
    let queue = Queue {
        seats: 1,
        tasks: vec![listed("rust-x"), listed("rust-y")],
    };
    tasks.sync(&queue).expect("sync");
    let layers: Vec<_> = (tasks.queue().expect("queue").tasks.into_iter())
        .map(|t| t.layer)
        .collect();
    assert_eq!(
        layers,
        [Some("design:timeline".into()), Some("model:task".into())]
    );
    // an add naming only its layer still reaches the older app; an add of a task listed already (a duplicate the older app
    // refuses) leaves the listed task's layer as it was (/code-review)
    let bare = asked(&[
        "task",
        "add",
        "rust-z",
        "--layer",
        "design:board",
        "--epic",
        "rust-poc",
    ]);
    assert!(bare.contains("POST /dashboard/cli"), "{bare}");
    let again = asked(&[
        "task",
        "add",
        "rust-x",
        "--layer",
        "model:draft",
        "--title",
        "T",
        "--epic",
        "rust-poc",
    ]);
    assert!(again.contains("POST /dashboard/cli"), "{again}");
    let read = store::tasks::open(&dir.0, &dir.0).expect("store");
    let x = read.queue().expect("queue").tasks.into_iter().next();
    assert_eq!(x.and_then(|t| t.layer).as_deref(), Some("design:timeline"));
    // task rust-add-layer-load, done-when 1: once the list is read, an add says its layer's open tasks after the older app's
    // answer, that they run one at a time, and the closest other sections
    let shared = asked(&[
        "task",
        "add",
        "rust-w",
        "--layer",
        "design:timeline",
        "--epic",
        "rust-poc",
    ]);
    assert!(shared.contains("POST /dashboard/cli"), "{shared}");
    let (_, said) = shared
        .split_once("rust-w: on design:timeline")
        .expect("its load: {shared}");
    assert!(
        said.starts_with(", 1 open task(s) share it, and its tasks run one at a time: rust-x; closest other sections: design:"),
        "{said}"
    );
    // task rust-add-layer-load, done-when 3: a done-when bin/dash sends on stdin (« - ») is read for a file:line too
    let mut curl = Command::new("curl");
    curl.args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"]);
    for field in [
        "cwd=/tmp",
        "args[]=task",
        "args[]=add",
        "args[]=klar-z",
        "args[]=--epic",
        "args[]=e",
        "args[]=--done-when",
        "args[]=-",
        "stdin=1. the guard at main.rs:2001 holds: a test",
    ] {
        curl.args(["--data-urlencode", field]);
    }
    let piped = curl
        .arg(format!("http://127.0.0.1:{want}/dashboard/cli"))
        .output();
    let piped = String::from_utf8_lossy(&piped.expect("curl").stdout).into_owned();
    assert!(piped.contains("x-exit: 1\r\n"), "{piped}");
    assert!(
        piped.contains("klar-z: refused, its done-when cites main.rs:2001"),
        "{piped}"
    );
    assert!(!piped.contains("POST /dashboard/cli"), "{piped}");
    // task rust-orch-pick-dynamic, done-when 1: no epic and not urgent is refused here, the older app never asked; urgent with
    // a deadline reaches it without --deadline, which it does not know
    let stray = asked(&["task", "add", "klar-x", "--title", "T"]);
    assert!(stray.contains("x-exit: 1\r\n"), "{stray}");
    assert!(
        stray.contains("klar-x: refused, a task belongs to an epic or is urgent with a deadline"),
        "{stray}"
    );
    assert!(!stray.contains("POST /dashboard/cli"), "{stray}");
    let urgent = asked(&[
        "task",
        "add",
        "klar-y",
        "--priority",
        "1",
        "--deadline",
        "2026-11-12",
        "--title",
        "T",
    ]);
    assert!(urgent.contains("POST /dashboard/cli"), "{urgent}");
    assert!(!urgent.contains("deadline"), "{urgent}");
    assert!(
        urgent.ends_with("args[]=klar-y&args[]=--priority&args[]=1&args[]=--title&args[]=T"),
        "{urgent}"
    );
}

/// Done-when 2 of ci-wait-gh-retry: `dash ci wait` through the front door, as bin/dash asks it. The merge hook's stand-in says
/// « gh did not answer » (its exit 3 when gh times out) on the first two reads and green on the third: the wait asks again each
/// poll and answers 0, never 3 on a lost read.
#[test]
fn the_front_door_answers_dash_ci_wait_through_two_lost_reads() {
    use std::os::unix::fs::PermissionsExt;
    let _budget = TestBudget::new();
    let dir = TestDir::new("front-ci-wait");
    let hooks = dir.0.join("repo/.claude/hooks");
    std::fs::create_dir_all(&hooks).expect("hooks");
    let count = dir.0.join("reads");
    let hook = format!(
        "import json, os, sys\nassert sys.argv[1:] == ['--checks', '806'], sys.argv\nassert os.getcwd() == {cwd:?}\n\
         p = {count:?}\nn = int(open(p).read()) + 1 if os.path.exists(p) else 1\nopen(p, 'w').write(str(n))\n\
         if n <= 2:\n    print('gh did not answer', file=sys.stderr)\n    sys.exit(3)\n\
         print(json.dumps([['rust', 'SUCCESS'], ['old run', 'SKIPPED']]))\n",
        cwd = std::fs::canonicalize(&dir.0)
            .expect("real")
            .to_string_lossy(),
        count = count.to_string_lossy(),
    );
    std::fs::write(hooks.join("merge-claim.py"), hook).expect("the stand-in");
    std::fs::set_permissions(
        hooks.join("merge-claim.py"),
        std::fs::Permissions::from_mode(0o644),
    )
    .expect("chmod");
    let (_server, want) = serve(&dir.0, |c| {
        c.env("DASH_REPO", dir.0.join("repo"))
            .env("DASH_WORKTREES", &dir.0)
            .env("DASH_CI_POLL_MS", "10")
    });
    let out = Command::new("curl")
        .args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"])
        .args(["--data-urlencode", &format!("cwd={}", dir.0.display())])
        .args([
            "--data-urlencode",
            "args[]=ci",
            "--data-urlencode",
            "args[]=wait",
        ])
        .args(["--data-urlencode", "args[]=806"])
        .arg(format!("http://127.0.0.1:{want}/dashboard/cli"))
        .output()
        .expect("curl");
    let answer = String::from_utf8_lossy(&out.stdout);
    assert!(answer.contains("x-exit: 0\r\n"), "{answer}");
    assert!(
        answer.ends_with("ci wait: green on the head (2 checks)\n"),
        "{answer}"
    );
    assert_eq!(std::fs::read_to_string(&count).expect("reads"), "3");
}

/// Done-when 1 of ci-wait-gh-retry: the front door's dash ci wait keeps the older app's rules, read through the real merge
/// hook (`merge-claim.py --checks`) with a stand-in gh on PATH. PR 806: a job cancelled in an older run and green in a newer
/// one is green (the superseded cancel is never the verdict). PR 807: a job GitHub refused over billing is no verdict, exit 3,
/// never red. The public mirror holds rust/ alone, no hook: there the test says so and stops.
#[test]
fn the_front_doors_ci_wait_drops_a_superseded_cancel_and_never_judges_a_billing_refusal() {
    use std::os::unix::fs::PermissionsExt;
    let _budget = TestBudget::new();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    if !repo.join(".claude/hooks/merge-claim.py").is_file() {
        eprintln!("no merge hook beside rust/ here (the public mirror): this test stops");
        return;
    }
    let dir = TestDir::new("front-ci-wait-rules");
    let job = |name: &str, run: u32, id: u32, conclusion: &str| {
        serde_json::json!({"__typename": "CheckRun", "name": name, "workflowName": "ci", "status": "COMPLETED",
            "conclusion": conclusion, "detailsUrl": format!("https://github.com/o/r/actions/runs/{run}/job/{id}")})
    };
    let pr = |n: u32, rollup: Vec<serde_json::Value>| {
        let view = serde_json::json!({"url": format!("https://github.com/o/r/pull/{n}"), "statusCheckRollup": rollup,
            "files": [], "changedFiles": 0});
        std::fs::write(dir.0.join(format!("{n}.json")), view.to_string()).expect("a PR's view");
    };
    pr(
        806,
        vec![
            job("check", 100, 1000, "CANCELLED"),
            job("check", 101, 1010, "SUCCESS"),
        ],
    );
    pr(
        807,
        vec![
            job("check", 101, 1010, "SUCCESS"),
            job("deploy", 101, 1021, "FAILURE"),
        ],
    );
    let bin = dir.0.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let refusal = "The job was not started because your account is locked due to a spending limit.";
    let gh = format!(
        "#!/bin/sh\ncase \"$*\" in\n\
         'pr view '*' --json url,statusCheckRollup,files,changedFiles') cat '{d}'/\"$3\".json ;;\n\
         'api repos/o/r/actions/jobs/1021') echo '{{\"steps\": []}}' ;;\n\
         'api repos/o/r/check-runs/1021/annotations?per_page=100') echo '[{{\"message\": \"{refusal}\"}}]' ;;\n\
         'api repos/o/r/check-runs/'*'/annotations?per_page=100') echo '[]' ;;\n\
         *) echo \"unexpected gh $*\" >&2; exit 9 ;;\nesac\n",
        d = dir.0.display()
    );
    std::fs::write(bin.join("gh"), gh).expect("the stand-in gh");
    std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let (_server, want) = serve(&dir.0, |c| {
        c.env("DASH_REPO", &repo)
            .env("DASH_WORKTREES", &dir.0)
            .env("CRM_LIVE_DIR", &dir.0)
            .env("PATH", path)
    });
    let wait = |n: &str| {
        let out = Command::new("curl")
            .args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"])
            .args(["--data-urlencode", &format!("cwd={}", dir.0.display())])
            .args([
                "--data-urlencode",
                "args[]=ci",
                "--data-urlencode",
                "args[]=wait",
            ])
            .args(["--data-urlencode", &format!("args[]={n}")])
            .arg(format!("http://127.0.0.1:{want}/dashboard/cli"))
            .output()
            .expect("curl");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let superseded = wait("806");
    assert!(superseded.contains("x-exit: 0\r\n"), "{superseded}");
    assert!(
        superseded.ends_with("ci wait: green on the head (1 checks)\n"),
        "{superseded}"
    );
    let billing = wait("807");
    assert!(billing.contains("x-exit: 3\r\n"), "{billing}");
    assert!(
        billing.contains("ci wait: no verdict, GitHub ran none of deploy (billing)"),
        "{billing}"
    );
}

#[test]
fn a_pod_whose_ui_tree_cannot_draw_is_refused_at_load_named() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("ext-bad-ui");
    let folder = dir.0.join("extensions.d");
    std::fs::create_dir_all(&folder).expect("folder");
    let manifest = serde_json::json!({
        "command": ["sh", "-c", "exit 0"],
        "extractors": {"text/plain": "t"},
        "ui": {"bad-page": "not a tree"}
    });
    std::fs::write(folder.join("10-bad-ui.json"), manifest.to_string()).expect("manifest");
    let listing = dash(&["extensions"], &dir);
    assert!(
        listing.contains("refused 10-bad-ui.json: bad-page:"),
        "{listing}"
    );
}
