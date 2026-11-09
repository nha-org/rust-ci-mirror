//! The one binary (section 3). `dash serve --data <dir>`, on loopback, prints `port <n>`, then answers GET /ping, /memory,
//! /scan, /search?q=<words>, /ui/<slot> and /deps, the crate and module graphs as built. With `--port <n>` it holds that port, on his tailnet address too, serves Home
//! at / (what waits on him, today's daily, what runs), the tasks at /tasks as the board (?layout=list the list, ?layout=graph the
//! graph, also at /graph), each task at /tasks/<id>, the code map at /deps, every access at /tools (`dash access` prints the same: access.rs), and `--data` defaults to the live folder; `--older <n>` makes it the front door, every other path the older app's on loopback port n. `dash ui` opens the app in the Mac window, each page rendered in its process from the same folder, no server under it. Plain std::net until the surface crate grows HTTP. On the same data folder:
//! `dash mcp` serves the registry's tools over stdio, `dash ingest --mime <key> <file>` runs an extractor and stores its
//! text, `dash extensions` lists every key with what serves it (section 11). `dash task add|waits|close|show|run` and `dash orch` are the
//! orchestrator of section 20b: tasks.rs, with `dash claude`, one agent isolated by MXC in a pane. `dash telemetry [on|off] --data <dir>` is the switch of the numbers they send. `dash draft` keeps each draft and its outcome:
//! drafts.rs. `dash comment pass` runs one pass of the comment queue `dash orch` runs every 30 s, `dash comment work` one
//! comment of its agent's: comments.rs.
mod access;
mod agents;
mod alerts;
mod comments;
mod dmarc;
mod drafts;
mod fresh;
mod home;
mod keys;
mod kinds;
mod mail;
mod morning;
mod proxy;
mod reporting;
mod tasks;
mod today;
mod watchers;

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
use dash_surface::{self as surface, TaskView};
use reporting::{armed, reporting, say_memory};
use serde_json::json;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
#[global_allocator]
static ALLOCATOR: Capped<std::alloc::System> = Capped(std::alloc::System, reporting::at_stop);

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: Capped<mimalloc::MiMalloc> = Capped(mimalloc::MiMalloc, reporting::at_stop);

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: Capped<tikv_jemallocator::Jemalloc> =
    Capped(tikv_jemallocator::Jemalloc, reporting::at_stop);

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
/// The crate graph as Mermaid, nested by layer: what /deps draws above its text (task rust-deps-view).
const DEPS_DIAGRAM: &str = include_str!(concat!(env!("OUT_DIR"), "/deps.mmd"));

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
    // A stop at the cap names its command only once armed: a new command calls `reporting` first, or takes its folder by `armed`.
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve", "--data", dir] => serve(PathBuf::from(dir), None, began, out),
        ["serve", "--port", port] => serve(tasks::live_data(), Some(port_of(port)), began, out),
        ["serve", "--data", dir, "--port", port] => {
            serve(PathBuf::from(dir), Some(port_of(port)), began, out)
        }
        ["serve", "--port", port, "--older", older] => {
            let _ = OLDER.set(port_of(older));
            serve(tasks::live_data(), Some(port_of(port)), began, out)
        }
        ["serve", "--data", dir, "--port", port, "--older", older] => {
            let _ = OLDER.set(port_of(older));
            serve(PathBuf::from(dir), Some(port_of(port)), began, out)
        }
        ["ui"] => ui(tasks::live_data()),
        ["ui", "--data", dir] => ui(PathBuf::from(dir)),
        ["ui", "--shot", target, png] => ui_shot(tasks::live_data(), target, Path::new(png)),
        ["ui", "--data", dir, "--shot", target, png] => {
            ui_shot(PathBuf::from(dir), target, Path::new(png))
        }
        ["ui", "--design"] => design_window(),
        ["ui", "--check", png] => ui_check(Path::new(png), false),
        ["ui", "--check", png, "--stop-front-door"] => ui_check(Path::new(png), true),
        ["mcp", "--data", dir] => mcp(Path::new(dir), out),
        ["mcp-proxy", "--cache", cache, "--", ref server @ ..] if !server.is_empty() => {
            let server: Vec<String> = server.iter().map(|w| w.to_string()).collect();
            let input = std::io::stdin().lock();
            proxy::serve(Path::new(cache), &server, input, out).expect("the MCP stream");
        }
        ["extensions", "--data", dir] => extensions(armed(dir, "extensions"), out),
        ["ingest", "--data", dir, "--mime", key, file] => ingest(Path::new(dir), key, file, out),
        ["task", "add", "--data", dir, id] => tasks::add(armed(dir, "add"), id, "", out),
        ["task", "add", "--data", dir, id, "--waits-on", waits] => {
            tasks::add(armed(dir, "add"), id, waits, out)
        }
        ["task", "waits", "--data", dir, id, "--on", waits] => {
            tasks::waits(armed(dir, "waits"), id, waits, out)
        }
        ["task", "close", "--data", dir, id, why] => {
            tasks::close(armed(dir, "close"), id, why, out)
        }
        ["task", "show", "--data", dir, id] => tasks::show(armed(dir, "show"), id, out),
        ["task", "run", "--data", dir, id] => tasks::run(Path::new(dir), id, None, out),
        ["task", "run", "--data", dir, id, "--account", account] => {
            tasks::run(Path::new(dir), id, Some(account), out)
        }
        ["access"] => access::list(out),
        ["alerts", "--data", dir] => alerts::once(Path::new(dir), out),
        ["watchers", "--data", dir] => {
            watchers::list(Path::new(dir), &comments::fed(Path::new(dir)), out)
        }
        ["agents", ref rest @ ..] => keys_command(rest, out),
        ["login", ref rest @ ..] => agents::login(rest, out),
        ["limits"] => tasks::limits(out),
        ["dmarc", file] => dmarc::run(file, out),
        ["claude"] => tasks::claude(&tasks::live_data(), out),
        ["claude", "--data", dir] => tasks::claude(Path::new(dir), out),
        ["claude", "--in-pane", "--data", dir] => tasks::claude_in_pane(Path::new(dir)),
        ["orch", "--data", dir] => {
            comments::watch(PathBuf::from(dir));
            alerts::watch(PathBuf::from(dir));
            morning::watch(PathBuf::from(dir));
            comments::work(PathBuf::from(dir));
            tasks::orchestrate(Path::new(dir), false, began, out)
        }
        ["orch", "--data", dir, "--once"] => tasks::orchestrate(Path::new(dir), true, began, out),
        ["orch", "--data", dir, "--sync"] => tasks::sync_once(Path::new(dir), out),
        ["orch", "--data", dir, "--reap"] => tasks::reap_once(Path::new(dir), out),
        ["draft", "import", "--data", dir, "--from", from] => {
            drafts::import(armed(dir, "draft"), Path::new(from), out)
        }
        ["draft", "new", "--data", dir, "--file", file] => {
            drafts::new(armed(dir, "draft"), Path::new(file), None, out)
        }
        [
            "draft",
            "new",
            "--data",
            dir,
            "--file",
            file,
            "--from",
            from,
        ] => drafts::new(
            armed(dir, "draft"),
            Path::new(file),
            Some(Path::new(from)),
            out,
        ),
        ["draft", "show", "--data", dir, id] => drafts::show(armed(dir, "draft"), id, out),
        ["draft", "rates", "--data", dir] => drafts::rates(armed(dir, "draft"), out),
        ["comment", "pass", "--data", dir] => comments::once(armed(dir, "comment"), out),
        ["comment", "work", "--data", dir] => comments::work_one(armed(dir, "comment"), out),
        ["telemetry", "--data", dir] => telemetry(armed(dir, "telemetry"), None, out),
        ["telemetry", "on", "--data", dir] => telemetry(armed(dir, "telemetry"), Some(true), out),
        ["telemetry", "off", "--data", dir] => telemetry(armed(dir, "telemetry"), Some(false), out),
        _ => usage(),
    }
}

/// `dash agents`, on the live data unless `--data <dir>` names another folder, wherever it sits.
fn keys_command(args: &[&str], out: File) {
    let mut args = args.to_vec();
    let live = tasks::live_data().display().to_string();
    let dir = match args.iter().position(|a| *a == "--data") {
        Some(at) if at + 1 < args.len() => args.drain(at..at + 2).nth(1).unwrap_or_default(),
        Some(_) => usage(),
        None => &live,
    };
    let dir = armed(dir, "agents");
    let tools = &keys::live_tools(&tasks::repo());
    match args[..] {
        [] => keys::list(dir, out),
        ["key", "add", kind, name] => keys::add(dir, tools, kind, name, None, out),
        ["key", "add", kind, name, "--cap", cap] => {
            keys::add(dir, tools, kind, name, Some(cap), out)
        }
        ["key", "rm", kind] => keys::rm(dir, kind, out),
        ["key", "run", kind, prompt] => keys::run(dir, tools, kind, prompt, out),
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: dash serve|mcp|extensions --data <dir>\n       dash mcp-proxy --cache <file> -- <server> [<argument>...]\n       dash serve [--data <dir>] --port <n> [--older <n>]\n       dash ui [--data <dir>] [--shot <page> <png>]|--design|--check <png> [--stop-front-door]\n       dash ingest --data <dir> --mime <key> <file>\n       dash task add --data <dir> <id> [--waits-on <ids, a comma between>]\n       dash task waits --data <dir> <id> --on <ids, a comma between>\n       dash task close --data <dir> <id> <why>\n       dash task show|run --data <dir> <id> [--account <name>]\n       dash orch --data <dir> [--once|--sync|--reap]\n       dash claude [--data <dir>]\n       dash limits\n       dash dmarc <file of domains, one a line>\n       dash telemetry [on|off] --data <dir>\n       dash alerts --data <dir>\n       dash draft import --data <dir> --from <export>\n       dash draft new --data <dir> --file <labelled.md> [--from <export>]\n       dash draft show --data <dir> <id>\n       dash draft rates --data <dir>\n       dash comment pass|work --data <dir>\n       dash agents [--data <dir>]\n       dash agents key add <kind> <secret name> [--data <dir>] [--cap <usd>]\n       dash agents key rm|run <kind> [<prompt>] [--data <dir>]\n       dash login add <name> [--dry-run]|sign-in <name>|status|remove <name>\n       dash access"
    );
    std::process::exit(2);
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
    // the daily's « today » lines (task rust-daily-today): each plugin's own, the page reads them by slot
    for (origin, slot) in [("drafts", drafts::slot(dir)), ("mail", mail::slot())] {
        registry
            .add(origin, slot)
            .expect("a slot taken twice among our own crates");
    }
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
    let done = dash_ingest::read_file(file)
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
        "/tasks" => "route_tasks",
        "/inbox" => "route_inbox",
        "/plan" => "route_plan",
        "/insights" => "route_insights",
        "/graph" => "route_graph",
        "/deps" => "route_deps",
        "/agents" => "route_agents",
        "/comments" => "route_comments",
        "/tools" => "route_tools",
        "/watchers" => "route_watchers",
        "/drafts" => "route_drafts",
        "/today" => "route_today",
        "/design" => "route_design",
        "/mail" => "route_mail",
        path if path.starts_with("/mail/t/") => "route_mail_thread",
        path if path.starts_with("/tasks/") => "route_task",
        path if path.starts_with("/d/") => "route_draft",
        path if path.starts_with("/projects/") => "route_project",
        "/ping" => "route_ping",
        "/built" => "route_built",
        "/memory" => "route_memory",
        "/scan" => "route_scan",
        "/search" => "route_search",
        "/compass" => "route_compass",
        path if path.starts_with("/ui/") => "route_ui",
        _ => "route_other",
    }
}

