//! The server: one request a connection, a thread each.
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::store::Store;

/// Fly's hard limit is 250 requests (fly.toml): past this many open connections a new one is closed unanswered.
const MAX_OPEN: usize = 256;
const HEAD_LIMIT: u64 = 16 * 1024;
/// The whole head's, however it trickles in (Deadline); a body's is HOOK_READ.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// HooksController's numbers: 15 real payloads were 10 to 34 KB; a forged signature costs a body, so 8 at once, 5 s each.
const HOOK_MAX: u64 = 1_000_000;
const HOOK_READS: usize = 8;
const HOOK_READ: Duration = Duration::from_secs(5);
const EVENTS: [&str; 4] = [
    "pull_request",
    "pull_request_review",
    "check_run",
    "check_suite",
];
const HSTS: &str = "max-age=31536000";
const JSON: &str = "application/json; charset=utf-8";

pub struct Site {
    /// site/'s folder.
    pub root: PathBuf,
    /// The bare domain `www.` redirects to.
    pub host: String,
    /// `CHAT_OPS_TOKEN`, `HOOKS_PULL_TOKEN`, `GITHUB_WEBHOOK_SECRET`: empty is unset.
    pub ops_token: String,
    pub pull_token: String,
    pub hook_secret: String,
    /// /proc on Linux: where /ops/relay finds the relay.
    pub proc: PathBuf,
    pub store: Mutex<Store>,
    reads: AtomicUsize,
}

impl Site {
    pub fn new(root: PathBuf, store: Store) -> Site {
        Site {
            root,
            host: "nicolas-ha.com".into(),
            ops_token: String::new(),
            pull_token: String::new(),
            hook_secret: String::new(),
            proc: "/proc".into(),
            store: Mutex::new(store),
            reads: AtomicUsize::new(0),
        }
    }
}

/// Answers every connection on `listener`, for ever.
pub fn serve(listener: TcpListener, site: Site) {
    let site = Arc::new(site);
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming().flatten() {
        let held = Held::take(&open);
        if open.load(Ordering::SeqCst) > MAX_OPEN {
            continue;
        }
        let site = site.clone();
        std::thread::spawn(move || {
            let _held = held;
            let _ = site.answer(stream);
        });
    }
}

/// One of a counter's slots, given back when dropped, a panic included.
struct Held(Arc<AtomicUsize>);

impl Held {
    fn take(n: &Arc<AtomicUsize>) -> Held {
        n.fetch_add(1, Ordering::SeqCst);
        Held(n.clone())
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Request {
    pub method: String,
    /// The raw path, query cut.
    pub path: String,
    pub query: String,
    /// Names in lower case, in order.
    pub headers: Vec<(String, String)>,
}

impl Request {
    fn header<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        self.all(name).next()
    }

    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.headers
            .iter()
            .filter(move |(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn param(&self, name: &str) -> Option<String> {
        self.query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k == name).then(|| decode(v).map(|b| String::from_utf8_lossy(&b).into_owned()))?
        })
    }

    /// Plug's path_info: the raw segments, empty ones dropped.
    fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }

    fn bearer(&self) -> &str {
        self.header("authorization")
            .and_then(|a| a.strip_prefix("Bearer "))
            .unwrap_or("")
    }
}

pub enum Body {
    Bytes(Vec<u8>),
    File(File, u64, u64),
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

impl Response {
    fn new(status: u16, kind: &str, body: impl Into<Vec<u8>>) -> Response {
        Response {
            status,
            headers: vec![("content-type", kind.into())],
            body: Body::Bytes(body.into()),
        }
    }

    fn json(status: u16, v: Value) -> Response {
        Response::new(status, JSON, v.to_string())
    }

    fn empty(status: u16) -> Response {
        Response {
            status,
            headers: vec![],
            body: Body::Bytes(vec![]),
        }
    }

