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
use dash_statsd::Statsd;
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

/// One slow or endless client must neither stall the serial loop nor grow its memory past the budget: a connection has this long
/// to its first byte, then the whole request has this long again, however its bytes trickle in.
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
        let (sender, live, report) = (sender.clone(), Arc::clone(&live), Arc::clone(&report));
        std::thread::spawn(move || accept_on_tailnet(bound, &sender, &live, &report));
    }
    let doors = Arc::clone(&report);
    std::thread::spawn(move || accept(&listener, &sender, &live, &doors));
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

/// One thread per connection, each over by four deadlines (its first byte, its read, the store's answer, its write): past this many at once, a
/// new one is closed and logged, so a client opening sockets cannot grow memory past the budget.
const CONNECTIONS: usize = 64;
/// The status tag of a connection closed for want of room: no answer, but a request refused all the same.
const CLOSED: &str = "closed";

fn accept(
    listener: &TcpListener,
    requests: &SyncSender<Request>,
    live: &Arc<AtomicUsize>,
    report: &Arc<Statsd>,
) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                // Out of file handles, say: the next try would fail at once, for as long as it lasts.
                eprintln!("no connection accepted: {e}");
                report.refused(CLOSED, "route_other");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let held = Held::take(live);
        if held.past_the_cap {
            eprintln!("past {CONNECTIONS} connections: one closed");
            report.refused(CLOSED, "route_other");
            continue;
        }
        let (requests, its_report) = (requests.clone(), Arc::clone(report));
        on_its_thread(std::thread::Builder::new(), report, move || {
            let _held = held;
            answer(
                &stream,
                &requests,
                &its_report,
                REQUEST_DEADLINE,
                ANSWER_DEADLINE,
            );
        });
    }
}

