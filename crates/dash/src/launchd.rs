//! `dash launchd`: our one launchd agent, com.nicolasha.dash (task rust-one-launch-agent; @nha, 2026-11-11: « I don't see why
//! we can't have one startup job for the dash and dash runs the watchers internally for example »). `run` keeps every job of
//! `OURS` and of each other repo's `~/.config/<repo>/dash-jobs.conf` on the host, as launchd kept one plist each; `install`
//! writes the one plist and removes each plist those jobs replace. launchd runs the binary the host builds from main
//! (bin/crm-phone; the answer in the task's dialog, 2026-11-11: « Host builds its own »), never the sandbox's
//! private/live/rust/dash, which a session may write. The CI runners stay launchd's own until rust-ci-off-mac (the same dialog).
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

pub const LABEL: &str = "com.nicolasha.dash";
/// Another repo's job file, under `~/.config/<repo>/`: its host scripts write it, which no sandboxed session can.
pub const OTHERS: &str = "dash-jobs.conf";

/// Ours, the lines another repo's file holds too: `<label> <how> <command>`. `<how>`, comma-separated: `keepalive` (started at
/// once, again 30 s after its last start once it ends), `load` (once at the start), `every=<s>` (launchd's StartInterval: first
/// after <s>, never while it runs), `watch=<path>` (run when that file changes, after the run in progress), `restart=<path>` (a
/// keepalive job stopped and started again when that file changes), `code=<path>` (another file whose change restarts a
/// keepalive job, as sandbox-apply's kick did), `log=<name>` (`~/Library/Logs/<name>.log`, else the label's last part). Ours run
/// in the checkout and a relative path is the checkout's; another repo's run in the home folder and give absolute paths. `~/` is
/// the home folder, `{host}` ours alone, the host copy. A keepalive job is also started again when a file of its command under
/// its root (the host copy, `~/.config/<repo>`) changes: a sync's new code.
/// Not sandbox/jobs.conf: its lint keeps the harness oblivious of the orchestration (`Dashboard.Lint.harness`), and a line
/// here is built into the host's binary, so it reaches the host as main does.
pub const OURS: &str = "\
# the Rust app on :7788 (make rust/serve, its job of sandbox/jobs.conf): dash serve, its look and their watchers
com.nicolasha.rust-serve keepalive {host}/bin/sandbox-apply launch rust-serve
# the older Elixir app; comment-wake claims dashboard.request by renaming it, which restarts it, as its kickstart did
com.nicolasha.dashboard keepalive,restart=.claude/run/dashboard.request.taken,code={host}/sandbox.sb,code={host}/.safehouse,code={host}/sandbox/jobs.conf,code={host}/bin/sandbox-apply /usr/bin/make --no-print-directory -f {host}/Makefile crm/serve
com.nicolasha.comment-wake keepalive,code={host}/bin/approved-only {host}/bin/comment-wake
# who watches the watcher: a few lines of sh in crm-phone, nothing of comment-wake's
com.nicolasha.watcher-check every=60 {host}/bin/crm-phone --watcher-check
com.nicolasha.orchestrator load,every=60,watch=.claude/run/orchestrator.request,log=orchestrator-job {host}/bin/orchestrator-job
com.nicolasha.tailscale-bridge watch=.claude/run/tailscale.request {host}/bin/tailscale-bridge
# every 10 s too: they serve the worktrees, whose request files nothing watches
com.nicolasha.denials-bridge every=10,watch=.claude/run/denials.request {host}/bin/denials-bridge
com.nicolasha.slack-bridge every=10,watch=.claude/run/slack.request {host}/bin/slack-bridge
# /approve's host half: each run beats in approve.ready
com.nicolasha.approve-bridge every=300,watch=.claude/run/approve.request,log=sandbox-apply-tap {host}/bin/sandbox-apply tap
com.nicolasha.freework-sweep every=86400,watch=.claude/run/freework.request {host}/bin/sandbox-apply launch freework-sweep
";

/// One job, as a line reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub label: String,
    pub keepalive: bool,
    pub load: bool,
    pub every: Option<u64>,
    pub watch: Option<PathBuf>,
    pub restart: Option<PathBuf>,
    /// More files whose change restarts a keepalive job.
    pub code: Vec<String>,
    pub log: String,
    pub argv: Vec<String>,
    /// Where its code lives: a change of a command file under it restarts a keepalive job.
    pub root: String,
    /// Ours: run in the checkout with SITE_PERSO_REPO; another repo's run in the home folder.
    pub ours: bool,
}

