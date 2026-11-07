//! The one binary (section 3). `dash serve --data <dir>`, on loopback, prints `port <n>`, then answers GET /ping, /memory,
//! /scan, /search?q=<words>, /ui/<slot> and /deps, the crate and module graphs as built. With `--port <n>` it holds that port, on his tailnet address too, serves the tasks
//! list at / with the older app's queue, and `--data` defaults to the live folder. Plain std::net until the surface crate grows HTTP. On the same data folder:
//! `dash mcp` serves the registry's tools over stdio, `dash ingest --mime <key> <file>` runs an extractor and stores its
//! text, `dash extensions` lists every key with what serves it (section 11). `dash task show|run` and `dash orch` are the
//! orchestrator of section 20b: tasks.rs. `dash telemetry [on|off] --data <dir>` is the switch of the numbers they send.
mod reporting;
mod tasks;

use dash_core::registry::Registry;
use dash_core::{Store, ping};
use dash_ext as ext;
// Linked for what it submits to the registry: nothing here names it, and an unnamed crate is not linked at all.
use dash_ingest as _;
use dash_meter::cap::Capped;
use dash_meter::memory::memory;
use dash_statsd as statsd;
use dash_store as store;
use dash_surface as surface;
use reporting::{reporting, say_memory};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant};

#[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
#[global_allocator]
static ALLOCATOR: Capped<std::alloc::System> = Capped(std::alloc::System, ext::pod::kill_running);

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: Capped<mimalloc::MiMalloc> = Capped(mimalloc::MiMalloc, ext::pod::kill_running);

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: Capped<tikv_jemallocator::Jemalloc> =
    Capped(tikv_jemallocator::Jemalloc, ext::pod::kill_running);

/// One slow or endless client must neither stall the serial loop nor grow its memory past the budget: the whole request
/// has this long, however its bytes trickle in.
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
/// How long a connection waits on the serial store: past a slow /scan or /search, never a limit a route has to meet.
/// ponytail: an abandoned request still runs when its turn comes; a flag on it if slow searches pile up.
const ANSWER_DEADLINE: Duration = Duration::from_secs(60);
const REQUEST_LIMIT: u64 = 64 * 1024;
/// Tailscale down at a start, as at a boot, or the bind failing: tried again this often.
const TAILNET_RETRY: Duration = Duration::from_secs(60);
/// Section 8's rerank window.
const SEARCH_LIMIT: u32 = 40;
const TEXT: &str = "text/plain; charset=utf-8";
const HTML: &str = "text/html; charset=utf-8";
/// build.rs: dash_gate's text of this build's graphs.
const DEPS: &str = include_str!(concat!(env!("OUT_DIR"), "/deps.txt"));

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
    let began = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = claim_stdout();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve", "--data", dir] => serve(PathBuf::from(dir), None, began, out),
        ["serve", "--port", port] => serve(tasks::live_data(), Some(port_of(port)), began, out),
        ["serve", "--data", dir, "--port", port] => {
            serve(PathBuf::from(dir), Some(port_of(port)), began, out)
        }
        ["mcp", "--data", dir] => mcp(Path::new(dir), out),
        ["extensions", "--data", dir] => extensions(Path::new(dir), out),
        ["ingest", "--data", dir, "--mime", key, file] => ingest(Path::new(dir), key, file, out),
        ["task", "show", "--data", dir, id] => tasks::show(Path::new(dir), id, out),
        ["task", "run", "--data", dir, id] => tasks::run(Path::new(dir), id, None, out),
        ["task", "run", "--data", dir, id, "--account", account] => {
            tasks::run(Path::new(dir), id, Some(account), out)
        }
        ["orch", "--data", dir] => tasks::orchestrate(Path::new(dir), false, began, out),
        ["orch", "--data", dir, "--once"] => tasks::orchestrate(Path::new(dir), true, began, out),
        ["telemetry", "--data", dir] => telemetry(Path::new(dir), None, out),
        ["telemetry", "on", "--data", dir] => telemetry(Path::new(dir), Some(true), out),
        ["telemetry", "off", "--data", dir] => telemetry(Path::new(dir), Some(false), out),
        _ => {
            eprintln!(
                "usage: dash serve|mcp|extensions --data <dir>\n       dash serve [--data <dir>] --port <n>\n       dash ingest --data <dir> --mime <key> <file>\n       dash task show|run --data <dir> <id> [--account <name>]\n       dash orch --data <dir> [--once]\n       dash telemetry [on|off] --data <dir>"
            );
            std::process::exit(2);
        }
    }
}

