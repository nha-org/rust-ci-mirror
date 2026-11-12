//! Git's hooks in Rust: `dash hook pre-commit` and `dash hook pre-push` (task rust-git-hooks-port; @nha, 2026-11-10: « there
//! is no reason for elixir or python or bash in this repo long-term. »; a shell hook bug refused every rust/ commit for hours).
//! .githooks/pre-commit and .githooks/pre-push call them in one line; each step is a function in `pre_commit` or `pre_push`,
//! its refusal naming its reason, and each has its test (crates/dash/tests/hooks.rs, through git itself). The history of
//! each step, and the incident that made it, is in `git log -- .githooks`.
use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

pub mod anchored;
pub mod cv_locks;
pub mod no_elixir;
pub mod offline;
pub mod pipe_cut;
pub mod pre_commit;
pub mod pre_push;

/// The repository a hook runs for, and where what it says goes.
pub struct Ctx {
    pub top: PathBuf,
    /// This checkout's own git dir, absolute: a linked worktree's, under the common one
    pub git_dir: PathBuf,
    pub common: PathBuf,
    /// The default branch, as forge.env names it (task move-to-nha-org)
    pub main: String,
    pub hook: &'static str,
    /// pre-push's log (task prepush-quiet): all it says, kept in a file as it goes so a killed push still leaves it, and
    /// printed in full on a refusal; pre-commit prints as it goes
    log: Option<RefCell<(Option<fs::File>, String)>>,
    began: Instant,
    /// a step ran, so the total is worth a line
    timed_any: RefCell<bool>,
}

/// Git runs a hook at the top of the work tree, with GIT_DIR set in a linked worktree, under which git takes the current
/// folder for the top: from rust/, the shell rust-gate's git named rust/bin/bounded and listed paths from rust/, refusing
/// every rust/ commit (rust-mail-design-parts). .githooks call dash from rust/, so GIT_DIR and GIT_WORK_TREE are dropped and
/// git finds the repository itself; GIT_INDEX_FILE (a `git commit <path>`'s own index) is made absolute against the top, so
/// every git below, and dash_gate::ratchet in this process, still reads the index being committed.
pub fn from_hook_env(hook: &'static str) -> Result<Ctx, String> {
    // SAFETY: the first thing a hook command does, before any thread starts
    unsafe {
        std::env::remove_var("GIT_DIR");
        std::env::remove_var("GIT_WORK_TREE");
    }
    let top = PathBuf::from(git_in(Path::new("."), &["rev-parse", "--show-toplevel"])?.trim());
    if let Some(idx) = std::env::var_os("GIT_INDEX_FILE").map(PathBuf::from)
        && idx.is_relative()
    {
        // SAFETY: as above
        unsafe { std::env::set_var("GIT_INDEX_FILE", top.join(idx)) };
    }
    Ctx::new(&top, hook)
}

/// The default branch `top`'s forge.env names, else master.
pub fn default_branch(top: &Path) -> String {
    fs::read_to_string(top.join("forge.env"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("DEFAULT_BRANCH=").map(String::from))
        })
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "master".into())
}

impl Ctx {
    pub fn new(top: &Path, hook: &'static str) -> Result<Ctx, String> {
        let git_dir = PathBuf::from(git_in(top, &["rev-parse", "--absolute-git-dir"])?.trim());
        let common = PathBuf::from(
            git_in(
                top,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )?
            .trim(),
        );
        Ok(Ctx {
            top: top.to_path_buf(),
            git_dir,
            common,
            main: default_branch(top),
            hook,
            log: None,
            began: Instant::now(),
            timed_any: RefCell::new(false),
        })
    }

    /// From here on all it says goes to `file` (truncated) and the buffer; a file that cannot be written leaves the buffer.
    pub fn quiet(&mut self, file: &Path) {
        let f = fs::File::create(file).ok();
        self.log = Some(RefCell::new((f, String::new())));
    }

    /// What it said so far (pre-push's log)
    pub fn said(&self) -> String {
        self.log
            .as_ref()
            .map(|l| l.borrow().1.clone())
            .unwrap_or_default()
    }

    pub fn say(&self, text: &str) {
        let text = text.trim_end_matches('\n');
        if text.is_empty() {
            return;
        }
        match &self.log {
            None => tell(&format!("{text}\n")),
            Some(log) => {
                let (file, buf) = &mut *log.borrow_mut();
                buf.push_str(text);
                buf.push('\n');
                if let Some(f) = file {
                    let _ = writeln!(f, "{text}");
                }
            }
        }
    }