/// A line's words, split at blanks except inside '…' or "…", the quotes dropped.
fn words(line: &str) -> Result<Vec<String>, String> {
    let (mut out, mut w, mut quote, mut has) = (vec![], String::new(), None, false);
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => w.push(c),
            None if c == '\'' || c == '"' => (quote, has) = (Some(c), true),
            None if c.is_whitespace() => {
                if has || !w.is_empty() {
                    out.push(std::mem::take(&mut w));
                }
                has = false;
            }
            None => w.push(c),
        }
    }
    if quote.is_some() {
        return Err(format!("an unclosed quote: {line}"));
    }
    if has || !w.is_empty() {
        out.push(w);
    }
    Ok(out)
}

/// The jobs of one file's text. `host`: ours, its `{host}`; `root` the folder whose files restart a keepalive job.
pub fn parse(text: &str, host: Option<&str>, root: &str, home: &str) -> Result<Vec<Job>, String> {
    let fill = |w: &str| {
        let w = w
            .strip_prefix("~/")
            .map_or(w.to_string(), |r| format!("{home}/{r}"));
        host.map_or(w.clone(), |h| w.replace("{host}", h))
    };
    let mut jobs = vec![];
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let ws = words(line)?;
        let [label, how, argv @ ..] = &ws[..] else {
            return Err(format!("a label, how, then the command: {line}"));
        };
        if argv.is_empty()
            || !label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(format!("a label, how, then the command: {line}"));
        }
        let mut job = Job {
            label: label.clone(),
            keepalive: false,
            load: false,
            every: None,
            watch: None,
            restart: None,
            code: vec![],
            log: label.rsplit('.').next().unwrap_or(label).to_string(),
            argv: argv.iter().map(|w| fill(w)).collect(),
            root: root.to_string(),
            ours: host.is_some(),
        };
        // another repo's job runs in the home folder: a path relative to site_perso's checkout would name the wrong repo
        let path = |p: &str| {
            let p = PathBuf::from(fill(p));
            match host.is_some() || p.is_absolute() {
                true => Ok(p),
                false => Err(format!("{}: an absolute path or ~/: {line}", p.display())),
            }
        };
        for part in how.split(',') {
            match part.split_once('=') {
                None if part == "keepalive" => job.keepalive = true,
                None if part == "load" => job.load = true,
                Some(("every", s)) => {
                    job.every = Some(
                        s.parse()
                            .ok()
                            .filter(|s| *s > 0)
                            .ok_or(format!("every={s}: {line}"))?,
                    );
                }
                Some(("watch", p)) => job.watch = Some(path(p)?),
                Some(("restart", p)) => job.restart = Some(path(p)?),
                // read from the supervisor's own folder: absolute, ours too
                Some(("code", p)) if Path::new(&fill(p)).is_absolute() => job.code.push(fill(p)),
                Some(("log", n)) if !n.is_empty() && !n.contains('/') => job.log = n.to_string(),
                _ => {
                    return Err(format!(
                        "{part}: not keepalive, load, every=, watch=, restart=, code= or log=: {line}"
                    ));
                }
            }
        }
        jobs.push(job);
    }
    Ok(jobs)
}

/// Every job, by label: ours, then each other repo's file. What a file cannot give is said, never fatal: one repo's typo stops
/// none of the others. Another repo never takes one of our labels, nor a label already given.
pub fn all(host: &str, home: &Path) -> (BTreeMap<String, Job>, Vec<String>) {
    let home_s = home.to_string_lossy();
    let mut jobs = BTreeMap::new();
    let mut said = vec![];
    for job in parse(OURS, Some(host), host, &home_s).expect("OURS reads") {
        jobs.insert(job.label.clone(), job);
    }
    let mut files: Vec<PathBuf> = fs::read_dir(home.join(".config"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name() != "site_perso")
        .map(|e| e.path().join(OTHERS))
        .filter(|f| f.is_file())
        .collect();
    files.sort();
    for file in files {
        let root = file
            .parent()
            .expect("a folder")
            .to_string_lossy()
            .to_string();
        let read = fs::read_to_string(&file).map_err(|e| e.to_string());
        match read.and_then(|t| parse(&t, None, &root, &home_s)) {
            Err(e) => said.push(format!("{}: {e}", file.display())),
            Ok(theirs) => {
                for job in theirs {
                    if job.label.starts_with("com.nicolasha.") || jobs.contains_key(&job.label) {
                        said.push(format!("{}: {} is taken", file.display(), job.label));
                    } else {
                        jobs.insert(job.label.clone(), job);
                    }
                }
            }
        }
    }
    (jobs, said)
}

/// A second of the supervisor's clock: `DASH_LAUNCHD_SECOND_MS` shortens it for the tests, every interval and throttle with it.
fn second() -> Duration {
    let ms = std::env::var("DASH_LAUNCHD_SECOND_MS")
        .ok()
        .and_then(|s| s.parse().ok());
    Duration::from_millis(ms.unwrap_or(1000))
}

/// A file as launchd's WatchPaths sees it: its time and size, `None` when missing.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    fs::metadata(path)
        .ok()
        .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
}

