//! `dash doctor` (task rust-doctor-xprotect; @nha, 2026-11-14: « Let's add a check to make setup. I assume it needs to
//! capture sudo anyway already »): is this Mac quick to build on? macOS scans each new executable on its first run
//! (XProtect, through syspolicyd), 0.3 s here and 8 to 21 s under load, unless the app macOS holds responsible for the
//! process holds the Developer Tools grant, which a process started before the grant never sees. Spotlight indexes every
//! build output unless its folder holds `.metadata_never_index`. Then the older checklist,
//! `private/dashboard/priv/commands/doctor`. The steps left go to [`RECORD`], which `dash handoff` lists until a later run
//! finds them held.
use dash_core::WallClock;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const USAGE: &str = "dash doctor [--sudo] (a fresh binary's first-run cost, each builder's Developer Tools grant, \
Spotlight off the build caches, then this Mac's checklist; make setup passes --sudo, its one password ask)";

/// The most a fresh binary's first run may cost over its second.
pub const GAP: Duration = Duration::from_millis(100);
/// The steps left, under the repository: `dash_store::older_handoff::DOCTOR` reads it (dash's tests/doctor.rs holds the two
/// the same).
pub const RECORD: &str = "private/live/doctor.json";
/// The step for an app the grant lacks, its name after it.
pub const DEV_TOOLS: &str =
    "System Settings > Privacy & Security > Developer Tools: add and turn on";
/// The grants' database and Developer Tools' service in it; 2 is allowed.
const TCC: &str = "/Library/Application Support/com.apple.TCC/TCC.db";
const QUERY: &str = "select client, last_modified from access where service = 'kTCCServiceDeveloperTool' and auth_value = 2";
const MARKER: &str = ".metadata_never_index";
/// What runs a build: its responsible app is the one the grant must name. Not sccache's server, its own responsible
/// process: SCCACHE_CLIENT_SIDE (rust/.cargo/config.toml) runs each compile in cargo's process, the server only keeps the
/// cache (its first run here named « sccache » an app to grant). rust-analyzer's app is the editor that runs its cargo.
const BUILDERS: [&str; 3] = ["cargo", "rustc", "rust-analyzer"];

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// libSystem's, private and there since macOS 10.14: the process macOS holds responsible for `pid`, the app at the top
    /// of its chain, whose grants the chain gets.
    fn responsibility_get_pid_responsible_for_pid(pid: libc::pid_t) -> libc::pid_t;
}

/// Part 1: a fresh binary's two runs, none when the first costs under [`GAP`] more than the second.
pub fn first_run(first: Duration, second: Duration) -> Option<String> {
    let gap = first.saturating_sub(second);
    (gap >= GAP).then(|| {
        format!(
            "a fresh binary's first run costs {} ms more than its second (under {} ms passes): macOS scans it",
            gap.as_millis(),
            GAP.as_millis()
        )
    })
}

/// A binary no one ran yet, built by cc (the linker every Rust build needs) around a nonce of its own so its code hash is
/// new, run twice: each run's time.
fn probe() -> Result<(Duration, Duration), String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let nonce = format!("{}-{nanos}", std::process::id());
    let path = std::env::temp_dir().join(format!("dash-doctor-probe-{nonce}"));
    let src =
        format!("const char *nonce = \"{nonce}\";\nint main(void) {{ return nonce[0] == 0; }}\n");
    let mut cc = Command::new("cc")
        .args(["-x", "c", "-", "-o"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "no cc: xcode-select --install".to_string(),
            _ => format!("cc: {e}"),
        })?;
    let fed = cc.stdin.take().expect("piped").write_all(src.as_bytes());
    let built = cc.wait_with_output().map_err(|e| format!("cc: {e}"))?;
    if fed.is_err() || !built.status.success() {
        return Err(format!(
            "cc: {}",
            String::from_utf8_lossy(&built.stderr).trim()
        ));
    }
    let run = || {
        let t = Instant::now();
        Command::new(&path)
            .status()
            .map(|_| t.elapsed())
            .map_err(|e| format!("the probe: {e}"))
    };
    let times = run().and_then(|first| run().map(|second| (first, second)));
    let _ = std::fs::remove_file(&path);
    times
}