    pub fn seconds(&self) -> u64 {
        self.began.elapsed().as_secs()
    }

    /// A linked worktree's top holds a .git file, the main checkout a folder.
    pub fn is_worktree(&self) -> bool {
        self.top.join(".git").is_file()
    }

    /// A command run in `dir`: under the top, on the hook's index (GIT_INDEX_FILE, a `git commit <path>`'s own), or in a
    /// checkout `checkout` made under the git dir (pre_commit's staged one, pre_push's), which reads its own index, never the
    /// hook's. Its folder decides, never `dir == top`: rust-gate runs in <top>/rust and must read the index being committed
    /// (/code-review). No stdin: pre-push's refs are read once, by the hook.
    pub fn cmd(&self, dir: &Path, program: impl AsRef<OsStr>) -> Command {
        let mut c = Command::new(program);
        c.current_dir(dir).stdin(Stdio::null());
        if dir.starts_with(&self.git_dir) {
            c.env_remove("GIT_INDEX_FILE");
        }
        c
    }

    /// `bin/bounded <secs> <program>`: the whole process group ended at the limit, exit 124; a missing program exits 127.
    /// The live tree's, as an old commit may predate it.
    pub fn bounded(&self, dir: &Path, secs: u64, program: &str) -> Command {
        let mut c = self.cmd(dir, self.top.join("bin/bounded"));
        c.arg(secs.to_string()).arg(program);
        c
    }

    /// git at the top, on the hook's index; its stdout, or why it failed
    pub fn git(&self, args: &[&str]) -> Result<String, String> {
        git_in(&self.top, args)
    }

