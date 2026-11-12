//! Task rust-compass-queue through the real binary: a page's sidebar carries the compass and names Rust as what served it,
//! a comment posted from it lands as one row of the comment queue, and `dash comment work`, the queue's one agent, takes the
//! oldest first, shows on the comment while it works, and closes the row with its answer.
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

mod common;
use common::{Server, serve};

fn boot(dir: &Path) -> (Server, u16) {
    // /agents reads no real login and no real herdr
    serve(dir, |c| {
        c.env("HOME", dir)
            .env("HERDR_BIN", dir.join("no-herdr"))
            .stderr(Stdio::null())
    })
}

/// The status line and the body of one request.
fn asked(port: u16, request: String) -> (String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.write_all(request.as_bytes()).expect("write");
    stream.shutdown(Shutdown::Write).expect("done writing");
    let mut text = String::new();
    stream.read_to_string(&mut text).expect("read");
    let (head, body) = text.split_once("\r\n\r\n").expect("a head");
    let status = head.lines().next().expect("a status").to_string();
    (status, body.to_string())
}

/// A page's status and markup: a class or attribute is asserted on the markup alone, never the page's own CSS or script
/// (task rust-test-markup-only). A text view (`?view=text`) is no HTML: it comes back whole.
fn get(port: u16, target: &str) -> (String, String) {
    let (status, body) = asked(
        port,
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/html\r\n\r\n"),
    );
    let body = if target.contains("view=text") {
        body
    } else {
        dash_surface::markup(&body)
    };
    (status, body)
}

fn post(port: u16, origin: Option<&str>, form: &str) -> (String, String) {
    posted_to(port, "/compass", origin, form)
}

fn posted_to(port: u16, path: &str, origin: Option<&str>, form: &str) -> (String, String) {
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    asked(
        port,
        format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{origin}Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{form}",
            form.len()
        ),
    )
}

fn compass(port: u16) -> serde_json::Value {
    let (status, body) = get(port, "/compass?page=%2Fdeps");
    assert_eq!(status, "HTTP/1.1 200 OK", "{body}");
    serde_json::from_str(&body).expect("json")
}

/// What the queue's agent is on, the line under /comments' heading in its text form.
fn agent_says(port: u16) -> String {
    let (status, body) = get(port, "/comments?view=text");
    assert_eq!(status, "HTTP/1.1 200 OK", "{body}");
    body.lines().next().expect("the agent's line").to_string()
}

/// The agent's stand-in: it keeps its prompt, waits for the test's word, then answers.
fn stand_in(dir: &Path) -> String {
    let agent = dir.join("agent");
    let script = format!(
        "#!/bin/sh\nfor a in \"$@\"; do last=\"$a\"; done\nprintf '%s' \"$last\" > '{0}/asked'\ni=0\nwhile [ ! -f '{0}/go' ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done\nrm -f '{0}/go'\necho \"Done: $(wc -c < '{0}/asked' | tr -d ' ') bytes read\"\n",
        dir.display()
    );
    std::fs::write(&agent, script).expect("the stand-in");
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    agent.display().to_string()
}