/// An app a build runs under: the one macOS holds responsible for a builder.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    /// What TCC.db names it by: its bundle id, else its executable's path.
    pub client: String,
    /// When its process started, in Unix seconds.
    pub started: i64,
}

#[cfg(target_os = "macos")]
fn path_of(pid: libc::pid_t) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is as long as the length handed with it.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..n as usize])))
}

#[cfg(target_os = "macos")]
fn started(pid: libc::pid_t) -> Option<i64> {
    // SAFETY: proc_bsdinfo is plain data, and proc_pidinfo writes at most its size.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (n == size).then_some(info.pbi_start_tvsec as i64)
}

/// The innermost `.app` a path is inside.
pub fn bundle(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
}

/// Part 2: the app responsible for `pid`, as TCC.db names it.
#[cfg(target_os = "macos")]
fn app(pid: libc::pid_t) -> Option<App> {
    // SAFETY: a pid in, a pid out (or -1).
    let top = unsafe { responsibility_get_pid_responsible_for_pid(pid) };
    let top = if top > 0 { top } else { pid };
    let exe = path_of(top)?;
    let started = started(top)?;
    let (name, client) = match bundle(&exe) {
        Some(b) => {
            let id = Command::new("plutil")
                .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
                .arg(b.join("Contents/Info.plist"))
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            let name = b
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            (name, id.unwrap_or_else(|| exe.display().to_string()))
        }
        None => (
            exe.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            exe.display().to_string(),
        ),
    };
    Some(App {
        name,
        client,
        started,
    })
}

/// No responsible app off macOS, whose scan and grants this checks: the CI mirror's Linux builds the crate.
#[cfg(not(target_os = "macos"))]
fn app(_pid: libc::pid_t) -> Option<App> {
    None
}

/// Each builder: this process, and every cargo, rustc and rust-analyzer running.
fn builders() -> Vec<(libc::pid_t, &'static str)> {
    let mut found = vec![(std::process::id() as libc::pid_t, "dash doctor")];
    for name in BUILDERS {
        let Ok(o) = Command::new("pgrep").args(["-x", name]).output() else {
            continue;
        };
        let pids = String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect::<Vec<_>>();
        found.extend(pids.into_iter().map(|p| (p, name)));
    }
    found
}

/// TCC.db's lines, `client\tlast_modified`: each app granted Developer Tools and when.
pub fn parse_grants(text: &str) -> HashMap<String, i64> {
    text.lines()
        .filter_map(|l| {
            let (client, at) = l.split_once('\t')?;
            Some((client.to_string(), at.trim().parse().ok()?))
        })
        .collect()
}

/// The programs TCC.db's read runs and their environment: the system's, or a test's stand-ins.
pub struct Readers {
    pub sudo: PathBuf,
    pub sqlite3: PathBuf,
    pub env: Vec<(String, String)>,
}

impl Readers {
    pub fn system() -> Self {
        Self {
            sudo: "sudo".into(),
            sqlite3: "sqlite3".into(),
            env: vec![],
        }
    }
}

/// Every Developer Tools grant, in one sqlite3 read of TCC.db, by sudo when `sudo`: its one password ask.
fn grants(sudo: bool, r: &Readers) -> Result<HashMap<String, i64>, String> {
    let mut read = match sudo {
        true => {
            let mut c = Command::new(&r.sudo);
            c.arg(&r.sqlite3);
            c
        }
        false => Command::new(&r.sqlite3),
    };
    let out = read
        .envs(r.env.iter().map(|(k, v)| (k, v)))
        .args(["-separator", "\t", TCC, QUERY])
        .stdin(Stdio::inherit())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| unread(&format!("sqlite3: {e}"), sudo))?;
    match out.status.success() {
        true => Ok(parse_grants(&String::from_utf8_lossy(&out.stdout))),
        false => Err(unread(String::from_utf8_lossy(&out.stderr).trim(), sudo)),
    }
}

