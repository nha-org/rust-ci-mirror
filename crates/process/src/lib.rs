//! A command line tool's call, bounded: a tool that hangs never holds a run. Every adapter on such a tool takes it.
//!
//! The tool starts in its own process group, and a kill ends the whole group: a wrapper's child (`sh -c`, a script) would
//! outlive the wrapper and hold a reader thread on its pipe (/code-review of PR 1162). Its own group is also outside a
//! terminal's foreground group, so a tool that asks on /dev/tty is stopped by SIGTTIN, never prompts, and fails by its
//! limit (task process-run-group-kill: chosen over setsid, which would need an unsafe pre_exec for an earlier failure).
//! The cost: a kill of dash's own group (a Ctrl-C, `bin/bounded`) no longer reaches the tool, which then runs to its own
//! end or dies writing to its closed pipe, the trade `ext::pod` makes too; while dash lives, the call's limit ends it.
//! A command that must say what it did when cut (dash ship) takes [`on_cut`], which ends its tools' groups first.
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// `binary`, built beside this program, run in its place with its words: a role this one links none of (task
/// rust-binary-split; dash-gate's sizes::ROLES). It returns only when that one cannot start, saying why.
/// Its folder is this program's own, links resolved: a `dash` linked from elsewhere still finds its dash-window (/code-review).
pub fn beside(binary: &str) -> String {
    let (me, path) = match me() {
        Ok(me) => (me.clone(), me.with_file_name(binary)),
        // never a bare name, which PATH would find
        Err(e) => return format!("{binary}: this program's own path: {e}"),
    };
    // a role linked as the binary it hands to would exec itself forever (/code-review of #1627)
    if std::fs::canonicalize(&path).is_ok_and(|p| p == me) {
        return format!(
            "{}: is this program itself, without the role; rust/bin/dash builds it",
            path.display()
        );
    }
    let why = Command::new(&path).args(std::env::args_os().skip(1)).exec();
    format!("{}: {why}; rust/bin/dash builds it", path.display())
}

/// This program's path, links resolved: a server reached as dash-serve is `dash`, as a deploy's restart_look finds it.
pub fn me() -> Result<std::path::PathBuf, String> {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| e.to_string())
}

/// The Mac's free memory in percent, as `memory_pressure` prints it (« System-wide memory free percentage: 57% »), the reading
/// the older app's start gate and bin/orchestrator-job take; `None` when it cannot be read, which holds nothing back.
pub fn free_memory() -> Option<u8> {
    let out = std::process::Command::new("memory_pressure")
        .output()
        .ok()?;
    free_percent_in(&String::from_utf8_lossy(&out.stdout))
}

/// The free percentage in what `memory_pressure` printed; none when it says none.
pub fn free_percent_in(text: &str) -> Option<u8> {
    let (_, rest) = text.split_once("free percentage: ")?;
    rest.split('%').next()?.trim().parse().ok()
}

/// The machine's load per core: `at` 0 the 1-minute load, 1 the 5-minute one. Over the host's online cores, never
/// `available_parallelism`, which a cgroup quota or taskset lowers while the load stays host-wide (PR 1463's /code-review).
/// `None` unread.
pub fn load_per_core(at: usize) -> Option<f64> {
    let mut load = [0f64; 2];
    // SAFETY: at most two f64 written into a buffer of two
    let read = unsafe { libc::getloadavg(load.as_mut_ptr(), 2) };
    // SAFETY: sysconf reads a constant
    let cores = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    (read > at as i32 && cores > 0).then(|| load[at] / cores as f64)
}