    fn redirect(to: String, query: &str) -> Response {
        let to = if query.is_empty() {
            to
        } else {
            format!("{to}?{query}")
        };
        let mut r = Response::empty(301);
        r.headers.push(("location", to));
        r
    }
}

fn not_found() -> Response {
    Response::new(404, "text/html; charset=utf-8", "Not Found")
}

impl Site {
    fn answer(&self, stream: TcpStream) -> io::Result<()> {
        // ponytail: a reader slower than a byte a minute is cut; a video to a crawling phone gets that long a write
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let mut reader = BufReader::new(Deadline::new(stream.try_clone()?, HEAD_TIMEOUT));
        let Some(req) = read_head(&mut reader)? else {
            return write(
                &stream,
                "GET",
                Response::new(400, "text/plain", "Bad Request"),
            );
        };
        reader.get_mut().until = Instant::now() + HOOK_READ;
        let Some(resp) = self.route(&req, &mut reader) else {
            return Ok(());
        };
        eprintln!("{} {} {}", req.method, req.path, resp.status);
        write(&stream, &req.method, resp)?;
        // a body left unread (a refused delivery) would make the close a reset, which can drop the answer before the
        // client reads it: the write side ends first, then what is left is read away, 1 MB and 1 s at most
        stream.shutdown(std::net::Shutdown::Write)?;
        reader.get_mut().until = Instant::now() + Duration::from_secs(1);
        let _ = io::copy(&mut reader.take(HOOK_MAX), &mut io::sink());
        Ok(())
    }

    /// The answer to `req`; None closes the connection unanswered (a body too slow to read).
    pub fn route(&self, req: &Request, body: &mut impl Read) -> Option<Response> {
        if let Some(r) = self.files(req) {
            return Some(r);
        }
        let ops = |answer: &dyn Fn() -> Value| guarded(&self.ops_token, req, answer);
        Some(match (req.method.as_str(), &req.segments()[..]) {
            ("GET", ["ops", "chat"]) => ops(&|| json!({"records": []})),
            ("GET", ["ops", "relay"]) => {
                ops(&|| json!({"rss_bytes": rss("nanolathe-serve", &self.proc)}))
            }
            ("GET", ["hooks", "envelopes"]) => guarded(&self.pull_token, req, &|| {
                let after = req.param("after").and_then(|a| a.parse().ok()).unwrap_or(0);
                lock(&self.store).envelopes(after, 500)
            }),
            ("POST", ["hooks", "github"]) => return self.github(req, body),
            _ => not_found(),
        })
    }

    fn files(&self, req: &Request) -> Option<Response> {
        let host = req.header("host").unwrap_or("").to_ascii_lowercase();
        let host = host
            .rsplit_once(':')
            .filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit()))
            .map_or(host.as_str(), |(h, _)| h);
        if host.trim_end_matches('.') == format!("www.{}", self.host) {
            return Some(Response::redirect(
                format!("https://{}{}", self.host, req.path),
                &req.query,
            ));
        }
        if req.method != "GET" && req.method != "HEAD" {
            return None;
        }
        let raw = req.segments();
        let mut path = self.root.clone();
        for seg in &raw {
            let seg = String::from_utf8(decode(seg)?).ok()?;
            if [".", "..", "CLAUDE.md"].contains(&seg.as_str()) || seg.contains(['/', '\\', '\0']) {
                return None;
            }
            path.push(seg);
        }
        if path.is_file() {
            return Some(send(req, &path));
        }
        let index = path.join("index.html");
        if !index.is_file() {
            None
        } else if req.path.ends_with('/') {
            Some(send(req, &index))
        } else {
            // from the segments, not the raw path: "//public" would redirect to the host "public"
            Some(Response::redirect(
                format!("/{}/", raw.join("/")),
                &req.query,
            ))
        }
    }

    fn github(&self, req: &Request, body: &mut impl Read) -> Option<Response> {
        let fail = |status, why: &str| Some(Response::json(status, json!({"error": why})));
        if self.hook_secret.is_empty() {
            return fail(404, "not found");
        }
        let signature = req.header("x-hub-signature-256").unwrap_or("");
        if !signature.starts_with("sha256=") {
            return fail(401, "bad signature");
        }
        let Some(length) = req.header("content-length") else {
            return fail(411, "length required");
        };
        let length = match length.trim().parse::<u64>() {
            Ok(n) if n <= HOOK_MAX => n,
            _ => return fail(413, "too large"),
        };
        // registered before the count, so a burst cannot pass the check together
        self.reads.fetch_add(1, Ordering::SeqCst);
        let read = if self.reads.load(Ordering::SeqCst) > HOOK_READS {
            None
        } else {
            Some(read_within(body, length))
        };
        self.reads.fetch_sub(1, Ordering::SeqCst);
        let bytes = match read {
            None => return fail(503, "busy"),
            Some(None) => return None,
            Some(Some(b)) => b,
        };
        let want = format!("sha256={}", hex(&hmac(self.hook_secret.as_bytes(), &bytes)));
        if !same(signature.as_bytes(), want.as_bytes()) {
            return fail(401, "bad signature");
        }
        let event = req.header("x-github-event").unwrap_or("");
        if !EVENTS.contains(&event) {
            return Some(Response::json(202, json!({"kept": false})));
        }
        let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
            return fail(400, "not JSON");
        };
        let fields = envelope(event, req.header("x-github-delivery"), &payload);
        // not kept (the volume full): a failure GitHub shows, never a 202 for nothing
        Some(match lock(&self.store).envelope(fields) {
            Ok(()) => Response::json(202, json!({"kept": true})),
            Err(e) => {
                eprintln!("site: envelope not kept: {e}");
                Response::json(503, json!({"error": "not kept"}))
            }
        })
    }
}