fn serve(dir: PathBuf, port: Option<u16>, began: Instant, mut out: File) {
    let report = reporting(&dir, "serve");
    comments::serve_from(&dir);
    let db = report.step("open_store", || store::open(&dir).expect("open the store"));
    let registry = report.step("registry", || registry_naming_refusals(&dir));
    let board = report.step("board", || {
        port.map(|_| tasks::Board::open(&dir).expect("open the tasks"))
    });
    if let Some(board) = board {
        let _ = BOARD.set(Mutex::new(board));
    }
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
    for (line, accept, queued, reply) in requests {
        let response = route(&line, &accept, &db, &registry);
        let took = queued.elapsed();
        // After the reply: no client waits on its own number.
        let _ = reply.send(response);
        report.step_took(route_step(&line), took);
    }
}

/// The board, once `serve --port` opened it: the pages that read it lock it on their connection's thread.
static BOARD: std::sync::OnceLock<Mutex<tasks::Board>> = std::sync::OnceLock::new();
/// What never reads the board, so never waits on its lock: /mail on its server (10 s at most) holds no board page.
const NO_BOARD: [&str; 14] = [
    "/",
    "/inbox",
    "/plan",
    "/insights",
    "/ping",
    "/built",
    "/memory",
    "/deps",
    "/design",
    "/agents",
    "/tools",
    "/watchers",
    "/comments",
    "/mail",
];

/// What needs neither the store nor the registry, /ping, /memory and every page, answered on its connection's own thread: none
/// waits behind a slow /scan or /search on the serial one (rust/CLAUDE.md rule 5: one slow request never stalls the others).
/// Any other page locks the board, so a new board page never reads « no port » for want of a line here. `None`: the serial
/// route's.
/// ponytail: one board lock, its pages reading polled caches; a lock per read if a board page grows slow.
fn unqueued(line: &str, accept: &str, board: Option<&Mutex<tasks::Board>>) -> Option<Response> {
    let mut words = line.split_whitespace();
    if words.next() != Some("GET") {
        return None;
    }
    let target = words.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    // a draft's page reads the older app, never the board (task rust-draft-approve)
    let mine = NO_BOARD.contains(&path) || path.starts_with("/mail/t/") || path.starts_with("/d/");
    let held = (board.filter(|_| !mine))
        .map(|b| b.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    let board = held.as_deref();
    // The older app's link to a task Rust holds opens Rust's page (task rust-task-page-linear); a 404 leaves any other, and a
    // list not read, to the older app, which still answers it. Here, on the connection's thread, where the board is held.
    if let Some(id) = old_issue(path, query) {
        return Some(match board.map(|board| board.holds(&id)) {
            Some(Ok(true)) => ("303 See Other", TEXT, surface::task_href(&id, None)),
            Some(Err(e)) => {
                // Behind the front door this 404 goes to the older app and its text to nobody: the log keeps it.
                eprintln!("old-issue redirect: the task list is not read: {e}");
                (
                    "404 Not Found",
                    TEXT,
                    format!("the task list is not read: {e}"),
                )
            }
            _ => ("404 Not Found", TEXT, "no task of that id here".into()),
        });
    }
    Some(match path {
        "/ping" => ("200 OK", TEXT, ping().into()),
        "/built" => ("200 OK", TEXT, tasks::built()),
        "/memory" => {
            let m = memory();
            let body = format!("{:.1} {:.1}", m.current_mb, m.peak_mb);
            ("200 OK", TEXT, body)
        }
        page if is_page(page) => forms(accept, path, query, board),
        _ => return None,
    })
}

/// A request's first line, its Accept header, when it joined the queue, and where its answer goes. A route's time starts at the
/// queue: a read held behind a slow one is the delay these numbers are there to show.
type Request = (String, String, Instant, SyncSender<Response>);

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
        Ok(head) => &head.line,
        Err(line) => line,
    });
    let older = OLDER.get().copied();
    let response = match &arrived {
        Err(_) => None,
        Ok(head) => {
            let host = head.host.as_deref();
            let ours = matches!((host, local), (Some(host), Some(local)) if allowed_host(host, local.ip()));
            if ours && is_a_cli_post(&head.line) && cli(stream, head, older, report, limit) {
                return;
            }
            // Behind the front door the older app judges its own Host (Tailscale Serve's name among them), and answers
            // what is not ours, a browser's own fetches included.
            if let Some(port) = older
                && !(ours && !head.upgrade && served_here(&head.line))
            {
                return to_the_older_app(stream, head, port, report);
            }
            Some(if !ours {
                ("403 Forbidden", TEXT, "not this server's host".into())
            } else if asked_by_a_browser_itself(&head.line) {
                ("200 OK", TEXT, String::new())
            } else if is_a_compass_post(&head.line) {
                compass_post(stream, head, limit)
            } else if let Some(thread) = a_mail_reply(&head.line) {
                mail_reply(stream, head, &thread, limit)
            } else if let Some(page) = compass_read(&head.line) {
                // on its connection's thread, never the serial route's: every page view asks it (/code-review)
                match comments::serve_from_dir().and_then(|dir| comments::compass(dir, &page)) {
                    Ok(body) => ("200 OK", "application/json", body),
                    Err(e) => ("500 Internal Server Error", TEXT, e),
                }
            } else if let Some(response) = {
                let begun = Instant::now();
                let response = unqueued(&head.line, &head.accept, BOARD.get());
                // a UDP send, never a wait: timed as the serial route's are
                if response.is_some() {
                    report.step_took(route, begun.elapsed());
                }
                response
            } {
                response
            } else {
                let (reply, answered) = sync_channel(1);
                let routed = requests
                    .try_send((
                        head.line.clone(),
                        head.accept.clone(),
                        Instant::now(),
                        reply,
                    ))
                    .ok()
                    .and_then(|()| answered.recv_timeout(wait).ok());
                routed.unwrap_or(("503 Service Unavailable", TEXT, "busy".into()))
            })
        }
    };
    // A path of ours with nothing there (a task the older app knows, say) is the older app's to answer.
    if let (Some(port), Ok(head), Some(("404 Not Found", _, _))) = (older, &arrived, &response) {
        return to_the_older_app(stream, head, port, report);
    }
    let (status, kind, body) =
        response.unwrap_or_else(|| ("400 Bad Request", TEXT, "bad request".into()));
    let mut timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    // a redirect's body is where it leads, a path of ours that `task_href` encoded
    let location = match status.starts_with('3') {
        true => format!("Location: {body}\r\n"),
        false => String::new(),
    };
    let sent = write!(
        timed,
        "HTTP/1.1 {status}\r\n{location}Content-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

/// A request's head as it arrived. `headers`: each line as sent, its line end kept, for the older app; `rest`: what was read
/// past the head, a body's start.
struct Head {
    line: String,
    host: Option<String>,
    /// Its Accept header's value, or empty: each page answers HTML or text by it (`wants_html`).
    accept: String,
    /// A WebSocket's: it stays open both ways.
    upgrade: bool,
    headers: Vec<String>,
    rest: Vec<u8>,
}

/// The request's first line and its Host, its headers read to the blank line: closing on unread bytes sends a reset
/// that can destroy the answer. Bytes that end before that line are a bad request: a client that gave up has no wrong Host.
/// `Err` holds what arrived of the first line, so a request cut short still names its route.
fn request_line(stream: &TcpStream, limit: Duration) -> Result<Head, String> {
    let timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let mut reader = BufReader::new(timed.take(REQUEST_LIMIT));
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return Err(first);
    }
    let mut head = Head {
        line: String::new(),
        host: None,
        accept: String::new(),
        upgrade: false,
        headers: vec![],
        rest: vec![],
    };
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(n) if n > 0 => {}
            _ => return Err(first),
        }
        if header == "\r\n" || header == "\n" {
            head.line = first;
            head.rest = reader.buffer().to_vec();
            return Ok(head);
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => head.host = Some(value.to_string()),
                "accept" => head.accept = value.to_string(),
                // h2c and the like are refused upgrades: the request stays a plain one
                "upgrade" => {
                    head.upgrade =
                        (value.split(',')).any(|p| p.trim().eq_ignore_ascii_case("websocket"))
                }
                _ => {}
            }
        }
        head.headers.push(header);
    }
}

use tasks::FRONT_DOOR;
/// The older app's port behind the front door (`dash serve --older <n>`): what this server does not serve goes there.
static OLDER: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
/// Paths both apps serve, kept the older app's until a page of ours replaces its own: his search, its ping that
/// bin/comment-wake reads. /agents is ours since rust-agents-comments-tabs: it lists herdr's agents as the older one did.
const THE_OLDER_APPS: [&str; 2] = ["/search", "/ping"];

/// A GET of a path this server routes, and the compass's post; every other method, the paths above and the older agents
/// page's own query (`agents::OLDER_PAGE`) are the older app's.
fn served_here(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let path = target.split('?').next().unwrap_or_default();
    (method == Some("GET")
        && !THE_OLDER_APPS.contains(&path)
        && target != agents::OLDER_PAGE
        && !asked_by_a_browser_itself(line))
        || is_a_compass_post(line)
        || a_mail_reply(line).is_some()
}

/// `POST /mail/t/<thread>/reply`: the thread, decoded.
fn a_mail_reply(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    let path = words.next()?.split('?').next()?;
    let thread = path.strip_prefix("/mail/t/")?.strip_suffix("/reply")?;
    (!thread.is_empty() && !thread.contains('/')).then(|| form_decoded(&thread.replace('+', "%2B")))
}

fn is_a_compass_post(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some("/compass"))
}

/// `GET /compass?page=<path>`, the compass's list of a page's comments as JSON for its script: the page asked.
fn compass_read(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (method == Some("GET") && path == "/compass").then(|| param(query, "page").unwrap_or_default())
}

