//! A sandbox denial in a session becomes a candidate and his one tap (task rust-denial-hook; @nha, 2026-11-10: « " The sandbox
//! won't let me read" hook to trigger a change + approve request. In rust of course. »). SANDBOX.md: a denial is a rule to
//! propose, never a limit to report.
//!
//! 1. `dash sandbox denied`, Claude's PostToolUse hook on Bash and Read, reads « Operation not permitted » on a path under home
//!    in the tool's own error words (`read`) and hands it to `dash sandbox propose`. A task session cannot write the main
//!    checkout, so the front door, which runs there, answers that command (`answer`).
//! 2. `propose` appends `add-dirs-ro=` (a read) or `add-dirs=` (a write) to the main checkout's `.safehouse.candidate`,
//!    started from `.safehouse` as `dash sandbox candidate .safehouse` starts it (a copy only when none is there), the denial's
//!    date, command and line above it.
//! 3. It ships the candidate with `dash ship`, once per path a day, so /approve lists it for his tap
//!    (`Dashboard.Approvals.section` reads the main checkout's candidate).
use crate::pages::repo;
use dash_process as process;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

const CANDIDATE: &str = ".safehouse.candidate";
/// What each day's proposals were: `<day> <path>` a line, so a path ships once a day however often a session is denied it.
const LEDGER: &str = ".claude/run/denials-proposed";

/// Never proposed: keys, keychains and all of Claude's folder (its settings, accounts and transcripts).
const SECRET: [&str; 6] = [
    ".ssh",
    ".aws",
    ".gnupg",
    ".config/gh",
    "Library/Keychains",
    ".claude",
];
/// Never proposed for writing: what the host runs outside the sandbox and another repo's checkout (Dashboard.Sandbox's
/// @never). Reading them may be proposed.
const HOST: [&str; 6] = [
    "repo",
    "Library/Launch",
    ".config/site_perso",
    ".local/bin",
    ".nvm",
    ".cargo/bin",
];
/// A denial these programs print asks for a write rule.
const WRITERS: [&str; 13] = [
    "touch", "mkdir", "cp", "mv", "rm", "rmdir", "ln", "tee", "install", "chmod", "chown", "rsync",
    "truncate",
];
/// Whole words of a denial's message, its paths left out, that say it was a write (git's « Unable to create »).
const WRITES: [&str; 8] = [
    "create", "creating", "write", "writing", "mkdir", "rename", "unlink", "remove",
];
const SHELLS: [&str; 3] = ["bash", "sh", "zsh"];

#[derive(Clone, Debug, PartialEq)]
pub struct Denial {
    /// What the rule opens, `~/`-relative.
    pub path: String,
    pub write: bool,
    pub tool: String,
    /// The command, its program and paths alone (`redacted`), or the Read's file.
    pub command: String,
    /// The tool's line that names the denial.
    pub line: String,
}

/// The denial a hook's input names: a line of the tool's own error words with « Operation not permitted » and a path under
/// `home` that a rule may open. A Bash's stdout or a Read's text that quotes a denial is no denial. A Read names its file
/// when its error line does not.
pub fn read(hook: &Value, home: &str) -> Option<Denial> {
    if home.len() < 2 || !home.starts_with('/') {
        return None;
    }
    let tool = hook["tool_name"].as_str()?;
    let input = &hook["tool_input"];
    let command = match tool {
        "Bash" => input["command"].as_str()?,
        "Read" => input["file_path"].as_str()?,
        _ => return None,
    };
    let errors = [
        hook["error"].as_str(),
        hook["tool_response"]["stderr"].as_str(),
    ];
    let line = errors
        .into_iter()
        .flatten()
        .flat_map(str::lines)
        .find(|l| l.to_lowercase().contains("operation not permitted"))?;
    let named = paths_on(line).into_iter().find_map(|w| under_home(w, home));
    let raw = match (named, tool) {
        (Some(p), _) => p,
        (None, "Read") => under_home(command, home)?,
        (None, _) => return None,
    };
    let write = tool == "Bash" && writes(line);
    let path = folder(&raw, home, write);
    allowed(&path, write).then(|| Denial {
        path,
        write,
        tool: tool.into(),
        command: redacted(command),
        line: line.trim().replace(home, "~"),
    })
}