/// The repository root the tool runs against: `DASH_REPO`, or `~/repo/site_perso`. Under nextest (`NEXTEST` set, inherited by
/// the dash a test starts) an unset `DASH_REPO` panics naming the test: a test never reads the live checkout, it gets a
/// scratch one (task rust-tests-no-live-repo; rust-search-port's tests read the live repo and cost a rerun).
pub fn repo() -> std::path::PathBuf {
    if let Some(v) = std::env::var_os("DASH_REPO") {
        return std::path::PathBuf::from(v);
    }
    if std::env::var_os("NEXTEST").is_some() {
        let test = std::env::var("NEXTEST_TEST_NAME").unwrap_or_else(|_| "a test".into());
        panic!(
            "{test}: DASH_REPO is unset under nextest: give the test a scratch repo, never the live checkout"
        );
    }
    let home = std::env::var("HOME").unwrap_or_else(|e| {
        eprintln!("HOME: {e}");
        std::process::exit(1);
    });
    std::path::PathBuf::from(home).join("repo/site_perso")
}

/// Now in Unix milliseconds: a clock set before 1970 reads 0.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Now in Unix seconds.
pub fn now_s() -> u64 {
    now_ms() as u64 / 1000
}

/// What a call holds of a tool's stdout, in bytes. Past it the tool is killed and the call fails naming it, under the memory cap.
pub const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// What a failure's reason keeps of stderr: its end, where a tool says what went wrong. The rest is read and dropped: a noisy tool
/// is not a failed one.
const REASON_LIMIT: usize = 64 * 1024;

/// Stdout; a failure names what it said on stderr, which the task's record then keeps.
pub fn run(
    bin: &str,
    args: &[&str],
    dir: Option<&Path>,
    limit: Duration,
) -> Result<String, String> {
    let (status, text, reason) = answer(bin, args, dir, &[], limit)?;
    if status.success() {
        return Ok(text);
    }
    let what = format!("{bin} {}", args.join(" "));
    Err(format!(
        "{what}: {status}: {} {}",
        reason.trim(),
        text.trim()
    ))
}

/// No answer from a tool. `started`: it ran, so it may have done its work (and spent) before it was killed or lost.
/// `timed_out`: it outlived its limit, killed then, told apart from output past its size or a lost wait (task hooks-run-built-dash).
#[derive(Debug)]
pub struct NoAnswer {
    pub started: bool,
    pub timed_out: bool,
    pub why: String,
}

impl From<NoAnswer> for String {
    fn from(no: NoAnswer) -> String {
        no.why
    }
}

/// Its exit status, stdout and, on a failure, stderr's end, whatever the status: a tool that failed may still say what it did.
/// `env` is set in the tool's environment alone, never on its command line, so no failure's words name it.
pub fn answer(
    bin: &str,
    args: &[&str],
    dir: Option<&Path>,
    env: &[(&str, &str)],
    limit: Duration,
) -> Result<(std::process::ExitStatus, String, String), NoAnswer> {
    answer_fed(bin, args, dir, env, None, limit)
}