fn work(dir: &Path, agent: &str) -> Child {
    Command::new(DASH)
        .args(["comment", "work", "--data"])
        .arg(dir.join("data"))
        .env("DASH_CLAUDE", agent)
        .env("DASH_REPO", dir)
        // the older app's dash, the one command the agent may run: a stand-in that keeps what it was asked
        .env("DASH_CLI", dir.join("older-dash"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("dash comment work")
}

fn said(child: Child) -> String {
    let out = child.wait_with_output().expect("its end");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).expect("utf-8")
}

fn until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(20), "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_compass_comment_lands_as_one_row_and_the_agent_answers_the_oldest_first() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("compass");
    let data = dir.0.join("data");
    std::fs::create_dir_all(&data).expect("data");
    let (_server, port) = boot(&data);
    let origin = format!("http://127.0.0.1:{port}");

    // Done-when 1 and 3: the page's sidebar has the compass, anchored to the page, and its footer names Rust.
    let (status, page) = get(port, "/deps");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(page.contains(r#"id="dash-compass""#), "{page}");
    assert!(
        page.contains(r#"id="dash-notes" data-page="/deps""#),
        "{page}"
    );
    assert!(
        page.contains(r#"id="dash-served">Served by Rust"#),
        "{page}"
    );

    // Only a page of this app posts: no Origin, or another site's, is refused and lands nothing.
    let form = "page=%2Fdeps&quote=the+code+map&text=Why+is+dash-ui+here%3F";
    assert_eq!(post(port, None, form).0, "HTTP/1.1 403 Forbidden");
    assert_eq!(
        post(port, Some("http://evil.example"), form).0,
        "HTTP/1.1 403 Forbidden"
    );
    assert_eq!(
        post(port, Some(&origin), "page=%2Fdeps&text=+").0,
        "HTTP/1.1 400 Bad Request"
    );
    assert_eq!(
        post(port, Some(&origin), "page=%2F%5Cevil.example&text=x").0,
        "HTTP/1.1 400 Bad Request",
        "a backslash is no path of ours"
    );
    assert_eq!(compass(port)["notes"], serde_json::json!([]));

    // a form with no script goes back to its page, at its comment (task rust-task-brief-linear)
    let (status, body) = post(port, Some(&origin), form);
    assert_eq!(status, "HTTP/1.1 303 See Other", "{body}");
    let first = body
        .strip_prefix("/deps#c-")
        .expect("its page, at its id")
        .to_string();
    assert_eq!(
        post(
            port,
            Some(&origin),
            "page=%2Fdeps%0D%0ASet-Cookie%3A+x&text=x"
        )
        .0,
        "HTTP/1.1 400 Bad Request",
        "a control character is no path of ours: the 303 names it in a header"
    );
    std::thread::sleep(Duration::from_millis(5));
    let (status, body) = post(port, Some(&origin), "page=%2Fdeps&quote=&text=second");
    assert_eq!(status, "HTTP/1.1 303 See Other", "{body}");
    let listed = compass(port);
    let notes = listed["notes"].as_array().expect("notes");
    assert_eq!(notes.len(), 2, "one row each: {listed}");
    assert_eq!(notes[0]["id"], first.as_str());
    assert_eq!(notes[0]["quote"], "the code map");
    assert_eq!(notes[0]["text"], "Why is dash-ui here?");
    assert_eq!(notes[0]["state"], "waiting");
    assert_eq!(
        agent_says(port),
        "The comment agent starts on the oldest of 2 open."
    );
    // Done-when 1 of rust-agents-comments-tabs: /comments lists the queue oldest first, each opening it on its page, and
    // /agents names the comment agent with what it works on
    // Done-when 1 of rust-comments-table: the design system's table, its head then a row a comment, oldest first, each
    // with its link
    let (_, queue) = get(port, "/comments?view=text");
    // task comments-by-state: the board's Backlog column, a card a comment: its page, its words, its state, its link
    let rows: Vec<Vec<&str>> = queue
        .lines()
        .skip_while(|l| !l.starts_with("Backlog ("))
        .skip(1)
        .take_while(|l| !l.is_empty())
        .map(|l| l.split('\t').collect())
        .collect();
    assert_eq!(rows.len(), 2, "{queue}");
    assert_eq!(rows[0][..2], ["/deps", "Why is dash-ui here?"], "{queue}");
    assert!(rows[0][2].starts_with("waits for the agent, 20"), "{queue}");
    assert_eq!(rows[1][1], "second", "{queue}");
    assert_eq!(rows[0][3], format!("/deps#c-{first}"));
    let (status, agents) = get(port, "/agents?view=text");
    assert_eq!(status, "HTTP/1.1 200 OK", "{agents}");
    assert!(
        agents.contains(
            "Comment agent\tThe comment agent starts on the oldest of 2 open.\t/comments"
        ),
        "{agents}"
    );
    let (_, html) = get(port, "/comments");
    assert!(
        html.contains(r#"href="/comments" aria-current="page">"#),
        "{html}"
    );
    // the same cards in the HTML: the tasks board's Backlog column, the oldest first, each opening the comment on its page
    let column = &html[html.find(r#"data-status="backlog""#).expect(&html)..];
    let link = format!(r#"href="/deps#c-{first}""#);
    let (oldest, second) = (column.find(&link).expect(&html), column.find("second"));
    assert!(second.is_some_and(|s| oldest < s), "{html}");
    assert!(html.contains(r#"class="board""#), "{html}");

    // A run that fails, or Claude's JSON with no result, gives the row back open: an outage closes none of his comments.
    for (name, body) in [
        ("down", "exit 1"),
        ("no-result", r#"echo '{"type":"result","is_error":true}'"#),
    ] {
        let broken = dir.0.join(name);
        std::fs::write(&broken, format!("#!/bin/sh\n{body}\n")).expect("a broken agent");
        std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let out = work(&dir.0, &broken.display().to_string())
            .wait_with_output()
            .expect("its end");
        assert!(!out.status.success(), "{name}: {out:?}");
        let listed = compass(port);
        assert_eq!(listed["notes"][0]["state"], "waiting", "{name}: {listed}");
        assert_eq!(
            agent_says(port),
            "The comment agent starts on the oldest of 2 open."
        );
    }

    // Done-when 2: the one agent takes the oldest, and the compass names it on that comment while it works.
    let agent = stand_in(&dir.0);
    let running = work(&dir.0, &agent);
    until("the agent is asked", || dir.0.join("asked").exists());
    let listed = compass(port);
    assert_eq!(
        agent_says(port),
        format!(
            "The comment agent works comment {first} on /deps: « Why is dash-ui here? » (2 open)."
        )
    );
    assert_eq!(listed["notes"][0]["state"], "working");
    assert_eq!(listed["notes"][1]["state"], "waiting");
    let prompt = std::fs::read_to_string(dir.0.join("asked")).expect("its prompt");
    assert!(
        prompt.contains("« the code map »") && prompt.contains("« Why is dash-ui here? »"),
        "{prompt}"
    );
    assert!(prompt.contains(&format!("/deps#c-{first}")), "{prompt}");
    std::fs::write(dir.0.join("go"), "").expect("go");
    assert_eq!(said(running), format!("answered {first}\n"));

    // Its answer is on the comment and the row is closed; the next run takes the second.
    let listed = compass(port);
    assert_eq!(listed["notes"][0]["state"], "answered");
    assert!(
        listed["notes"][0]["answer"]
            .as_str()
            .expect("an answer")
            .starts_with("Done: "),
        "{listed}"
    );
    assert_eq!(
        agent_says(port),
        "The comment agent starts on the oldest of 1 open."
    );
    std::fs::write(dir.0.join("go"), "").expect("go");
    let second = said(work(&dir.0, &agent));
    assert!(
        second.starts_with("answered compass-") && !second.contains(&first),
        "{second}"
    );
    assert_eq!(
        agent_says(port),
        "The comment agent is idle: no comment waits."
    );
    assert_eq!(said(work(&dir.0, &agent)), "no comment waits\n");
}

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}")).expect("a stand-in");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// Task rust-comment-files-task: a comment asking a change gets its task filed by the agent through the older app's
/// `task add`, the one command it may run, and its answer links it; a question gets an answer only; /comments shows the
/// task per comment, and a link to a task never filed shows none.
#[test]
fn a_change_asked_files_its_task_and_a_question_is_answered_only() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("compass-files");
    let d = dir.0.display().to_string();
    let data = dir.0.join("data");
    std::fs::create_dir_all(&data).expect("data");
    // the older app's list, as much of it as the check of a filed task reads
    let older = dir.0.join("private/dashboard");
    std::fs::create_dir_all(&older).expect("its folder");
    let list = dash_store::Connection::open(older.join("dashboard_dev.db")).expect("its db");
    // rust-poc is filed already: an answer that only mentions it files nothing
    list.execute_batch(
        "CREATE TABLE tasks (collection TEXT, id TEXT); INSERT INTO tasks VALUES ('tasks.edn', 'rust-poc')",
    )
        .expect("its table");
    // its dash keeps what `task add` was asked; the test files the row, as the app would
    script(
        &dir.0.join("older-dash"),
        // written whole, then moved in place: `until` sees the file and reads it at once, never between a `>`'s create and its write
        &format!(
            "printf '%s\\n' \"$@\" > '{d}/added.part' && mv '{d}/added.part' '{d}/added'\necho \"added $3\"\n"
        ),
    );
    // the agent: a change asked runs task add and links it once filed, a question is answered, « Invent » links a task never filed
    let agent = dir.0.join("agent");
    script(
        &agent,
        &format!(
            "printf '%s\\n' \"$@\" > '{d}/argv'\nfor a in \"$@\"; do last=\"$a\"; done\ncase \"$last\" in\n\
             *Rename*) \"$DASH_CLI\" task add rename-deps --title 'Rename the tab' --why 'his comment' --done-when 'it reads Crates' --owner rust --priority 2 --start >/dev/null\n\
             i=0; while [ ! -f '{d}/go' ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done\n\
             printf 'Filed, see x?issue=rust-poc too. In forms.rs.\\nTask: http://100.72.158.115:7788/projects/site_perso?issue=rename-deps\\n' ;;\n\
             *Invent*) echo 'Task: http://100.72.158.115:7788/projects/site_perso?issue=made-up' ;;\n\
             *) echo 'It draws the crates each one uses, as x?issue=rust-poc asked.' ;;\nesac\n"
        ),
    );
    let agent = agent.display().to_string();
    let (_server, port) = boot(&data);
    let origin = format!("http://127.0.0.1:{port}");
    for text in [
        "What+is+this+map%3F",
        "Rename+this+tab+to+Crates",
        "Invent+a+task",
    ] {
        let (status, body) = post(port, Some(&origin), &format!("page=%2Fdeps&text={text}"));
        assert_eq!(status, "HTTP/1.1 303 See Other", "{body}");
        std::thread::sleep(Duration::from_millis(5));
    }

    // Done-when 2: the question is answered, nothing filed. The agent may run the older app's task add and nothing else.
    assert!(said(work(&dir.0, &agent)).starts_with("answered "));
    assert!(!dir.0.join("added").exists(), "a question files nothing");
    let argv = std::fs::read_to_string(dir.0.join("argv")).expect("its arguments");
    let cli = dir.0.join("older-dash").display().to_string();
    assert!(
        argv.contains("\n--allowedTools\n")
            && argv.contains(&format!("\nBash({cli} task add:*)\n"))
            && argv.contains("\ndontAsk\n"),
        "{argv}"
    );
    assert!(argv.contains(&format!("`{cli} task add <id>")), "{argv}");

    // Done-when 1: the change asked runs task add; once the list holds it, the answer links it
    let running = work(&dir.0, &agent);
    until("the agent runs task add", || dir.0.join("added").exists());
    let added = std::fs::read_to_string(dir.0.join("added")).expect("what it was asked");
    assert!(
        added.starts_with("task\nadd\nrename-deps\n--title\n"),
        "{added}"
    );
    list.execute("INSERT INTO tasks VALUES ('tasks.edn', 'rename-deps')", [])
        .expect("filed");
    std::fs::write(dir.0.join("go"), "").expect("go");
    assert!(said(running).starts_with("answered "));
    assert!(said(work(&dir.0, &agent)).starts_with("answered "));

    // Done-when 2: /comments shows the task per comment, newest first; a link to no filed task shows none
    let (_, page) = get(port, "/comments?view=text");
    let rows: Vec<Vec<&str>> = page
        .lines()
        .filter(|l| l.contains("\ton /deps") || l.contains(", on /deps"))
        .map(|l| l.split('\t').collect())
        .collect();
    let shown: Vec<(&str, &str)> = rows.iter().map(|r| (r[0], r[1])).collect();
    // Printed so a report can quote what /comments showed (`--no-capture`): the PM check judges output, not assertions.
    for r in &rows {
        eprintln!("/comments row: {}", r.join(" | "));
    }
    // a board (task comments-by-state): the answered, each its page then its words; then the comment whose task was filed,
    // as that task's card: its key, his words
    assert_eq!(
        shown,
        [
            ("/deps", "Invent a task"),
            ("/deps", "What is this map?"),
            ("rename-deps", "Rename this tab to Crates"),
        ],
        "{page}"
    );
    assert!(page.contains("\nTask created (1)\n"), "{page}");
    let (_, html) = get(port, "/comments?view=html");
    assert!(
        html.contains(r#"href="/projects/site_perso?issue=rename-deps""#),
        "{html}"
    );
    // Task rust-comment-answer-on-page: the page a comment was left on shows it and its answer in its rail, in its HTML and
    // its text (@nha, 2026-11-09: « answers are not visible in most pages. [...] So answered just means lost. »)
    let answer = "It draws the crates each one uses, as x?issue=rust-poc asked.";
    let (_, deps) = get(port, "/deps?view=html");
    let rail = &deps[deps.find(r#"id="dash-notes""#).expect(&deps)..];
    assert!(
        rail.contains("What is this map?") && rail.contains(answer),
        "{rail}"
    );
    assert!(rail.contains(r#"class="doc-thread" id="c-"#), "{rail}");
    let (_, text) = get(port, "/deps?view=text");
    assert!(
        text.contains("\nComments on this page (") && text.contains(answer),
        "{text}"
    );
}

/// Task rust-watchers-port, done-when 2: the comment watcher, Rust's comment queue, is fed by the compass. A comment posted
/// from a page's sidebar shows on /watchers' comment queue row, in its text for agents and its HTML for him, served by Rust.
#[test]
fn a_compass_comment_feeds_the_comment_watcher_on_watchers() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("compass-watchers");
    let data = dir.0.join("data");
    std::fs::create_dir_all(&data).expect("data");
    // the old app answers nothing here: its rows say so, the comment queue's row stands
    let (_server, port) = serve(&data, |c| {
        c.env("DASH_CLI", dir.0.join("no-old-app"))
            .stderr(Stdio::null())
    });
    let text = |port: u16| {
        asked(
            port,
            format!("GET /watchers HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: */*\r\n\r\n"),
        )
    };
    let queue = |page: &str| {
        page.split("comment queue")
            .nth(1)
            .expect("its row")
            .to_string()
    };

    let (status, before) = text(port);
    assert_eq!(status, "HTTP/1.1 200 OK", "{before}");
    let row = queue(&before);
    assert!(row.contains("Served by: Rust"), "{before}");
    assert!(
        row.contains("Fed by: the compass on every Rust page (0 open; The comment agent is idle: no comment waits.)"),
        "{before}"
    );

    let origin = format!("http://127.0.0.1:{port}");
    let (status, _) = post(
        port,
        Some(&origin),
        "page=%2Fwatchers&quote=Last+beat&text=Is+the+mail+poll+late%3F",
    );
    assert!(
        status.starts_with("HTTP/1.1 30") || status == "HTTP/1.1 200 OK",
        "{status}"
    );

    let (_, after) = text(port);
    let row = queue(&after);
    assert!(
        row.contains("Fed by: the compass on every Rust page (1 open; The comment agent starts on the oldest of 1 open.) and the old app's comments (0 open)"),
        "the compass's comment feeds the watcher: {after}"
    );
    let (status, html) = get(port, "/watchers");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(
        html.contains("Fed by: the compass on every Rust page (1 open;"),
        "the same row in his form"
    );
}

/// Task rust-mail-thread: the thread's reply box posts to Rust, which takes a form only from a page of this app and an address
/// /mail reads, so neither another site nor a forged account sends anything (the send itself: src/mail.rs' tests, on a fake
/// mail server: this one never reaches a real one).
#[test]
fn a_mail_reply_is_this_app_s_and_refuses_a_foreign_page_or_address() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("mail-reply-route");
    let (_server, port) = boot(&dir.0);
    let path = "/mail/t/brq/reply";
    let (status, body) = posted_to(port, path, None, "account=nicolas%40klar.im&text=Hi");
    assert_eq!(
        (status.as_str(), body.as_str()),
        (
            "HTTP/1.1 403 Forbidden",
            "a form comes from a page of this app"
        )
    );
    let page = format!("http://127.0.0.1:{port}");
    let (status, body) = posted_to(
        port,
        path,
        Some(&page),
        "account=someone%40example.org&text=Hi+there",
    );
    assert_eq!(status, "HTTP/1.1 502 Bad Gateway");
    assert_eq!(
        body,
        "Not sent: someone@example.org is not an address /mail reads\n\nYour words:\nHi there\n"
    );
}

/// A multipart form, as the rail's box sends it: its fields, then each file its name, type and bytes.
fn multipart(
    port: u16,
    origin: &str,
    fields: &[(&str, &str)],
    files: &[(&str, &str, &[u8])],
) -> (String, String) {
    let mut body = vec![];
    for (name, value) in fields {
        body.extend(
            format!("--XB\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                .as_bytes(),
        );
    }
    for (name, kind, bytes) in files {
        body.extend(
            format!("--XB\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: {kind}\r\n\r\n")
                .as_bytes(),
        );
        body.extend(*bytes);
        body.extend(b"\r\n");
    }
    body.extend(b"--XB--\r\n");
    let head = format!(
        "POST /compass HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: multipart/form-data; boundary=XB\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.write_all(head.as_bytes()).expect("its head");
    stream.write_all(&body).expect("its body");
    stream.shutdown(Shutdown::Write).expect("done writing");
    let mut answer = vec![];
    stream.read_to_end(&mut answer).expect("read");
    let text = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("a head");
    (head.lines().next().expect("a status").into(), body.into())
}

/// Bytes and the head of a GET, a file's.
fn fetched(port: u16, target: &str) -> (String, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let ask = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: */*\r\n\r\n");
    stream.write_all(ask.as_bytes()).expect("write");
    stream.shutdown(Shutdown::Write).expect("done writing");
    let mut answer = vec![];
    stream.read_to_end(&mut answer).expect("read");
    let at = answer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a head");
    (
        String::from_utf8_lossy(&answer[..at]).into(),
        answer[at + 4..].to_vec(),
    )
}

/// Task rust-comment-files-filter, done-when 1: the page's comment box attaches images and PDFs, kept with the comment on the
/// fixture store: the row names them, each opens at its link as the type it is, never sniffed, and the page's rail shows
/// them under the comment; a file of any other kind refuses the whole comment, nothing kept.
#[test]
fn a_page_comment_keeps_its_images_and_pdfs() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("compass-attach");
    let data = dir.0.join("data");
    std::fs::create_dir_all(&data).expect("data");
    let (_server, port) = boot(&data);
    let origin = format!("http://127.0.0.1:{port}");
    let png: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR--boundary-like bytes \r\n--X";
    let pdf: &[u8] = b"%PDF-1.4\n1 0 obj\n%%EOF\n";
    let fields = [
        ("page", "/deps"),
        ("quote", ""),
        ("text", "This chart is off, see the shot"),
    ];

    // another kind than an image or a PDF: the whole comment refused, no row, no file
    let (status, body) = multipart(
        port,
        &origin,
        &fields,
        &[
            ("shot.png", "image/png", png),
            ("x.html", "text/html", b"alert(1)"),
        ],
    );
    assert_eq!(status, "HTTP/1.1 400 Bad Request", "{body}");
    assert!(body.contains("each an image or a PDF"), "{body}");
    // a name that is only an extension is kept with none (« .png » as « 0-png »): refused, never a link to nothing
    // (/code-review); more files than a comment keeps, or a body no part of which reads, neither
    let six = [("a.png", "image/png", png); 6];
    let none = [(".png", "image/png", png)];
    for files in [&none[..], &six[..]] {
        let (status, body) = multipart(port, &origin, &fields, files);
        assert_eq!(status, "HTTP/1.1 400 Bad Request", "{body}");
    }
    let (status, body) = asked(
        port,
        format!(
            "POST /compass HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: multipart/form-data; boundary=XB\r\nContent-Length: 1\r\n\r\nx"
        ),
    );
    assert_eq!(
        (status.as_str(), body.as_str()),
        (
            "HTTP/1.1 400 Bad Request",
            "the form's parts did not read: send it again"
        )
    );
    assert_eq!(compass(port)["notes"], serde_json::json!([]));
    assert!(!data.join("comment-files").exists(), "nothing kept");
    // and only a page of this app posts it
    assert_eq!(
        multipart(
            port,
            "http://evil.example",
            &fields,
            &[("shot.png", "image/png", png)]
        )
        .0,
        "HTTP/1.1 403 Forbidden"
    );

    // a picture and a PDF, and the picker's empty part when nothing else was chosen
    let (status, body) = multipart(
        port,
        &origin,
        &fields,
        &[
            ("my shot.png", "image/png", png),
            ("brief.PDF", "application/pdf", pdf),
            ("", "application/octet-stream", b""),
        ],
    );
    assert_eq!(status, "HTTP/1.1 303 See Other", "{body}");
    let id = body
        .strip_prefix("/deps#c-")
        .expect("its page, at its id")
        .to_string();
    let listed = compass(port);
    eprintln!("GET /compass?page=/deps: {listed}");
    let note = &listed["notes"][0];
    assert_eq!(note["text"], "This chart is off, see the shot");
    let files: Vec<&str> = note["files"]
        .as_array()
        .expect("its files")
        .iter()
        .filter_map(|f| f.as_str())
        .collect();
    assert_eq!(
        files,
        [
            format!("/compass/files/{id}/0-my_shot.png"),
            format!("/compass/files/{id}/1-brief.PDF")
        ]
    );
    // kept on the store's folder, each opening as the type it is, never sniffed
    for (at, kind, bytes) in [
        (files[0], "image/png", png),
        (files[1], "application/pdf", pdf),
    ] {
        let (head, got) = fetched(port, at);
        assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
        assert!(
            head.contains(&format!("Content-Type: {kind}"))
                && head.contains("X-Content-Type-Options: nosniff"),
            "{head}"
        );
        assert!(!head.contains("attachment;"), "shown in place: {head}");
        assert_eq!(got, bytes);
    }
    assert!(
        data.join("comment-files")
            .join(&id)
            .join("0-my_shot.png")
            .is_file()
    );
    // a name outside the folder, or one no comment keeps, is no file of ours
    for at in [
        format!("/compass/files/{id}/..%2F..%2Fcomments.sqlite"),
        format!("/compass/files/{id}/9-none.png"),
        "/compass/files/../comments.sqlite".into(),
    ] {
        let (head, _) = fetched(port, &at);
        assert!(!head.starts_with("HTTP/1.1 200"), "{at}: {head}");
    }

    // the rail shows them under the comment: the picture drawn, the PDF its tile
    let (_, page) = get(port, "/deps?view=html");
    let thread = &page[page.find(&format!(r#"id="c-{id}""#)).expect(&page)..];
    assert!(
        thread.contains(&format!(r#"href="{}""#, files[0]))
            && thread.contains(&format!(r#"src="{}""#, files[0])),
        "{thread}"
    );
    assert!(
        thread.contains(&format!(r#"href="{}""#, files[1])) && thread.contains("brief.PDF"),
        "{thread}"
    );

    // and an agent reads them on the page's text: its whole page named, each file's link after the words
    let (_, text) = get(port, "/deps?view=text");
    let line = text
        .lines()
        .find(|l| l.starts_with(&format!("c-{id}\t")))
        .expect(&text);
    eprintln!("/deps?view=text: {line}");
    assert!(
        line.starts_with(&format!(
            "c-{id}\tWhole page\tThis chart is off, see the shot\t"
        )) && line.ends_with(&format!("\t{}\t{}", files[0], files[1])),
        "{line}"
    );

    // the agent reads them: its prompt names each file where it is kept
    let agent = stand_in(&dir.0);
    std::fs::write(dir.0.join("go"), "").expect("go");
    assert!(said(work(&dir.0, &agent)).starts_with("answered "));
    let prompt = std::fs::read_to_string(dir.0.join("asked")).expect("its prompt");
    assert!(
        prompt.contains("He attached, read each first: ")
            && prompt.contains("0-my_shot.png")
            && prompt.contains("1-brief.PDF"),
        "{prompt}"
    );
}

/// Task rust-window-compass through the real binary: a page loads dioxus-liveview's client from /live.js, and its socket at
/// /live is upgraded on the connection's thread, which sends the compass's first patches (a binary frame) at once.
#[test]
fn the_live_compass_serves_its_client_and_its_socket() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("live-compass-serve");
    let (_server, port) = boot(&dir.0);
    let (status, client) = asked(
        port,
        format!("GET /live.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
    );
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(
        client.contains("new WebSocket(WS_ADDR)")
            && client.contains(r#"__dioxusGetWsUrl("/live")"#)
            && client.contains(r#"getElementById("dash-live")"#)
            && !client.contains(r#"getElementById("main")"#),
        "liveview's own client, opening /live, mounting in the menu's root"
    );
    let upgrade = |origin: &str| {
        format!(
            "GET /live HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{origin}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
    };
    // another site's page, or none named: refused, as POST /compass refuses it (/code-review)
    for origin in ["", "Origin: http://evil.example\r\n"] {
        let (status, _) = asked(port, upgrade(origin));
        assert_eq!(status, "HTTP/1.1 403 Forbidden", "{origin}");
    }
    let mut socket = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let origin = format!("Origin: http://127.0.0.1:{port}\r\n");
    socket
        .write_all(upgrade(&origin).as_bytes())
        .expect("write");
    let mut got = vec![];
    let mut chunk = [0; 4096];
    let started = Instant::now();
    // the handshake's answer, then the first frame's opening byte
    while !got.windows(4).any(|w| w == b"\r\n\r\n")
        || got.len() <= got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4
    {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{}",
            String::from_utf8_lossy(&got)
        );
        let n = socket.read(&mut chunk).expect("read");
        assert!(n > 0, "closed: {}", String::from_utf8_lossy(&got));
        got.extend_from_slice(&chunk[..n]);
    }
    let end = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&got[..end]);
    // a 16-zero-byte key and its RFC 6455 accept, base64(sha1(key + GUID)); never RFC's own sample key, which the
    // mirror's gitleaks reads as an API key after « Key: » (ci-mirror/rust refused PR 1449)
    assert!(
        head.starts_with("HTTP/1.1 101 Switching Protocols\r\n")
            && head.contains("Sec-WebSocket-Accept: ICX+Yqv66kxgM0FcWaLWlFLwTAI=\r\n"),
        "{head}"
    );
    assert_eq!(
        got[end], 0x82,
        "a whole binary frame: the compass's patches"
    );
}