/// A comment's form at most: its words and quote at their limits, each byte of them encoded as %XX.
const COMPASS_BODY: usize = 32 * 1024;

/// A form a page of this app posted, read whole: its Origin is the Host it came to, so no other site a browser shows can post
/// it, and it is 32 KB at most. Its text, or the answer refusing it.
fn form_posted(stream: &TcpStream, head: &Head, limit: Duration) -> Result<String, Response> {
    let named = |name: &str| {
        head.headers.iter().find_map(|h| {
            let (n, v) = h.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };
    let host = head.host.as_deref().unwrap_or_default();
    if named("origin") != Some(format!("http://{host}")) {
        return Err((
            "403 Forbidden",
            TEXT,
            "a form comes from a page of this app".into(),
        ));
    }
    let Some(length) = named("content-length").and_then(|l| l.parse::<usize>().ok()) else {
        return Err(("411 Length Required", TEXT, "a form has a length".into()));
    };
    if length > COMPASS_BODY {
        return Err((
            "413 Content Too Large",
            TEXT,
            "a form is 32 KB at most".into(),
        ));
    }
    let mut body = head.rest.clone();
    body.truncate(length);
    let mut timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let mut more = vec![0; length - body.len()];
    if timed.read_exact(&mut more).is_err() {
        return Err(("400 Bad Request", TEXT, "the form ended early".into()));
    }
    body.extend(more);
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// `POST /mail/t/<thread>/reply`: his reply, sent from the address the form names (task rust-mail-thread), then back to the
/// thread saying so. On the connection's thread, never the serial route's: the mail server answers in seconds.
fn mail_reply(stream: &TcpStream, head: &Head, thread: &str, limit: Duration) -> Response {
    let form = match form_posted(stream, head, limit) {
        Ok(form) => form,
        Err(refused) => return refused,
    };
    let field = |name| param(&form, name).unwrap_or_default();
    let account = field("account");
    let sent = comments::serve_from_dir()
        .and_then(|dir| mail::reply(dir, &account, thread, &field("text")));
    match sent {
        Ok(email) => (
            "303 See Other",
            TEXT,
            mail::thread_href(thread, &account, &email),
        ),
        // his words come back with why, so nothing he wrote is lost
        Err(e) => (
            "502 Bad Gateway",
            TEXT,
            format!("Not sent: {e}\n\nYour words:\n{}\n", field("text")),
        ),
    }
}

/// `POST /compass`: the sidebar's comment, a form of `page`, `quote` and `text`, one row of the comment queue (task
/// rust-compass-queue). Only a page of this app posts it: its Origin is the Host it came to, so no other site a browser shows
/// can put words before the queue's agent. Its answer, for a browser with no script, links back.
fn compass_post(stream: &TcpStream, head: &Head, limit: Duration) -> Response {
    let form = match form_posted(stream, head, limit) {
        Ok(form) => form,
        Err(refused) => return refused,
    };
    let field = |name| param(&form, name).unwrap_or_default();
    let posted = comments::serve_from_dir().and_then(|dir| {
        comments::post(
            dir,
            &field("page"),
            &field("quote"),
            &field("text"),
            tasks::now_ms(),
        )
    });
    match posted {
        Ok(id) => (
            "200 OK",
            HTML,
            surface::forms::compass_sent(&field("page"), &id),
        ),
        Err(e) if e.starts_with("a comment") => ("400 Bad Request", TEXT, e),
        Err(e) => ("500 Internal Server Error", TEXT, e),
    }
}

fn is_a_cli_post(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST") && words.next() == Some("/dashboard/cli")
}

/// A command's form at most: bin/dash sends its words, its folder and its whole environment.
const CLI_BODY: usize = 1024 * 1024;

/// `dash outbox`, answered here (task rust-daily-today): bin/dash posts every command to /dashboard/cli, read whole here when it
/// fits `CLI_BODY`. This one Rust answers as the older app's CLI does (`x-exit`, `x-stdout-bytes`: DashboardWeb.ApiController),
/// on loopback only with bin/dash's header and no forward, as that CLI refuses any other caller. Any other command goes on to
/// the older app, its body as read. False: nothing done, the request goes its usual way.
fn cli(
    stream: &TcpStream,
    head: &Head,
    older: Option<u16>,
    report: &Statsd,
    limit: Duration,
) -> bool {
    let named = |name: &str| {
        head.headers.iter().find_map(|h| {
            let (n, v) = h.split_once(':')?;
            n.trim().eq_ignore_ascii_case(name).then(|| v.trim())
        })
    };
    let Some(length) = named("content-length").and_then(|l| l.parse::<usize>().ok()) else {
        return false;
    };
    if length > CLI_BODY || head.rest.len() > length {
        return false;
    }
    let mut body = head.rest.clone();
    let mut more = vec![0; length - body.len()];
    let mut timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    // curl asks before a body past 1 KB and waits a second for the answer: every dash command would wait it
    let expects = named("expect").is_some_and(|e| e.eq_ignore_ascii_case("100-continue"));
    if expects && !more.is_empty() && timed.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").is_err() {
        return true;
    }
    if timed.read_exact(&mut more).is_err() {
        let _ = write!(
            timed,
            "HTTP/1.1 400 Bad Request\r\nContent-Type: {TEXT}\r\nContent-Length: 30\r\nConnection: close\r\n\r\nthe command's form ended early"
        );
        report.refused("400", "route_cli");
        return true;
    }
    body.extend(more);
    let form = String::from_utf8_lossy(&body);
    let args: Vec<String> = (form.split('&'))
        .filter_map(|pair| pair.split_once('='))
        .filter(|(key, _)| form_decoded(key) == "args[]")
        .map(|(_, value)| form_decoded(value))
        .collect();
    let loopback = |a: std::io::Result<SocketAddr>| a.is_ok_and(|a| a.ip().is_loopback());
    let from_bin_dash = loopback(stream.local_addr())
        && loopback(stream.peer_addr())
        && named("x-dashboard-cli").is_some()
        && named("x-forwarded-for").is_none();
    // the command may take long (ci wait: up to three hours): the answer's own deadline starts after it (/code-review)
    let answered = |(exit, out, err): (i32, String, String)| {
        let mut timed = ByDeadline {
            stream,
            deadline: Instant::now() + limit,
        };
        let _ = write!(
            timed,
            "HTTP/1.1 200 OK\r\nContent-Type: {TEXT}\r\nx-exit: {exit}\r\nx-stdout-bytes: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}{err}",
            out.len(),
            out.len() + err.len()
        );
        true
    };
    if args == ["outbox"] && from_bin_dash {
        return answered(match drafts::outbox_lines() {
            Ok(lines) if lines.is_empty() => (0, lines, String::new()),
            Ok(lines) => (0, lines + "\n", String::new()),
            Err(e) => (1, String::new(), format!("outbox: {e}\n")),
        });
    }
    if let [ci, wait, target] = &args[..]
        && (ci.as_str(), wait.as_str()) == ("ci", "wait")
        && from_bin_dash
    {
        let cwd = param(&form, "cwd").unwrap_or_default();
        return answered(ci_wait(stream, target, &cwd));
    }
    // the body is read now: the usual way would answer a request whose body is gone (/code-review)
    let Some(port) = older else {
        let why = "no older app here to run this command";
        let _ = write!(
            timed,
            "HTTP/1.1 404 Not Found\r\nContent-Type: {TEXT}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{why}",
            why.len()
        );
        report.refused("404", "route_cli");
        return true;
    };
    // the body is read: the older app's own 100 Continue would reach the client after ours
    let headers = (head.headers.iter())
        .filter(|h| {
            !h.split(':')
                .next()
                .is_some_and(|n| n.trim().eq_ignore_ascii_case("expect"))
        })
        .cloned()
        .collect();
    let whole = Head {
        line: head.line.clone(),
        host: head.host.clone(),
        accept: head.accept.clone(),
        upgrade: head.upgrade,
        headers,
        rest: body,
    };
    to_the_older_app(stream, &whole, port, report);
    true
}

/// `dash ci wait <pr|sha>` (task ci-wait-gh-retry), on its connection's thread: a poll a minute through the merge hook of the
/// checkout `DASH_REPO` names, run in the caller's folder, as the older app ran it; a caller gone between two polls ends it, its
/// slot freed. `DASH_CI_POLL_MS` shortens the minute for a test.
fn ci_wait(stream: &TcpStream, target: &str, cwd: &str) -> (i32, String, String) {
    let cwd = Path::new(cwd);
    if !cwd.is_dir() {
        let why = format!(
            "ci wait: no folder {} to read the checks in\n",
            cwd.display()
        );
        return (2, String::new(), why);
    }
    let hook = tasks::repo().join(".claude/hooks/merge-claim.py");
    if !hook.is_file() {
        return (
            3,
            String::new(),
            format!("ci wait: no merge hook at {}\n", hook.display()),
        );
    }
    let every = std::env::var("DASH_CI_POLL_MS")
        .ok()
        .and_then(|ms| ms.parse().ok());
    let every = Duration::from_millis(every.unwrap_or(60_000));
    let listens = || {
        std::thread::sleep(every);
        // bin/dash sends nothing after its form: a read of 0 bytes, or a reset, is its socket closed (an interrupted dash)
        use std::io::ErrorKind;
        let mut byte = [0];
        let open = stream.set_nonblocking(true).is_ok()
            && match stream.peek(&mut byte) {
                Ok(n) => n > 0,
                Err(e) => matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted),
            };
        let _ = stream.set_nonblocking(false);
        open
    };
    let read = || dash_forge::hook_rows(&hook, cwd, target);
    dash_forge::ci_wait(read, listens, Instant::now() + dash_forge::CI_DEADLINE)
}

/// The head the older app gets. Connection: close, or a browser would send its next request, a page of ours too, down the same
/// socket; a WebSocket keeps its own. A caller outside this Mac is named in X-Forwarded-For, any it wrote dropped: the older app
/// reads a loopback peer with that header as Tailscale Serve's forward (`Dashboard.Handoff.caller/2`, /approve's tap).
fn forwarded(head: &Head, peer: Option<IpAddr>) -> String {
    let outside = peer.filter(|ip| !ip.is_loopback());
    let mut text = head.line.clone();
    for header in &head.headers {
        let name = header
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let dropped = (outside.is_some() && name == "x-forwarded-for")
            || (!head.upgrade && matches!(name.as_str(), "connection" | "keep-alive"));
        if !dropped {
            text += header;
        }
    }
    if let Some(ip) = outside {
        text += &format!("X-Forwarded-For: {ip}\r\n");
    }
    if !head.upgrade {
        text += "Connection: close\r\n";
    }
    text + "\r\n"
}

/// The request to the older app on loopback, its head and the body's start sent; or the 502 its client gets.
fn asked_of_the_older_app(
    client: &TcpStream,
    head: &Head,
    port: u16,
    report: &Statsd,
) -> Option<TcpStream> {
    let to = (Ipv4Addr::LOCALHOST, port).into();
    let upstream = TcpStream::connect_timeout(&to, Duration::from_secs(2)).and_then(|upstream| {
        let mut sent = &upstream;
        sent.write_all(forwarded(head, client.peer_addr().ok().map(|a| a.ip())).as_bytes())?;
        sent.write_all(&head.rest)?;
        Ok(upstream)
    });
    match upstream {
        Ok(upstream) => Some(upstream),
        Err(e) => {
            let body = format!("the older app on :{port} does not answer: {e}");
            let _ = write!(
                &mut &*client,
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: {TEXT}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            report.refused("502", "route_older");
            eprintln!("answered 502 on route_older: {e}");
            None
        }
    }
}

/// The view an older page is asked in: a page a browser opens at the top is `html`, and `?view=html|text|both` names one, as
/// on our pages (rust/CLAUDE.md rule 4). `None`: any other request, which goes on as bytes: a frame inside a page
/// (Sec-Fetch-Dest: iframe, as the older app's previews), a script's or a crawler's (no HTML asked).
fn older_view(head: &Head) -> Option<&'static str> {
    if head.upgrade || !head.line.starts_with("GET ") {
        return None;
    }
    let target = head.line.split_whitespace().nth(1).unwrap_or_default();
    let query = target.split_once('?').map_or("", |(_, q)| q);
    let dest = head.headers.iter().find_map(|h| {
        let (name, value) = h.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("sec-fetch-dest")
            .then(|| value.trim().to_ascii_lowercase())
    });
    match view_named(query) {
        Some(view) => Some(view),
        None if head.accept.contains("text/html")
            && dest.as_deref().is_none_or(|d| d == "document") =>
        {
            Some("html")
        }
        None => None,
    }
}

/// `?view=`'s one of our three, in any case; any other value is the page's own.
fn view_named(query: &str) -> Option<&'static str> {
    match param(query, "view")
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => Some("html"),
        Some("text") => Some("text"),
        Some("both") => Some("both"),
        _ => None,
    }
}