/// The paths a line may name, likeliest first: quoted, then a `: ` field (`ls: <path>: Operation…`, spaces kept), then a word.
fn paths_on(line: &str) -> Vec<&str> {
    let quoted = line.split(['\'', '"']).skip(1).step_by(2);
    let fields = line.split(": ");
    let words = line.split_whitespace();
    quoted
        .chain(fields)
        .chain(words)
        .map(|w| {
            w.trim()
                .trim_matches(|c: char| "'\"`,:;()[]<>«»".contains(c))
        })
        .filter(|w| w.starts_with('/') || w.starts_with("~/"))
        .collect()
}

fn under_home(path: &str, home: &str) -> Option<String> {
    let rel = path
        .strip_prefix("~/")
        .or_else(|| path.strip_prefix(home)?.strip_prefix('/'))?;
    (!rel.is_empty()).then(|| format!("~/{rel}"))
}

/// What a rule opens for `path`. A read: the path itself, file or folder, as `.safehouse` takes either; the hook's own stat
/// of it is denied too, so it never guesses which. A write: the folder the write lands in, or the path when it is a folder.
fn folder(path: &str, home: &str, write: bool) -> String {
    let path = path.strip_suffix('/').unwrap_or(path);
    if !write || Path::new(home).join(&path[2..]).is_dir() {
        return path.into();
    }
    path.rsplit_once('/').map_or(path, |(dir, _)| dir).into()
}

/// Two folders deep at least under home, nothing a `.safehouse` line would read as two paths, never a secret, and for a write
/// never the host's.
fn allowed(path: &str, write: bool) -> bool {
    let Some(rel) = path.strip_prefix("~/") else {
        return false;
    };
    let under = |p: &str| match p {
        "Library/Launch" => rel.starts_with(p),
        _ => rel == p || rel.starts_with(&format!("{p}/")),
    };
    rel.split('/').count() >= 2
        && !rel.split('/').any(|part| part.is_empty() || part == "..")
        && !rel.contains([':', '\n', '\r'])
        && !SECRET.iter().any(|p| under(p))
        && !(write && HOST.iter().any(|p| under(p)))
}

/// A write: a writing program printed it, a shell's redirection did (`bash: <path>: Operation…`), or its message says so.
fn writes(line: &str) -> bool {
    let base = |f: &str| f.rsplit('/').next().unwrap_or_default().to_string();
    let fields: Vec<&str> = line.split(": ").map(str::trim).collect();
    let shelled = fields
        .first()
        .is_some_and(|f| SHELLS.contains(&base(f).as_str()));
    let first = fields
        .iter()
        .find(|f| !SHELLS.contains(&base(f).as_str()) && !f.starts_with("line "))
        .copied()
        .unwrap_or_default();
    let redirected = shelled && (first.starts_with('/') || first.starts_with("~/"));
    let mut message = line.to_lowercase();
    for p in paths_on(line) {
        message = message.replace(&p.to_lowercase(), " ");
    }
    let said = message
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| WRITES.contains(&w));
    WRITERS.contains(&base(first).as_str()) || redirected || said
}

/// A command as the candidate's comment and the commit keep it: its program and its paths, the rest (flags, headers, tokens)
/// as `…`, so no secret on its line is ever pushed.
fn redacted(command: &str) -> String {
    let mut kept: Vec<&str> = vec![];
    for (i, w) in command.split_whitespace().enumerate() {
        let path = w.starts_with('/') || w.starts_with("~/");
        let w = match (i == 0 || path) && !w.contains('=') {
            true => w,
            false => "…",
        };
        if !(w == "…" && kept.last() == Some(&"…")) {
            kept.push(w);
        }
    }
    kept.join(" ")
}