/// `answer`, `stdin` written to the tool's input then closed: a text kept off its command line, where a tool may read a
/// leading `-` as a flag (/code-review of rust-task-page-edits: the description « --dry-run is broken » was refused).
pub fn answer_fed(
    bin: &str,
    args: &[&str],
    dir: Option<&Path>,
    env: &[(&str, &str)],
    stdin: Option<&str>,
    limit: Duration,
) -> Result<(std::process::ExitStatus, String, String), NoAnswer> {
    let what = format!("{bin} {}", args.join(" "));
    let ran = |why: String| NoAnswer {
        started: true,
        timed_out: false,
        why,
    };
    let failed = |e: std::io::Error| ran(format!("{what}: {e}"));
    let past = || ran(format!("{what}: its output is past {OUTPUT_LIMIT} bytes"));
    let mut command = Command::new(bin);
    command
        .args(args)
        .envs(env.iter().copied())
        .stdin(match stdin {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    // spawned under the cut's lock once a command took on_cut: a call either waits for the cut's exit or is in `running`
    // before the reporter looks; any other command takes no lock (/code-review of ship-outlives-curl)
    let mut cut = ARMED.load(std::sync::atomic::Ordering::SeqCst).then(held);
    let mut child = command.spawn().map_err(|e| NoAnswer {
        started: false,
        timed_out: false,
        why: format!("{what}: {e}"),
    })?;
    if let Some(cut) = cut.as_mut() {
        let mut line = vec![bin.to_string()];
        line.extend(args.iter().map(|a| a.to_string()));
        cut.running.push((child.id(), line));
    }
    drop(cut);
    let _running = Running(child.id());
    // on its own thread, so a tool that answers before it reads never blocks the wait; dropped there, the input closes
    if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = input.write_all(text.as_bytes());
        });
    }
    let out = drain(child.stdout.take().expect("piped"));
    let err = drain_reason(child.stderr.take().expect("piped"));
    // one deadline for the whole call: the waits after the leader's exit take what is left, never a limit each (a second,
    // at least, for a tool that ended at the deadline to have its pipes read)
    let deadline = Instant::now() + limit;
    let left = || {
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_secs(1))
    };
    let mut text = None;
    let succeeded = loop {
        text = text.or_else(|| out.try_recv().ok());
        if let Some(None) = &text {
            end(&mut child);
            return Err(past());
        }
        match exited(&child) {
            Ok(Some(succeeded)) => break succeeded,
            Ok(None) => {}
            // ECHILD: another reaper took the leader, so its group id may be a stranger's now: no kill (/code-review)
            Err(e) => return Err(failed(e)),
        }
        if Instant::now() > deadline {
            end(&mut child);
            return Err(NoAnswer {
                timed_out: true,
                ..ran(format!("{what}: no answer in {limit:?}"))
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // The leader is still unreaped, so its group id stays its own: each failure below ends what it left behind (a process
    // holding the pipes) and no stranger's group. Bounded too.
    let text = match text.map(Ok).unwrap_or_else(|| out.recv_timeout(left())) {
        Ok(Some(text)) => text,
        Ok(None) => {
            end(&mut child);
            return Err(past());
        }
        Err(_) => {
            end(&mut child);
            return Err(ran(format!("{what}: its output never closed")));
        }
    };
    let reason = match succeeded {
        true => String::new(),
        false => err.recv_timeout(left()).unwrap_or_else(|_| {
            end(&mut child);
            String::new()
        }),
    };
    untrack(child.id());
    let status = child.wait().map_err(failed)?;
    Ok((status, text, reason))
}

/// Whether the tool ended, and with success, its leader left unreaped (WNOWAIT) so its group id cannot pass to another.
fn exited(child: &std::process::Child) -> std::io::Result<Option<bool>> {
    // SAFETY: a zeroed siginfo_t is valid, and waitid writes only into it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    if unsafe { libc::waitid(libc::P_PID, child.id(), &mut info, flags) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if info.si_signo == 0 {
        return Ok(None);
    }
    // SAFETY: waitid filled a SIGCHLD siginfo, whose status field is the one set.
    let status = unsafe { info.si_status() };
    Ok(Some(info.si_code == libc::CLD_EXITED && status == 0))
}

/// The tool's whole group, then its leader reaped: a wrapper's child dies with it.
fn end(child: &mut std::process::Child) {
    untrack(child.id());
    // SAFETY: killpg takes no pointer; the group is the tool's own (process_group(0)) and its leader is not yet reaped,
    // so the id names no other group. ESRCH, a group already gone, needs nothing.
    unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
    let _ = child.wait();
}

/// A pipe read beside the wait: an answer past its size would block the tool and read as a hang. The text, or none past
/// `OUTPUT_LIMIT`: the limit counts the text, since a stray byte's replacement is up to three times its size, and never smaller.
fn drain(pipe: impl Read + Send + 'static) -> Receiver<Option<String>> {
    let (send, receive) = channel();
    std::thread::spawn(move || {
        let mut bytes = vec![];
        let _ = pipe.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes);
        let text = text_of(bytes);
        let _ = send.send((text.len() as u64 <= OUTPUT_LIMIT).then_some(text));
    });
    receive
}

fn drain_reason(mut pipe: impl Read + Send + 'static) -> Receiver<String> {
    let (send, receive) = channel();
    std::thread::spawn(move || {
        let (mut tail, mut chunk) = (vec![], [0; 8192]);
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => tail.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
            if tail.len() > 2 * REASON_LIMIT {
                tail.drain(..tail.len() - REASON_LIMIT);
            }
        }
        let _ = send.send(text_of(
            tail.split_off(tail.len().saturating_sub(REASON_LIMIT)),
        ));
    });
    receive
}