fn port_of(word: &str) -> u16 {
    word.parse().unwrap_or_else(|_| {
        eprintln!("{word}: not a port");
        std::process::exit(2);
    })
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
    let report = reporting(dir, "ingest");
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
    say_memory(&report);
    match done {
        Ok(text) => writeln!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("{key}: {e}");
            std::process::exit(1);
        }
    }
}

fn mcp(dir: &Path, out: File) {
    let report = reporting(dir, "mcp");
    let registry = registry_naming_refusals(dir);
    surface::serve_mcp(&registry, std::io::stdin().lock(), out).expect("the MCP stream");
    say_memory(&report);
}

/// `dash telemetry`: the switch, saved, then what it now says.
fn telemetry(dir: &Path, on: Option<bool>, mut out: File) {
    if let Some(on) = on
        && let Err(e) = statsd::set(dir, on)
    {
        eprintln!("{}: {e}", dir.display());
        std::process::exit(1);
    }
    let state = statsd::endpoint(dir).map_or("off".into(), |to| format!("on, statsd to {to}"));
    writeln!(out, "telemetry: {state}").expect("stdout");
}

/// A request's step: the routes by name, so a client's paths never mint a metric. Only GET is routed.
fn route_step(line: &str) -> &'static str {
    let mut words = line.split_whitespace();
    if words.next() != Some("GET") {
        return "route_other";
    }
    let target = words.next().unwrap_or_default();
    match target.split('?').next().unwrap_or_default() {
        "/" => "route_page",
        "/ping" => "route_ping",
        "/memory" => "route_memory",
        "/scan" => "route_scan",
        "/search" => "route_search",
        path if path.starts_with("/ui/") => "route_ui",
        _ => "route_other",
    }
}

fn serve(dir: PathBuf, port: Option<u16>, began: Instant, mut out: File) {
    let report = reporting(&dir, "serve");
    let db = report.step("open_store", || store::open(&dir).expect("open the store"));
    let registry = report.step("registry", || registry_naming_refusals(&dir));
    let board = report.step("board", || {
        port.map(|_| tasks::Board::open(&dir).expect("open the tasks"))
    });
    let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(0))).expect("bind");
    let bound = listener.local_addr().expect("address").port();
    writeln!(out, "port {bound}").expect("stdout");
    // A connection is read and written on its own thread, so an idle browser socket never holds the next request. The store stays
    // on this one, which only routes.
    let (sender, requests) = sync_channel::<Request>(CONNECTIONS);
    let live = Arc::new(AtomicUsize::new(0));
    if port.is_some() {
        let (sender, live) = (sender.clone(), Arc::clone(&live));
        std::thread::spawn(move || accept_on_tailnet(bound, &sender, &live));
    }
    std::thread::spawn(move || accept(&listener, &sender, &live));
    report.started(began);
    for (line, queued, reply) in requests {
        let response = route(&line, &db, &registry, board.as_ref());
        let took = queued.elapsed();
        // After the reply: no client waits on its own number.
        let _ = reply.send(response);
        report.step_took(route_step(&line), took);
    }
}

/// A request's first line, when it joined the queue, and where its answer goes. A route's time starts at the queue: a read held
/// behind a slow one is the delay these numbers are there to show.
type Request = (String, Instant, SyncSender<Response>);

