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

fn get(port: u16, target: &str) -> (String, String) {
    asked(
        port,
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/html\r\n\r\n"),
    )
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
    body.lines().nth(1).expect("the agent's line").to_string()
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

    let (status, body) = post(port, Some(&origin), form);
    assert_eq!(status, "HTTP/1.1 200 OK", "{body}");
    let first = body
        .split("Landed as ")
        .nth(1)
        .and_then(|b| b.split('.').next())
        .expect("its id")
        .to_string();
    assert!(
        body.contains(&format!(r##"href="/deps#c-{first}""##)),
        "{body}"
    );
    std::thread::sleep(Duration::from_millis(5));
    let (status, body) = post(port, Some(&origin), "page=%2Fdeps&quote=&text=second");
    assert_eq!(status, "HTTP/1.1 200 OK", "{body}");
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
    let rows: Vec<Vec<&str>> = queue
        .lines()
        .skip(2)
        .map(|l| l.split('\t').collect())
        .collect();
    assert_eq!(rows.len(), 3, "{queue}");
    assert_eq!(
        rows[0],
        ["Comment", "Page", "State", "Since", "Link"],
        "{queue}"
    );
    assert_eq!(
        rows[1][..3],
        ["Why is dash-ui here?", "/deps", "waits for the agent"],
        "{queue}"
    );
    assert_eq!(rows[2][0], "second", "{queue}");
    assert!(rows[1][3].starts_with("20"), "{queue}");
    assert_eq!(rows[1][4], format!("/deps#c-{first}"));
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
        html.contains(r#"href="/comments" aria-current="page">Comments"#),
        "{html}"
    );
    // the same rows in the HTML: a table, no card, the oldest's first cell its link
    let table = &html[html.find(r#"class="table" id="queue""#).expect(&html)..];
    let link = format!(r#"href="/deps#c-{first}">Why is dash-ui here?"#);
    let (oldest, second) = (table.find(&link).expect(&html), table.find(">second<"));
    assert!(second.is_some_and(|s| oldest < s), "{html}");
    assert!(table.contains(r#"scope="col">Comment"#), "{html}");
    assert!(!html.contains(r#"class="card""#), "{html}");

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
        &format!("printf '%s\\n' \"$@\" > '{d}/added'\necho \"added $3\"\n"),
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
        assert_eq!(status, "HTTP/1.1 200 OK", "{body}");
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
    let shown: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r[0], r[1].split(',').next().unwrap_or("")))
        .collect();
    // Printed so a report can quote what /comments showed (`--no-capture`): the PM check judges output, not assertions.
    for r in &rows {
        eprintln!("/comments row: {}", r.join(" | "));
    }
    assert_eq!(
        shown,
        [
            ("Invent a task", "answered"),
            ("Rename this tab to Crates", "task rename-deps"),
            ("What is this map?", "answered"),
        ],
        "{page}"
    );
    assert_eq!(
        rows[1][2], "/projects/site_perso?issue=rename-deps",
        "{page}"
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