/// A keepalive job's command files under its root and its code= files, each with a hash of its bytes.
fn sums(job: &Job) -> Vec<(String, u64)> {
    let root = format!("{}/", job.root.trim_end_matches('/'));
    let words = job.argv.iter().filter(|w| w.starts_with(&root));
    words
        .chain(&job.code)
        .filter_map(|w| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            fs::read(w).ok()?.hash(&mut h);
            Some((w.clone(), h.finish()))
        })
        .collect()
}

struct Kept {
    job: Job,
    child: Option<Child>,
    /// When its TERM went: KILL 10 s later, the loop never waiting on it.
    stopping: Option<Instant>,
    started: Option<Instant>,
    next: Option<Instant>,
    seen: Stamp,
    restart_seen: Stamp,
    pending: bool,
    sums: Vec<(String, u64)>,
    /// A keepalive job's group and its start (`lstart`): what `record_groups` writes.
    group: Option<(libc::pid_t, String)>,
}

impl Kept {
    fn new(job: Job, repo: &Path, now: Instant) -> Kept {
        let at = |p: &Option<PathBuf>| p.as_ref().and_then(|p| stamp(&repo.join(p)));
        Kept {
            next: job.every.map(|s| now + second() * s as u32),
            seen: at(&job.watch),
            restart_seen: at(&job.restart),
            pending: job.load || job.keepalive,
            job,
            child: None,
            stopping: None,
            started: None,
            sums: vec![],
            group: None,
        }
    }

    fn say(&self, what: &str) {
        eprintln!("{} dash launchd: {} {what}", stamp_now(), self.job.label);
    }

    fn start(&mut self, repo: &Path, home: &Path) {
        self.pending = false;
        self.started = Some(Instant::now());
        self.sums = if self.job.keepalive {
            sums(&self.job)
        } else {
            vec![]
        };
        let logs = home.join("Library/Logs");
        let _ = fs::create_dir_all(&logs);
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs.join(format!("{}.log", self.job.log)));
        let spawned = log.and_then(|log| {
            let mut c = Command::new(&self.job.argv[0]);
            c.args(&self.job.argv[1..]);
            match self.job.ours {
                true => c.current_dir(repo).env("SITE_PERSO_REPO", repo),
                false => c.current_dir(home).env_remove("SITE_PERSO_REPO"),
            };
            c.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
            // its own group, so a stop reaches what it started (make's child, safehouse's)
            c.process_group(0).spawn()
        });
        match spawned {
            Ok(child) => {
                let pid = child.id() as libc::pid_t;
                self.group = lstart(pid).map(|at| (pid, at));
                self.child = Some(child);
            }
            Err(e) => self.say(&format!("not started: {e}")),
        }
    }

    /// TERM to its group, once; `reap` sends KILL 10 s later.
    fn stop(&mut self) {
        if let Some(child) = &self.child
            && self.stopping.is_none()
        {
            unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGTERM) };
            self.stopping = Some(Instant::now());
        }
    }

    /// Its child reaped once it ended; KILL to its group 10 s after a TERM it outlived. True when it ended.
    fn reap(&mut self, now: Instant) -> bool {
        let Some(child) = self.child.as_mut() else {
            return false;
        };
        if let Ok(Some(status)) = child.try_wait() {
            // stopped: what it started goes too, though it outlived its leader's TERM (a server holding a port); a run that
            // ended of itself keeps what it left in the background (sandbox-apply tap's crm-phone)
            if self.stopping.is_some() {
                unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
            }
            (self.child, self.stopping, self.group) = (None, None, None);
            if self.job.keepalive {
                self.say(&format!("ended ({status}), started again"));
            }
            return true;
        }
        if self
            .stopping
            .is_some_and(|t| now.duration_since(t) >= Duration::from_secs(10))
        {
            unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
        }
        false
    }

    /// One tick: reaped, then started when due.
    fn step(&mut self, repo: &Path, home: &Path, now: Instant) {
        self.reap(now);
        let at = |p: &PathBuf| match self.job.ours {
            true => stamp(&repo.join(p)),
            false => stamp(p),
        };
        if let Some(s) = self.job.watch.as_ref().map(at)
            && s != self.seen
        {
            self.seen = s;
            self.pending = true;
        }
        if let Some(s) = self.job.restart.as_ref().map(at)
            && s != self.restart_seen
        {
            self.restart_seen = s;
            self.say("restarted on its request");
            self.stop();
            self.pending = true;
        }
        if let (Some(at), Some(s)) = (self.next, self.job.every)
            && now >= at
        {
            self.next = Some(now + second() * s as u32);
            // launchd's StartInterval: a run due while one runs is skipped
            self.pending |= self.child.is_none();
        }
        if self.child.is_none() && self.job.keepalive {
            let throttled = self
                .started
                .is_some_and(|t| now.duration_since(t) < second() * 30);
            self.pending |= !throttled;
        }
        if self.pending && self.child.is_none() {
            self.start(repo, home);
        }
    }
}