/// One thread per connection, each over by three deadlines (its read, the store's answer, its write): past this many at once, a
/// new one is closed and logged, so a client opening sockets cannot grow memory past the budget.
const CONNECTIONS: usize = 64;

fn accept(listener: &TcpListener, requests: &SyncSender<Request>, live: &Arc<AtomicUsize>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            // Out of file handles, say: the next try would fail at once, for as long as it lasts.
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let held = Held::take(live);
        if held.past_the_cap {
            eprintln!("past {CONNECTIONS} connections: one closed");
            continue;
        }
        let requests = requests.clone();
        let started = std::thread::Builder::new().spawn(move || {
            let _held = held;
            answer(&stream, &requests, REQUEST_DEADLINE, ANSWER_DEADLINE);
        });
        // No thread to be had must not end this listener while the process stays up, where launchd would never restart it.
        if let Err(e) = started {
            eprintln!("no thread for a connection: {e}");
        }
    }
}

/// A connection's slot, freed when it drops, a panic included: a leaked slot would refuse every connection in the end, with the
/// process still up, where launchd would never restart it.
struct Held {
    live: Arc<AtomicUsize>,
    past_the_cap: bool,
}

impl Held {
    fn take(live: &Arc<AtomicUsize>) -> Held {
        Held {
            live: Arc::clone(live),
            past_the_cap: live.fetch_add(1, Ordering::Relaxed) >= CONNECTIONS,
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

fn answer(stream: &TcpStream, requests: &SyncSender<Request>, limit: Duration, wait: Duration) {
    let local = stream.local_addr().ok();
    let response = request_line(stream, limit).map(|(line, host)| match (host, local) {
        (Some(host), Some(local)) if allowed_host(&host, local.ip()) => {
            let (reply, answered) = sync_channel(1);
            let routed = requests
                .try_send((line, Instant::now(), reply))
                .ok()
                .and_then(|()| answered.recv_timeout(wait).ok());
            routed.unwrap_or(("503 Service Unavailable", TEXT, "busy".into()))
        }
        _ => ("403 Forbidden", TEXT, "not this server's host".into()),
    });
    let (status, kind, body) =
        response.unwrap_or_else(|| ("400 Bad Request", TEXT, "bad request".into()));
    let mut timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let _ = write!(
        timed,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// His tailnet address, never every interface: /search reads private text, and a Host header is whatever a client on his
/// Wi-Fi writes. Read again each minute: a new address (a re-login) ends the process for launchd to start it on that one, since
/// a listener in accept() cannot be told. Tailscale merely down keeps the listener.
fn accept_on_tailnet(port: u16, requests: &SyncSender<Request>, live: &Arc<AtomicUsize>) {
    let mut bound = None;
    loop {
        let ifconfig = std::process::Command::new("/sbin/ifconfig").output();
        let text = ifconfig.map_or(String::new(), |o| {
            String::from_utf8_lossy(&o.stdout).into_owned()
        });
        let here = tailnet_in(&text);
        if tailnet_moved(bound, here) {
            eprintln!(
                "the tailnet's address moved from {bound:?} to {here:?}: ending, for a restart"
            );
            std::process::exit(1);
        }
        if let (None, Some(ip)) = (bound, here) {
            match TcpListener::bind((ip, port)) {
                Ok(listener) => {
                    eprintln!("on {}", listener.local_addr().expect("address"));
                    bound = Some(ip);
                    let (requests, live) = (requests.clone(), Arc::clone(live));
                    std::thread::spawn(move || accept(&listener, &requests, &live));
                }
                Err(e) => eprintln!("the tailnet's port {port}: {e}"),
            }
        }
        std::thread::sleep(TAILNET_RETRY);
    }
}

fn tailnet_moved(bound: Option<Ipv4Addr>, here: Option<Ipv4Addr>) -> bool {
    matches!((bound, here), (Some(old), Some(new)) if old != new)
}

/// This Mac's Tailscale address: a 100.64.0.0/10 one on a `utun` interface. A CGNAT Wi-Fi or hotspot gives en0 a 100.x too, and
/// binding there would open this to that network (as `Dashboard.Tailnet`).
fn tailnet_in(ifconfig: &str) -> Option<Ipv4Addr> {
    let mut on_utun = false;
    for line in ifconfig.lines() {
        if !line.starts_with(char::is_whitespace) {
            on_utun = line.starts_with("utun");
        } else if on_utun
            && let Some(rest) = line.trim_start().strip_prefix("inet ")
            && let Some(Ok(ip)) = rest.split_whitespace().next().map(str::parse::<Ipv4Addr>)
            && ip.octets()[0] == 100
            && (64..128).contains(&ip.octets()[1])
        {
            return Some(ip);
        }
    }
    None
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
    matches!(host_name(host), "127.0.0.1" | "localhost")
}

fn host_name(host: &str) -> &str {
    host.rsplit_once(':').map_or(host, |(name, _port)| name)
}

/// A connection names the address it came in on: loopback's names on loopback, the tailnet's own address there.
fn allowed_host(host: &str, local: IpAddr) -> bool {
    if local.is_loopback() {
        is_loopback_host(host)
    } else {
        host_name(host) == local.to_string()
    }
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

fn route(
    line: &str,
    db: &dyn Store,
    registry: &Registry,
    board: Option<&tasks::Board>,
) -> Response {
    let failed = |e| ("500 Internal Server Error", TEXT, e);
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match (method, path) {
        (Some("GET"), "/") => match board.map(tasks::Board::page) {
            Some(Ok(text)) => ("200 OK", TEXT, text),
            Some(Err(e)) => failed(e),
            None => ("404 Not Found", TEXT, "no port, no page".into()),
        },
        (Some("GET"), "/deps") => ("200 OK", TEXT, DEPS.into()),
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
    fn a_connection_must_name_the_address_it_came_in_on() {
        let _budget = TestBudget::new();
        let loopback: IpAddr = "127.0.0.1".parse().expect("ip");
        let tailnet: IpAddr = "100.72.158.115".parse().expect("ip");
        for good in ["100.72.158.115", "100.72.158.115:7799"] {
            assert!(allowed_host(good, tailnet), "{good}");
            assert!(!allowed_host(good, loopback), "{good}");
        }
        for good in ["127.0.0.1:7799", "localhost:7799"] {
            assert!(allowed_host(good, loopback), "{good}");
            assert!(!allowed_host(good, tailnet), "{good}");
        }
        for bad in [
            "100.72.158.116:7799",
            "evil.example:7799",
            "100.72.158.115.evil.example",
            "",
        ] {
            assert!(!allowed_host(bad, tailnet), "{bad}");
            assert!(!allowed_host(bad, loopback), "{bad}");
        }
    }

    #[test]
    fn only_a_utun_tailscale_address_is_the_tailnet() {
        let _budget = TestBudget::new();
        let hotspot = "en0: flags=8863<UP>\n\tinet 100.72.1.1 netmask 0xffffff00\nutun3: flags=8051<UP>\n\tinet6 fe80::1 prefixlen 64\n";
        assert_eq!(
            tailnet_in(hotspot),
            None,
            "a CGNAT Wi-Fi is not his tailnet"
        );
        let live = format!(
            "{hotspot}utun4: flags=8051<UP>\n\tinet 100.72.158.115 --> 100.72.158.115 netmask 0xffffffff\n"
        );
        assert_eq!(tailnet_in(&live), Some(Ipv4Addr::new(100, 72, 158, 115)));
        let outside = "utun5: flags=8051<UP>\n\tinet 100.128.0.1 --> 100.128.0.1\n";
        assert_eq!(tailnet_in(outside), None, "outside 100.64.0.0/10");
    }

    #[test]
    fn an_idle_connection_holds_no_other_request() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, requests) = sync_channel::<Request>(16);
        let live = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&live);
        std::thread::spawn(move || accept(&listener, &sender, &counted));
        std::thread::spawn(move || {
            for (_, _, reply) in requests {
                let _ = reply.send(("200 OK", TEXT, "ok".into()));
            }
        });
        // as a browser's preconnect: open, sends nothing
        let _idle = TcpStream::connect(address).expect("connect");
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        let mut stream = TcpStream::connect(address).expect("connect");
        write!(stream, "GET /x HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
        let mut text = String::new();
        stream.read_to_string(&mut text).expect("read");
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_request_s_time_starts_when_it_joins_the_queue() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, requests) = sync_channel::<Request>(1);
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || accept(&listener, &sender, &live));
        let mut stream = TcpStream::connect(address).expect("connect");
        write!(stream, "GET /x HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
        let (_, queued, reply) = requests.recv().expect("the request");
        let before = queued.elapsed();
        std::thread::sleep(Duration::from_millis(50));
        assert!(queued.elapsed() >= before + Duration::from_millis(50));
        let _ = reply.send(("200 OK", TEXT, "ok".into()));
    }

    #[test]
    fn a_store_that_does_not_answer_is_a_503_by_the_deadline() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let (sender, _requests) = sync_channel::<Request>(1);
        write!(client, "GET /x HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
        let wait = Duration::from_millis(100);
        answer(&server, &sender, wait, wait);
        drop(server);
        let mut text = String::new();
        client.read_to_string(&mut text).expect("read");
        assert!(text.starts_with("HTTP/1.1 503"), "{text}");
    }

    #[test]
    fn a_panic_frees_the_connection_slot() {
        let _budget = TestBudget::new();
        let live = Arc::new(AtomicUsize::new(0));
        let held = Held::take(&live);
        assert_eq!(live.load(Ordering::Relaxed), 1);
        let died = std::thread::spawn(move || {
            let _held = held;
            panic!("a connection's panic, on purpose");
        });
        assert!(died.join().is_err());
        assert_eq!(live.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn only_a_new_address_moves_the_tailnet() {
        let _budget = TestBudget::new();
        let (a, b) = (Ipv4Addr::new(100, 64, 0, 1), Ipv4Addr::new(100, 64, 0, 2));
        assert!(tailnet_moved(Some(a), Some(b)));
        for (bound, here) in [
            (Some(a), Some(a)),
            (Some(a), None),
            (None, Some(a)),
            (None, None),
        ] {
            assert!(!tailnet_moved(bound, here), "{bound:?} {here:?}");
        }
    }

    #[test]
    fn past_the_cap_a_connection_is_closed() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, _requests) = sync_channel::<Request>(1);
        let live = Arc::new(AtomicUsize::new(CONNECTIONS));
        let counted = Arc::clone(&live);
        std::thread::spawn(move || accept(&listener, &sender, &counted));
        let mut stream = TcpStream::connect(address).expect("connect");
        let mut text = String::new();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        assert_eq!(
            stream.read_to_string(&mut text).expect("closed, no answer"),
            0
        );
        assert_eq!(live.load(Ordering::Relaxed), CONNECTIONS, "as it was");
    }

    #[test]
    fn a_client_s_path_never_names_a_step() {
        let _budget = TestBudget::new();
        for (line, step) in [
            ("GET / HTTP/1.1\r\n", "route_page"),
            ("GET /search?q=a HTTP/1.1\r\n", "route_search"),
            ("GET /ui/menu HTTP/1.1\r\n", "route_ui"),
            ("GET /whatever/they/like HTTP/1.1\r\n", "route_other"),
            ("POST /search HTTP/1.1\r\n", "route_other"),
            ("GET\t/ping HTTP/1.1\r\n", "route_ping"),
            ("", "route_other"),
        ] {
            assert_eq!(route_step(line), step, "{line}");
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