/// Why TCC.db was not read, and what reads it: sudo, then Full Disk Access, which TCC asks of root too.
pub fn unread(why: &str, sudo: bool) -> String {
    match sudo {
        true => format!(
            "{why}: TCC.db asks Full Disk Access even of root, so add this terminal under System Settings > Privacy & \
Security > Full Disk Access, then make setup"
        ),
        false => format!("{why}: make setup reads it by sudo"),
    }
}

/// Part 3: what an app needs before a fresh binary runs unscanned under it, none when nothing. `unscanned`: the probe
/// ran under it and passed, so a grant row rewritten after it started (a toggle, an update) asks no relaunch.
pub fn grant_step(
    app: &App,
    grants: &Result<HashMap<String, i64>, String>,
    unscanned: bool,
) -> Option<String> {
    match grants {
        Err(e) => Some(format!("{}'s Developer Tools grant unread ({e})", app.name)),
        Ok(g) => match g.get(&app.client) {
            None => Some(format!("{DEV_TOOLS} {}", app.name)),
            Some(&at) if app.started < at && !unscanned => Some(format!(
                "relaunch {}: it started before its Developer Tools grant",
                app.name
            )),
            Some(_) => None,
        },
    }
}

/// Parts 2 and 3 for every app at once: one read of TCC.db for them all (by sudo when `sudo`), then each app's step;
/// `unscanned` is the app the probe ran under and passed. Whether the grants were read, and each app's step.
pub fn judge(
    apps: &[App],
    unscanned: Option<&App>,
    sudo: bool,
    r: &Readers,
) -> (bool, Vec<Option<String>>) {
    let grants = grants(sudo, r);
    let steps = apps
        .iter()
        .map(|a| grant_step(a, &grants, unscanned == Some(a)))
        .collect();
    (grants.is_ok(), steps)
}

/// Each Cargo target folder under `root`, `depth` levels down at most: a `target` holding cargo's CACHEDIR.TAG. Hidden
/// folders, node_modules and links are skipped, and nothing under a target is read.
pub fn targets(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = vec![];
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let (path, name) = (e.path(), e.file_name());
        let name = name.to_string_lossy();
        if !e.file_type().is_ok_and(|t| t.is_dir())
            || name.starts_with('.')
            || name == "node_modules"
        {
            continue;
        }
        if name == "target" && path.join("CACHEDIR.TAG").is_file() {
            found.push(path);
        } else if depth > 1 {
            found.extend(targets(&path, depth - 1));
        }
    }
    found.sort();
    found
}

/// Whether a folder sits under one whose name ends `.noindex`, which Spotlight skips with all under it.
fn under_noindex(dir: &Path) -> bool {
    std::fs::canonicalize(dir).is_ok_and(|real| {
        real.components()
            .any(|c| c.as_os_str().to_string_lossy().ends_with(".noindex"))
    })
}

/// The folders Spotlight should skip: sccache's cache, then every target under `~/repo` and herdr's worktrees, but for a
/// root already under a `.noindex` folder (the reap moves the worktrees there, task rust-worktrees-noindex).
fn caches(home: &Path) -> Vec<PathBuf> {
    let sccache = std::env::var_os("SCCACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Library/Caches/Mozilla.sccache"));
    let mut dirs = vec![sccache];
    for root in ["repo", ".herdr/worktrees"].map(|r| home.join(r)) {
        if !under_noindex(&root) {
            dirs.extend(targets(&root, 5));
        }
    }
    dirs
}

/// Part 4: `.metadata_never_index` written in each folder lacking it: each one, written or why not.
pub fn unindex(dirs: &[PathBuf]) -> Vec<(PathBuf, Result<(), String>)> {
    dirs.iter()
        .filter(|d| d.is_dir() && !d.join(MARKER).exists())
        .map(|d| {
            (
                d.join(MARKER),
                File::create(d.join(MARKER))
                    .map(drop)
                    .map_err(|e| e.to_string()),
            )
        })
        .collect()
}

/// A step left: what he does, and the file that makes it hold before the next run, if one does.
#[derive(Debug, PartialEq)]
pub struct Step {
    pub says: String,
    pub file: Option<PathBuf>,
}

/// The system's time, through the core's clock port.
struct Now;

impl WallClock for Now {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }
}

