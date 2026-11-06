//! The one binary (section 3). `dash serve --data <dir>`, on loopback, prints `port <n>`, then answers GET /ping, /memory,
//! /scan, /search?q=<words> and /ui/<slot>. Plain std::net until the surface crate grows HTTP. On the same data folder:
//! `dash mcp` serves the registry's tools over stdio, `dash ingest --mime <key> <file>` runs an extractor and stores its
//! text, `dash extensions` lists every key with what serves it (section 11). `dash task show|run` and `dash orch` are the
//! orchestrator of section 20b: tasks.rs.
mod tasks;

use dash_core::registry::Registry;
use dash_core::{Store, ping};
use dash_ext as ext;
// Linked for what it submits to the registry: nothing here names it, and an unnamed crate is not linked at all.
use dash_ingest as _;
use dash_meter::memory::memory;
use dash_store as store;
use dash_surface as surface;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// One slow or endless client must neither stall the serial loop nor grow its memory past the budget: the whole request
/// has this long, however its bytes trickle in.
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const REQUEST_LIMIT: u64 = 64 * 1024;
/// Section 8's rerank window.
const SEARCH_LIMIT: u32 = 40;
const TEXT: &str = "text/plain; charset=utf-8";
const HTML: &str = "text/html; charset=utf-8";

/// Stdout is what a command answers: a port, a listing, a text, the MCP protocol. A crate's `println!` or a child that
/// inherits it would mix into it, so from here on the rest of the process writes to stderr and only the commands hold this.
fn claim_stdout() -> File {
    let answers = std::io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .expect("dup stdout");
    assert!(unsafe { libc::dup2(2, 1) } == 1, "stdout to stderr");
    File::from(answers)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = claim_stdout();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve", "--data", dir] => serve(PathBuf::from(dir), out),
        ["mcp", "--data", dir] => mcp(Path::new(dir), out),
        ["extensions", "--data", dir] => extensions(Path::new(dir), out),
        ["ingest", "--data", dir, "--mime", key, file] => ingest(Path::new(dir), key, file, out),
        ["task", "show", "--data", dir, id] => tasks::show(Path::new(dir), id, out),
        ["task", "run", "--data", dir, id] => tasks::run(Path::new(dir), id, None, out),
        ["task", "run", "--data", dir, id, "--account", account] => {
            tasks::run(Path::new(dir), id, Some(account), out)
        }
        ["orch", "--data", dir] => tasks::orchestrate(Path::new(dir), false, out),
        ["orch", "--data", dir, "--once"] => tasks::orchestrate(Path::new(dir), true, out),
        _ => {
            eprintln!(
                "usage: dash serve|mcp|extensions --data <dir>\n       dash ingest --data <dir> --mime <key> <file>\n       dash task show|run --data <dir> <id> [--account <name>]\n       dash orch --data <dir> [--once]"
            );
            std::process::exit(2);
        }
    }
}

/// The compiled-in entries, then extensions.d. Each refused file is answered, to name it; the app starts without it.
fn registry(dir: &Path) -> (Registry, Vec<String>) {
    let mut registry = Registry::with_compiled().expect("a key taken twice among our own crates");
    let refused = ext::load(&mut registry, dir);
    (registry, refused)
}

fn registry_naming_refusals(dir: &Path) -> Registry {
    let (registry, refused) = registry(dir);
    refused.iter().for_each(|r| eprintln!("refused {r}"));
    registry
}

fn extensions(dir: &Path, mut out: File) {
    let (registry, refused) = registry(dir);
    let lines = registry.listing().into_iter();
    for line in lines.chain(refused.iter().map(|r| format!("refused {r}"))) {
        writeln!(out, "{line}").expect("stdout");
    }
}

fn ingest(dir: &Path, key: &str, file: &str, mut out: File) {
    let (registry, refused) = registry(dir);
    refused.iter().for_each(|r| eprintln!("refused {r}"));
    if registry.extractors.get(key).is_none() {
        // A refused file may be the one that serves it: the default's copy would be stored for good.
        if !refused.is_empty() {
            eprintln!("{key}: no entry while a file is refused, so nothing is stored");
            std::process::exit(1);
        }
        eprintln!("{key}: no entry, the default serves it");
    }
    let done = std::fs::read(file)
        .map_err(|e| format!("{file}: {e}"))
        .and_then(|input| registry.extract(key, &input))
        .and_then(|text| {
            let db = store::open(dir).map_err(|e| e.to_string())?;
            let title = Path::new(file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            db.add_fact("file", &title, &text)
                .map_err(|e| e.to_string())?;
            Ok(text)
        });
    match done {
        Ok(text) => writeln!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("{key}: {e}");
            std::process::exit(1);
        }
    }
}

fn mcp(dir: &Path, out: File) {
    let registry = registry_naming_refusals(dir);
    surface::serve_mcp(&registry, std::io::stdin().lock(), out).expect("the MCP stream");
}