/// Whether `text`, a .safehouse, already opens `path`: an `add-dirs=` folder at or above it, or for a read an `add-dirs-ro=` one.
pub fn covered(text: &str, path: &str, write: bool) -> bool {
    text.lines().any(|l| {
        let dirs = match l.trim().split_once('=') {
            Some(("add-dirs", d)) => d,
            Some(("add-dirs-ro", d)) if !write => d,
            _ => return false,
        };
        dirs.split(':')
            .map(|d| d.trim().trim_end_matches('/'))
            .any(|d| path == d || path.starts_with(&format!("{d}/")))
    })
}

/// The `Other repos:` answer a new rule carries (SANDBOX.md, A denial, rule 4): each other repo under `home`'s `repo/` with an
/// inbox, not needed, since only this repo's session was denied it.
fn other_repos(home: &Path, me: &str) -> String {
    let mut names: Vec<String> = std::fs::read_dir(home.join("repo"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("to_process").is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != me)
        .collect();
    names.sort();
    match names.is_empty() {
        true => "# Other repos: none with an inbox.\n".into(),
        false => names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let head = if i == 0 { "# Other repos: " } else { "# " };
                format!("{head}{n} not needed: only {me}'s session was denied it;\n")
            })
            .collect(),
    }
}

#[derive(Debug, PartialEq)]
pub enum Proposed {
    /// The approved .safehouse opens it already: the denial is another profile's.
    Known,
    /// Proposed and shipped today already.
    Today,
    /// Written (or already in the candidate): ship it with this message; its ledger line, dropped if the ship fails.
    Ship { message: String, key: String },
}

/// One proposal at a time: two denials at once would each append to the candidate they read.
static ONE: Mutex<()> = Mutex::new(());
/// One ship at a time: two in the main checkout would race on its index.
static SHIP: Mutex<()> = Mutex::new(());

/// The rule for `d` in `repo`'s `.safehouse.candidate`, `now` its `YYYY-MM-DDTHH:MM`, with the ledger's line for the day. A
/// read of a path that does not exist is refused: safehouse refuses a missing read-only path, and his whole tap with it.
pub fn propose(repo: &Path, home: &Path, d: &Denial, now: &str) -> Result<Proposed, String> {
    let _one = ONE.lock().unwrap_or_else(PoisonError::into_inner);
    let day = &now[..10.min(now.len())];
    let ledger = repo.join(LEDGER);
    let seen = std::fs::read_to_string(&ledger).unwrap_or_default();
    let key = format!("{day} {}", d.path);
    if seen.lines().any(|l| l == key) {
        return Ok(Proposed::Today);
    }
    let base = std::fs::read_to_string(repo.join(".safehouse"))
        .map_err(|e| format!("no .safehouse in {}: {e}", repo.display()))?;
    if covered(&base, &d.path, d.write) {
        return Ok(Proposed::Known);
    }
    let on_disk = std::fs::symlink_metadata(home.join(&d.path[2..]));
    if !d.write && on_disk.is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Err(format!("{} does not exist: no read rule for it", d.path));
    }
    let cand = repo.join(CANDIDATE);
    let text = std::fs::read_to_string(&cand).unwrap_or(base);
    let (key_word, how) = match d.write {
        true => ("add-dirs", "read-write"),
        false => ("add-dirs-ro", "read-only"),
    };
    if !covered(&text, &d.path, d.write) {
        let clip = |s: &str, n: usize| -> String {
            s.replace(['\n', '\r', '`'], " ").chars().take(n).collect()
        };
        let me = repo
            .file_name()
            .map_or("site_perso".into(), |n| n.to_string_lossy());
        let block = format!(
            "\n# Denied {now} ({}): `{}` « {} ».\n# Proposed {how} by dash sandbox denied, for his tap on /approve (task rust-denial-hook).\n{}{key_word}={}\n",
            d.tool,
            clip(&d.command, 100),
            clip(&d.line, 160),
            other_repos(home, &me),
            d.path
        );
        std::fs::write(&cand, text.trim_end().to_string() + "\n" + &block)
            .map_err(|e| format!("{}: {e}", cand.display()))?;
    }
    if let Some(dir) = ledger.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ledger)
        .and_then(|mut f| writeln!(f, "{key}"))
        .map_err(|e| format!("{}: {e}", ledger.display()))?;
    let message = format!(
        "Sandbox candidate: {how} {}, denied {day}\n\n{} « {} » (dash sandbox denied, task rust-denial-hook)",
        d.path,
        d.tool,
        d.line.chars().take(200).collect::<String>()
    );
    Ok(Proposed::Ship { message, key })
}