/// Part 5: the steps left, replacing the last run's, for `dash handoff`; `at` its clock's Unix seconds.
pub fn record(root: &Path, clock: &dyn WallClock, steps: &[Step]) -> std::io::Result<()> {
    let at = clock.now_ms() / 1000;
    let path = root.join(RECORD);
    std::fs::create_dir_all(path.parent().expect("a folder"))?;
    let steps: Vec<Value> = steps
        .iter()
        .map(|s| json!({"says": s.says, "file": s.file}))
        .collect();
    let fresh = path.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&fresh, json!({"at": at, "steps": steps}).to_string())?;
    std::fs::rename(fresh, path)
}

fn say(out: &mut File, left: bool, line: &str) {
    // a reader that closed early (`dash doctor | head`) is no failure: the record and the checklist still run
    let _ = writeln!(out, "{}{line}", if left { "LEFT  " } else { "ok    " });
}

/// `dash doctor [--sudo]`, from the repository `root`: each part's line, the older checklist's, exit 1 when a step is left.
pub fn command(root: &Path, args: &[&str], mut out: File) {
    let sudo = match args {
        [] => false,
        ["--sudo"] => true,
        _ => {
            eprintln!("usage: {USAGE}");
            std::process::exit(2)
        }
    };
    let mut steps = vec![];
    let mut left = |out: &mut File, says: String, file: Option<PathBuf>| {
        say(out, true, &says);
        steps.push(Step { says, file });
    };
    let probed = probe();
    let unscanned = matches!(&probed, Ok((a, b)) if first_run(*a, *b).is_none());
    match probed {
        Ok((a, b)) => match first_run(a, b) {
            Some(s) => left(&mut out, s, None),
            None => say(
                &mut out,
                false,
                &format!(
                    "a fresh binary: first run {} ms, second {} ms",
                    a.as_millis(),
                    b.as_millis()
                ),
            ),
        },
        Err(e) => left(
            &mut out,
            format!("a fresh binary could not be timed: {e}"),
            None,
        ),
    }
    let mut apps: Vec<(App, Vec<&str>)> = vec![];
    let own = app(std::process::id() as libc::pid_t);
    for (pid, what) in builders() {
        let Some(a) = app(pid) else { continue };
        match apps.iter_mut().find(|(seen, _)| *seen == a) {
            Some((_, whats)) => whats.push(what),
            None => apps.push((a, vec![what])),
        }
    }
    let unscanned = own.as_ref().filter(|_| unscanned);
    let found: Vec<App> = apps.iter().map(|(a, _)| a.clone()).collect();
    let (read, verdicts) = judge(&found, unscanned, sudo, &Readers::system());
    for ((a, whats), verdict) in apps.iter().zip(verdicts) {
        let _ = writeln!(
            out,
            "      {} run under {} ({})",
            whats.join(", "),
            a.name,
            a.client
        );
        match verdict {
            Some(s) => left(&mut out, s, None),
            None => say(
                &mut out,
                false,
                &format!("{} holds Developer Tools", a.name),
            ),
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for (marker, written) in unindex(&caches(&home)) {
        match written {
            Ok(()) => say(&mut out, false, &format!("wrote {}", marker.display())),
            Err(e) => left(
                &mut out,
                format!("touch {} ({e})", marker.display()),
                Some(marker),
            ),
        }
    }
    // a run that read no grant and asked no sudo (a sandboxed agent's) never replaces a full run's record
    if !sudo && !read {
        let _ = writeln!(
            out,
            "the handoff keeps the last full run's steps: this one read no grant"
        );
    } else if let Err(e) = record(root, &Now, &steps) {
        let _ = writeln!(
            out,
            "WARNING: the steps left are not on the handoff: {RECORD}: {e}"
        );
    }
    let older = Command::new(root.join("private/dashboard/priv/commands/doctor"))
        .env("DASH_REPO", root)
        .stdout(
            out.try_clone()
                .map(Stdio::from)
                .unwrap_or_else(|_| Stdio::inherit()),
        )
        .status()
        .is_ok_and(|s| s.success());
    if !steps.is_empty() || !older {
        std::process::exit(1)
    }
}

pub mod temps;

#[cfg(test)]
mod tests;
