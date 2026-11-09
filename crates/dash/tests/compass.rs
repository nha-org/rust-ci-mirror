//! Task rust-compass-queue through the real binary: a page's sidebar carries the compass and names Rust as what served it,
//! a comment posted from it lands as one row of the comment queue, and `dash comment work`, the queue's one agent, takes the
//! oldest first, shows on the comment while it works, and closes the row with its answer.
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn boot(dir: &Path) -> (Running, u16) {
    let mut child = Command::new(DASH)
        .args(["serve", "--data"])
        .arg(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("boot");
    let stdout = child.stdout.take().expect("stdout");
    let server = Running(child);
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("its port");
    let port = line.trim().strip_prefix("port ").expect("port");
    (server, port.parse().expect("a number"))
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
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    asked(
        port,
        format!(
            "POST /compass HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{origin}Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{form}",
            form.len()
        ),
    )
}

fn compass(port: u16) -> serde_json::Value {
    let (status, body) = get(port, "/compass?page=%2Fdeps");
    assert_eq!(status, "HTTP/1.1 200 OK", "{body}");
    serde_json::from_str(&body).expect("json")
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
        listed["agent"],
        "The comment agent starts on the oldest of 2 open."
    );

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
            listed["agent"],
            "The comment agent starts on the oldest of 2 open."
        );
    }

    // Done-when 2: the one agent takes the oldest, and the compass names it on that comment while it works.
    let agent = stand_in(&dir.0);
    let running = work(&dir.0, &agent);
    until("the agent is asked", || dir.0.join("asked").exists());
    let listed = compass(port);
    assert_eq!(
        listed["agent"],
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
        listed["agent"],
        "The comment agent starts on the oldest of 1 open."
    );
    std::fs::write(dir.0.join("go"), "").expect("go");
    let second = said(work(&dir.0, &agent));
    assert!(
        second.starts_with("answered compass-") && !second.contains(&first),
        "{second}"
    );
    assert_eq!(
        compass(port)["agent"],
        "The comment agent is idle: no comment waits."
    );
    assert_eq!(said(work(&dir.0, &agent)), "no comment waits\n");
}
