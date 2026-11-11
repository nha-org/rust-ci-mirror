//! :7799, the Elixir app frozen at COMMIT, for side-by-side comparison with each Rust port (task rust-elixir-reference; @nha,
//! 2026-11-10: « I would like http://localhost:64151/mail to be on some port I can always compare like
//! http://localhost:7799/mail »). Each port deletes its Elixir route, so the live older app on :7787 loses pages; this one
//! keeps every route from before the Rust takeover. `dash serve --port <n> --older <m>` with `DASH_REFERENCE=<port>` holds
//! that port and starts the app on the first request, then stops it after 30 idle minutes, as `mcp-proxy --lazy` does for
//! an MCP server. A variable, not a flag, as DASH_ORCH: a rollback to the binary before ignores it and still serves.
//!
//! The app runs from its own detached worktree, `.claude/run/reference/<sha>`, built once, on a copy of the live database
//! taken at each start. `DASHBOARD_LIVE` names that worktree: no code reloader and no watchers (config/dev.exs), and the
//! checkout is not the main one, so it runs no jobs, no dump and no backup (Dashboard.Application, Dashboard.Dump).
//!
//! Each connection is a span, `reference.request`, its resource the first request's path, and a start of the app is its child,
//! `reference.cold_start` (task rust-alerts-rust-pages). Never `http.request`: the slow-page monitor reads that operation only,
//! so a boot of minutes, or a LiveView socket open for hours, is seen in APM and never alerted.
use dash_statsd::Statsd;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The port the reference answers on, from `DASH_REFERENCE` (Makefile's rust/serve sets 7799); unset, none.
pub fn asked() -> Option<u16> {
    std::env::var("DASH_REFERENCE").ok()?.parse().ok()
}
/// The last commit before Rust took Elixir's routes (rust/CLAUDE.md rule 5's, 2026-11-09).
pub const COMMIT: &str = "72dca0cd25e5ad03a30712a78ea6e8404bc3fdbf";
const IDLE: Duration = Duration::from_secs(30 * 60);
/// How often an idle app is looked for.
const TICK: Duration = Duration::from_secs(60);
/// The longest a boot may take before its /dashboard/ping answers; the build before it has no limit.
const BOOT: Duration = Duration::from_secs(300);
/// A connection's span and its start's child: their names, never the slow-page monitor's `http.request`.
const SPAN: &str = "reference.request";
const COLD_START: &str = "reference.cold_start";
/// The most of a request's head read before the app: its first line, for the span's resource.
const HEAD: usize = 8192;
/// How long a connection may wait to send its head: one that sends nothing by then is handed on with no resource.
const HEAD_WAIT: Duration = Duration::from_secs(10);

/// A started app: its port, and how it stops.
pub struct Running {
    pub port: u16,
    pub stop: Box<dyn FnOnce() + Send>,
}

struct State {
    running: Option<Running>,
    /// Connections open now: a page's LiveView socket holds one for as long as the tab is open.
    open: usize,
    last: Instant,
    /// The last start's failure and when: the connections queued behind it get it at once, never a start each (/code-review).
    failed: Option<(Instant, String)>,
}

/// How long a failed start answers every request with its failure before one tries again.
const RETRY: Duration = Duration::from_secs(30);

/// An app started on its first connection and stopped once no connection was open for `idle`.
pub struct Lazy<S> {
    start: S,
    idle: Duration,
    state: Mutex<State>,
}

impl<S: Fn() -> Result<Running, String>> Lazy<S> {
    pub fn new(start: S, idle: Duration) -> Self {
        let state = State {
            running: None,
            open: 0,
            last: Instant::now(),
            failed: None,
        };
        Lazy {
            start,
            idle,
            state: Mutex::new(state),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The app's port, started first if it is not running, `started` called once that start ends, failed or not: a connection
    /// opens. The lock is held through a start, so a second first request waits for the same app instead of starting another.
    fn opened(&self, started: impl FnOnce()) -> Result<u16, String> {
        let mut state = self.state();
        if let Some((at, why)) = &state.failed
            && at.elapsed() < RETRY
        {
            return Err(why.clone());
        }
        if state.running.is_none() {
            let start = (self.start)();
            started();
            match start {
                Ok(running) => state.running = Some(running),
                Err(why) => {
                    state.failed = Some((Instant::now(), why.clone()));
                    return Err(why);
                }
            }
        }
        state.failed = None;
        state.open += 1;
        state.last = Instant::now();
        Ok(state.running.as_ref().map_or(0, |r| r.port))
    }

    /// A connection ends. `dead`: the app refused it, so it is stopped and the next request starts it again.
    fn closed(&self, dead: bool) {
        let mut state = self.state();
        state.open = state.open.saturating_sub(1);
        state.last = Instant::now();
        if dead && let Some(running) = state.running.take() {
            (running.stop)();
        }
    }

    /// Stops the app once no connection was open for `idle`; whether it did.
    pub fn reap(&self) -> bool {
        let mut state = self.state();
        if state.open > 0 || state.last.elapsed() < self.idle {
            return false;
        }
        match state.running.take() {
            Some(running) => {
                (running.stop)();
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    fn running(&self) -> bool {
        self.state().running.is_some()
    }
}

/// Answers `listener`, each connection on its own thread, handed byte for byte to the app; a reaper looks every `tick`.
pub fn serve<S>(listener: TcpListener, lazy: Arc<Lazy<S>>, tick: Duration, report: Arc<Statsd>)
where
    S: Fn() -> Result<Running, String> + Send + Sync + 'static,
{
    let reaper = Arc::clone(&lazy);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(tick);
            if reaper.reap() {
                eprintln!(
                    "reference: stopped after {} idle minutes",
                    reaper.idle.as_secs() / 60
                );
            }
        }
    });
    for client in listener.incoming().flatten() {
        let (lazy, report) = (Arc::clone(&lazy), Arc::clone(&report));
        std::thread::spawn(move || handed(&client, &lazy, &report));
    }
}

fn handed<S: Fn() -> Result<Running, String>>(client: &TcpStream, lazy: &Lazy<S>, report: &Statsd) {
    // the head read first, for the span's resource, then handed on to the app as it came
    let mut head = vec![0u8; HEAD];
    let _ = client.set_read_timeout(Some(HEAD_WAIT));
    let read = (&*client).read(&mut head).unwrap_or(0);
    let _ = client.set_read_timeout(None);
    head.truncate(read);
    let root = report.span(SPAN, path_of(&head));
    // finished only when this connection ran a start, a failed one too: one that waited behind another's is no cold start
    let boot = root.child(COLD_START, "boot");
    let port = match lazy.opened(|| boot.finish()) {
        Ok(port) => port,
        Err(e) => {
            eprintln!("reference: {e}");
            // what follows the head read too, a body sent apart included: a close with bytes unread resets the connection
            // before the answer is read (/code-review)
            let _ = client.set_read_timeout(Some(Duration::from_secs(1)));
            let _ = (&*client).read(&mut [0u8; HEAD]);
            let body = format!("the Elixir reference did not start: {e}\n");
            let _ = write!(
                &*client,
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            root.finish();
            return;
        }
    };
    // tried three times before the app counts as dead: a full accept queue refuses one connect, and a stop would cut every tab
    let mut upstream = TcpStream::connect(("127.0.0.1", port));
    for _ in 0..2 {
        if upstream.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
        upstream = TcpStream::connect(("127.0.0.1", port));
    }
    // an app that takes the connection and drops it before the head is written is dead too (/code-review)
    let dead = match upstream {
        Ok(upstream) if (&upstream).write_all(&head).is_ok() => {
            piped(client, &upstream);
            false
        }
        _ => true,
    };
    lazy.closed(dead);
    root.finish();
}

/// The path of a request's first line, its query cut: a span's resource. No head, or not one, is `-`.
fn path_of(head: &[u8]) -> &str {
    let line = head.split(|&b| b == b'\n').next().unwrap_or_default();
    let path = std::str::from_utf8(line).map_or("", line_path);
    if path.is_empty() { "-" } else { path }
}

/// The path of a request line, its query cut; none is empty. The front door's span resource too (`request_path`).
pub(crate) fn line_path(line: &str) -> &str {
    line.split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
}

/// Bytes both ways until either side ends: a page, a post and a LiveView socket alike.
fn piped(client: &TcpStream, upstream: &TcpStream) {
    let (Ok(up), Ok(down)) = (client.try_clone(), upstream.try_clone()) else {
        return;
    };
    let sending = std::thread::spawn(move || {
        let _ = std::io::copy(&mut &up, &mut &down);
        // its write side only: a client that half-closes after its request still reads the answer (/code-review)
        let _ = down.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut &*upstream, &mut &*client);
    let _ = client.shutdown(Shutdown::Both);
    let _ = upstream.shutdown(Shutdown::Both);
    let _ = sending.join();
}

/// `at` (:7799) for good, beside `dash serve`: a port held by another process is tried again each minute, so the server takes
/// it once that one ends, with no restart.
pub fn hold(repo: PathBuf, at: u16, report: Arc<Statsd>) {
    std::thread::spawn(move || {
        // at once, not on the next request: an orphan holds hundreds of MB, and :7799 may not be asked for days
        orphan_ended(&home(&repo).join("app.pid"));
        let listener = loop {
            match TcpListener::bind(("127.0.0.1", at)) {
                Ok(listener) => break listener,
                Err(e) => {
                    eprintln!("reference: :{at}: {e}, tried again in a minute");
                    std::thread::sleep(TICK);
                }
            }
        };
        let lazy = Lazy::new(move || elixir(&repo, &home(&repo)), IDLE);
        serve(listener, Arc::new(lazy), TICK, report);
    });
}

/// The reference's folder: its worktree, its logs and the pid of the app running.
fn home(repo: &Path) -> PathBuf {
    repo.join(".claude/run/reference")
}

/// The app at COMMIT, built if it is not, on a copy of the live database, booted on a free port.
fn elixir(repo: &Path, home: &Path) -> Result<Running, String> {
    let tree = home.join(&COMMIT[..12]);
    built(repo, &tree, &home.join("build.log"))?;
    let app = tree.join("private/dashboard");
    let pid_file = home.join("app.pid");
    orphan_ended(&pid_file);
    // the copy taken after: that app read the last one
    dash_store::older::snapshot(
        &repo.join("private/dashboard/dashboard_dev.db"),
        &app.join("dashboard_dev.db"),
    )?;
    let port = free_port()?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("app.log"))
        .map_err(|e| format!("{}: {e}", home.display()))?;
    let mut boot = in_app(&app, &tree);
    boot.args(["mix", "phx.server"])
        .env("DASHBOARD_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    // its own group, so the toolchain's beam ends with it (private/dashboard/CLAUDE.md rule 1)
    std::os::unix::process::CommandExt::process_group(&mut boot, 0);
    let mut child = boot.spawn().map_err(|e| format!("mix phx.server: {e}"))?;
    let pid = child.id() as i32;
    let _ = std::fs::write(&pid_file, pid.to_string());
    let began = Instant::now();
    while !pings(port) {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the app ended at boot ({status}), see {}",
                home.join("app.log").display()
            ));
        }
        if began.elapsed() > BOOT {
            ended(pid, &mut child);
            return Err(format!(
                "no /dashboard/ping in {} s, see {}",
                BOOT.as_secs(),
                home.join("app.log").display()
            ));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    eprintln!(
        "reference: {} up on :{port} in {} s",
        &COMMIT[..12],
        began.elapsed().as_secs()
    );
    Ok(Running {
        port,
        stop: Box::new(move || {
            ended(pid, &mut child);
            let _ = std::fs::remove_file(&pid_file);
        }),
    })
}

/// A command run by the worktree's toolchain in its app folder, the live app's variables gone: the build bin/live names
/// (DASHBOARD_BUILD) would take :7787, and CRM_LIVE_DIR would write his live queue.
fn in_app(app: &Path, tree: &Path) -> Command {
    let mut command = Command::new(app.join("bin/toolchain"));
    command.current_dir(app);
    for (name, _) in std::env::vars_os() {
        let name_ = name.to_string_lossy();
        if ["DASHBOARD_", "CRM_", "DASH_"]
            .iter()
            .any(|p| name_.starts_with(p))
        {
            command.env_remove(&name);
        }
    }
    command.env("MIX_ENV", "dev").env("DASHBOARD_LIVE", tree);
    command
}

/// The worktree at COMMIT, compiled once as bin/live builds a commit, `.built` its mark.
fn built(repo: &Path, tree: &Path, log: &Path) -> Result<(), String> {
    if tree.join(".built").exists() {
        return Ok(());
    }
    eprintln!(
        "reference: building {} in {}, see {}",
        &COMMIT[..12],
        tree.display(),
        log.display()
    );
    let parent = tree.parent().unwrap_or(tree);
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let logged = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .map_err(|e| format!("{}: {e}", log.display()))
    };
    let run = |mut command: Command, what: &str| -> Result<(), String> {
        let out = logged()?;
        let status = command
            .stdin(Stdio::null())
            .stdout(out.try_clone().map_err(|e| e.to_string())?)
            .stderr(out)
            .status()
            .map_err(|e| format!("{what}: {e}"))?;
        match status.success() {
            true => Ok(()),
            false => Err(format!("{what}: {status}, see {}", log.display())),
        }
    };
    if !tree.join(".git").exists() {
        // a folder removed by hand stays registered, and `worktree add` refuses its path until pruned (/code-review)
        let mut prune = Command::new("git");
        prune.arg("-C").arg(repo).args(["worktree", "prune"]);
        run(prune, "git worktree prune")?;
        let mut add = Command::new("git");
        // no hooks: a post-checkout hook is for a session's checkout, not this copy (bin/live does the same)
        add.arg("-C").arg(repo).args([
            "-c",
            "core.hooksPath=/dev/null",
            "worktree",
            "add",
            "-q",
            "--detach",
        ]);
        add.arg(tree).arg(COMMIT);
        run(add, "git worktree add")?;
    }
    let app = tree.join("private/dashboard");
    let mix = |words: &[&str]| {
        let mut command = in_app(&app, tree);
        command.arg("mix").args(words);
        command
    };
    run(mix(&["deps.get"]), "mix deps.get")?;
    run(mix(&["deps.compile"]), "mix deps.compile")?;
    // the deps that render HEEx compile again with DASHBOARD_LIVE, so the pages carry no debug annotations (bin/live's heex_deps)
    let mut heex = vec!["deps.compile", "--force", "phoenix_live_view"];
    let names = heex_deps(&app.join("deps"));
    heex.extend(names.iter().map(String::as_str));
    run(mix(&heex), "mix deps.compile --force")?;
    run(mix(&["do", "compile", "+", "assets.build"]), "mix compile")?;
    std::fs::write(tree.join(".built"), "").map_err(|e| e.to_string())
}

/// The deps under `deps` that depend on LiveView.
fn heex_deps(deps: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(deps) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| {
            std::fs::read_to_string(e.path().join("mix.exs"))
                .is_ok_and(|mix| mix.contains("{:phoenix_live_view,"))
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

/// Whether the app on `port` answers its ping with a 200.
fn pings(port: u16) -> bool {
    let Ok(mut stream) =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2))
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let asked = write!(
        stream,
        "GET /dashboard/ping HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    );
    let mut head = [0u8; 12];
    asked.is_ok() && stream.read_exact(&mut head).is_ok() && head.ends_with(b"200")
}

/// Its group sent TERM, then KILL after 20 s, and the child reaped.
fn ended(pid: i32, child: &mut Child) {
    stopped(pid);
    for _ in 0..20 {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    unsafe { libc::kill(-pid, libc::SIGKILL) };
    let _ = child.wait();
}

fn stopped(pid: i32) {
    // never 0 or 1: kill(0) is this server's own group, kill(-1) every process
    if pid > 1 {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
    }
}

/// The app a server before this one left running, named by `pid_file` (a deploy ends the server, not the group it started):
/// its group sent TERM, KILL if it still lives 20 s later, so no snapshot is written under it. ponytail: a group id reused
/// since is ended too; a start token beside the pid if that ever bites.
fn orphan_ended(pid_file: &Path) {
    let pid = std::fs::read_to_string(pid_file).ok();
    let Some(pid) = pid
        .and_then(|p| p.trim().parse::<i32>().ok())
        .filter(|&p| p > 1)
    else {
        return;
    };
    stopped(pid);
    let gone = || unsafe { libc::kill(-pid, 0) } != 0;
    for _ in 0..20 {
        if gone() {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if !gone() {
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = std::fs::remove_file(pid_file);
    eprintln!("reference: pid {pid}, left running by a server before, ended");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// A stand-in app: a loopback server answering `hi`, its starts counted and its stop seen.
    fn stand_in(
        starts: Arc<AtomicUsize>,
        stopped: Arc<AtomicBool>,
    ) -> impl Fn() -> Result<Running, String> {
        move || {
            starts.fetch_add(1, Ordering::SeqCst);
            stopped.store(false, Ordering::SeqCst);
            let app = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
            let port = app.local_addr().map_err(|e| e.to_string())?.port();
            std::thread::spawn(move || {
                for mut stream in app.incoming().flatten() {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
                    );
                }
            });
            let stopped = Arc::clone(&stopped);
            Ok(Running {
                port,
                stop: Box::new(move || stopped.store(true, Ordering::SeqCst)),
            })
        }
    }

    fn asked(port: u16) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("the reference port");
        write!(stream, "GET /mail HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("asked");
        let mut answer = String::new();
        stream.read_to_string(&mut answer).expect("its answer");
        answer
    }

    /// A report whose switch is off: no span sent.
    fn quiet() -> Arc<Statsd> {
        Arc::new(Statsd::new(
            Path::new("/nonexistent/dash-reference"),
            "serve",
        ))
    }

    fn proxy(idle: Duration, tick: Duration) -> (u16, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let (starts, stopped) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let lazy = Arc::new(Lazy::new(
            stand_in(Arc::clone(&starts), Arc::clone(&stopped)),
            idle,
        ));
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
        std::thread::spawn(move || serve(listener, lazy, tick, quiet()));
        (port, starts, stopped)
    }

    /// Done-when 2 of rust-elixir-reference, its first case: nothing starts before a request; the first starts the app and
    /// gets its answer, and the next one reaches the same app.
    #[test]
    fn the_app_starts_on_the_first_request_and_serves_the_next() {
        let _budget = dash_meter::memory::TestBudget::new();
        let (port, starts, _) = proxy(Duration::from_secs(3600), Duration::from_secs(3600));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            starts.load(Ordering::SeqCst),
            0,
            "started before any request"
        );
        assert!(asked(port).ends_with("\r\n\r\nhi"));
        assert!(asked(port).ends_with("\r\n\r\nhi"));
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    /// Done-when 2, its second case: once no connection was open for the idle time, the app stops; an open one keeps it up,
    /// and a request after the stop starts it again.
    #[test]
    fn the_app_stops_once_idle_and_a_request_starts_it_again() {
        let _budget = dash_meter::memory::TestBudget::new();
        let starts = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let lazy = Lazy::new(
            stand_in(Arc::clone(&starts), Arc::clone(&stopped)),
            Duration::from_millis(200),
        );
        let cold = std::cell::Cell::new(false);
        let port = lazy.opened(|| cold.set(true)).expect("started");
        assert!(cold.get(), "the first open started no app");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!lazy.reap(), "stopped while a connection was open");
        lazy.closed(false);
        assert!(!lazy.reap(), "stopped before the idle time");
        std::thread::sleep(Duration::from_millis(300));
        assert!(lazy.reap() && stopped.load(Ordering::SeqCst) && !lazy.running());
        assert_ne!(lazy.opened(|| ()).expect("started again"), 0);
        assert_eq!((starts.load(Ordering::SeqCst), port != 0), (2, true));
        // the server's own reaper does the same unasked
        let (port, _, stopped) = proxy(Duration::from_millis(200), Duration::from_millis(50));
        assert!(asked(port).ends_with("hi"));
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            stopped.load(Ordering::SeqCst),
            "the reaper left an idle app up"
        );
    }

    /// The roots of the next `n` traces put to `agent`, each with its children's names: (name, resource, children).
    fn traces(agent: &TcpListener, n: usize) -> Vec<(String, String, Vec<String>)> {
        let mut roots = vec![];
        for stream in agent.incoming().take(n) {
            let mut stream = std::io::BufReader::new(stream.expect("a trace"));
            let mut length = 0;
            loop {
                let mut line = String::new();
                std::io::BufRead::read_line(&mut stream, &mut line).expect("its head");
                if let Some(n) = line.strip_prefix("Content-Length: ") {
                    length = n.trim().parse().expect("a length");
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut body = vec![0; length];
            stream.read_exact(&mut body).expect("its body");
            let _ = stream
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            let trace: Vec<Vec<serde_json::Value>> = serde_json::from_slice(&body).expect("v0.3");
            let spans = &trace[0];
            let field =
                |s: &serde_json::Value, f: &str| s[f].as_str().unwrap_or_default().to_string();
            let root = spans.iter().find(|s| s["parent_id"] == 0).expect("a root");
            let children = spans
                .iter()
                .filter(|s| s["parent_id"] == root["span_id"])
                .map(|s| field(s, "name"))
                .collect();
            roots.push((field(root, "name"), field(root, "resource"), children));
        }
        roots
    }

    /// Task rust-alerts-rust-pages, done-when 3: each request through :7799 reaches Datadog as a `reference.request` span, its
    /// path the resource; the request that started the app carries a `reference.cold_start` child, the next none. Never
    /// `http.request`, the one operation the slow-page monitor reads (rfc-rust-unified-datadog.exs), so never alerted.
    #[test]
    fn each_request_is_a_span_and_its_cold_start_is_named() {
        let _budget = dash_meter::memory::TestBudget::new();
        let dir = dash_meter::TestDir::new("reference-spans");
        let agent = TcpListener::bind(("127.0.0.1", 0)).expect("the trace agent");
        let to = agent.local_addr().expect("its address");
        std::fs::write(dir.0.join("telemetry"), "127.0.0.1:9").expect("the switch on");
        let report = Arc::new(Statsd::with_traces(&dir.0, "serve", to));
        let (starts, stopped) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let lazy = Arc::new(Lazy::new(stand_in(Arc::clone(&starts), stopped), IDLE));
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
        std::thread::spawn(move || serve(listener, lazy, TICK, report));
        assert!(
            asked(port).ends_with("\r\n\r\nhi"),
            "the head was not handed on"
        );
        assert!(asked(port).ends_with("\r\n\r\nhi"));
        let mut roots = traces(&agent, 2);
        roots.sort_by_key(|(_, _, children)| children.len());
        assert_eq!(
            roots,
            vec![
                (SPAN.into(), "/mail".into(), vec![]),
                (SPAN.into(), "/mail".into(), vec![COLD_START.to_string()]),
            ]
        );
        assert_ne!(SPAN, "http.request", "the slow-page monitor would read it");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(path_of(b""), "-");
        assert_eq!(
            path_of(b"GET /v/a.html?x=1 HTTP/1.1\r\nHost: h\r\n"),
            "/v/a.html"
        );
    }

    /// Done-when 1 of rust-elixir-reference, on this Mac only (it builds Elixir, 6 to 10 minutes the first time): the app at
    /// COMMIT, in its own worktree, on a copy of the live database, answers /mail with a 200, then stops. Its home is a temp
    /// folder, kept between runs so the build is made once: a session's sandbox cannot write the main checkout's .claude/run.
    /// `cargo nextest run -p dash --profile long --run-ignored only -E 'test(the_pinned_app_answers_mail)'`
    #[test]
    #[ignore = "host: builds and boots the Elixir app"]
    fn the_pinned_app_answers_mail() {
        let _budget = dash_meter::memory::TestBudget::new();
        let (repo, at) = (
            crate::pages::repo(),
            std::env::temp_dir().join("dash-reference"),
        );
        let tree = at.join(&COMMIT[..12]);
        let lazy = Arc::new(Lazy::new(move || elixir(&repo, &at), IDLE));
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
        let served = Arc::clone(&lazy);
        std::thread::spawn(move || serve(listener, served, TICK, quiet()));
        let answer = asked(port);
        assert!(
            answer.starts_with("HTTP/1.1 200"),
            "{}",
            &answer[..answer.len().min(400)]
        );
        let head = Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git");
        assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), COMMIT);
        let mut state = lazy.state();
        (state.running.take().expect("running").stop)();
        eprintln!(
            "/mail answered {} bytes from {}",
            answer.len(),
            tree.display()
        );
    }

    /// A start that fails answers a 502 naming why; the requests queued behind it get the same failure with no start of their
    /// own, and one after RETRY tries again (/code-review: each queued connection reran the whole build).
    #[test]
    fn a_failed_start_answers_502_and_is_tried_again() {
        let _budget = dash_meter::memory::TestBudget::new();
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&tries);
        let lazy = Arc::new(Lazy::new(
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Err("no build".to_string())
            },
            IDLE,
        ));
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("address").port();
        let served = Arc::clone(&lazy);
        std::thread::spawn(move || serve(listener, served, TICK, quiet()));
        let answer = asked(port);
        assert!(
            answer.starts_with("HTTP/1.1 502") && answer.ends_with("no build\n"),
            "{answer}"
        );
        assert!(asked(port).ends_with("no build\n"));
        assert_eq!(
            tries.load(Ordering::SeqCst),
            1,
            "a queued request started again"
        );
        if let Some((at, _)) = &mut lazy.state().failed {
            *at = Instant::now().checked_sub(RETRY).expect("an instant");
        }
        asked(port);
        assert_eq!(
            tries.load(Ordering::SeqCst),
            2,
            "not tried again after RETRY"
        );
    }
}
