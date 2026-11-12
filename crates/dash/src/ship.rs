//! `dash ship "<message>" <path>...`: commits the named paths and gets them to origin, then Codeberg, whatever the other
//! sessions did meanwhile. git-ship's steps (private/dashboard/priv/commands/git-ship, which the older app keeps for its own
//! ships; the why of each step is in its comments), run in this binary: bin/dash execs it before any call to :7788, so a
//! deploy restarting the front door cannot cut it, and it ends on its commit or its failure, never « may have run partway »
//! (task dash-cli-survives-deploy: three ships lost their answer to curl's exit 52 on 2026-11-10; @nha, 2026-11-10:
//! « remember we move to rust. no more elixir, so if something needs fixing port and then fix »).
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const QUICK: Duration = Duration::from_secs(60);
/// The pre-commit hook runs whole test suites under load.
const CHECKS: Duration = Duration::from_secs(3600);
const NET: Duration = Duration::from_secs(300);
/// The empty tree, what a repository with no commit yet compares against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The app's code: everything of the older app and the Rust tool but their notes (`.md`): lib and tests, config, scripts,
/// manifests. The live app runs from the main checkout, so its code changes only in a task's worktree and a PR
/// (private/dashboard/CLAUDE.md rule 0); the orchestrator scripted launch.ex there and shipped it (task dash-cli-survives-deploy).
const APP_CODE: [&str; 2] = ["private/dashboard/", "rust/"];

fn app_code(name: &str) -> bool {
    APP_CODE.iter().any(|a| name.starts_with(a)) && !name.ends_with(".md")
}

struct Ran {
    ok: bool,
    code: Option<i32>,
    out: String,
}

/// git in the checkout, on its own index or a temporary one.
struct Git {
    dir: PathBuf,
    index: Option<String>,
}

impl Git {
    /// stdout, or stdout and stderr together when `merged`: git says what went wrong on stderr.
    fn call(&self, args: &[&str], env: &[(&str, &str)], limit: Duration, merged: bool) -> Ran {
        let mut env = env.to_vec();
        if let Some(index) = &self.index {
            env.push(("GIT_INDEX_FILE", index));
        }
        let (bin, mut all) = if merged {
            ("sh", vec!["-c", "exec git \"$@\" 2>&1", "git"])
        } else {
            ("git", vec![])
        };
        all.extend_from_slice(args);
        match dash_process::answer(bin, &all, Some(&self.dir), &env, limit) {
            Ok((status, out, err)) => Ran {
                ok: status.success(),
                code: status.code(),
                out: if merged { out } else { out + &err },
            },
            Err(no) => Ran {
                ok: false,
                code: None,
                out: no.why,
            },
        }
    }

    fn said(&self, args: &[&str]) -> Ran {
        self.call(args, &[], QUICK, true)
    }

    fn ok(&self, args: &[&str]) -> bool {
        self.call(args, &[], QUICK, false).ok
    }

    fn text(&self, args: &[&str]) -> Option<String> {
        let ran = self.call(args, &[], QUICK, false);
        ran.ok.then(|| ran.out.trim().to_string())
    }

    fn path(&self, name: &str) -> PathBuf {
        let p = self
            .text(&["rev-parse", "--git-path", name])
            .unwrap_or_default();
        self.dir.join(p)
    }