/// No thread to be had must not end the listener while the process stays up, where launchd would never restart it: `work`, its
/// connection and its slot, drops here, and that is a refused request.
fn on_its_thread(
    thread: std::thread::Builder,
    report: &Statsd,
    work: impl FnOnce() + Send + 'static,
) {
    if let Err(e) = thread.spawn(work) {
        eprintln!("no thread for a connection: {e}");
        report.refused(CLOSED, "route_other");
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

/// Each answer of 400 and over is counted and logged here, the one place they all pass. A connection that sent nothing gets no
/// answer: a browser's preconnect left open would show a 400 as its page (@nha, 2026-11-06: « had one "bad request" already fyi.
/// we need to monitor »).
fn answer(
    stream: &TcpStream,
    requests: &SyncSender<Request>,
    report: &Statsd,
    limit: Duration,
    wait: Duration,
) {
    if sent_nothing(stream, limit) {
        return;
    }
    let local = stream.local_addr().ok();
    let arrived = request_line(stream, limit);
    let route = route_step(match &arrived {
        Ok((line, _)) | Err(line) => line,
    });
    let response = arrived.ok().map(|(line, host)| match (host, local) {
        (Some(host), Some(local)) if allowed_host(&host, local.ip()) => {
            if asked_by_a_browser_itself(&line) {
                return ("200 OK", TEXT, String::new());
            }
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
    let sent = write!(
        timed,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .is_ok();
    // After the write: a log that fails costs a line, never the answer.
    if counts_as_refused(status, sent) {
        let code = &status[..3];
        report.refused(code, route);
        let gone = if sent { "" } else { ", the client gone" };
        eprintln!("answered {code} on {route}{gone}");
    }
}

/// A 4xx that reached nobody refused nobody: a browser cancelling mid-request, as with a silent close. A 5xx is this server's fault
/// whoever is left to read it. Every status is three digits, so the text compares as the number.
fn counts_as_refused(status: &str, sent: bool) -> bool {
    status >= "500" || (status >= "400" && sent)
}

/// What a browser fetches by itself, never a page's link: an empty 200 with no turn on the serial store, or each visit could count
/// as a refusal, a 404, or a 503 behind a slow /search.
fn asked_by_a_browser_itself(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let path = target.split('?').next().unwrap_or_default();
    method == Some("GET")
        && (path == "/favicon.ico"
            || path.starts_with("/apple-touch-icon")
            || path == "/.well-known/appspecific/com.chrome.devtools.json")
}

/// No byte by the limit, or the client gone first. Peeks, so the request stays to read. Any other error, a signal's for one, is left
/// for the read that follows: it retries, or fails into a counted 400.
fn sent_nothing(stream: &TcpStream, limit: Duration) -> bool {
    use std::io::ErrorKind::{
        ConnectionAborted, ConnectionReset, NotConnected, TimedOut, WouldBlock,
    };
    if stream.set_read_timeout(Some(limit)).is_err() {
        return true;
    }
    match stream.peek(&mut [0]) {
        Ok(n) => n == 0,
        Err(e) => matches!(
            e.kind(),
            TimedOut | WouldBlock | ConnectionReset | ConnectionAborted | NotConnected
        ),
    }
}

/// His tailnet address, never every interface: /search reads private text, and a Host header is whatever a client on his
/// Wi-Fi writes. Read again each minute: a new address (a re-login) ends the process for launchd to start it on that one, since
/// a listener in accept() cannot be told. Tailscale merely down keeps the listener.
fn accept_on_tailnet(
    port: u16,
    requests: &SyncSender<Request>,
    live: &Arc<AtomicUsize>,
    report: &Arc<Statsd>,
) {
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
                    let (requests, live, report) =
                        (requests.clone(), Arc::clone(live), Arc::clone(report));
                    std::thread::spawn(move || accept(&listener, &requests, &live, &report));
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
/// that can destroy the answer. Bytes that end before that line are a bad request: a client that gave up has no wrong Host.
/// `Err` holds what arrived of the first line, so a request cut short still names its route.
fn request_line(stream: &TcpStream, limit: Duration) -> Result<(String, Option<String>), String> {
    let timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let mut reader = BufReader::new(timed.take(REQUEST_LIMIT));
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return Err(first);
    }
    let (mut header, mut host) = (String::new(), None);
    loop {
        header.clear();
        match reader.read_line(&mut header) {
            Ok(n) if n > 0 => {}
            _ => return Err(first),
        }
        if header == "\r\n" || header == "\n" {
            return Ok((first, host));
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

    /// A report on a folder with no switch: it sends nothing.
    fn silent() -> Arc<Statsd> {
        Arc::new(Statsd::new(Path::new("/nonexistent"), "test"))
    }

    /// A report whose lines reach the socket it comes with. The folder holds the switch, so it stays as long as the report.
    fn reporting_to_a_listener(
        name: &str,
    ) -> (dash_meter::TestDir, std::net::UdpSocket, Arc<Statsd>) {
        let dir = dash_meter::TestDir::new(name);
        let port = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        port.set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let to = port.local_addr().expect("address").to_string();
        std::fs::write(dir.0.join("telemetry"), to).expect("the switch");
        let report = Arc::new(Statsd::new(&dir.0, "test"));
        (dir, port, report)
    }

    fn assert_a_closed_count(port: &std::net::UdpSocket) {
        let mut line = [0u8; 256];
        let n = port.recv(&mut line).expect("a count");
        let line = String::from_utf8_lossy(&line[..n]);
        assert!(line.contains("status:closed,route:route_other"), "{line}");
    }

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
        assert!(request_line(&server, Duration::from_millis(200)).is_err());
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
        std::thread::spawn(move || accept(&listener, &sender, &counted, &silent()));
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
        std::thread::spawn(move || accept(&listener, &sender, &live, &silent()));
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
        let (text, _) = answered(
            |client| write!(client, "GET /x HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write"),
            Duration::from_millis(100),
        );
        assert!(text.starts_with("HTTP/1.1 503"), "{text}");
    }

    /// `send` writes to the client, then `answer` runs with `wait` as its limit; what the client reads back.
    fn answered(send: impl FnOnce(&mut TcpStream), wait: Duration) -> (String, Duration) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let (sender, _requests) = sync_channel::<Request>(1);
        send(&mut client);
        let start = Instant::now();
        answer(&server, &sender, &silent(), wait, wait);
        let took = start.elapsed();
        drop(server);
        let mut text = String::new();
        let _ = client.read_to_string(&mut text);
        (text, took)
    }

    #[test]
    fn a_connection_that_sends_nothing_closes_with_no_answer_at_its_limit() {
        let _budget = TestBudget::new();
        let limit = Duration::from_millis(150);
        let (text, took) = answered(|_| {}, limit);
        assert_eq!(text, "", "silent to the limit");
        // Half: a kernel's timeout may wake a hair early, and the point is that it waited.
        assert!(
            took >= limit / 2 && took < Duration::from_secs(2),
            "{took:?}"
        );
        let (text, took) = answered(
            |client| client.shutdown(std::net::Shutdown::Write).expect("close"),
            Duration::from_secs(5),
        );
        assert_eq!(text, "", "closed before a byte");
        assert!(took < Duration::from_secs(2), "at once: {took:?}");
    }

    #[test]
    fn a_request_begun_and_never_finished_is_still_a_400() {
        let _budget = TestBudget::new();
        let limit = Duration::from_millis(150);
        let (text, _) = answered(|client| client.write_all(b"GET /").expect("write"), limit);
        assert!(text.starts_with("HTTP/1.1 400"), "{text}");
        let (text, _) = answered(
            |client| {
                client.write_all(b"GET /").expect("write");
                client.shutdown(std::net::Shutdown::Write).expect("close");
            },
            limit,
        );
        assert!(
            text.starts_with("HTTP/1.1 400"),
            "a client that gave up: {text}"
        );
    }

    #[test]
    fn a_preconnect_used_late_has_the_whole_limit_for_its_request() {
        let _budget = TestBudget::new();
        let (text, _) = answered(
            |client| {
                let mut late = client.try_clone().expect("clone");
                std::thread::spawn(move || {
                    let step = Duration::from_millis(600);
                    std::thread::sleep(step);
                    late.write_all(b"GET /x HTTP/1.1\r\n").expect("write");
                    std::thread::sleep(step);
                    late.write_all(b"Host: localhost\r\n\r\n").expect("write");
                });
            },
            Duration::from_secs(1),
        );
        assert!(
            text.starts_with("HTTP/1.1 503"),
            "read whole, then no store: {text}"
        );
    }

    #[test]
    fn a_4xx_nobody_read_is_not_counted_and_a_5xx_always_is() {
        let _budget = TestBudget::new();
        assert!(counts_as_refused("400 Bad Request", true));
        assert!(!counts_as_refused("400 Bad Request", false));
        assert!(counts_as_refused("503 Service Unavailable", false));
        assert!(!counts_as_refused("200 OK", true));
    }

    #[test]
    fn what_a_browser_fetches_by_itself_is_known_by_its_path_and_method() {
        let _budget = TestBudget::new();
        for line in [
            "GET /favicon.ico HTTP/1.1\r\n",
            "GET /apple-touch-icon-precomposed.png HTTP/1.1\r\n",
            "GET /.well-known/appspecific/com.chrome.devtools.json?x=1 HTTP/1.1\r\n",
        ] {
            assert!(asked_by_a_browser_itself(line), "{line}");
        }
        assert!(!asked_by_a_browser_itself("POST /favicon.ico HTTP/1.1\r\n"));
        assert!(!asked_by_a_browser_itself("GET /secret HTTP/1.1\r\n"));
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
        let (_dir, port, report) = reporting_to_a_listener("past-the-cap");
        std::thread::spawn(move || accept(&listener, &sender, &counted, &report));
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
        assert_a_closed_count(&port);
    }

    #[test]
    fn no_thread_to_be_had_counts_a_closed_connection_and_frees_its_slot() {
        let _budget = TestBudget::new();
        let (_dir, port, report) = reporting_to_a_listener("no-thread");
        let live = Arc::new(AtomicUsize::new(0));
        let held = Held::take(&live);
        // A stack no address space holds: the one way to make a spawn fail in a test.
        let thread = std::thread::Builder::new().stack_size(1 << 62);
        on_its_thread(thread, &report, move || {
            let _held = held;
            panic!("this work must not run");
        });
        assert_a_closed_count(&port);
        assert_eq!(live.load(Ordering::Relaxed), 0, "its slot is free");
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