    /// git at the top exits 0 (`diff --quiet` and the like)
    pub fn git_ok(&self, args: &[&str]) -> bool {
        Command::new("git")
            .args(["-c", "core.quotePath=false"])
            .args(args)
            .current_dir(&self.top)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// The first commit `rev` names, if any
    pub fn rev(&self, rev: &str) -> Option<String> {
        self.git(&["rev-parse", "-q", "--verify", rev])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Runs a step and appends « <UTC time> <hook> <step> <seconds>s exit=<n> load=<1 min> » to the common git dir's
    /// hook-times.log, kept to its last 500 lines past 1000 (task precommit-fast; @nha, 2026-10-30: « Things slowing us down
    /// should be picked as priority items. »). pre-push also says each step's time in its log.
    /// ponytail: two hooks trimming at once can drop a few lines of a timing log; a lock if the log ever has to be exact
    pub fn timed<T: Exit>(&self, step: &str, f: impl FnOnce() -> T) -> T {
        let t0 = Instant::now();
        let r = f();
        let (secs, code) = (t0.elapsed().as_secs(), r.exit());
        *self.timed_any.borrow_mut() = true;
        let file = self.common.join("hook-times.log");
        let line = format!(
            "{} {} {step} {secs}s exit={code} load={}\n",
            utc_now(),
            self.hook,
            load()
        );
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&file) {
            let _ = f.write_all(line.as_bytes());
        }
        if let Ok(text) = fs::read_to_string(&file) {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() > 1000 {
                let tmp = file.with_extension(format!("log.{}", std::process::id()));
                if fs::write(&tmp, lines[lines.len() - 500..].join("\n") + "\n").is_ok() {
                    let _ = fs::rename(&tmp, &file);
                }
            }
        }
        if self.log.is_some() {
            self.say(&format!("{}: {step} {secs}s (exit {code})", self.hook));
        }
        r
    }

    /// The hook's total since it began, once a step ran (pre-push's log)
    pub fn total(&self) {
        if *self.timed_any.borrow() && self.log.is_some() {
            self.say(&format!("{}: total {}s", self.hook, self.seconds()));
        }
    }

    /// A checkout of `commit` at `<git dir>/<name>`, made once and moved to each new commit, cleaned of what earlier runs
    /// left (_build, deps, target and the toolchain's caches kept) as CI's fresh clone (task prepush-tests-pushed-tree; @nha,
    /// 2026-10-29: « You fixed the mistake but didn't find the cause and what could actually fix that class of errors right ? »).
    /// A new one gets main's compiled deps from bin/toolchain; a broken one (its registration pruned, its git dir moved) is
    /// removed, else moved aside and queued for bin/prune-paths. No repo hook runs on it. None when it cannot be made.
    pub fn checkout(&self, name: &str, commit: &str) -> Option<PathBuf> {
        let wt = self.git_dir.join(name);
        let top = &self.top;
        let git = |dir: &Path, args: &[&str]| {
            self.cmd(dir, "git")
                .env_remove("GIT_INDEX_FILE")
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        // only a worktree's own folder (its .git file): in a stray one, git would walk up to the live worktree (/code-review)
        let moved = |wt: &Path| {
            wt.join(".git").is_file()
                && git(
                    wt,
                    &[
                        "-c",
                        "core.hooksPath=/dev/null",
                        "checkout",
                        "-q",
                        "--force",
                        "--detach",
                        commit,
                    ],
                )
                && git(
                    wt,
                    &[
                        "clean",
                        "-q",
                        "-fdx",
                        "-e",
                        "_build",
                        "-e",
                        "deps",
                        "-e",
                        "target",
                        "-e",
                        ".toolchain",
                        "-e",
                        ".toolchain.*",
                    ],
                )
        };
        if wt.exists() && !moved(&wt) {
            self.drop_checkout(name);
        }
        if !wt.exists() {
            let w = wt.to_string_lossy();
            // a folder removed while still registered fails every later add: prune first (bin/live does the same)
            let added = git(top, &["worktree", "prune"])
                && self
                    .bounded(top, 40, "git")
                    .env_remove("GIT_INDEX_FILE")
                    .args(["-c", "core.hooksPath=/dev/null", "worktree", "add"])
                    .args(["-q", "--detach", &w, commit])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
            if !added {
                return None;
            }
        }
        // bin/toolchain copies the main checkout's compiled deps into a new one on its first run, never fatal: only for the
        // main checkout's mix.lock, so a branch bumping a dep compiles its own (/code-review)
        let app = wt.join("private/dashboard");
        let main_lock = self.common.join("../private/dashboard/mix.lock");
        let lock_moved = main_lock.is_file()
            && app.join("mix.lock").is_file()
            && fs::read(&main_lock).ok() != fs::read(app.join("mix.lock")).ok();
        if !app.join("_build/test").exists() && is_exec(&app.join("bin/toolchain")) && !lock_moved {
            let _ = self
                .bounded(&app, 40, "bin/toolchain")
                .arg("true")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        for note in partial_deps(&wt) {
            self.say(&note);
        }
        Some(wt)
    }

    /// Removes `<git dir>/<name>`, a checkout `checkout` made: through git, else moved to this checkout's run folder, the one
    /// bin/prune-paths deletes from (the command guard refuses a recursive delete).
    pub fn drop_checkout(&self, name: &str) {
        let wt = self.git_dir.join(name);
        if !wt.exists() {
            return;
        }
        let removed = self
            .cmd(&self.top, "git")
            .env_remove("GIT_INDEX_FILE")
            .args(["worktree", "remove", "--force"])
            .arg(&wt)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !removed {
            let run = self.top.join(".claude/run");
            let aside = run.join(format!("{name}-broken.{}", std::process::id()));
            if fs::create_dir_all(&run).is_ok() && fs::rename(&wt, &aside).is_ok() {
                let _ = Command::new(self.top.join("bin/prune-paths"))
                    .arg(&aside)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

/// A step's exit code for hook-times.log
pub trait Exit {
    fn exit(&self) -> i32;
}

impl<T, E> Exit for Result<T, E> {
    fn exit(&self) -> i32 {
        i32::from(self.is_err())
    }
}

impl<T> Exit for Option<T> {
    fn exit(&self) -> i32 {
        i32::from(self.is_none())
    }
}

/// a log read for its lines, never a verdict
impl Exit for String {
    fn exit(&self) -> i32 {
        0
    }
}

impl Exit for Ran {
    fn exit(&self) -> i32 {
        self.code.unwrap_or(-1)
    }
}

/// What a command printed, stdout and stderr in the order it wrote them, and its exit (None: a signal ended it).
pub struct Ran {
    pub code: Option<i32>,
    pub said: String,
}

impl Ran {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Writes to stderr whole and never panics. A hook's stderr may be a non-blocking pipe (an agent's harness): there
/// eprintln! panics on EAGAIN once the pipe fills, and the refusal it carried is lost (the push of db3e077a3 printed only
/// « failed printing to stderr », 2026-11-11). Every line the hooks print goes through here.
pub fn tell(text: &str) {
    let mut rest = text.as_bytes();
    let mut err = std::io::stderr().lock();
    while !rest.is_empty() {
        match err.write(rest) {
            Ok(0) => return,
            Ok(n) => rest = &rest[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            Err(_) => return,
        }
    }
}

/// Runs `c` with both its outputs in one file, so they keep their order; a command that cannot start exits 127.
pub fn ran(c: &mut Command) -> Ran {
    static N: AtomicUsize = AtomicUsize::new(0);
    let file = std::env::temp_dir().join(format!(
        "dash-hook-{}-{}.out",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let out = fs::File::create(&file).and_then(|f| Ok((f.try_clone()?, f)));
    let r = match out {
        Err(e) => Ran {
            code: Some(127),
            said: format!("{file:?}: {e}"),
        },
        Ok((o, e)) => match c.stdout(o).stderr(e).status() {
            Err(e) => Ran {
                code: Some(127),
                said: format!("{:?}: {e}", c.get_program()),
            },
            Ok(s) => Ran {
                code: s.code(),
                said: String::from_utf8_lossy(&fs::read(&file).unwrap_or_default()).into_owned(),
            },
        },
    };
    let _ = fs::remove_file(&file);
    r
}

pub(crate) fn git_in(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

pub fn is_exec(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The first 9 characters of a commit, as the hooks name one
pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}

fn utc_now() -> String {
    Command::new("date")
        .args(["-u", "+%FT%TZ"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// The 1-minute load average, as `uptime` says it
fn load() -> String {
    let up = Command::new("uptime")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    up.split_once("load average")
        .and_then(|(_, r)| {
            r.trim_start_matches('s')
                .trim_start_matches(':')
                .split_whitespace()
                .next()
        })
        .unwrap_or("")
        .trim_end_matches(',')
        .to_string()
}

/// Removes each compiled dep whose ebin lacks a module its .app lists, so the compile builds it again (task
/// prepush-partial-deps): Mix reads a dep with an .app as built and never looks at its beams. The app's own build is Mix's
/// manifest's to judge. A beam copied empty counts as lacking. Returns a note per dep removed.
/// ponytail: a beam torn mid-write passes, an md5 check if that ever shows.
pub fn partial_deps(checkout: &Path) -> Vec<String> {
    let mut notes = vec![];
    let build = checkout.join("private/dashboard/_build");
    let Ok(envs) = fs::read_dir(&build) else {
        return notes;
    };
    for env in envs.flatten() {
        let Ok(libs) = fs::read_dir(env.path().join("lib")) else {
            continue;
        };
        for lib in libs.flatten().map(|l| l.path()) {
            if lib.file_name() == Some(OsStr::new("dashboard")) {
                continue;
            }
            let ebin = lib.join("ebin");
            let Ok(files) = fs::read_dir(&ebin) else {
                continue;
            };
            for app in files.flatten().map(|f| f.path()) {
                if app.extension() != Some(OsStr::new("app")) {
                    continue;
                }
                let text = fs::read_to_string(&app).unwrap_or_default();
                let lacking = app_modules(&text).into_iter().find(|m| {
                    fs::metadata(ebin.join(format!("{m}.beam"))).map_or(true, |md| md.len() == 0)
                });
                if let Some(m) = lacking {
                    let rel = lib
                        .strip_prefix(checkout)
                        .unwrap_or(&lib)
                        .display()
                        .to_string();
                    notes.push(format!(
                        "pre-push: {rel} lacked {m}.beam (a cut compile or a torn copy): removed, the compile builds it again"
                    ));
                    let _ = fs::remove_dir_all(&lib);
                    break;
                }
            }
        }
    }
    notes
}

/// The modules an .app file lists: the `{modules,[...]}` at depth 2, inside `{application,Name,[...]}`; an env value holding
/// one, before or after it (rebar3 appends modules after env), is no module list (/code-review).
pub fn app_modules(app: &str) -> Vec<String> {
    let text: String = app
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\'')
        .collect();
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '{' | '[' => {
                if depth == 2 && text[i..].starts_with("{modules,[") {
                    let rest = &text[i + 10..];
                    let list = &rest[..rest.find(']').unwrap_or(rest.len())];
                    return list
                        .split(',')
                        .filter(|m| !m.is_empty())
                        .map(String::from)
                        .collect();
                }
                depth += 1;
            }
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_modules_reads_the_list_at_depth_two_only() {
        let _budget = dash_meter::memory::TestBudget::new();
        let app = "{application,jason,[{env,[{modules,[nope]}]},{modules,['Elixir.Jason', 'Elixir.Jason.Encode']}]}.";
        assert_eq!(app_modules(app), ["Elixir.Jason", "Elixir.Jason.Encode"]);
        let none = app_modules("{application,x,[{env,[]}]}.");
        assert!(
            none.is_empty(),
            "an .app with no module list lists none. Found: {none:?}"
        );
    }
}