/// A process's start time: a group recorded with it is never a later process of the same pid. Read from the kernel, no `ps`
/// (the sandbox refuses it, and a call per start would cost a process).
#[cfg(target_os = "macos")]
fn lstart(pid: libc::pid_t) -> Option<String> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let ptr = (&raw mut info).cast::<libc::c_void>();
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, ptr, size) };
    (n == size).then(|| format!("{}.{}", info.pbi_start_tvsec, info.pbi_start_tvusec))
}

/// Linux (CI): /proc/<pid>/stat's 22nd field, the start in clock ticks since boot.
#[cfg(not(target_os = "macos"))]
fn lstart(pid: libc::pid_t) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit(')')
        .next()?
        .split_whitespace()
        .nth(19)
        .map(str::to_string)
}

/// The keepalive jobs' groups, `<pgid> <lstart>` a line, in the host's own folder: launchd kills only the supervisor's group,
/// so a supervisor that crashed or was killed leaves its jobs running, and the next one ends them before it starts its own
/// (a second dashboard or comment-wake otherwise). Never in the checkout, which a session writes.
fn groups_file(home: &Path) -> PathBuf {
    home.join(".config/site_perso/dash/launchd.groups")
}

fn record_groups(home: &Path, kept: &[&Kept]) -> String {
    let lines: String = kept
        .iter()
        .filter_map(|k| k.group.as_ref())
        .map(|(pid, at)| format!("{pid} {at}\n"))
        .collect();
    let file = groups_file(home);
    let _ = fs::create_dir_all(file.parent().expect("a folder"));
    let _ = fs::write(&file, &lines);
    lines
}