/// What is kept of one delivery: ids and states, never a title, body, branch or author (R23).
pub fn envelope(event: &str, delivery: Option<&str>, p: &Value) -> Map<String, Value> {
    let pr = match &p["pull_request"] {
        Value::Null => &p[event]["pull_requests"][0],
        pr => pr,
    };
    let text = |v: &Value| v.as_str().map_or(Value::Null, |s| json!(s));
    let head = match pr["head"]["sha"].as_str() {
        Some(s) => json!(s),
        None => text(&p[event]["head_sha"]),
    };
    Map::from_iter([
        ("forge".into(), json!("github")),
        (
            "delivery".into(),
            delivery.map_or(Value::Null, |d| json!(d)),
        ),
        ("event".into(), json!(event)),
        ("action".into(), text(&p["action"])),
        ("repo".into(), text(&p["repository"]["full_name"])),
        (
            "number".into(),
            pr["number"].as_i64().map_or(Value::Null, |n| json!(n)),
        ),
        ("head".into(), head),
        ("conclusion".into(), text(&p[event]["conclusion"])),
    ])
}

fn guarded(token: &str, req: &Request, answer: &dyn Fn() -> Value) -> Response {
    if token.is_empty() {
        Response::json(404, json!({"error": "not found"}))
    } else if !same(req.bearer().as_bytes(), token.as_bytes()) {
        Response::json(401, json!({"error": "unauthorized"}))
    } else {
        Response::json(200, answer())
    }
}

fn lock(store: &Mutex<Store>) -> std::sync::MutexGuard<'_, Store> {
    store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A socket's reads with one deadline for them all: a timeout per read restarts on each byte, so a client trickling a
/// byte every 9 s would hold its thread and slot for hours (/code-review). Past it, a read fails as timed out.
struct Deadline {
    stream: TcpStream,
    until: Instant,
}

impl Deadline {
    fn new(stream: TcpStream, within: Duration) -> Deadline {
        Deadline {
            stream,
            until: Instant::now() + within,
        }
    }
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buf)
    }
}

/// `length` bytes, or None on a short read or past the connection's Deadline (HOOK_READ from the end of the head).
fn read_within(body: &mut impl Read, length: u64) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(length as usize);
    let mut chunk = [0u8; 16 * 1024];
    while (out.len() as u64) < length {
        let want = chunk.len().min((length - out.len() as u64) as usize);
        match body.read(&mut chunk[..want]) {
            Ok(0) | Err(_) => return None,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
        }
    }
    Some(out)
}

/// The resident bytes of the first process whose /proc/<pid>/comm is `comm`, or None.
pub fn rss(comm: &str, proc: &Path) -> Option<u64> {
    std::fs::read_dir(proc).ok()?.flatten().find_map(|e| {
        let dir = e.path();
        let name = std::fs::read_to_string(dir.join("comm")).ok()?;
        if name.trim() != comm {
            return None;
        }
        let status = std::fs::read_to_string(dir.join("status")).ok()?;
        let kb = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
        Some(
            kb.trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()?
                * 1024,
        )
    })
}

/// None on a bad escape, as URI.decode raised.
fn decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