/// An older page through the front door (task rust-nav-board; the orchestrator's answer, 2026-11-08: « Older pages too »):
/// asked as a browser asks, with no view of ours, no compression and no validator, so a 304 never meets a dressed page. Its
/// head is read first: only a page `older_dressable` takes is read whole and handed on with the menu, or as its words;
/// anything else streams on as it comes, as the plain byte pump does.
fn dressed_by_the_front_door(
    client: &TcpStream,
    head: &Head,
    port: u16,
    report: &Statsd,
    view: &str,
) {
    let target = head.line.split_whitespace().nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if client.set_write_timeout(Some(ANSWER_DEADLINE)).is_err() {
        return;
    }
    if view == "both" {
        let body = surface::forms::both(path, query);
        let _ = write!(
            &mut &*client,
            "HTTP/1.1 200 OK\r\nContent-Type: {HTML}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        return;
    }
    let ours = |p: &&str| p.starts_with("view=") && view_named(p).is_some();
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !ours(p))
        .collect();
    let bare = if kept.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", kept.join("&"))
    };
    let left_out = |h: &String| {
        let name = h
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        matches!(
            name.as_str(),
            "accept" | "accept-encoding" | "if-none-match" | "if-modified-since"
        )
    };
    let mut headers: Vec<String> = head
        .headers
        .iter()
        .filter(|h| !left_out(h))
        .cloned()
        .collect();
    headers.push("Accept: text/html\r\n".into());
    let asked = Head {
        line: head.line.replacen(target, &bare, 1),
        host: head.host.clone(),
        accept: String::new(),
        upgrade: false,
        headers,
        rest: head.rest.clone(),
    };
    let Some(upstream) = asked_of_the_older_app(client, &asked, port, report) else {
        return;
    };
    let mut raw = vec![];
    let mut chunk = [0u8; 8192];
    let ends = loop {
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(at);
        }
        match (&upstream).read(&mut chunk) {
            Ok(n) if n > 0 && raw.len() < REQUEST_LIMIT as usize => {
                raw.extend_from_slice(&chunk[..n])
            }
            _ => break None,
        }
    };
    let older = ends.and_then(|at| Some((at, std::str::from_utf8(&raw[..at]).ok()?.to_string())));
    let length = older
        .as_ref()
        .and_then(|(_, h)| surface::forms::older_dressable(h));
    if let (Some((at, older)), Some(length)) =
        (older, length.filter(|n| *n as u64 <= OLDER_PAGE_LIMIT))
    {
        let missing = (at + 4 + length).saturating_sub(raw.len());
        let _ = (&upstream).take(missing as u64).read_to_end(&mut raw);
        let body = std::str::from_utf8(&raw[at + 4..]).ok();
        let page = body.and_then(|body| {
            surface::forms::older_page(&older, body, path, &kept.join("&"), view == "text")
        });
        if let Some(page) = page {
            let _ = (&*client).write_all(&page);
            let _ = client.shutdown(std::net::Shutdown::Both);
            return;
        }
    }
    let _ = (&*client).write_all(&raw);
    let _ = std::io::copy(&mut &upstream, &mut &*client);
    let _ = client.shutdown(std::net::Shutdown::Both);
}

/// The most of an older page read whole to dress it: its pages run to a few hundred KB (/mail, 223 KB on 2026-11-08).
const OLDER_PAGE_LIMIT: u64 = 16 * 1024 * 1024;

/// The request to the older app on loopback, then its bytes both ways until either side ends: a body, a long answer and a
/// WebSocket alike; a browser's page is dressed with the menu first. ponytail: a WebSocket holds one of `CONNECTIONS` while
/// open; a cap of their own if tabs ever crowd it out.
fn to_the_older_app(client: &TcpStream, head: &Head, port: u16, report: &Statsd) {
    if let Some(view) = older_view(head) {
        return dressed_by_the_front_door(client, head, port, report, view);
    }
    let Some(upstream) = asked_of_the_older_app(client, head, port, report) else {
        return;
    };
    // No read deadline either way: bin/dash's long commands (ci wait, task run) stay silent for minutes, and a WebSocket for as
    // long as it lives. The older app closes a plain answer itself (Connection: close). A client that stops reading is cut.
    let (Ok(()), Ok(()), Ok(()), Ok(up), Ok(down)) = (
        client.set_read_timeout(None),
        client.set_write_timeout(Some(ANSWER_DEADLINE)),
        upstream.set_read_timeout(None),
        client.try_clone(),
        upstream.try_clone(),
    ) else {
        return;
    };
    let sending = std::thread::Builder::new().spawn(move || {
        let _ = std::io::copy(&mut &up, &mut &down);
        // the client gone (or done) ends the upstream side too: a hung older app never holds this slot past its client
        let _ = down.shutdown(std::net::Shutdown::Both);
    });
    let _ = std::io::copy(&mut &upstream, &mut &*client);
    // the client's read above ends here, whether it closed or not
    let _ = client.shutdown(std::net::Shutdown::Both);
    let _ = upstream.shutdown(std::net::Shutdown::Both);
    if let Ok(sending) = sending {
        let _ = sending.join();
    }
}

/// `dash ui`: the app in the Mac window (task rust-window-in-process; @nha, 2026-11-09: « the native app will probably be the
/// real thing for end users. browser is useful in dev. »): each page `window_page` renders, so it opens with nothing on any port.
fn ui(dir: PathBuf) {
    comments::serve_from(&dir);
    let board = tasks::Board::open(&dir).unwrap_or_else(|e| {
        eprintln!("dash ui: {e}");
        std::process::exit(1);
    });
    #[cfg(feature = "window")]
    dash_ui::open_pages("/tasks", "dash", window_pages(board));
    #[cfg(not(feature = "window"))]
    {
        let _ = window_pages(board);
        eprintln!("dash ui: this dash was built without its window: cargo build --features window");
        std::process::exit(2);
    }
}

/// The pages that read the store, rendered again as the window asks; any other is rendered once a visit: /mail's or /agents'
/// reads cost a server or a terminal, and dash serve too reads them only when asked.
fn reads_the_store(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or_default();
    matches!(
        path,
        "/tasks" | "/inbox" | "/plan" | "/insights" | "/graph" | "/comments" | "/watchers"
    ) || path.starts_with("/tasks/")
}

/// The window's render: `window_page`, kept for a page that reads no store until another page is asked, unless `refill` says
/// its skeleton is drawn again.
fn window_pages(board: tasks::Board) -> impl FnMut(&str) -> String + Send + 'static {
    // the page shown, when its visit began, when it was last drawn
    let mut kept: Option<(String, String, Instant, Instant)> = None;
    move |target| {
        let now = Instant::now();
        match &mut kept {
            Some((at, page, _, _)) if at == target && reads_the_store(target) => {
                *page = window_page(target, &board);
                page.clone()
            }
            Some((at, page, began, drawn)) if at == target => {
                if refill(page, now - *began, now - *drawn) {
                    *page = window_page(target, &board);
                    *drawn = now;
                }
                page.clone()
            }
            _ => {
                let page = window_page(target, &board);
                kept = Some((target.to_string(), page.clone(), now, now));
                page
            }
        }
    }
}

/// Whether a page that reads no store is drawn again: the window runs no script, so it fills a skeleton as the browser's
/// LIVE_JS does, once a second while one is on the page, for five minutes a visit (/design's catalog keeps one for good).
fn refill(page: &str, visit: Duration, since: Duration) -> bool {
    page.contains(BEING_READ) && since >= Duration::from_secs(1) && visit < Duration::from_secs(300)
}

/// What a page holds while a read of the older app is still on its thread: a skeleton.
const BEING_READ: &str = surface::components::BUSY;

/// `dash ui --shot <page> <png>`: that page of the window painted into a PNG, no window opened, once the board's reads landed
/// or 10 s passed, as the window shows it then.
fn ui_shot(dir: PathBuf, target: &str, png: &Path) {
    comments::serve_from(&dir);
    let shot = tasks::Board::open(&dir).and_then(|board| {
        let until = Instant::now() + Duration::from_secs(10);
        let mut page = window_page(target, &board);
        while page.contains(BEING_READ) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(250));
            page = window_page(target, &board);
        }
        painted(page, png)
    });
    if let Err(e) = shot {
        eprintln!("dash ui: {e}");
        std::process::exit(1);
    }
}