fn serve(dir: PathBuf, mut out: File) {
    let db = store::open(&dir).expect("open the store");
    let registry = registry_naming_refusals(&dir);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    writeln!(out, "port {port}").expect("stdout");
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let answer = request_line(&stream, REQUEST_DEADLINE).map(|(line, host)| {
            if host.as_deref().is_some_and(is_loopback_host) {
                route(&line, &db, &registry)
            } else {
                ("403 Forbidden", TEXT, "not a loopback host".into())
            }
        });
        let (status, kind, body) =
            answer.unwrap_or_else(|| ("400 Bad Request", TEXT, "bad request".into()));
        let mut timed = ByDeadline {
            stream: &stream,
            deadline: Instant::now() + REQUEST_DEADLINE,
        };
        let _ = write!(
            timed,
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

/// A stream whose reads, or writes, all end by one deadline: a timeout per call restarts at each byte.
struct ByDeadline<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for ByDeadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        (&*self.stream).read(buf)
    }
}

impl Write for ByDeadline<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_write_timeout(Some(left))?;
        (&*self.stream).write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// /search reads private text, and any web page can reach loopback by a DNS name of its own: a Host check stops that.
fn is_loopback_host(host: &str) -> bool {
    let name = host.rsplit_once(':').map_or(host, |(name, _port)| name);
    matches!(name, "127.0.0.1" | "localhost")
}

/// The request's first line and its Host, its headers read to the blank line: closing on unread bytes sends a reset
/// that can destroy the answer.
fn request_line(stream: &TcpStream, limit: Duration) -> Option<(String, Option<String>)> {
    let timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let mut reader = BufReader::new(timed.take(REQUEST_LIMIT));
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let (mut header, mut host) = (String::new(), None);
    loop {
        header.clear();
        let n = reader.read_line(&mut header).ok()?;
        if n == 0 || header == "\r\n" || header == "\n" {
            return Some((first, host));
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("host")
        {
            host = Some(value.trim().to_string());
        }
    }
}

/// `+` and `%XX` as a form sends them; a bad escape stays as it is.
fn form_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit)) // from_str_radix alone takes a leading +
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
                continue;
            }
            (b'+', _) => out.push(b' '),
            (byte, _) => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The store's hits, in the order the rerank hooks leave them, one `id<TAB>title` a line.
fn search(query: &str, db: &dyn Store, registry: &Registry) -> Result<String, String> {
    let (hits, refused) = registry.rerank(query, db.search(query, SEARCH_LIMIT)?);
    refused.iter().for_each(|r| eprintln!("{r}"));
    let lines: Vec<String> = hits
        .iter()
        .map(|h| format!("{}\t{}", h.id, h.title))
        .collect();
    Ok(lines.join("\n"))
}

type Response = (&'static str, &'static str, String);

fn route(line: &str, db: &dyn Store, registry: &Registry) -> Response {
    let failed = |e| ("500 Internal Server Error", TEXT, e);
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match (method, path) {
        (Some("GET"), "/ping") => ("200 OK", TEXT, ping().into()),
        (Some("GET"), "/memory") => {
            let m = memory();
            (
                "200 OK",
                TEXT,
                format!("{:.1} {:.1}", m.current_mb, m.peak_mb),
            )
        }
        (Some("GET"), "/scan") => match db.scan() {
            Ok(s) => ("200 OK", TEXT, format!("{} {}", s.rows, s.bytes)),
            Err(e) => failed(e),
        },
        (Some("GET"), "/search") => {
            let q = query.split('&').find_map(|pair| pair.strip_prefix("q="));
            match search(&form_decoded(q.unwrap_or_default()), db, registry) {
                Ok(lines) => ("200 OK", TEXT, lines),
                Err(e) => failed(e),
            }
        }
        (Some("GET"), slot) if slot.starts_with("/ui/") => {
            match registry.ui.get(&slot["/ui/".len()..]) {
                Some(tree) => match surface::render(tree) {
                    Ok(html) => ("200 OK", HTML, html),
                    Err(e) => failed(e),
                },
                None => ("404 Not Found", TEXT, "no such slot".into()),
            }
        }
        _ => ("404 Not Found", TEXT, "no such path".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use std::net::TcpListener;

    #[test]
    fn a_request_trickling_in_ends_at_its_deadline() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        std::thread::spawn(move || {
            // each byte well inside what a timeout per read would allow
            while client.write_all(b"x").is_ok() {
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        assert_eq!(request_line(&server, Duration::from_millis(200)), None);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_client_that_stops_reading_ends_at_the_deadline() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let _client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let mut timed = ByDeadline {
            stream: &server,
            deadline: Instant::now() + Duration::from_millis(200),
        };
        let piece = vec![b'x'; 64 * 1024];
        let start = Instant::now();
        while timed.write_all(&piece).is_ok() {}
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn only_a_loopback_name_passes_the_host_check() {
        let _budget = TestBudget::new();
        for good in ["127.0.0.1", "127.0.0.1:7788", "localhost", "localhost:1"] {
            assert!(is_loopback_host(good), "{good}");
        }
        for bad in [
            "evil.example",
            "evil.example:80",
            "127.0.0.1.evil.example",
            "",
        ] {
            assert!(!is_loopback_host(bad), "{bad}");
        }
    }

    #[test]
    fn a_form_s_query_decodes() {
        let _budget = TestBudget::new();
        assert_eq!(form_decoded("a+b%20c%C3%A9"), "a b c\u{e9}");
        assert_eq!(form_decoded("100%"), "100%");
        assert_eq!(form_decoded("%zz%2"), "%zz%2");
        assert_eq!(form_decoded("%+a"), "% a", "not a byte");
    }
}