/// No copy of valid text: at the limit a second one is 16 MB for nothing.
fn text_of(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// The cut's state: [`held`]'s lock.
pub struct Cut {
    on: bool,
    /// the command's work ended: a cut after it reports nothing, the command exits with its own code
    pub done: bool,
    /// each call's tool, by its group's leader and its command line, the binary first
    running: Vec<(u32, Vec<String>)>,
}

static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CUT: std::sync::Mutex<Cut> = std::sync::Mutex::new(Cut {
    on: false,
    done: false,
    running: vec![],
});
thread_local! {
    static REPORTER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn cut() -> std::sync::MutexGuard<'static, Cut> {
    CUT.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Out of `running` under the lock before its leader is reaped: the reporter never kills a group id the OS may reuse.
fn untrack(pid: u32) {
    if ARMED.load(std::sync::atomic::Ordering::SeqCst) {
        cut().running.retain(|(p, _)| *p != pid);
    }
}

/// A call's tool, out of `running` once its call ends, on every return.
struct Running(u32);

impl Drop for Running {
    fn drop(&mut self) {
        untrack(self.0);
    }
}

/// The cut's lock, or no return once a cut began, but on the reporter's thread. A command that reports on its cut calls it
/// last, so its own thread never exits before the report.
pub fn held() -> std::sync::MutexGuard<'static, Cut> {
    let cut = cut();
    if cut.on && !REPORTER.with(std::cell::Cell::get) {
        drop(cut);
        loop {
            std::thread::park();
        }
    }
    cut
}

/// On SIGINT, SIGTERM or SIGHUP (a caller's timeout, a closed terminal, a kill): no call starts a tool after it but the
/// report's; each running tool whose command line `kill` names (a long check, a network call) has its group killed, the others
/// (a commit, a rebase: killed, they leave a lock or a half-done rebase) run to their end; then `report(signal)` prints and
/// the program exits 128 + the signal. A report prints what it knows locally first: a caller's timeout sends its KILL soon
/// after (bin/bounded, 2 s). Once per process: dash runs one command per process. Task ship-outlives-curl: a ship cut after
/// 5 minutes said nothing of what landed.
/// The others are waited on 10 s at most, then the report says what it sees. A report that panics (a closed terminal) still
/// exits. A cut after the command set [`Cut::done`] reports nothing.
pub fn on_cut(
    report: impl FnOnce(i32) + Send + 'static,
    kill: impl Fn(&[String]) -> bool + Send + 'static,
) {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP]) else {
        return;
    };
    std::thread::spawn(move || {
        let Some(signal) = signals.forever().next() else {
            return;
        };
        REPORTER.with(|r| r.set(true));
        let mut c = cut();
        if c.done {
            return;
        }
        c.on = true;
        for (pid, what) in &c.running {
            if kill(what) {
                unsafe { libc::killpg(*pid as libc::pid_t, libc::SIGKILL) };
            }
        }
        drop(c);
        let limit = Instant::now() + Duration::from_secs(10);
        while Instant::now() < limit && cut().running.iter().any(|(_, what)| !kill(what)) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| report(signal)));
        std::process::exit(128 + signal);
    });
    ARMED.store(true, std::sync::atomic::Ordering::SeqCst);
}