/// A failed ship's ledger line taken out, so the path's next denial ships it again.
fn unlog(repo: &Path, key: &str) {
    let _one = ONE.lock().unwrap_or_else(PoisonError::into_inner);
    let ledger = repo.join(LEDGER);
    let seen = std::fs::read_to_string(&ledger).unwrap_or_default();
    let kept: String = seen
        .lines()
        .filter(|l| *l != key)
        .map(|l| format!("{l}\n"))
        .collect();
    let _ = std::fs::write(&ledger, kept);
}

/// The older CLI, `DASH_CLI` naming another, as the bridge's.
fn dash_bin() -> String {
    std::env::var("DASH_CLI").unwrap_or_else(|_| {
        repo()
            .join("private/dashboard/bin/dash")
            .display()
            .to_string()
    })
}

/// `dash ship <message> .safehouse.candidate` in `repo`, in a process group of its own; its error words kept for the log.
pub fn ship(bin: &str, repo: &Path, message: &str) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new(bin);
    c.args(["ship", message, CANDIDATE])
        .current_dir(repo)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .process_group(0);
    c
}

/// Ships on a thread of its own, one at a time, past the hook's wait (a ship rebases and pushes). A failure goes to the front
/// door's log and drops the ledger line; the candidate stays on disk, where /approve reads it anyway.
fn ship_later(bin: String, repo: std::path::PathBuf, message: String, key: String) {
    std::thread::spawn(move || {
        let _ship = SHIP.lock().unwrap_or_else(PoisonError::into_inner);
        let failed = match ship(&bin, &repo, &message).output() {
            Ok(o) if o.status.success() => return,
            Ok(o) => format!(
                "{}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => e.to_string(),
        };
        eprintln!("sandbox propose: dash ship of {key} failed: {failed}");
        unlog(&repo, &key);
    });
}

/// `dash sandbox propose PATH rw|ro TOOL COMMAND LINE`, answered by the front door in the main checkout: exit, out, err.
pub fn answer(args: &[String]) -> (i32, String, String) {
    let [path, rw, tool, command, line] = args else {
        return (
            2,
            String::new(),
            "usage: dash sandbox propose PATH rw|ro TOOL COMMAND LINE\n".into(),
        );
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let d = Denial {
        path: path.clone(),
        write: rw == "rw",
        tool: tool.clone(),
        command: redacted(command),
        line: line.clone(),
    };
    if home.len() < 2 || !allowed(&d.path, d.write) {
        let why = format!("sandbox propose: {path} is never proposed\n");
        return (1, String::new(), why);
    }
    let repo = repo();
    let said = |s: String| (0, s, String::new());
    match propose(
        &repo,
        Path::new(&home),
        &d,
        &crate::automations::local_now(),
    ) {
        Ok(Proposed::Known) => said(format!(
            "{}: .safehouse opens it already: the denial is sandbox.sb's\n",
            d.path
        )),
        Ok(Proposed::Today) => said(format!(
            "{}: proposed and shipped today already: his tap on /approve\n",
            d.path
        )),
        Ok(Proposed::Ship { message, key }) => {
            ship_later(dash_bin(), repo, message, key);
            said(format!(
                "{}: proposed in .safehouse.candidate, shipping it for his tap on /approve\n",
                d.path
            ))
        }
        Err(e) => (1, String::new(), format!("sandbox propose: {e}\n")),
    }
}

inventory::submit! { crate::usage::Command::usage("dash sandbox denied (a hook: its input on stdin)") }
/// `dash sandbox denied`: the hook. Silent unless its input names a denial; then the proposal's answer, as context for the
/// session, which carries on by another route and says nothing of it. Always exit 0: a hook never stops the work.
pub fn hook(mut out: std::fs::File) {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(hook) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let Some(d) = read(&hook, &home) else { return };
    let rw = if d.write { "rw" } else { "ro" };
    let args = [
        "sandbox", "propose", &d.path, rw, &d.tool, &d.command, &d.line,
    ];
    let said = match process::answer(&dash_bin(), &args, None, &[], Duration::from_secs(30)) {
        Ok((_, out, err)) => format!("{}{}", out.trim(), err.trim()),
        Err(e) => e.why,
    };
    let event = hook["hook_event_name"].as_str().unwrap_or("PostToolUse");
    let context = format!(
        "Sandbox denial of {} handled by dash sandbox denied: {said}. Carry on by a route that works; no need to report it.",
        d.path
    );
    let _ = writeln!(
        out,
        "{}",
        json!({"hookSpecificOutput": {"hookEventName": event, "additionalContext": context}})
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    const HOME: &str = "/Users/me";

    fn bash(command: &str, stderr: &str) -> Value {
        json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_input": {"command": command},
               "tool_response": {"stdout": "", "stderr": stderr, "interrupted": false}})
    }

    fn denied(stderr: &str) -> Option<(String, bool)> {
        read(&bash("x", stderr), HOME).map(|d| (d.path, d.write))
    }

    #[test]
    fn a_bash_denial_names_its_path_read_or_write() {
        let _budget = TestBudget::new();
        let d = read(
            &bash(
                "curl -H 'Authorization: Bearer sk-1' TOKEN=abc ls ~/Library/LaunchAgents/",
                "ls: /Users/me/Library/LaunchAgents/: Operation not permitted",
            ),
            HOME,
        )
        .expect("a denial");
        assert_eq!(
            (d.path.as_str(), d.write),
            ("~/Library/LaunchAgents", false)
        );
        // its program and paths alone: no header, no token
        assert_eq!(d.command, "curl … ~/Library/LaunchAgents/");
        assert_eq!(
            d.line,
            "ls: ~/Library/LaunchAgents/: Operation not permitted"
        );
        let some = |p: &str, w: bool| Some((p.to_string(), w));
        // a write opens the folder it lands in; a path with a space is kept whole
        assert_eq!(
            denied("touch: /Users/me/x/y/notes.txt: Operation not permitted"),
            some("~/x/y", true)
        );
        assert_eq!(
            denied("ls: /Users/me/Library/Application Support/Foo: Operation not permitted"),
            some("~/Library/Application Support/Foo", false)
        );
        assert_eq!(
            denied(
                "fatal: Unable to create '/Users/me/x/y/.git/index.lock': Operation not permitted"
            ),
            some("~/x/y/.git", true)
        );
        assert_eq!(
            denied("bash: /Users/me/x/y/out: Operation not permitted"),
            some("~/x/y", true)
        );
        // reads, however their path is spelled: cd, cat, a « lock » or « write » inside a path
        assert_eq!(
            denied("bash: line 1: cd: /Users/me/x/y: Operation not permitted"),
            some("~/x/y", false)
        );
        assert_eq!(
            denied("cat: /Users/me/Library/Caches/blocklist/rewrite: Operation not permitted"),
            some("~/Library/Caches/blocklist/rewrite", false)
        );
        assert_eq!(
            denied("ls: /Users/me/Library/Caches/com.example.app: Operation not permitted"),
            some("~/Library/Caches/com.example.app", false)
        );
        // a failed Bash reaches PostToolUseFailure, its words in `error`
        let failed = json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash", "tool_input": {"command": "x"},
                            "error": "Exit code 1\nmkdir: /Users/me/.cache/tool/sub: Operation not permitted"});
        assert_eq!(read(&failed, HOME).expect("a denial").path, "~/.cache/tool");
        // a command's stdout quoting a denial (dash denials, a grep) is none
        let quoted = json!({"tool_name": "Bash", "tool_input": {"command": "dash denials 6h"},
                            "tool_response": {"stdout": "ls: /Users/me/a/b: Operation not permitted", "stderr": ""}});
        assert_eq!(read(&quoted, HOME), None);
        // no denial, outside home, a secret, Claude's folder, one folder deep, a write to the host's, no HOME: nothing
        assert_eq!(denied("ls: /Users/me/a/b: No such file or directory"), None);
        assert_eq!(denied("ls: /opt/x/y: Operation not permitted"), None);
        assert_eq!(
            denied("ls: /Users/me/.ssh/keys/: Operation not permitted"),
            None
        );
        assert_eq!(
            denied("cat: /Users/me/.claude/accounts/w/settings.json: Operation not permitted"),
            None
        );
        assert_eq!(
            denied("ls: /Users/me/Documents/: Operation not permitted"),
            None
        );
        assert_eq!(
            denied("touch: /Users/me/repo/site_perso/.claude/run/f: Operation not permitted"),
            None
        );
        assert_eq!(
            read(&bash("x", "ls: /opt/x/y: Operation not permitted"), ""),
            None
        );
    }

    #[test]
    fn a_read_denial_names_its_file() {
        let _budget = TestBudget::new();
        let read_tool = |error: &str| {
            json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Read",
                   "tool_input": {"file_path": "/Users/me/.vibe/logs/today.log"}, "error": error})
        };
        let d = read(
            &read_tool("EPERM: operation not permitted, open '/Users/me/.vibe/logs/today.log'"),
            HOME,
        )
        .expect("a denial");
        assert_eq!(
            (d.path.as_str(), d.write, d.tool.as_str()),
            ("~/.vibe/logs/today.log", false, "Read")
        );
        // its error names no path: the file it was asked for
        let d = read(&read_tool("Operation not permitted"), HOME).expect("a denial");
        assert_eq!(d.path, "~/.vibe/logs/today.log");
        assert_eq!(read(&read_tool("File does not exist."), HOME), None);
        // a Read that worked, its text quoting a denial, is none
        let worked = json!({"tool_name": "Read", "tool_input": {"file_path": "/Users/me/x/y/log"},
                            "tool_response": {"file": {"content": "cat: /Users/me/a/b: Operation not permitted"}}});
        assert_eq!(read(&worked, HOME), None);
    }

    fn denial(path: &str, write: bool) -> Denial {
        Denial {
            path: path.into(),
            write,
            tool: "Bash".into(),
            command: "ls ~/Library/LaunchAgents/".into(),
            line: "ls: ~/Library/LaunchAgents/: Operation not permitted".into(),
        }
    }

    /// A home holding the main checkout, its `.safehouse`, a folder to read and another repo with an inbox.
    fn checkout(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let home = std::env::temp_dir().join(format!("denied-{name}-{}", std::process::id()));
        let repo = home.join("repo/site_perso");
        for dir in [
            &repo,
            &home.join("Library/LaunchAgents"),
            &home.join("repo/klar/to_process"),
        ] {
            std::fs::create_dir_all(dir).expect("dir");
        }
        let safehouse = "# policy\nadd-dirs=~/.vibe\nadd-dirs-ro=~/.local/bin/vibe\n";
        std::fs::write(repo.join(".safehouse"), safehouse).expect("safehouse");
        (repo, home)
    }

    #[test]
    fn the_rule_goes_in_the_candidate_with_its_date_and_command() {
        let _budget = TestBudget::new();
        let (repo, home) = checkout("rule");
        let d = denial("~/Library/LaunchAgents", false);
        let shipped = propose(&repo, &home, &d, "2026-11-11T10:42").expect("proposed");
        assert!(
            matches!(&shipped, Proposed::Ship { message, .. } if message.starts_with("Sandbox candidate: read-only ~/Library/LaunchAgents, denied 2026-11-11"))
        );
        let cand = std::fs::read_to_string(repo.join(CANDIDATE)).expect("a candidate");
        // started from .safehouse, as dash sandbox candidate .safehouse starts it
        assert!(cand.starts_with("# policy\nadd-dirs=~/.vibe\n"), "{cand}");
        assert!(
            cand.contains("# Denied 2026-11-11T10:42 (Bash): `ls ~/Library/LaunchAgents/`"),
            "{cand}"
        );
        assert!(
            cand.contains(
                "# Other repos: klar not needed: only site_perso's session was denied it;\n"
            ),
            "{cand}"
        );
        assert!(
            cand.ends_with("add-dirs-ro=~/Library/LaunchAgents\n"),
            "{cand}"
        );
        // a write gets a read-write rule, appended to the candidate that is there
        propose(
            &repo,
            &home,
            &denial("~/.cache/tool", true),
            "2026-11-11T10:43",
        )
        .expect("proposed");
        let cand = std::fs::read_to_string(repo.join(CANDIDATE)).expect("a candidate");
        assert!(
            cand.contains("add-dirs-ro=~/Library/LaunchAgents\n")
                && cand.ends_with("add-dirs=~/.cache/tool\n"),
            "{cand}"
        );
        // what .safehouse opens already is no proposal; a missing path is no read rule (safehouse would refuse his tap)
        let at = "2026-11-11T10:44";
        assert_eq!(
            propose(&repo, &home, &denial("~/.vibe/logs", true), at),
            Ok(Proposed::Known)
        );
        assert_eq!(
            propose(&repo, &home, &denial("~/.local/bin/vibe", false), at),
            Ok(Proposed::Known)
        );
        assert!(propose(&repo, &home, &denial("~/x/missing", false), at).is_err());
    }

    #[test]
    fn it_ships_the_candidate_once_per_path_a_day() {
        let _budget = TestBudget::new();
        let (repo, home) = checkout("ship");
        let d = denial("~/Library/LaunchAgents", false);
        let Ok(Proposed::Ship { message, key }) = propose(&repo, &home, &d, "2026-11-11T10:42")
        else {
            panic!("a ship")
        };
        assert_eq!(
            propose(&repo, &home, &d, "2026-11-11T18:00"),
            Ok(Proposed::Today)
        );
        assert!(matches!(
            propose(&repo, &home, &d, "2026-11-12T09:00"),
            Ok(Proposed::Ship { .. })
        ));
        // /approve lists a candidate that differs from .safehouse (bin/sandbox-apply's cand_differs)
        let cand = std::fs::read(repo.join(CANDIDATE)).expect("candidate");
        assert_ne!(
            cand,
            std::fs::read(repo.join(".safehouse")).expect("safehouse")
        );
        // the ship: dash ship <message> .safehouse.candidate, in the main checkout
        let fake = home.join("fake-dash");
        std::fs::write(&fake, "#!/bin/sh\nprintf '%s|' \"$PWD\" \"$@\" > called\n").expect("fake");
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&fake)
            .status()
            .expect("chmod");
        let status = ship(&fake.display().to_string(), &repo, &message)
            .status()
            .expect("ran");
        assert!(status.success());
        let called = std::fs::read_to_string(repo.join("called")).expect("called");
        let here = repo
            .canonicalize()
            .expect("canonical")
            .display()
            .to_string();
        assert_eq!(
            called,
            format!("{here}|ship|{message}|.safehouse.candidate|")
        );
        // a failed ship drops its ledger line: the path's next denial ships again that day
        unlog(&repo, &key);
        assert!(matches!(
            propose(&repo, &home, &d, "2026-11-11T19:00"),
            Ok(Proposed::Ship { .. })
        ));
    }
}