/// `dash ui --check <png> [--stop-front-door]`: done-when 1 of rust-window-in-process, the window on his screen with nothing on
/// :7788, run from his terminal (a task's sandbox can neither stop launchd's front door nor read the screen). It refuses while
/// anything listens on :7788, or with --stop-front-door boots launchd's job out for the run and back in after; then it opens
/// `dash ui`, times it to its window in the macOS window list (10 s at most), captures that window and closes it.
fn ui_check(png: &Path, stop_front_door: bool) {
    // from the start, before any bootout: a Ctrl-C ends the window alone (it shares the terminal's process group), never this
    // process before FrontDoor's Drop loads the front door back
    unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let said = FrontDoor::stopped(stop_front_door).and_then(|_door| window_check(png));
    match said {
        Ok(line) => println!("dash ui --check: {line}"),
        Err(why) => {
            eprintln!("dash ui --check: {why}");
            std::process::exit(1);
        }
    }
}

fn window_check(png: &Path) -> Result<String, String> {
    if let Some(holder) = on_7788() {
        return Err(format!(
            "{holder} listens on :7788: stop it first, or add --stop-front-door"
        ));
    }
    let began = Instant::now();
    let me = std::env::current_exe().map_err(|e| format!("this dash: {e}"))?;
    let child = std::process::Command::new(me).arg("ui").spawn();
    let mut window = Window(child.map_err(|e| format!("dash ui: {e}"))?);
    let (id, size, asked, seen) = loop {
        if let Ok(Some(status)) = window.0.try_wait() {
            return Err(format!("dash ui ended before its window showed: {status}"));
        }
        let asked = began.elapsed();
        match on_screen(window.0.id())? {
            Some((id, size)) => break (id, size, asked, began.elapsed()),
            None if began.elapsed() > Duration::from_secs(10) => {
                return Err("no window on screen within 10 s".into());
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    // its first page drawn, then :7788 asked again, so the window started nothing there either, then the picture
    std::thread::sleep(Duration::from_secs(2));
    if let Some(holder) = on_7788() {
        return Err(format!("{holder} took :7788 while the window ran"));
    }
    let captured = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-o", "-x", "-l", &id])
        .arg(png)
        .status();
    if !captured.is_ok_and(|s| s.success()) {
        return Err(format!(
            "window {id} on screen, but screencapture failed (Screen Recording granted to this terminal?)"
        ));
    }
    Ok(format!(
        "nothing on :7788, window {id} {size} on screen after {:.2} to {:.2} s (the window list asked then answered), captured {}; closed",
        asked.as_secs_f64(),
        seen.as_secs_f64(),
        png.display()
    ))
}

/// The check's `dash ui`, closed however the check ends, a panic included.
struct Window(std::process::Child);

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// launchd's job that keeps the front door on :7788 (Makefile's rust/serve), with KeepAlive: a kill alone restarts it. The bare
/// name is what launch.request takes (comment-wake matches it in sandbox/jobs.conf), the label what launchctl takes.
const FRONT_JOB: &str = "rust-serve";
const FRONT_LABEL: &str = "com.nicolasha.rust-serve";

/// The front door booted out for the check, bootstrapped again when dropped, a panic or an early return included. It boots
/// nothing out it cannot load back: a sandboxed launchctl bootstraps nothing (« Bootstrap failed: 5 », the orchestrator's run
/// of 2026-11-09 left :7788 dark), so a sandbox or an unreadable plist refuses first; a bootstrap that fails anyway asks
/// comment-wake, on the host, through .claude/run/launch.request.
struct FrontDoor(Option<(String, PathBuf)>);

impl FrontDoor {
    fn stopped(stop: bool) -> Result<FrontDoor, String> {
        if !stop {
            return Ok(FrontDoor(None));
        }
        let refused = |why: String| {
            Err(format!(
                "--stop-front-door cannot load {FRONT_LABEL} back ({why}), so nothing was stopped; run it from his terminal"
            ))
        };
        let home = match std::env::var("HOME") {
            Ok(home) if Path::new(&home).is_absolute() => home,
            _ => return refused("HOME is not an absolute path".into()),
        };
        let plist = Path::new(&home).join(format!("Library/LaunchAgents/{FRONT_LABEL}.plist"));
        if let Some(why) = cannot_bootstrap() {
            return refused(why.into());
        }
        if let Err(e) = File::open(&plist) {
            return refused(format!("{}: {e}", plist.display()));
        }
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        // armed before the bootout: a bootout that errs while launchd still unloads the job is loaded back on the way out too
        let door = FrontDoor(Some((domain.clone(), plist)));
        launchctl(&["bootout", &format!("{domain}/{FRONT_LABEL}")])?;
        let until = Instant::now() + Duration::from_secs(10);
        while on_7788().is_some() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(door)
    }
}

impl Drop for FrontDoor {
    fn drop(&mut self) {
        let Some((domain, plist)) = &self.0 else {
            return;
        };
        // a bootout launchd has not finished refuses a bootstrap (« already in progress », 5): tried again for 10 s, so the
        // request below is written only once the job is gone (comment-wake skips a job launchctl print still finds)
        let bootstrap = || launchctl(&["bootstrap", domain, &plist.to_string_lossy()]);
        let until = Instant::now() + Duration::from_secs(10);
        let mut tried = bootstrap();
        while tried.is_err() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(500));
            tried = bootstrap();
        }
        let Err(e) = tried else {
            eprintln!("dash ui --check: the front door is back ({FRONT_LABEL})");
            return;
        };
        let request = tasks::repo().join(".claude/run/launch.request");
        let asked = std::fs::create_dir_all(request.parent().expect("a folder")).and_then(|()| {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&request)?;
            writeln!(file, "{FRONT_JOB}")
        });
        let by_hand = format!("launchctl bootstrap {domain} {}", plist.display());
        match asked {
            Ok(()) => eprintln!(
                "dash ui --check: the front door did not come back ({e}); {FRONT_JOB} written to {}, comment-wake loads it within a minute, else: {by_hand}",
                request.display()
            ),
            Err(w) => eprintln!(
                "dash ui --check: the front door did not come back ({e}), nor could {} be written ({w}): {by_hand}",
                request.display()
            ),
        }
    }
}

/// Why this process cannot run launchctl bootstrap, if it cannot: a macOS sandbox (sandbox_check of libSystem, no operation
/// named) answers « Bootstrap failed: 5 », and launchd is macOS's alone.
#[cfg(target_os = "macos")]
fn cannot_bootstrap() -> Option<&'static str> {
    unsafe extern "C" {
        fn sandbox_check(
            pid: libc::pid_t,
            operation: *const libc::c_char,
            kind: libc::c_int,
            ...
        ) -> libc::c_int;
    }
    // 1 sandboxed, 0 not; an error (-1) is not taken for a sandbox, so his terminal is never refused on it
    let sandboxed = unsafe { sandbox_check(libc::getpid(), std::ptr::null(), 0) > 0 };
    sandboxed.then_some("this process is sandboxed: launchctl bootstrap fails there")
}

#[cfg(not(target_os = "macos"))]
fn cannot_bootstrap() -> Option<&'static str> {
    Some("launchd is macOS's")
}

fn launchctl(args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| format!("launchctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "launchctl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Who listens on :7788 on any address, as lsof names it; with no lsof, "something" when 127.0.0.1:7788 answers.
fn on_7788() -> Option<String> {
    let lsof = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP:7788", "-sTCP:LISTEN", "-Fpc"])
        .output();
    if let Ok(out) = lsof {
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |tag| {
            text.lines()
                .find_map(|l| l.strip_prefix(tag))
                .map(str::to_string)
        };
        if let Some(pid) = field('p') {
            return Some(format!("{}, pid {pid},", field('c').unwrap_or_default()));
        }
    }
    let front = SocketAddr::from(([127, 0, 0, 1], 7788));
    TcpStream::connect_timeout(&front, Duration::from_millis(500))
        .is_ok()
        .then(|| "something".to_string())
}

/// The id and size of `pid`'s first window on screen, from the macOS window list (osascript's JavaScript reads CoreGraphics);
/// an osascript that fails says why, never taken for a window not shown yet.
fn on_screen(pid: u32) -> Result<Option<(String, String)>, String> {
    const LIST: &str = r#"ObjC.import("CoreGraphics");
function run(argv) {
  const all = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0)));
  const w = all.find(w => w.kCGWindowOwnerPID === parseInt(argv[0]) && w.kCGWindowLayer === 0);
  return w ? `${w.kCGWindowNumber} ${w.kCGWindowBounds.Width}x${w.kCGWindowBounds.Height}` : "";
}"#;
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", LIST, &pid.to_string()])
        .output()
        .map_err(|e| format!("osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "osascript: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let line = String::from_utf8_lossy(&out.stdout);
    Ok(line
        .trim()
        .split_once(' ')
        .map(|(id, size)| (id.to_string(), size.to_string())))
}

#[cfg(feature = "window")]
fn painted(html: String, png: &Path) -> Result<(), String> {
    dash_ui::shot(html, 1280, 860, png)
}

#[cfg(not(feature = "window"))]
fn painted(_: String, png: &Path) -> Result<(), String> {
    Err(format!(
        "{}: this dash was built without its window",
        png.display()
    ))
}

/// A page as the window draws it: the very HTML `forms` gives a browser at `target`, sidebar and style included, rendered here.
/// An answer that is not HTML (a page's text view, a 404, an older app's page) is the same sidebar over its words.
fn window_page(target: &str, board: &tasks::Board) -> String {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (status, kind, body) = forms("text/html", path, query, Some(board));
    if kind == HTML {
        return body;
    }
    let tree = if is_page(path) {
        json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["status", "said"]},
            "status": {"type": "heading", "props": {"text": status}},
            "said": {"type": "quote", "props": {"text": body}},
        }})
    } else {
        // the older app's: a card that opens it at the front door, in the browser
        let line = "The older app's page: the window draws Rust's pages, the browser the rest.";
        let href = format!("{FRONT_DOOR}{target}");
        json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["status", "cards"]},
            "status": {"type": "heading", "props": {"text": path}},
            "cards": {"type": "cards", "children": ["older"]},
            "older": {"type": "card", "props": {"title": "Open it in the browser", "line": line, "href": href}},
        }})
    };
    let page = surface::screen::page(status, &tree.to_string()).unwrap_or_else(|e| e);
    surface::forms::with_menu(&page, path, query)
}