    /// Waits for another session's commit to leave the index, up to 2 minutes.
    fn unlocked(&self) {
        let lock = self.path("index.lock");
        for _ in 0..120 {
            if !lock.exists() {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    fn rev(&self, rev: &str) -> String {
        self.text(&["rev-parse", "-q", "--verify", rev])
            .unwrap_or_default()
    }

    /// A linked worktree (a task's) has a git dir of its own; the main checkout's is the common one.
    fn task_worktree(&self) -> bool {
        let dir = |which| self.text(&["rev-parse", "--path-format=absolute", which]);
        dir("--git-dir") != dir("--git-common-dir")
    }
}

fn with<'a>(head: &[&'a str], paths: &[&'a str]) -> Vec<&'a str> {
    let mut all = head.to_vec();
    all.extend_from_slice(paths);
    all
}

/// The temporary index, removed however the ship ends.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

inventory::submit! { crate::usage::Command::usage("dash ship \"<message>\" <file>...") }
/// Runs `dash ship <message> <path>...` in DASH_REPO, else the checkout holding the current folder; its exit code.
pub fn run(args: &[&str]) -> i32 {
    let [message, paths @ ..] = args else {
        println!("git-ship: name the message and the files to commit");
        return 2;
    };
    if paths.is_empty() {
        println!("git-ship: name the files to commit");
        return 2;
    }
    let here = Git {
        dir: PathBuf::from("."),
        index: None,
    };
    let dir = match std::env::var("DASH_REPO") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => match here.text(&["rev-parse", "--show-toplevel"]) {
            Some(top) => PathBuf::from(top),
            None => {
                println!("git-ship: this folder is in no git checkout, nothing committed");
                return 1;
            }
        },
    };
    let wait = std::env::var("SHIP_RETRY_WAIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    Ship {
        git: Git { dir, index: None },
        wait: Duration::from_secs(wait),
        backup: None,
        restored: false,
    }
    .ship(message, paths)
}

struct Ship {
    git: Git,
    wait: Duration,
    /// refs/ship-backup/<time>: the uncommitted edits before a rebase.
    backup: Option<String>,
    restored: bool,
}

impl Ship {
    fn ship(&mut self, message: &str, paths: &[&str]) -> i32 {
        if let Err(why) = self.refused() {
            println!("git-ship: {why}");
            return 1;
        }
        let git = &self.git;
        let head = git.rev("HEAD");
        let index =
            TempIndex(std::env::temp_dir().join(format!("ship-index.{}", std::process::id())));
        let staged = Git {
            dir: git.dir.clone(),
            index: Some(index.0.display().to_string()),
        };
        let from = if head.is_empty() { "--empty" } else { &head };
        let added = staged.said(&["read-tree", from]);
        if !added.ok {
            println!(
                "{}git-ship: git read-tree failed, nothing committed",
                added.out
            );
            return 1;
        }
        if let Err(why) = add(&staged, paths) {
            println!("{why}");
            return 1;
        }
        let base = if head.is_empty() { EMPTY_TREE } else { &head };
        // fails closed: a diff that did not answer is no proof the app's code is out of it
        let Some(names) = staged.text(&with(
            &["diff", "--cached", "--name-only", base, "--"],
            paths,
        )) else {
            println!("git-ship: git diff of the named paths failed, nothing committed");
            return 1;
        };
        let app: Vec<&str> = names.lines().filter(|n| app_code(n)).collect();
        if !app.is_empty() && !git.task_worktree() {
            println!(
                "git-ship: {} is the app's code, shipped from {}, a checkout that is no task worktree: nothing committed. \
                 Change it in a task's worktree and its PR (dash task add)",
                app.join(" "),
                git.dir.display()
            );
            return 1;
        }
        git.unlocked();
        if let Err(why) = add(git, paths) {
            println!("{why}");
            return 1;
        }
        let began = SystemTime::now();
        if !staged.ok(&with(&["diff", "--cached", "--quiet", "HEAD", "--"], paths)) {
            let (passed, said) = checks(&staged);
            if !passed {
                println!("{}\ngit-ship: commit failed", said.trim_end());
                return 1;
            }
        }
        let checks = secs(began);
        let began = SystemTime::now();
        let mut committed = false;
        for _ in 0..5 {
            git.unlocked();
            if git.ok(&with(&["diff", "--cached", "--quiet", "--"], paths)) {
                // already as committed (another session's commit swept them up): what is pushed is said as HEAD
                println!("git-ship: nothing new to commit in {}", paths.join(" "));
                committed = true;
                break;
            }
            if !staged.ok(&with(&["diff", "--quiet", "--"], paths)) {
                let moved = staged
                    .text(&with(&["diff", "--name-only", "--"], paths))
                    .unwrap_or_default()
                    .replace('\n', " ");
                println!(
                    "git-ship: {moved} changed during the checks: nothing committed, ship again"
                );
                return 1;
            }
            let commit = git.said(&with(
                &["commit", "-q", "--no-verify", "-m", message, "--"],
                paths,
            ));
            if commit.ok {
                committed = true;
                break;
            }
            if !(commit.out.contains("index.lock") || commit.out.contains("cannot lock ref")) {
                print!("{}", commit.out);
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        if !committed {
            println!("git-ship: commit failed");
            return 1;
        }
        println!(
            "git-ship: checks {checks}s, then the commit {}s",
            secs(began)
        );
        if git.rev("HEAD^") != head && git.rev("HEAD") != head {
            println!(
                "git-ship: another commit landed during the checks: they ran without it, CI checks the two together"
            );
        }
        drop(staged);
        drop(index);
        self.prune_backups();
        self.push()
    }

    /// The refusals before anything is staged: a detached HEAD, a master GitHub renamed, a branch tracking another remote.
    fn refused(&self) -> Result<(), String> {
        let git = &self.git;
        let Some(branch) = git.text(&["symbolic-ref", "--short", "HEAD"]) else {
            return Err("HEAD is detached (a rebase in progress?), nothing committed".into());
        };
        let default = std::fs::read_to_string(git.dir.join("forge.env"))
            .unwrap_or_default()
            .lines()
            .find_map(|l| {
                l.strip_prefix("DEFAULT_BRANCH=")
                    .map(|v| v.trim_matches(['"', '\'']).to_string())
            })
            .unwrap_or_default();
        if branch == "master" && !default.is_empty() && default != "master" {
            let gone = git.call(
                &[
                    "ls-remote",
                    "--exit-code",
                    "-q",
                    "origin",
                    "refs/heads/master",
                ],
                &[],
                Duration::from_secs(20),
                false,
            );
            if gone.code == Some(2) {
                return Err(format!(
                    "origin has no master, its branch is {default}: nothing committed. Fix: private/dashboard/bin/dash forge move"
                ));
            }
        }
        let remote = git
            .text(&["config", &format!("branch.{branch}.remote")])
            .unwrap_or_default();
        if !["", "origin", "."].contains(&remote.as_str()) {
            let up = git
                .text(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
                .unwrap_or_default();
            let fix = if git.ok(&[
                "rev-parse",
                "-q",
                "--verify",
                &format!("refs/remotes/origin/{branch}"),
            ]) {
                format!("git branch -u origin/{branch} {branch}")
            } else {
                format!("git push -u origin {branch}")
            };
            return Err(format!(
                "{branch} tracks {up}, not origin: nothing committed. Fix: {fix}"
            ));
        }
        Ok(())
    }

    fn prune_backups(&self) {
        let cut = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            .saturating_sub(7 * 86400);
        let refs = self
            .git
            .text(&[
                "for-each-ref",
                "--format=%(committerdate:unix) %(refname)",
                "refs/ship-backup",
            ])
            .unwrap_or_default();
        for line in refs.lines() {
            if let Some((t, r)) = line.split_once(' ')
                && t.parse::<u64>().is_ok_and(|t| t < cut)
            {
                self.git.ok(&["update-ref", "-d", r]);
            }
        }
    }

    fn merge_dir(&self) -> PathBuf {
        self.git.path("rebase-merge")
    }

    fn rebasing(&self) -> bool {
        self.merge_dir().is_dir() || self.git.path("rebase-apply").is_dir()
    }

    /// A rebase that died after its autostash: its folder holds that autostash alone.
    fn only_stash(&self) -> bool {
        let merge = self.merge_dir();
        merge.join("autostash").is_file()
            && std::fs::read_dir(&merge).map_or(0, Iterator::count) == 1
    }

    fn stale(&self) -> bool {
        let old = std::fs::metadata(self.merge_dir())
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(600));
        self.only_stash() && old && self.git.ok(&["symbolic-ref", "-q", "HEAD"])
    }

    /// The uncommitted edits as they are before a rebase, one per HEAD.
    fn snapshot(&mut self) {
        let git = &self.git;
        if let Some(b) = &self.backup
            && git.rev("HEAD") == git.rev(&format!("{b}^1"))
        {
            return;
        }
        git.unlocked();
        let Some(stash) = git.text(&["stash", "create", "git-ship backup"]) else {
            println!("git-ship: git stash create failed: this rebase runs with no snapshot");
            return;
        };
        if stash.is_empty() {
            return;
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let backup = format!("refs/ship-backup/{now}-{}", std::process::id());
        git.ok(&["update-ref", &backup, &stash]);
        self.backup = Some(backup);
        self.restored = false;
    }

    fn kept(&self) {
        match (&self.backup, self.restored) {
            (Some(b), true) => {
                println!("git-ship: your edits are back on the tree, and also kept in {b}")
            }
            (Some(b), false) => {
                println!(
                    "git-ship: your uncommitted edits before the rebase are in {b}: git stash apply {b}"
                )
            }
            (None, _) => {}
        }
    }

    /// Our own rebase stopped: abort it; one that died after `created`, its own autostash, is quit; the snapshot goes back
    /// on each file the reset took back to HEAD, while HEAD has not moved.
    fn undo(&mut self, created: Option<&str>) {
        let git = &self.git;
        git.unlocked();
        if git.ok(&["rebase", "--abort"]) {
            self.restored = true;
            return;
        }
        if let Some(created) = created
            && self.only_stash()
            && git.rev(&format!("{created}^{{commit}}"))
                == std::fs::read_to_string(self.merge_dir().join("autostash"))
                    .unwrap_or_default()
                    .trim()
        {
            git.ok(&["rebase", "--quit"]);
        }
        let Some(b) = self.backup.clone() else { return };
        if git.rev("HEAD") != git.rev(&format!("{b}^1")) {
            return;
        }
        let names = |rev: &str| git.text(&["diff", "--name-only", rev]).unwrap_or_default();
        let at_head = names("HEAD");
        let at_head: Vec<&str> = at_head.lines().collect();
        let off = names(&b);
        let reset: Vec<&str> = off.lines().filter(|f| !at_head.contains(f)).collect();
        if reset.is_empty() {
            return;
        }
        if git.ok(&with(&["restore", "-s", &b, "-W", "--"], &reset)) {
            self.restored = true;
            println!("git-ship: the rebase died after its autostash: edits put back from {b}");
        }
    }

    fn push(&mut self) -> i32 {
        let branch = self
            .git
            .text(&["symbolic-ref", "--short", "HEAD"])
            .unwrap_or_default();
        let upstream = format!("origin/{branch}");
        // the last push's own words: a pre-push refusal, a key, a rejected tip
        let mut refused = String::new();
        for _ in 0..5 {
            if self.rebasing() {
                if !self.stale() {
                    std::thread::sleep(self.wait);
                    continue;
                }
                let dead =
                    std::fs::read_to_string(self.merge_dir().join("autostash")).unwrap_or_default();
                if !self.git.ok(&["rebase", "--quit"]) {
                    std::thread::sleep(self.wait);
                    continue;
                }
                println!(
                    "git-ship: cleared a dead rebase (only its autostash, over 10 minutes old): its edits are off the tree, in git stash list: git stash apply {}",
                    dead.trim()
                );
            }
            let fetched = self
                .git
                .call(&["fetch", "-q", "origin", &branch], &[], NET, false)
                .ok;
            if fetched
                && !self
                    .git
                    .ok(&["merge-base", "--is-ancestor", &upstream, "HEAD"])
            {
                self.snapshot();
                let rebase = self.git.call(
                    &["rebase", "-q", "--autostash", &upstream],
                    &[("LC_ALL", "C")],
                    // local work, bounded only against a hang: a rebase killed past its autostash is the worse outcome
                    CHECKS,
                    true,
                );
                if !rebase.ok {
                    println!("{}", rebase.out.trim_end());
                    let out = &rebase.out;
                    if ["Created autostash", "Could not apply", "CONFLICT"]
                        .iter()
                        .any(|s| out.contains(s))
                    {
                        let created = out
                            .lines()
                            .find_map(|l| l.strip_prefix("Created autostash: "))
                            .map(str::to_string);
                        self.undo(created.as_deref());
                    }
                    std::thread::sleep(self.wait);
                    continue;
                }
                if rebase.out.contains("safe in the stash") {
                    println!("{}", rebase.out.trim_end());
                    self.kept();
                } else if let Some(b) = self.backup.take() {
                    self.git.ok(&["update-ref", "-d", &b]);
                }
            }
            // DASH_SHIP: the one push to the default branch .githooks/pre-push lets through from a worktree
            let pushed = self.git.call(
                &["push", "-q", "origin", &branch],
                &[("DASH_SHIP", "1")],
                NET,
                true,
            );
            if !pushed.ok {
                refused = pushed.out;
                std::thread::sleep(self.wait);
                continue;
            }
            let commit = self
                .git
                .text(&["log", "-1", "--format=%H %s"])
                .unwrap_or_default();
            println!("git-ship: pushed to {upstream}: {commit}");
            self.mirror(&branch);
            let conflicted = self
                .git
                .text(&["diff", "--name-only", "--diff-filter=U"])
                .unwrap_or_default();
            if !conflicted.is_empty() {
                println!(
                    "git-ship: shipped, but uncommitted edits to {} conflicted when put back: resolve the markers now (git stash list keeps the edits)",
                    conflicted.replace('\n', " ")
                );
                self.kept();
                return 3;
            }
            return 0;
        }
        let status = self.git.text(&["status", "-sb"]).unwrap_or_default();
        println!(
            "{}git-ship: PUSH FAILED, {branch} is not on origin, nothing pushed:\n{}",
            refused,
            status.lines().next().unwrap_or_default()
        );
        self.kept();
        1
    }

    /// Codeberg mirrors origin (@nha, 2026-10-09: « private/dashboard/bin/dash ship should also push to codeberge please »): a
    /// failed push is said, and Dashboard.Mirror's next pass catches it up. A key prompt, a dead host or a stalled transfer
    /// fails fast.
    fn mirror(&self, branch: &str) {
        let git = &self.git;
        let ssh = std::env::var("GIT_SSH_COMMAND")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| git.text(&["config", "core.sshCommand"]))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "ssh".into());
        let ssh = format!(
            "{ssh} -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=10 -o ServerAliveCountMax=3"
        );
        let env = [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_SSH_COMMAND", ssh.as_str()),
        ];
        let pushed = git.call(&["push", "-q", "codeberg", branch], &env, NET, true);
        if pushed.ok {
            return;
        }
        // rejected because another session already pushed a newer tip that holds ours: not a failure
        let tip = git
            .call(
                &["ls-remote", "codeberg", &format!("refs/heads/{branch}")],
                &env,
                NET,
                false,
            )
            .out
            .split('\t')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if !tip.is_empty() && git.ok(&["merge-base", "--is-ancestor", "HEAD", &tip]) {
            return;
        }
        println!(
            "git-ship: codeberg push FAILED, origin has it:\n{}",
            pushed.out.trim_end()
        );
    }
}

/// Stages what exists or is still tracked: a path git rm removed is committed as git rm staged it. A path that is neither here,
/// nor in the index, nor in HEAD is a typo or a folder's relative path (paths are the checkout's): refused, never a ship of
/// nothing said as pushed.
fn add(git: &Git, paths: &[&str]) -> Result<(), String> {
    for p in paths {
        if git.dir.join(p).exists() || git.ok(&["ls-files", "--error-unmatch", "--", p]) {
            let added = git.said(&["add", "-A", "--", p]);
            if !added.ok {
                return Err(format!(
                    "{}git-ship: git add {p} failed, nothing committed",
                    added.out
                ));
            }
        } else if git
            .text(&["ls-tree", "--name-only", "HEAD", "--", p])
            .unwrap_or_default()
            .is_empty()
        {
            return Err(format!(
                "git-ship: {p} is no file of {} nor tracked (paths are the checkout's, from its root): nothing committed",
                git.dir.display()
            ));
        }
    }
    Ok(())
}

/// The pre-commit hook on the temporary index, in a process group of its own: past CHECKS the whole group is killed, never a
/// mix test or cargo run left behind holding its locks. Its output, stdout and stderr in one file.
fn checks(staged: &Git) -> (bool, String) {
    use std::os::unix::process::CommandExt;
    let out = std::env::temp_dir().join(format!("ship-checks.{}", std::process::id()));
    let ran = std::fs::File::create(&out)
        .and_then(|file| Ok((file.try_clone()?, file)))
        .and_then(|(err, file)| {
            let mut c = std::process::Command::new("git");
            c.args(["hook", "run", "--ignore-missing", "pre-commit"])
                .current_dir(&staged.dir)
                .stdin(std::process::Stdio::null())
                .stdout(file)
                .stderr(err)
                .process_group(0);
            if let Some(index) = &staged.index {
                c.env("GIT_INDEX_FILE", index);
            }
            c.spawn()
        })
        .map_err(|e| format!("git-ship: the checks did not start: {e}"))
        .and_then(|mut child| {
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => return Ok(status.success()),
                    Ok(None) if start.elapsed() > CHECKS => {
                        let group = i32::try_from(child.id()).unwrap_or(i32::MAX);
                        unsafe { libc::kill(-group, libc::SIGKILL) };
                        let _ = child.wait();
                        return Err(format!(
                            "git-ship: the checks gave no answer in {CHECKS:?}, stopped"
                        ));
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                    Err(e) => return Err(format!("git-ship: the checks: {e}")),
                }
            }
        });
    let said = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    match ran {
        Ok(passed) => (passed, said),
        Err(why) => (false, format!("{said}{why}")),
    }
}

fn secs(since: SystemTime) -> u64 {
    since.elapsed().map_or(0, |d| d.as_secs())
}