/// The groups an earlier supervisor left, each led by the process it recorded or leaderless: TERM, then KILL 10 s later.
fn end_left_groups(home: &Path) {
    let text = fs::read_to_string(groups_file(home)).unwrap_or_default();
    let left: Vec<libc::pid_t> = text
        .lines()
        .filter_map(|l| {
            let (pid, at) = l.split_once(' ')?;
            let pid = pid.parse().ok().filter(|p: &libc::pid_t| *p > 1)?;
            // the leader at its start, or gone while its group lives on: a pgid is never reused while its group exists
            match lstart(pid) {
                Some(now) => (now == at).then_some(pid),
                None => (unsafe { libc::killpg(pid, 0) } == 0).then_some(pid),
            }
        })
        .collect();
    for pid in &left {
        eprintln!(
            "{} dash launchd: an earlier run's group {pid} ended",
            stamp_now()
        );
        unsafe { libc::killpg(*pid, libc::SIGTERM) };
    }
    let until = Instant::now() + Duration::from_secs(10);
    while left.iter().any(|p| unsafe { libc::killpg(*p, 0) } == 0) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    for pid in left {
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
}

fn stamp_now() -> String {
    let s = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("@{s}")
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// `dash launchd run`: keeps every job until launchd stops it (TERM: each job's group stopped first), or until its own binary
/// moves (crm-phone's new build behind the same path: it exits, and launchd's KeepAlive starts the new one).
pub fn run(host: &str, repo: &Path, home: &Path) -> ! {
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
    }
    let me = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let built = me.as_ref().and_then(|p| fs::canonicalize(p).ok());
    end_left_groups(home);
    let mut kept: BTreeMap<String, Kept> = BTreeMap::new();
    // jobs gone or changed, until their stop ends them
    let mut retiring: Vec<Kept> = vec![];
    let mut said = vec![];
    let mut read = None::<Instant>;
    // what the groups file holds now, and the groups it was written from
    let (mut recorded, mut groups) = (String::new(), vec![]);
    loop {
        let now = Instant::now();
        let moved = built.is_some() && me.as_ref().and_then(|p| fs::canonicalize(p).ok()) != built;
        if STOP.load(Ordering::SeqCst) || moved {
            let why = if moved { "a new build" } else { "stopped" };
            eprintln!("{} dash launchd: {why}, every job stopped", stamp_now());
            // every group at once, so all end within launchd's ExitTimeOut (30 s), KILL at 10 s
            let mut every: Vec<Kept> = kept.into_values().chain(retiring).collect();
            every.iter_mut().for_each(Kept::stop);
            while every.iter().any(|k| k.child.is_some()) {
                every.iter_mut().for_each(|k| {
                    k.reap(Instant::now());
                });
                std::thread::sleep(Duration::from_millis(50));
            }
            // only its own: a successor already up may have written there
            if fs::read_to_string(groups_file(home)).is_ok_and(|t| t == recorded) {
                let _ = fs::remove_file(groups_file(home));
            }
            std::process::exit(0);
        }
        if read.is_none_or(|t| now.duration_since(t) >= second() * 5) {
            read = Some(now);
            let (jobs, errors) = all(host, home);
            if errors != said {
                errors
                    .iter()
                    .for_each(|e| eprintln!("{} dash launchd: {e}", stamp_now()));
                said = errors;
            }
            let labels: Vec<String> = kept.keys().cloned().collect();
            for label in labels {
                if jobs.get(&label) != Some(&kept[&label].job) {
                    let mut k = kept.remove(&label).expect("kept");
                    k.say("gone or changed, stopped");
                    k.stop();
                    // an idle one is simply dropped, so its new line is kept at once
                    if k.child.is_some() {
                        retiring.push(k);
                    }
                }
            }
            for (label, job) in jobs {
                // a changed job starts once its old run ended: never two of one label
                let busy = retiring.iter().any(|r| r.job.label == label);
                if !busy {
                    kept.entry(label)
                        .or_insert_with(|| Kept::new(job, repo, now));
                }
            }
            for k in kept
                .values_mut()
                .filter(|k| k.job.keepalive && k.child.is_some() && k.stopping.is_none())
            {
                if sums(&k.job) != k.sums {
                    k.say("code changed, restarted");
                    k.stop();
                    k.pending = true;
                }
            }
        }
        retiring.retain_mut(|r| {
            r.reap(now);
            r.child.is_some()
        });
        for k in kept.values_mut() {
            k.step(repo, home, now);
        }
        let alive: Vec<&Kept> = kept.values().chain(&retiring).collect();
        let now_groups: Vec<libc::pid_t> = alive
            .iter()
            .filter_map(|k| Some(k.group.as_ref()?.0))
            .collect();
        if now_groups != groups {
            recorded = record_groups(home, &alive);
            groups = now_groups;
        }
        std::thread::sleep(second() / 5);
    }
}

fn launchctl(args: &[&str]) -> Result<(), String> {
    let bin = std::env::var("DASH_LAUNCHCTL").unwrap_or_else(|_| "launchctl".into());
    let out = Command::new(&bin)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    match out.status.success() {
        true => Ok(()),
        false => Err(format!(
            "{bin} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// `dash launchd install`: the one plist written and loaded when it changed or launchd lost it, then, once it is loaded, each
/// plist it replaces booted out and removed (every com.nicolasha one but the CI runners', and each label another repo's file
/// names). Idempotent. Run in a process group of its own (`main`'s caller), so the bootout of the agent that started it (a sync run
/// by comment-wake, a job of this very agent) never stops it halfway.
pub fn install(bin: &str, host: &str, repo: &Path, home: &Path) -> Result<Vec<String>, String> {
    let agents = home.join("Library/LaunchAgents");
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    let (jobs, _) = all(host, home);
    let mut said = vec![];
    let path = agents.join(format!("{LABEL}.plist"));
    let args = [
        bin,
        "launchd",
        "run",
        "--host",
        host,
        "--repo",
        &repo.to_string_lossy(),
    ];
    let want = dash_launcher::plist(LABEL, &args, repo, &home.join("Library/Logs/dash.log"));
    let same = fs::read_to_string(&path).is_ok_and(|t| t == want);
    let loaded = launchctl(&["print", &format!("{domain}/{LABEL}")]).is_ok();
    if same && loaded {
        said.push(format!("{LABEL}: installed and current"));
    } else {
        let tmp = path.with_extension("plist.tmp");
        fs::write(&tmp, &want)
            .and_then(|()| fs::rename(&tmp, &path))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if loaded {
            let _ = launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
        }
        let file = path.to_string_lossy();
        // a bootout returns before the job is gone: a bootstrap right after can fail with « 5: Input/output error »
        let mut tried = launchctl(&["bootstrap", &domain, &file]);
        for _ in 0..5 {
            if tried.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
            tried = launchctl(&["bootstrap", &domain, &file]);
        }
        // not loaded: every other agent stays as it is, so nothing stops running
        tried.map_err(|e| format!("{e}; no other agent was touched"))?;
        said.push(format!("{LABEL}: installed ({file})"));
    }
    // Then each plist it replaces: its jobs start in the one agent while these stop, so for a moment two may run; a keepalive
    // job that finds its port taken ends and is started again 30 s later.
    let mut old: Vec<String> = fs::read_dir(&agents)
        .map_err(|e| format!("{}: {e}", agents.display()))?
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".plist")
                .map(str::to_string)
        })
        .filter(|l| {
            let ours = l.starts_with("com.nicolasha.")
                && l != LABEL
                && !l.starts_with("com.nicolasha.ci-runner");
            ours || jobs.contains_key(l)
        })
        .collect();
    old.sort();
    for label in old {
        let _ = launchctl(&["bootout", &format!("{domain}/{label}")]);
        let path = agents.join(format!("{label}.plist"));
        fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        said.push(format!("{label}: booted out and removed, dash runs it"));
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    fn ours() -> BTreeMap<String, Job> {
        let jobs = parse(OURS, Some("/h"), "/h", "/home").expect("OURS reads");
        jobs.into_iter().map(|j| (j.label.clone(), j)).collect()
    }

    fn job(label: &str) -> Job {
        ours()
            .remove(label)
            .unwrap_or_else(|| panic!("{label} is not in OURS"))
    }

    #[test]
    fn rust_serve_is_kept_alive_and_launches_dash_serve() {
        let _budget = TestBudget::new();
        let j = job("com.nicolasha.rust-serve");
        assert!(j.keepalive);
        assert_eq!(j.argv, ["/h/bin/sandbox-apply", "launch", "rust-serve"]);
        // sandbox-apply launch rust-serve runs jobs.conf's line, make rust/serve, whose recipe execs dash serve
        // in the monorepo a missing jobs.conf fails, never reads as the mirror, which holds rust/ alone
        let Some(repo) = dash_meter::monorepo() else {
            return;
        };
        let conf = fs::read_to_string(repo.join("sandbox/jobs.conf")).expect("jobs.conf");
        let line = conf
            .lines()
            .find(|l| l.starts_with("rust-serve "))
            .expect("its line");
        assert!(
            line.ends_with("make --no-print-directory rust/serve"),
            "{line}"
        );
        let make = fs::read_to_string(repo.join("Makefile")).expect("Makefile");
        let recipe = make
            .split("\nrust/serve:\n")
            .nth(1)
            .expect("rust/serve")
            .split("\n\n")
            .next()
            .unwrap_or_default();
        assert!(recipe.contains("/dash\" serve --data"), "{recipe}");
    }

    #[test]
    fn comment_wake_is_kept_alive() {
        let _budget = TestBudget::new();
        let j = job("com.nicolasha.comment-wake");
        assert!(j.keepalive);
        assert_eq!(j.argv, ["/h/bin/comment-wake"]);
        assert_eq!(j.log, "comment-wake");
    }

    #[test]
    fn watcher_check_runs_every_minute() {
        let _budget = TestBudget::new();
        let j = job("com.nicolasha.watcher-check");
        assert_eq!((j.every, j.keepalive), (Some(60), false));
        assert_eq!(j.argv, ["/h/bin/crm-phone", "--watcher-check"]);
    }

    #[test]
    fn the_bridges_run_on_their_request() {
        let _budget = TestBudget::new();
        for (label, file, every, argv) in [
            (
                "tailscale-bridge",
                "tailscale.request",
                None,
                vec!["/h/bin/tailscale-bridge"],
            ),
            (
                "denials-bridge",
                "denials.request",
                Some(10),
                vec!["/h/bin/denials-bridge"],
            ),
            (
                "slack-bridge",
                "slack.request",
                Some(10),
                vec!["/h/bin/slack-bridge"],
            ),
            (
                "approve-bridge",
                "approve.request",
                Some(300),
                vec!["/h/bin/sandbox-apply", "tap"],
            ),
        ] {
            let j = job(&format!("com.nicolasha.{label}"));
            assert_eq!(
                j.watch,
                Some(PathBuf::from(format!(".claude/run/{file}"))),
                "{label}"
            );
            assert_eq!(
                (j.every, j.argv),
                (every, argv.iter().map(|w| w.to_string()).collect()),
                "{label}"
            );
        }
        assert_eq!(job("com.nicolasha.approve-bridge").log, "sandbox-apply-tap");
    }

    #[test]
    fn the_orchestrator_runs_at_load_each_minute_and_on_start() {
        let _budget = TestBudget::new();
        let j = job("com.nicolasha.orchestrator");
        assert!(j.load && !j.keepalive);
        assert_eq!(j.every, Some(60));
        assert_eq!(
            j.watch,
            Some(PathBuf::from(".claude/run/orchestrator.request"))
        );
        assert_eq!(
            (j.argv, j.log),
            (
                vec!["/h/bin/orchestrator-job".to_string()],
                "orchestrator-job".to_string()
            )
        );
    }

    #[test]
    fn freework_sweep_runs_daily_in_its_sandbox() {
        let _budget = TestBudget::new();
        let j = job("com.nicolasha.freework-sweep");
        assert_eq!(j.every, Some(86400));
        assert_eq!(j.argv, ["/h/bin/sandbox-apply", "launch", "freework-sweep"]);
    }

    #[test]
    fn a_line_reads_quotes_home_and_refuses_what_it_does_not_know() {
        let _budget = TestBudget::new();
        let line =
            r#"im.klar.claude-rc keepalive /bin/zsh -lc 'cd "/r/klar" && exec make claude/rc'"#;
        let j = parse(line, None, "/home/.config/klar", "/home")
            .expect("reads")
            .remove(0);
        assert_eq!(
            j.argv,
            ["/bin/zsh", "-lc", r#"cd "/r/klar" && exec make claude/rc"#]
        );
        let j = parse(
            "dev.graal.t every=5,log=graal-t ~/.config/graal/bin/x tick {host}",
            None,
            "/r",
            "/home",
        )
        .expect("reads")
        .remove(0);
        assert_eq!(
            j.argv,
            ["/home/.config/graal/bin/x", "tick", "{host}"],
            "{{host}} is ours alone"
        );
        assert_eq!((j.every, j.log.as_str()), (Some(5), "graal-t"));
        let theirs = parse("a.b watch=.claude/run/x a", None, "/r", "/home");
        assert!(
            theirs.is_err(),
            "another repo's relative path names site_perso's checkout"
        );
        let ok = parse("a.b watch=~/r/x,code=/c a", None, "/r", "/home").expect("absolute");
        let read = (ok[0].watch.clone(), ok[0].code.clone(), ok[0].ours);
        assert_eq!(
            read,
            (
                Some(PathBuf::from("/home/r/x")),
                vec!["/c".to_string()],
                false
            )
        );
        for bad in [
            "a.b code=bin/x x",
            "a.b every=0 x",
            "a.b sometimes x",
            "a.b keepalive",
            "a/b keepalive x",
            "a.b log=../x x",
            "a.b keepalive 'x",
        ] {
            assert!(parse(bad, None, "/r", "/home").is_err(), "{bad} read");
        }
    }
}