/// `dash ui --design`: /design in the Mac window, the component dash serve renders there (task rust-design-page).
fn design_window() {
    #[cfg(feature = "window")]
    dash_ui::open(
        surface::design::Design,
        surface::design::DesignProps::default(),
        "Design system",
    );
    #[cfg(not(feature = "window"))]
    {
        eprintln!("dash ui: this dash was built without its window: cargo build --features window");
        std::process::exit(2);
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

/// The older app's link to a site_perso task, `/projects/site_perso?issue=<id>` or `/projects/site_perso/issue/<id>`: its id.
fn old_issue(path: &str, query: &str) -> Option<String> {
    let id = match path.strip_prefix("/projects/site_perso")? {
        "" | "/" => param(query, "issue"),
        // a path keeps its +, as /tasks/<id>'s
        rest => (rest.strip_prefix("/issue/")).map(|id| form_decoded(&id.replace('+', "%2B"))),
    };
    id.filter(|id| !id.is_empty())
}

/// The query's first `name=` value, decoded as a form sends it.
fn param(query: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&prefix))
        .map(form_decoded)
}

/// A page's form (rust/CLAUDE.md point 4): `?view=html|text` first, in any case, else HTML when the Accept header names it,
/// else the text. Any other view, an empty one included, leaves it to the header.
fn wants_html(accept: &str, query: &str) -> bool {
    let view = param(query, "view").map(|v| v.trim().to_lowercase());
    match view.as_deref() {
        Some("html") => true,
        Some("text") => false,
        _ => accept.contains("text/html"),
    }
}

/// The pages: each answers at its one URL in three views (rust/CLAUDE.md rule 4, task rust-design-page).
fn is_page(path: &str) -> bool {
    matches!(
        path,
        "/" | "/tasks"
            | "/inbox"
            | "/plan"
            | "/insights"
            | "/graph"
            | "/deps"
            | "/agents"
            | "/comments"
            | "/tools"
            | "/watchers"
            | "/design"
            | "/drafts"
            | "/mail"
    ) || path.starts_with("/tasks/")
        || path.starts_with("/d/")
        || path.starts_with("/mail/t/")
}

/// A page's view (rust/CLAUDE.md rule 4; @nha, 2026-11-08: « a toggle allows seeing both (change the URL) or just side by
/// side »): `?view=both` the two side by side, else `page`'s HTML with the toggle, or its text.
fn forms(accept: &str, path: &str, query: &str, board: Option<&tasks::Board>) -> Response {
    dressed(accept, path, query, |html| page(path, query, html, board))
}

/// A page's answer in the view asked, its HTML in the menu's frame; `page` answers HTML or its text, `html` asking which.
fn dressed(accept: &str, path: &str, query: &str, page: impl FnOnce(bool) -> Response) -> Response {
    if param(query, "view").is_some_and(|v| v.trim().eq_ignore_ascii_case("both")) {
        return ("200 OK", HTML, surface::forms::both(path, query));
    }
    let html = wants_html(accept, query);
    match page(html) {
        (status, HTML, body) => (status, HTML, surface::forms::with_menu(&body, path, query)),
        answer => answer,
    }
}

/// A page's one drawing as HTML or as its text, `html` asking which: the one place a page's form is picked (task
/// rust-page-forms; @nha, 2026-11-08: « this will be a rule for every page. /mail tasks tools status etc. »). `None`: no board.
fn drawn(html: bool, read: Option<Result<surface::Drawing, String>>) -> Response {
    use surface::Drawing::{Page, Tree};
    let body = |d: surface::Drawing| match d {
        Tree { title, tree } if html => surface::screen::page(&title, &tree),
        Tree { tree, .. } => surface::text(&tree),
        Page(page) if html => Ok(page),
        Page(page) => Ok(surface::forms::text_of(&page)),
    };
    match read.map(|read| read.and_then(body)) {
        Some(Ok(body)) => ("200 OK", if html { HTML } else { TEXT }, body),
        Some(Err(e)) => ("500 Internal Server Error", TEXT, e),
        None => ("404 Not Found", TEXT, "no port, no page".into()),
    }
}

/// A page's answer, HTML or its text (`html` asks which); `forms` picks the view.
fn page(path: &str, query: &str, html: bool, board: Option<&tasks::Board>) -> Response {
    let failed = |e| ("500 Internal Server Error", TEXT, e);
    let form = |read: Option<Result<surface::Drawing, String>>| drawn(html, read);
    match path {
        // Home reads nothing (task rust-home-prompt); its parts are /inbox's and /tasks', /plan and /insights beside them
        "/" => form(Some(Ok(tasks::home_page()))),
        "/inbox" => form(Some(Ok(tasks::inbox_page()))),
        "/plan" => form(Some(Ok(tasks::plan_page()))),
        "/insights" => form(Some(Ok(tasks::insights_page()))),
        // /graph is the board's dependency view at its own URL (task rust-nav-board: « /graph /deps => are these not the same ? »)
        "/graph" => form(board.map(tasks::Board::graph)),
        "/tasks" => match TaskView::of(param(query, "layout").as_deref()) {
            TaskView::Graph => form(board.map(tasks::Board::graph)),
            view => {
                let project = param(query, "project").filter(|p| !p.is_empty());
                form(board.map(|b| b.tasks(view, project.as_deref())))
            }
        },
        // The code map: the crate graph drawn by Mermaid, nested by layer (task rust-deps-view), then the graphs as built.
        "/deps" => {
            let tree = json!({"root": "page", "elements": {
                "page": {"type": "stack", "children": ["crates", "map"]},
                "crates": {"type": "diagram", "props": {"source": DEPS_DIAGRAM}},
                "map": {"type": "quote", "props": {"text": DEPS.trim_end()}},
            }});
            let tree = tree.to_string();
            form(Some(Ok(surface::Drawing::Tree {
                title: "Code map".into(),
                tree,
            })))
        }
        "/design" => {
            let tab = param(query, "tab").unwrap_or_default();
            form(Some(Ok(surface::Drawing::Page(surface::design::page(
                &tab,
                param(query, "theme").as_deref(),
            )))))
        }
        // No HOME is no login known: the page says so, the server stays up.
        "/agents" => match std::env::var("HOME") {
            Ok(home) => form(Some(agents::page(Path::new(&home)))),
            Err(e) => failed(format!("HOME: {e}")),
        },
        // One URL for both (task rust-access-map): a browser asks for HTML, an agent's curl gets the text.
        "/tools" => match std::env::var("HOME") {
            // an agent's text waits for the first read: a 202 is no page to a GET (the live check reopened rust-access-map
            // on it), and « no rows » must never read as « no access » (/code-review of rust-access-map); a read past the
            // wait is a 503, which a client asks again
            Ok(home) => match access::page(Path::new(&home), !html) {
                Ok((_, false)) if !html => (
                    "503 Service Unavailable",
                    TEXT,
                    "no read of every access has ended yet: ask again".into(),
                ),
                Ok((drawing, _)) => form(Some(Ok(drawing))),
                Err(e) => failed(e),
            },
            Err(e) => failed(format!("HOME: {e}")),
        },
        // the server's data folder, never the board's: with no --port there is no board, and the page still answers
        // (/code-review of rust-watchers-port, its finding 2, shown by compass.rs' test)
        "/watchers" => form(Some(
            comments::serve_from_dir().and_then(|dir| watchers::page(dir, &comments::fed(dir))),
        )),
        "/comments" => form(Some(comments::serve_from_dir().and_then(comments::page))),
        // the mail server's, answered on its connection's thread (unqueued), never the serial one
        thread if thread.starts_with("/mail/t/") => {
            let id = form_decoded(&thread["/mail/t/".len()..].replace('+', "%2B"));
            let account = param(query, "account").filter(|a| !a.is_empty());
            let sent = param(query, "sent").filter(|s| !s.is_empty());
            form(Some(Ok(mail::thread_page(
                account.as_deref(),
                &id,
                sent.as_deref(),
            ))))
        }
        "/mail" => {
            let page = param(query, "page").and_then(|p| p.trim().parse().ok());
            let (folder, q) = (param(query, "box"), param(query, "q"));
            let asked = dash_mail::Asked::of(folder.as_deref(), q.as_deref(), page);
            let account = param(query, "account").filter(|a| !a.is_empty());
            form(Some(Ok(mail::page(account.as_deref(), &asked))))
        }
        // The drafts waiting (task rust-drafts-list): no read ended is no list yet, never « no draft waiting ».
        "/drafts" => match board.map(|b| drafts::page(b.data(), param(query, "show").as_deref())) {
            Some(Ok((_, false))) if !html => (
                "202 Accepted",
                TEXT,
                "reading the older app's drafts: ask again in a few seconds".into(),
            ),
            read => form(read.map(|r| r.map(|(drawing, _)| drawing))),
        },
        // A draft's page (task rust-draft-approve), on this request's thread: the older app's read now, so his tap shows.
        draft if draft.starts_with("/d/") => {
            let id = form_decoded(&draft["/d/".len()..].replace('+', "%2B"));
            match comments::serve_from_dir().and_then(|dir| drafts::draft_page(dir, &id)) {
                Ok(Some(page)) => form(Some(Ok(page))),
                Ok(None) => ("404 Not Found", TEXT, "no draft of that id".into()),
                Err(e) => form(Some(Err(e))),
            }
        }
        task if task.starts_with("/tasks/") => {
            // a path keeps its + (only a form's + is a space; /code-review of #1197)
            let id = form_decoded(&task["/tasks/".len()..].replace('+', "%2B"));
            match board.map(|board| board.task(&id)) {
                Some(Ok(None)) => ("404 Not Found", TEXT, "no open task of that id".into()),
                page => form(page.and_then(Result::transpose)),
            }
        }
        _ => ("404 Not Found", TEXT, "no such path".into()),
    }
}