fn read_head(r: &mut impl BufRead) -> io::Result<Option<Request>> {
    let mut lines = vec![];
    let mut limited = r.take(HEAD_LIMIT);
    loop {
        let mut line = String::new();
        if limited.read_line(&mut line)? == 0 || !line.ends_with('\n') {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        if line.is_empty() {
            break;
        }
        lines.push(line);
    }
    let mut first = lines.first().map(|l| l.split(' ')).into_iter().flatten();
    let (Some(method), Some(target)) = (first.next(), first.next()) else {
        return Ok(None);
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let headers = lines[1..]
        .iter()
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Ok(Some(Request {
        method: method.into(),
        path: path.into(),
        query: query.into(),
        headers,
    }))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        206 => "Partial Content",
        301 => "Moved Permanently",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        411 => "Length Required",
        413 => "Content Too Large",
        416 => "Range Not Satisfiable",
        _ => "Service Unavailable",
    }
}

fn write(mut stream: &TcpStream, method: &str, resp: Response) -> io::Result<()> {
    let length = match &resp.body {
        Body::Bytes(b) => b.len() as u64,
        Body::File(_, _, n) => *n,
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nstrict-transport-security: {HSTS}\r\nconnection: close\r\n",
        resp.status,
        reason(resp.status)
    );
    for (n, v) in &resp.headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    if resp.status != 304 {
        head.push_str(&format!("content-length: {length}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if method != "HEAD" {
        match resp.body {
            Body::Bytes(b) => stream.write_all(&b)?,
            Body::File(mut f, offset, n) => {
                f.seek(SeekFrom::Start(offset))?;
                io::copy(&mut f.take(n), &mut stream)?;
            }
        }
    }
    stream.flush()
}

/// The browser asks again each time, and an unchanged file answers 304 (Netlify's caching).
fn send(req: &Request, path: &Path) -> Response {
    let Ok(file) = File::open(path) else {
        return not_found();
    };
    let Ok(meta) = file.metadata() else {
        return not_found();
    };
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default();
    let mut h = DefaultHasher::new();
    (size, mtime.as_nanos()).hash(&mut h);
    let etag = format!("\"{}\"", h.finish() % 4_294_967_296);
    let kind = mime(path);
    let kind = if kind.starts_with("text/") {
        format!("{kind}; charset=utf-8")
    } else {
        kind.to_string()
    };
    let headers = vec![
        ("etag", etag.clone()),
        ("cache-control", "public, max-age=0, must-revalidate".into()),
        ("accept-ranges", "bytes".into()),
        ("content-type", kind),
    ];
    let with = |status, body| Response {
        status,
        headers: headers.clone(),
        body,
    };
    if req.all("if-none-match").any(|v| v == etag) {
        return with(304, Body::Bytes(vec![]));
    }
    // an If-Range naming another version (the file changed under a resumed download) gets the whole file (RFC 9110)
    let if_range: Vec<&str> = req.all("if-range").collect();
    let ranges: Vec<&str> = if if_range.is_empty() || if_range == [etag.as_str()] {
        req.all("range").collect()
    } else {
        vec![]
    };
    match range(&ranges, size) {
        Range::Part(first, last) => {
            let mut r = with(206, Body::File(file, first, last - first + 1));
            r.headers
                .push(("content-range", format!("bytes {first}-{last}/{size}")));
            r
        }
        Range::Unsatisfiable => {
            let mut r = with(416, Body::Bytes(vec![]));
            r.headers.push(("content-range", format!("bytes */{size}")));
            r
        }
        Range::All => with(200, Body::File(file, 0, size)),
    }
}

#[derive(Debug, PartialEq)]
pub enum Range {
    All,
    Part(u64, u64),
    Unsatisfiable,
}

/// One range of RFC 9110 ("bytes=0-1", "bytes=100-", "bytes=-500", the unit in any case), its last byte clipped to the
/// file; anything else (no header, several, another unit) gets the whole file.
pub fn range(headers: &[&str], size: u64) -> Range {
    let [header] = headers else { return Range::All };
    let h = header.trim();
    let Some(spec) = h
        .get(..5)
        .filter(|u| u.eq_ignore_ascii_case("bytes"))
        .map(|_| h[5..].trim_start())
    else {
        return Range::All;
    };
    let Some((a, b)) = spec.strip_prefix('=').and_then(|s| s.split_once('-')) else {
        return Range::All;
    };
    let (a, b) = (a.trim(), b.trim());
    let digits = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
    if !digits(a) || !digits(b) {
        return Range::All;
    }
    let num = |s: &str| s.parse::<u64>().unwrap_or(u64::MAX);
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Range::All,
        (true, false) => match num(b) {
            0 => Range::Unsatisfiable,
            _ if size == 0 => Range::Unsatisfiable,
            n => Range::Part(size.saturating_sub(n), size - 1),
        },
        _ => {
            let first = num(a);
            let last = if b.is_empty() {
                size.saturating_sub(1)
            } else {
                num(b)
            };
            if first >= size {
                Range::Unsatisfiable
            } else if first <= last {
                Range::Part(first, last.min(size - 1))
            } else {
                Range::All
            }
        }
    }
}

/// What the Elixir MIME library answered for site/'s kinds of file.
fn mime(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "text/javascript",
        "txt" => "text/plain",
        "xml" => "text/xml",
        "md" => "text/markdown",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",
        "mp4" => "video/mp4",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

pub fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |x: u8| k.iter().map(|b| b ^ x).collect::<Vec<u8>>();
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .to_vec()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant time over the shorter's length, as Plug.Crypto.secure_compare.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
