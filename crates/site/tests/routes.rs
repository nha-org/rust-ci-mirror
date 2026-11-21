//! Every route of nicolas-ha.com against the real server on loopback, one test each (task rust-site-no-container):
//! the answers PublicSite.Static, OpsController and HooksController gave, which bin/site-check checks on the live host.
use dash_meter::dash_test;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("site-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The binary on a free loopback port, stopped when dropped.
struct Up {
    at: String,
    child: Child,
}

impl Drop for Up {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `site serve` of a site/ of a few files, in the folder `name`, with these secrets.
fn up(name: &str, env: &[(&str, &str)]) -> Up {
    let dir = scratch(name);
    let root = dir.join("site");
    for (path, text) in [
        ("index.html", "home"),
        ("public/index.html", "public"),
        ("a.txt", "0123456789"),
        ("CLAUDE.md", "guide"),
        ("sitemap.xml", "<x/>"),
    ] {
        std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        std::fs::write(root.join(path), text).unwrap();
    }
    start(&dir, env)
}

fn start(dir: &Path, env: &[(&str, &str)]) -> Up {
    // SITE_BIN: another build of it, the static linux-musl one in the mirror's site-musl job
    let bin = std::env::var("SITE_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_site").to_string());
    let mut child = Command::new(bin)
        .args(["serve"])
        .arg(dir.join("site"))
        .arg(dir.join("data"))
        .arg("0")
        .env("SITE_PROC", dir.join("proc"))
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let at = line
        .trim()
        .strip_prefix("site: ")
        .expect("its address")
        .to_string();
    Up { at, child }
}

struct Answer {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

fn ask(up: &Up, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Answer {
    let mut s = TcpStream::connect(&up.at).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\n");
    if !headers.iter().any(|(n, _)| *n == "host") {
        req.push_str("host: nicolas-ha.com\r\n");
    }
    for (n, v) in headers {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    if !body.is_empty() {
        req.push_str(&format!("content-length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut raw = vec![];
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..split].to_vec()).unwrap();
    let status = head[9..12].parse().unwrap();
    Answer {
        status,
        head,
        body: raw[split + 4..].to_vec(),
    }
}

fn get(up: &Up, path: &str) -> Answer {
    ask(up, "GET", path, &[], b"")
}

#[dash_test]
fn a_file_answers_its_bytes_its_type_and_hsts() {
    let at = up("file", &[]);
    let a = get(&at, "/a.txt");
    assert_eq!((a.status, a.body.as_slice()), (200, &b"0123456789"[..]));
    assert_eq!(
        a.header("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        a.header("strict-transport-security").unwrap(),
        "max-age=31536000"
    );
    assert_eq!(
        a.header("cache-control").unwrap(),
        "public, max-age=0, must-revalidate"
    );
    assert_eq!(
        get(&at, "/sitemap.xml").header("content-type").unwrap(),
        "text/xml; charset=utf-8"
    );
    let head = ask(&at, "HEAD", "/a.txt", &[], b"");
    assert_eq!((head.status, head.body.len()), (200, 0));
    assert_eq!(head.header("content-length").unwrap(), "10");
}

#[dash_test]
fn a_folder_answers_301_to_its_slash_then_its_index() {
    let at = up("folder", &[]);
    let a = get(&at, "/public?x=1");
    assert_eq!(
        (a.status, a.header("location").unwrap()),
        (301, "/public/?x=1".into())
    );
    assert_eq!(get(&at, "//public").header("location").unwrap(), "/public/");
    assert_eq!(get(&at, "/public/").body, b"public");
    assert_eq!(get(&at, "/").body, b"home");
}

#[dash_test]
fn claude_md_a_climb_and_an_unknown_path_answer_404_with_hsts() {
    let at = up("refused", &[]);
    for path in [
        "/CLAUDE.md",
        "/%2e%2e/a.txt",
        "/..%2fa.txt",
        "/%zz",
        "/dashboard",
        "/dashboard/model/write",
    ] {
        let a = get(&at, path);
        assert_eq!(a.status, 404, "{path}");
        assert!(a.header("strict-transport-security").is_some(), "{path}");
    }
    assert_eq!(ask(&at, "POST", "/a.txt", &[], b"x").status, 404);
}

#[dash_test]
fn www_answers_301_to_the_bare_domain() {
    let at = up("www", &[]);
    for host in ["www.nicolas-ha.com", "WWW.nicolas-ha.com.:443"] {
        let a = ask(&at, "GET", "/a.txt?q=1", &[("host", host)], b"");
        assert_eq!(
            (a.status, a.header("location").unwrap()),
            (301, "https://nicolas-ha.com/a.txt?q=1".into())
        );
    }
    // another host is no www of ours
    assert_eq!(
        ask(&at, "GET", "/a.txt", &[("host", "www.evil.com")], b"").status,
        200
    );
}

#[dash_test]
fn an_unchanged_file_answers_304_and_one_range_206() {
    let at = up("ranges", &[]);
    let etag = get(&at, "/a.txt").header("etag").unwrap();
    let a = ask(&at, "GET", "/a.txt", &[("if-none-match", &etag)], b"");
    assert_eq!((a.status, a.body.len()), (304, 0));
    let part = |range: &str, extra: &[(&str, &str)]| {
        let mut h = vec![("range", range)];
        h.extend_from_slice(extra);
        ask(&at, "GET", "/a.txt", &h, b"")
    };
    let a = part("bytes=2-4", &[]);
    assert_eq!((a.status, a.body.as_slice()), (206, &b"234"[..]));
    assert_eq!(a.header("content-range").unwrap(), "bytes 2-4/10");
    assert_eq!(part("Bytes = 7-", &[]).body, b"789");
    assert_eq!(part("bytes=-3", &[]).body, b"789");
    assert_eq!(part("bytes=8-99", &[]).body, b"89");
    let a = part("bytes=10-", &[]);
    assert_eq!(
        (a.status, a.header("content-range").unwrap()),
        (416, "bytes */10".into())
    );
    assert_eq!(part("bytes=0-1,4-5", &[]).status, 200);
    assert_eq!(part("bytes=5-2", &[]).status, 200);
    assert_eq!(part("bytes=0-1", &[("if-range", "\"other\"")]).status, 200);
    assert_eq!(part("bytes=0-1", &[("if-range", &etag)]).status, 206);
}

#[dash_test]
fn the_chat_is_off_its_page_post_and_socket_answer_404() {
    let at = up("chat", &[]);
    assert_eq!(get(&at, "/chat/").status, 404);
    assert_eq!(
        ask(
            &at,
            "POST",
            "/chat",
            &[("content-type", "application/json")],
            br#"{"question":"x"}"#
        )
        .status,
        404
    );
    assert_eq!(get(&at, "/live/websocket").status, 404);
}

#[dash_test]
fn ops_chat_answers_behind_its_token_and_keeps_no_record() {
    let unset = up("ops-unset", &[]);
    assert_eq!(get(&unset, "/ops/chat").status, 404);
    let at = up("ops-chat", &[("CHAT_OPS_TOKEN", "t0ken")]);
    assert_eq!(
        ask(
            &at,
            "GET",
            "/ops/chat",
            &[("authorization", "Bearer wrong")],
            b""
        )
        .status,
        401
    );
    let a = ask(
        &at,
        "GET",
        "/ops/chat?after=3",
        &[("authorization", "Bearer t0ken")],
        b"",
    );
    assert_eq!((a.status, a.json()), (200, json!({"records": []})));
}

#[dash_test]
fn ops_relay_reads_the_relays_resident_memory() {
    let at = up("ops-relay", &[("CHAT_OPS_TOKEN", "t0ken")]);
    let auth = [("authorization", "Bearer t0ken")];
    assert_eq!(
        ask(&at, "GET", "/ops/relay", &auth, b"").json(),
        json!({"rss_bytes": null})
    );
    let proc = std::env::temp_dir().join(format!(
        "site-test-ops-relay-{}/proc/42",
        std::process::id()
    ));
    std::fs::create_dir_all(&proc).unwrap();
    std::fs::write(proc.join("comm"), "nanolathe-serve\n").unwrap();
    std::fs::write(proc.join("status"), "Name:\tx\nVmRSS:\t  2048 kB\n").unwrap();
    assert_eq!(
        ask(&at, "GET", "/ops/relay", &auth, b"").json(),
        json!({"rss_bytes": 2048 * 1024})
    );
}

/// HMAC-SHA256 as GitHub signs, held to GitHub's own example below.
fn signed(secret: &str, body: &[u8]) -> String {
    let mut k = [0u8; 64];
    k[..secret.len()].copy_from_slice(secret.as_bytes());
    let pad = |x: u8| k.iter().map(|b| b ^ x).collect::<Vec<u8>>();
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(body)
        .finalize();
    let mac = Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize();
    format!(
        "sha256={}",
        mac.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

#[dash_test]
fn hooks_github_keeps_a_signed_deliverys_ids_and_states_only() {
    assert_eq!(
        ask(&up("hooks-unset", &[]), "POST", "/hooks/github", &[], b"{}").status,
        404
    );
    let at = up(
        "hooks",
        &[
            ("GITHUB_WEBHOOK_SECRET", "s3cret"),
            ("HOOKS_PULL_TOKEN", "pull"),
        ],
    );
    let pr = json!({"action": "opened", "repository": {"full_name": "nha-org/site_perso"},
        "pull_request": {"number": 7, "title": "secret title", "head": {"sha": "abc", "ref": "branch"}}})
    .to_string();
    let post = |event: &str, sig: &str, body: &[u8]| {
        let h = [
            ("x-github-event", event),
            ("x-github-delivery", "d-1"),
            ("x-hub-signature-256", sig),
        ];
        ask(&at, "POST", "/hooks/github", &h, body)
    };
    assert_eq!(
        ask(&at, "POST", "/hooks/github", &[], pr.as_bytes()).status,
        401
    );
    assert_eq!(
        post(
            "pull_request",
            &signed("other", pr.as_bytes()),
            pr.as_bytes()
        )
        .status,
        401
    );
    let a = post(
        "pull_request",
        &signed("s3cret", pr.as_bytes()),
        pr.as_bytes(),
    );
    assert_eq!((a.status, a.json()), (202, json!({"kept": true})));
    assert_eq!(
        post("ping", &signed("s3cret", b"{}"), b"{}").json(),
        json!({"kept": false})
    );
    assert_eq!(
        post("check_run", &signed("s3cret", b"nope"), b"nope").status,
        400
    );
    let check =
        json!({"action": "completed", "check_run": {"head_sha": "def", "conclusion": "success",
        "pull_requests": [{"number": 8}]}})
        .to_string();
    assert_eq!(
        post(
            "check_run",
            &signed("s3cret", check.as_bytes()),
            check.as_bytes()
        )
        .status,
        202
    );
    let big = [
        ("x-hub-signature-256", "sha256=x"),
        ("content-length", "1000001"),
    ];
    assert_eq!(ask(&at, "POST", "/hooks/github", &big, b"").status, 413);

    let rows = ask(
        &at,
        "GET",
        "/hooks/envelopes",
        &[("authorization", "Bearer pull")],
        b"",
    )
    .json();
    let kept: Vec<&Value> = rows["envelopes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "envelope")
        .collect();
    let fields = |r: &Value| {
        let mut r = r.clone();
        r.as_object_mut()
            .unwrap()
            .retain(|k, _| k != "id" && k != "at");
        r
    };
    assert_eq!(
        fields(kept[0]),
        json!({"kind": "envelope", "forge": "github", "delivery": "d-1",
        "event": "pull_request", "action": "opened", "repo": "nha-org/site_perso", "number": 7, "head": "abc"})
    );
    assert_eq!(
        fields(kept[1]),
        json!({"kind": "envelope", "forge": "github", "delivery": "d-1",
        "event": "check_run", "action": "completed", "number": 8, "head": "def", "conclusion": "success"})
    );
}

#[dash_test]
fn hooks_envelopes_answers_after_a_cursor_behind_its_token() {
    assert_eq!(get(&up("pull-unset", &[]), "/hooks/envelopes").status, 404);
    let at = up("pull", &[("HOOKS_PULL_TOKEN", "pull")]);
    assert_eq!(
        ask(
            &at,
            "GET",
            "/hooks/envelopes",
            &[("authorization", "Bearer no")],
            b""
        )
        .status,
        401
    );
    let auth = [("authorization", "Bearer pull")];
    let all = ask(&at, "GET", "/hooks/envelopes?after=x", &auth, b"").json();
    assert_eq!(all["envelopes"][0]["kind"], "boot");
    assert_eq!(all["oldest"], 1);
    assert_eq!(all["epoch"].as_str().unwrap().len(), 12);
    assert_eq!(
        ask(&at, "GET", "/hooks/envelopes?after=1", &auth, b"").json()["envelopes"],
        json!([])
    );
}

#[dash_test]
fn the_queue_keeps_its_epoch_and_ids_across_a_start_and_prunes_old_rows_but_the_newest() {
    let dir = scratch("store");
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let old = "2000-01-01T00:00:00.000Z";
    let rows: String = (1..=3)
        .map(|id| format!("{}\n", json!({"id": id, "at": old, "kind": "boot"})))
        .collect();
    std::fs::write(dir.join("data/hooks.jsonl"), rows + "{\"id\": 4, \"at\n").unwrap();
    let auth = [("authorization", "Bearer pull")];
    let read = |up: &Up| ask(up, "GET", "/hooks/envelopes", &auth, b"").json();
    let first = read(&start(&dir, &[("HOOKS_PULL_TOKEN", "pull")]));
    // 1 and 2 pruned, 3 kept as the newest, the cut line skipped, this start's boot row 4
    let ids = |v: &Value| -> Vec<u64> {
        v["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_u64().unwrap())
            .collect()
    };
    assert_eq!((ids(&first), &first["oldest"]), (vec![3, 4], &json!(3)));
    let again = read(&start(&dir, &[("HOOKS_PULL_TOKEN", "pull")]));
    assert_eq!(again["epoch"], first["epoch"]);
    // 3 is no longer the newest: pruned now
    assert_eq!(ids(&again), vec![4, 5]);
}

#[dash_test]
fn a_body_trickled_a_byte_a_second_is_cut_at_its_deadline_unanswered() {
    let up = up("slow", &[("GITHUB_WEBHOOK_SECRET", "s3cret")]);
    let mut s = TcpStream::connect(&up.at).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let head =
        "POST /hooks/github HTTP/1.1\r\nhost: nicolas-ha.com\r\nx-hub-signature-256: sha256=x\r\n";
    s.write_all(format!("{head}content-length: 100\r\n\r\n").as_bytes())
        .unwrap();
    let began = std::time::Instant::now();
    // a byte every 900 ms: under any timeout a single read would have, so only the whole read's deadline cuts it
    let mut sent = 0;
    while s.write_all(b"x").is_ok() && sent < 100 && began.elapsed() < Duration::from_secs(15) {
        sent += 1;
        std::thread::sleep(Duration::from_millis(900));
        let mut probe = [0u8; 1];
        s.set_nonblocking(true).unwrap();
        let closed = matches!(s.read(&mut probe), Ok(0));
        s.set_nonblocking(false).unwrap();
        if closed {
            break;
        }
    }
    let mut rest = vec![];
    let _ = s.read_to_end(&mut rest);
    assert!(rest.is_empty(), "no answer to a body past its deadline");
    let took = began.elapsed();
    assert!(
        took >= Duration::from_secs(4) && took < Duration::from_secs(8),
        "cut after {took:?}, {sent} bytes"
    );
}

#[dash_test]
fn a_lost_queue_file_starts_its_ids_again_under_a_new_epoch() {
    let dir = scratch("lost");
    let auth = [("authorization", "Bearer pull")];
    let read = |up: &Up| ask(up, "GET", "/hooks/envelopes", &auth, b"").json();
    let first = read(&start(&dir, &[("HOOKS_PULL_TOKEN", "pull")]));
    std::fs::remove_file(dir.join("data/hooks.jsonl")).unwrap();
    let again = read(&start(&dir, &[("HOOKS_PULL_TOKEN", "pull")]));
    assert_eq!(again["oldest"], 1);
    assert_ne!(again["epoch"], first["epoch"]);
}