/// The serial route: what reads the store or the registry, one at a time on `serve`'s thread.
fn route(line: &str, accept: &str, db: &dyn Store, registry: &Registry) -> Response {
    let failed = |e| ("500 Internal Server Error", TEXT, e);
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match (method, path) {
        (Some("GET"), "/scan") => match db.scan() {
            Ok(s) => ("200 OK", TEXT, format!("{} {}", s.rows, s.bytes)),
            Err(e) => failed(e),
        },
        (Some("GET"), "/search") => {
            match search(&param(query, "q").unwrap_or_default(), db, registry) {
                Ok(lines) => ("200 OK", TEXT, lines),
                Err(e) => failed(e),
            }
        }
        // the daily's page reads the registry's « today » slot, so it routes here; each line reads its plugin's last count,
        // never a source, so this thread never waits on one
        (Some("GET"), "/today") => dressed(accept, path, query, |html| {
            let (drawing, read) = today::page(registry);
            if !read && !html {
                let why = "reading a line of today: ask again in a few seconds";
                return ("202 Accepted", TEXT, why.into());
            }
            drawn(html, Some(Ok(drawing)))
        }),
        (Some("GET"), slot) if slot.starts_with("/ui/") => {
            let name = &slot["/ui/".len()..];
            match registry.ui.get(name) {
                Some(tree) => {
                    let drawing = surface::Drawing::Tree {
                        title: name.to_string(),
                        tree: tree.to_string(),
                    };
                    drawn(wants_html(accept, query), Some(Ok(drawing)))
                }
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

    fn head(headers: &[&str], upgrade: bool) -> Head {
        Head {
            line: "GET /approve HTTP/1.1\r\n".into(),
            host: Some("100.72.158.115:7788".into()),
            accept: String::new(),
            upgrade,
            headers: headers.iter().map(|h| format!("{h}\r\n")).collect(),
            rest: vec![],
        }
    }

    /// /approve refuses a tap from this Mac: a caller on the tailnet must reach the older app named, never as loopback, and
    /// never under an address it wrote itself. Tailscale Serve's own forward, from loopback, passes as it came.
    #[test]
    fn the_older_app_learns_who_called_and_no_caller_can_say_it_for_itself() {
        let _budget = TestBudget::new();
        let phone: IpAddr = "100.80.1.2".parse().expect("an address");
        let written = head(
            &[
                "Host: 100.72.158.115:7788",
                "X-Forwarded-For: 127.0.0.1",
                "Connection: keep-alive",
            ],
            false,
        );
        assert_eq!(
            forwarded(&written, Some(phone)),
            "GET /approve HTTP/1.1\r\nHost: 100.72.158.115:7788\r\nX-Forwarded-For: 100.80.1.2\r\nConnection: close\r\n\r\n"
        );
        let served = head(&["Host: mac.ts.net", "X-Forwarded-For: 100.80.1.2"], false);
        let loopback = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            forwarded(&served, loopback),
            "GET /approve HTTP/1.1\r\nHost: mac.ts.net\r\nX-Forwarded-For: 100.80.1.2\r\nConnection: close\r\n\r\n"
        );
        let socket = head(&["Connection: Upgrade", "Upgrade: websocket"], true);
        assert_eq!(
            forwarded(&socket, loopback),
            "GET /approve HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n"
        );
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

    /// Done-when 1 of rust-page-forms: the head holds Accept as its own field, the first line as sent.
    #[test]
    fn the_head_holds_its_accept_apart_from_its_first_line() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let sent = b"GET /tools HTTP/1.1\r\nHost: localhost\r\nAccept:  text/html,*/*\r\n\r\n";
        client.write_all(sent).expect("write");
        let head = request_line(&server, Duration::from_secs(2)).expect("a head");
        assert_eq!(
            (head.line.as_str(), head.accept.as_str()),
            ("GET /tools HTTP/1.1\r\n", "text/html,*/*")
        );
        assert!(wants_html(&head.accept, ""));
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
        for good in ["100.72.158.115", "100.72.158.115:7788"] {
            assert!(allowed_host(good, tailnet), "{good}");
            assert!(!allowed_host(good, loopback), "{good}");
        }
        for good in ["127.0.0.1:7788", "localhost:7788"] {
            assert!(allowed_host(good, loopback), "{good}");
            assert!(!allowed_host(good, tailnet), "{good}");
        }
        for bad in [
            "100.72.158.116:7788",
            "evil.example:7788",
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
            for (_, _, _, reply) in requests {
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
        let (_, _, queued, reply) = requests.recv().expect("the request");
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
            ("GET /graph HTTP/1.1\r\n", "route_graph"),
            ("GET /agents HTTP/1.1\r\n", "route_agents"),
            ("GET /tools HTTP/1.1\r\naccept: text/html\n", "route_tools"),
            ("GET /design?tab=data HTTP/1.1\r\n", "route_design"),
            ("GET /mail?page=2 HTTP/1.1\r\n", "route_mail"),
            ("GET /tasks/a-b HTTP/1.1\r\n", "route_task"),
            ("GET /d/a-draft?view=text HTTP/1.1\r\n", "route_draft"),
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

    /// rust/CLAUDE.md rule 5, one slow request never stalls the others: with the serial store's thread held by a slow /search,
    /// /ping and a page still answer within 200 ms, and only the store's routes queue behind it (Elixir answered no dash
    /// command for 120 s under disk load, 2026-11-09).
    #[test]
    fn a_slow_request_leaves_the_ping_and_the_pages_answering_under_200_ms() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, requests) = sync_channel::<Request>(1);
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || accept(&listener, &sender, &live, &silent()));
        let ask = |path: &str| {
            let mut stream = TcpStream::connect(address).expect("connect");
            write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
            stream
        };
        // the slow one: taken off the queue and never answered while the others ask
        let _slow = ask("/search?q=everything");
        let (line, _, _, _held) = requests.recv().expect("the slow request");
        assert!(line.starts_with("GET /search"), "{line}");
        // /deps is static text: a debug build's /design render under CI load is no part of this check
        for path in ["/ping", "/deps"] {
            let start = Instant::now();
            let mut text = String::new();
            ask(path).read_to_string(&mut text).expect("its answer");
            let took = start.elapsed();
            assert!(text.starts_with("HTTP/1.1 200"), "{path}: {text}");
            assert!(took < Duration::from_millis(200), "{path} took {took:?}");
        }
        assert!(
            unqueued("GET /scan HTTP/1.1", "*/*", None).is_none(),
            "the store's routes stay serial"
        );
        assert!(unqueued("POST /ping HTTP/1.1", "*/*", None).is_none());
    }

    /// /agents is ours since rust-agents-comments-tabs; the older agents page's own query stays the older app's.
    #[test]
    fn agents_is_ours_and_its_older_page_the_older_app_s() {
        let _budget = TestBudget::new();
        assert!(served_here("GET /agents HTTP/1.1"));
        assert!(
            !served_here("GET /agents?app=older HTTP/1.1"),
            "the orchestrator's controls stay reachable"
        );
    }

    #[test]
    fn a_browser_gets_html_an_agent_the_text_and_view_overrides_both() {
        let _budget = TestBudget::new();
        let (browser, curl) = ("text/html,application/xhtml+xml", "*/*");
        assert!(wants_html(browser, ""));
        assert!(!wants_html(curl, ""));
        assert!(!wants_html("", ""));
        assert!(!wants_html(browser, "view=text"));
        assert!(wants_html(curl, "a=1&view=html"));
        assert!(wants_html(curl, "view=HTML%0A"));
        assert!(wants_html(browser, "view="));
    }

    fn get(target: &str, accept: &str) -> Response {
        let dir = dash_meter::TestDir::new("forms");
        let db = store::open(&dir.0).expect("a store");
        let line = format!("GET {target} HTTP/1.1\r\n");
        unqueued(&line, accept, None).unwrap_or_else(|| {
            let registry = Registry::with_compiled().expect("a registry");
            route(&line, accept, &db, &registry)
        })
    }

    /// Done-when 4 of rust-daily-today: /today, as the server routes it, draws its one tree through dash-surface in both forms:
    /// its text for an agent, its HTML in the menu for him, drafts to send first; a line still reading answers an agent 202.
    #[test]
    fn today_answers_both_forms_through_the_surface_crate() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("today-forms");
        let db = store::open(&dir.0).expect("a store");
        let today = |drafts: Option<usize>, accept: &str, query: &str| {
            let mut registry = Registry::default();
            let to_send: Box<dyn dash_core::registry::Slot> =
                Box::new(dash_drafts::outbox::ToSend(move || Ok(drafts)));
            let replies: Box<dyn dash_core::registry::Slot> =
                Box::new(dash_mail::RepliesWaiting(|| Ok(Some(2))));
            let slot = |key: &str, line| dash_core::registry::Contribution {
                slots: vec![(key.to_string(), line)],
                ..Default::default()
            };
            registry
                .add("mail", slot(mail::SLOT, replies))
                .expect("mail");
            registry
                .add("drafts", slot(drafts::SLOT, to_send))
                .expect("drafts");
            let line = format!("GET /today{query} HTTP/1.1\r\n");
            route(&line, accept, &db, &registry)
        };
        let (status, kind, text) = today(Some(3), "*/*", "?view=text");
        assert_eq!((status, kind), ("200 OK", TEXT), "{text}");
        assert_eq!(
            text,
            "Today\n3 to send\tdrafts approved, not yet sent\t/drafts?show=to-send\n2 replies waiting\tunread answers in his inbox\t/mail\n"
        );
        let (status, kind, html) = today(Some(3), "*/*", "?view=html");
        assert_eq!((status, kind), ("200 OK", HTML));
        let (first, second) = (html.find("3 to send"), html.find("2 replies waiting"));
        assert!(
            first.is_some_and(|f| second.is_some_and(|s| f < s)),
            "drafts first: {html}"
        );
        assert!(
            html.contains(r#"href="/drafts?show=to-send""#) && html.contains(r#"id="dash-menu""#),
            "the line's link and the menu"
        );
        assert_eq!(
            today(Some(3), "text/html", "").1,
            HTML,
            "a browser gets the HTML"
        );
        let (status, _, said) = today(None, "*/*", "");
        assert_eq!(status, "202 Accepted", "{said}");
        assert_eq!(
            today(None, "text/html", "").0,
            "200 OK",
            "he sees the line reading"
        );
    }

    /// Task rust-page-forms: /deps and /design answer by `drawn` too, HTML for a browser and their text for an agent.
    #[test]
    fn deps_and_design_answer_both_forms_through_drawn() {
        let _budget = TestBudget::new();
        for path in ["/deps", "/design"] {
            assert_eq!(get(path, "text/html").1, HTML, "{path}");
            let (status, kind, body) = get(path, "*/*");
            assert_eq!((status, kind), ("200 OK", TEXT), "{path}");
            assert!(
                !body.contains("class=") && !body.trim().is_empty(),
                "{path}: {body}"
            );
        }
        // task rust-deps-view: the HTML draws the crate graph from its Mermaid, a subgraph a layer, with ELK's layout; the
        // text form keeps the graphs as built, after the same Mermaid
        let html = get("/deps", "text/html").2;
        assert!(
            html.contains(r#"class="diagram" title="Diagram" sandbox="allow-scripts" id="crates""#)
                && html.contains("/npm/mermaid@11.4.1/")
                && html.contains("layout:&#34;elk&#34;")
                && html.contains(r#"class="fold" id="crates-source""#)
                && html.contains("subgraph l4[&#34;Core&#34;]")
                && html.contains("c_store --&#62; c_core"),
            "{html}"
        );
        assert_eq!(
            get("/deps", "*/*").2,
            format!(
                "Mermaid:\n{}\n{}\n",
                DEPS_DIAGRAM.trim_end(),
                DEPS.trim_end()
            )
        );
    }

    /// Done-when 1: /design is the spike's page, the Dioxus component's server render, and the window draws its words.
    #[test]
    fn design_is_the_component_and_the_window_draws_the_same_words() {
        let _budget = TestBudget::new();
        let (status, kind, html) = get("/design", "text/html");
        assert_eq!((status, kind), ("200 OK", HTML));
        for part in [
            "Design system",
            r#"id="f-layout""#,
            r#"href="/design?tab=data&#38;theme=light""#,
            "Guidelines",
            ".ds-row",
        ] {
            assert!(html.contains(part), "{part} in /design");
        }
        let window = dash_ui::shown(dioxus::prelude::VirtualDom::new_with_props(
            surface::design::Design,
            surface::design::DesignProps::default(),
        ));
        // Every word, the dark cell's too, as the window draws it
        let words = |t: &str| t.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            words(&window),
            words(&surface::forms::every_word(&surface::design::page(
                "", None
            )))
        );
        let notion = surface::design::page("notion", None);
        assert!(
            notion.contains("app dark ds-root") && notion.contains(r#"class="ds-tab on""#),
            "Notion opens dark"
        );
    }

    /// Done-when 2 and 3: every page answers its text, HTML with the toggle, and both side by side, at its one URL.
    #[test]
    fn every_page_answers_three_views_and_the_toggle_changes_the_url() {
        let _budget = TestBudget::new();
        let (_, kind, text) = get("/design?tab=forms&view=text", "text/html");
        assert_eq!(kind, TEXT);
        assert!(text.starts_with("Design\tLayout\tData\t"), "{text}");
        assert!(
            text.contains("\nForms\nWhat he presses, picks and types"),
            "{text}"
        );
        assert!(
            text.contains("\nPrimary\tApprove\tReject\tLock\tCancel\n"),
            "{text}"
        );
        assert!(!text.contains('<'), "no markup: {text}");
        assert_eq!(
            get("/design?tab=forms", "*/*").2,
            text,
            "an agent's default is the text"
        );
        let (_, _, html) = get("/design?tab=forms&view=html", "*/*");
        assert!(
            html.contains(r#"href="/design?tab=forms&amp;view=both" target="_top">Both"#),
            "the toggle"
        );
        // a family tab keeps the theme picked, as the window does
        let (_, _, dark) = get("/design?tab=forms&theme=dark&view=html", "*/*");
        assert!(
            dark.contains("app dark ds-root")
                && dark.contains(r#"href="/design?tab=data&#38;theme=dark""#),
            "the theme stays"
        );
        for path in [
            "/",
            "/graph",
            "/agents",
            "/comments",
            "/tools",
            "/watchers",
            "/design",
            "/tasks/a-b",
            "/d/a-b",
        ] {
            assert!(is_page(path), "{path}");
            let (status, kind, both) = get(&format!("{path}?view=both"), "*/*");
            assert_eq!((status, kind), ("200 OK", HTML), "{path}");
            assert!(
                both.contains(&format!(r#"src="{path}?view=html""#)),
                "{path}: {both}"
            );
            assert!(
                both.contains(&format!(r#"src="{path}?view=text""#)),
                "{path}: {both}"
            );
        }
        assert!(!is_page("/ping") && !is_page("/search"));
        // A page that only draws gets its text from its HTML: /graph's 404 here stays its own words.
        assert_eq!(get("/graph?view=text", "*/*").2, "no port, no page");
    }

    #[test]
    fn a_form_s_query_decodes() {
        let _budget = TestBudget::new();
        assert_eq!(form_decoded("a+b%20c%C3%A9"), "a b c\u{e9}");
        assert_eq!(form_decoded("100%"), "100%");
        assert_eq!(form_decoded("%zz%2"), "%zz%2");
        assert_eq!(form_decoded("%+a"), "% a", "not a byte");
    }

    /// Done-when 1 and 3 of rust-window-in-process: the window's page is the web's answer at the same URL, sidebar and style
    /// included, rendered in this process with no server on any port; its document shows the menu. The web's answer is the
    /// route `dash serve` runs, which needs no window.
    #[test]
    fn the_window_draws_the_web_s_page_in_its_own_process() {
        let _budget = TestBudget::new();
        let dir = no_app("window");
        // the board as dash serve holds it, behind its lock
        let served = Mutex::new(tasks::Board::open(&dir.0).expect("the tasks"));
        let board = || served.lock().expect("the board");
        for target in [
            "/tasks",
            "/tasks?layout=list",
            "/design",
            "/deps",
            "/comments",
        ] {
            let line = format!("GET {target} HTTP/1.1\r\n");
            // the board's reads of the older app end on their threads, each the next render's: both read once they settled
            let (mut window, mut web) = (String::new(), String::from("unread"));
            for _ in 0..50 {
                let (status, kind, answer) =
                    unqueued(&line, "text/html", Some(&served)).expect("a page dash serve answers");
                assert_eq!((status, kind), ("200 OK", HTML), "{target}: {answer}");
                (window, web) = (window_page(target, &board()), answer);
                if window == web {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(window, web, "{target}");
            assert!(window.contains(r#"id="dash-nav""#), "{target}: the sidebar");
            assert!(window.contains(".app.light {"), "{target}: the theme");
            // done-when 4 of rust-home-prompt: the « dash » is the window's home link too
            assert!(
                window.contains(r#"href="/" id="dash-home""#),
                "{target}: the home link"
            );
            let shown = dash_ui::shown(dioxus::prelude::VirtualDom::new_with_props(
                dash_ui::Page,
                dash_ui::PageProps { html: window },
            ));
            assert!(
                shown.contains("Inbox\nTasks\nsite_perso\n"),
                "{target}, the window's menu: {shown}"
            );
        }
        // a page that reads no store is rendered once a visit; the board each time it is asked
        assert!(reads_the_store("/tasks?layout=list") && reads_the_store("/tasks/a"));
        assert!(!reads_the_store("/mail") && !reads_the_store("/agents"));
        // a page that is not HTML, and one only the older app serves: the same sidebar over its words
        let text = window_page("/deps?view=text", &board());
        assert!(
            text.contains(r#"id="dash-nav""#) && text.contains("Tasks: /tasks"),
            "{text}"
        );
        let older = window_page("/approve", &board());
        assert!(
            older.contains(r#"href="http://127.0.0.1:7788/approve""#),
            "{older}"
        );
    }

    /// A data folder a window and a server read, with no older app, no worktrees and no real repo: the queue fails fast.
    /// nextest gives each test its own process (rust/CLAUDE.md rule 1), so nothing else reads these variables.
    fn no_app(name: &str) -> dash_meter::TestDir {
        let dir = dash_meter::TestDir::new(name);
        unsafe {
            std::env::set_var("DASH_CLI", "/nonexistent/dash");
            std::env::set_var("DASH_REPO", &dir.0);
            std::env::set_var("DASH_WORKTREES", &dir.0);
        }
        comments::serve_from(&dir.0);
        dir
    }

    /// A page that reads no store is drawn again only while a skeleton is on it, once a second, for five minutes a visit.
    #[test]
    fn the_window_fills_a_skeleton_once_a_second_for_five_minutes() {
        let _budget = TestBudget::new();
        let secs = Duration::from_secs;
        let busy = surface::screen::page(
            "200 OK",
            r#"{"root":"s","elements":{"s":{"type":"skeleton","props":{"what":"Usage"}}}}"#,
        )
        .expect("a page");
        assert!(refill(&busy, secs(2), secs(1)));
        assert!(
            !refill(&busy, secs(2), Duration::from_millis(250)),
            "not four times a second"
        );
        assert!(!refill(&busy, secs(300), secs(1)), "not past five minutes");
        let filled = busy.replace(BEING_READ, "");
        assert!(!refill(&filled, secs(2), secs(1)), "kept once filled");
    }

    /// The column a task's row sits in on a /tasks page: the status of the last group opened before it.
    fn column_of(page: &str, task: &str) -> Option<String> {
        let at = page.find(&format!(r#"data-task="{task}""#))?;
        let group = page[..at].rfind(r#"data-status=""#)? + r#"data-status=""#.len();
        let end = page[group..].find('"')?;
        Some(page[group..group + end].to_string())
    }

    /// Done-when 2 of rust-window-in-process: the window and dash serve each read the one store through their own differential
    /// dataflow views (`Board`'s, fed by its op log), and a write to it shows in both within 1 s: the window through its real
    /// render loop, the web through the page dash serve answers.
    #[test]
    fn a_store_write_shows_in_the_window_and_the_web_within_a_second() {
        use dash_core::tasks::TaskStore;
        let _budget = TestBudget::new();
        let bar = Duration::from_secs(1);
        // the window's timer leaves a render's time under the bar
        assert!(dash_ui::REREAD * 2 <= bar, "REREAD {:?}", dash_ui::REREAD);
        let dir = no_app("window-sync");
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store's writer");
        tasks.add("w-task", &[]).expect("added");
        // two processes in life, two boards here: each its own views over the one store
        let web = Mutex::new(tasks::Board::open(&dir.0).expect("the web's board"));
        let window = tasks::Board::open(&dir.0).expect("the window's board");
        let (asks, asked) = std::sync::mpsc::channel::<String>();
        let (drew, drawn) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            dash_ui::render_loop("/tasks?layout=list", window_pages(window), asked, |page| {
                drew.send(page).is_ok()
            })
        });
        let line = "GET /tasks?layout=list HTTP/1.1\r\n";
        let served = || unqueued(line, "text/html", Some(&web)).expect("a page").2;
        let first = drawn
            .recv_timeout(Duration::from_secs(10))
            .expect("the window's first page");
        assert_eq!(
            column_of(&first, "w-task").as_deref(),
            Some("backlog"),
            "{first}"
        );
        assert_eq!(column_of(&served(), "w-task").as_deref(), Some("backlog"));

        // both surfaces watched side by side, each timed on its own, 20 ms between the web's asks
        let wrote = Instant::now();
        TaskStore::claim(&mut tasks, "w-task").expect("claimed");
        let claimed = |page: &str| column_of(page, "w-task").as_deref() == Some("in-progress");
        let (mut window_took, mut web_took) = (None, None);
        while (window_took.is_none() || web_took.is_none()) && wrote.elapsed() < 5 * bar {
            for page in drawn.try_iter() {
                if window_took.is_none() && claimed(&page) {
                    window_took = Some(wrote.elapsed());
                }
            }
            if web_took.is_none() && claimed(&served()) {
                web_took = Some(wrote.elapsed());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // the render loop ends at its next wait
        drop(asks);
        eprintln!("the claim shown: window {window_took:?}, web {web_took:?}");
        let window_took = window_took.expect("the window drew the claim");
        let web_took = web_took.expect("the web served the claim");
        assert!(window_took < bar, "window: {window_took:?}");
        assert!(web_took < bar, "web: {web_took:?}");
    }
}
