//! `dash orch takeover`: what a new orchestrator needs of the one before it, in one read (task orch-takeover-state; @nha,
//! 2026-11-14: « ah so you didn't think to resume ongoing work when taking over, right ? what about the existing tabs/panes ?
//! or the inflight work ? », « think about the next run and how to systematically recover »).
//!
//! 1. **The predecessor:** the thread record's `predecessor` (session::thread_start keeps it at each new session of an
//!    orchestrator thread), and its last report, the last text of its Claude transcript.
//! 2. **Every herdr pane,** Claude or shell, with the thread (its task) its last start line names.
//! 3. **Open PRs, worktrees holding work on no remote, and his questions:** `dash asks`, then his open comments, `dash queue`.
//!
//! The orchestrator's SessionStart hook (session::session_state) prints it when a session takes over a thread.
use crate::send::truthy;
use serde_json::Value;
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A program run in a folder: its stdout, or why not. The live one is [`live`]; a test hands in its own.
pub type Runner = dyn Fn(&str, &[&str], &Path) -> Result<String, String> + Sync;

/// Each call bounded: a hook that hangs holds his session's start. 10 s: part 4 runs four calls in turn, 40 s at most,
/// under Claude Code's 60 s hook default (/code-review of PR 1635).
pub fn live(bin: &str, args: &[&str], dir: &Path) -> Result<String, String> {
    // herdr as dash's own pane reads find it: HERDR_BIN, else ~/.local/bin, which a hook's PATH often lacks (/code-review)
    let herdr = std::env::var("HERDR_BIN").unwrap_or_else(|_| {
        format!(
            "{}/.local/bin/herdr",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let bin = if bin == "herdr" { herdr.as_str() } else { bin };
    let limit = Duration::from_secs(10);
    // a restarting app fails at once, never holding the start through bin/dash's 30 retries (/simplify of PR 1635)
    if bin.ends_with("/dash") {
        let env = [("DASH_RESTART_TRIES", "0")];
        let (status, out, err) =
            dash_process::answer(bin, args, Some(dir), &env, limit).map_err(String::from)?;
        return match status.success() {
            true => Ok(out),
            false => Err(format!("{bin}: {}", err.trim())),
        };
    }
    dash_process::run(bin, args, Some(dir), limit)
}

/// How much of a transcript's end is read for its last report: a long orchestrator's file runs to tens of MB.
const TAIL: u64 = 4 << 20;
/// The report kept, in characters: its six points fit, a runaway message does not take the start's whole context.
const REPORT: usize = 4000;
/// The start's context, in characters: Claude Code cuts a hook's context near 10,000 (/code-review of PR 1635).
const CONTEXT: usize = 9000;

/// The thread record's start lines, oldest first (dash's handover reads them too).
pub fn starts(threads: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(threads).unwrap_or_default();
    let lines = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok());
    lines.filter(|e| e["ev"] == "start").collect()
}

/// The takeover as the start's context: cut at [`CONTEXT`], saying where the whole of it is.
pub(crate) fn context(text: String) -> String {
    match text.chars().count() > CONTEXT {
        true => {
            let cut: String = text.chars().take(CONTEXT).collect();
            format!("{cut}\n(cut at {CONTEXT} characters: `dash orch takeover` prints it whole)\n")
        }
        false => text,
    }
}

/// `dash orch takeover` by hand: `repo`'s orchestrator thread as its last start line names it.
pub fn of_repo(home: &Path, repo: &Path, run: &Runner) -> String {
    let name = repo
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().to_string());
    let thread = format!("orchestrator:{name}");
    let starts = starts(&home.join(".claude/run/threads.jsonl"));
    let last = starts.iter().rev().find(|e| e["thread"] == thread.as_str());
    let predecessor = last.and_then(|e| e["predecessor"].as_str());
    text(home, repo, &thread, predecessor, &starts, run)
}

/// The takeover of `thread` from `predecessor`, `starts` the thread record's start lines: every part said, a part that
/// could not be read said as its error.
pub fn text(
    home: &Path,
    repo: &Path,
    thread: &str,
    predecessor: Option<&str>,
    starts: &[Value],
    run: &Runner,
) -> String {
    let mut out = format!("Takeover of {thread} (dash orch takeover)\n");
    out += "1. The predecessor:\n";
    match predecessor {
        None => out += "   none recorded in ~/.claude/run/threads.jsonl\n",
        Some(id) => {
            let _ = writeln!(out, "   {id}, its last report:");
            let report = last_report(&home.join(".claude/projects"), id)
                .unwrap_or_else(|| "(no text found in its transcript)".into());
            for l in report.lines() {
                let _ = writeln!(out, "   | {l}");
            }
        }
    }
    // site_perso's dash, which every repo's orchestrator asks (CLAUDE.md), never one under another repo (/code-review)
    let dash = home.join("repo/site_perso/private/dashboard/bin/dash");
    let dash = dash.to_string_lossy();
    // the parts at once: the hook's start waits for the slowest (part 4, four bounded calls in turn), never their sum
    type Part<'a> = Box<dyn FnOnce() -> Result<String, String> + Send + 'a>;
    let parts: [(&str, Part); 5] = [
        (
            "2. Every herdr pane (pane, kind, state, thread, folder)",
            Box::new(|| panes(starts, run, repo)),
        ),
        ("3. Open PRs", Box::new(|| prs(run, repo))),
        (
            "4. Worktrees with work on no remote",
            Box::new(|| unpushed(run, repo)),
        ),
        (
            "5. His questions, waiting on him (dash asks)",
            Box::new(|| run(&dash, &["asks"], repo)),
        ),
        (
            "6. His open comments (dash queue)",
            Box::new(|| run(&dash, &["queue"], repo)),
        ),
    ];
    let said: Vec<(&str, String)> = std::thread::scope(|scope| {
        let looks: Vec<_> = (parts.into_iter())
            .map(|(head, part)| (head, scope.spawn(part)))
            .collect();
        let joined = looks.into_iter().map(|(head, look)| {
            let part = look
                .join()
                .unwrap_or_else(|_| Err("its read panicked".into()));
            (head, indent(part))
        });
        joined.collect()
    });
    for (head, part) in said {
        let _ = write!(out, "{head}:\n{part}");
    }
    // a handover starts this session while the predecessor's turn runs on (Dashboard.Orch.AccountHandover)
    out += "Read at this session's start: once the predecessor's turn ends, `dash orch takeover` reads it all again.\n";
    out
}

/// Lines kept of each part: sixty panes must not cut parts 3 to 6 out of the start's context (/code-review of PR 1635).
const PART_LINES: usize = 25;

fn indent(part: Result<String, String>) -> String {
    let text = part.unwrap_or_else(|e| format!("not read: {e}"));
    let text = match text.trim() {
        "" => "none",
        t => t,
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut out: String = lines
        .iter()
        .take(PART_LINES)
        .map(|l| format!("   {l}\n"))
        .collect();
    if lines.len() > PART_LINES {
        out += &format!(
            "   ({} more: `dash orch takeover` prints them all)\n",
            lines.len() - PART_LINES
        );
    }
    out
}

/// The last text the session `id` wrote, from the end of `<projects>/*/<id>.jsonl`.
fn last_report(projects: &Path, id: &str) -> Option<String> {
    // an id is Claude's uuid: never a path a thread line could point elsewhere with
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let folders = std::fs::read_dir(projects).ok()?.flatten();
    let file = folders
        .map(|d| d.path().join(format!("{id}.jsonl")))
        .find(|f| f.is_file())?;
    let text = tail(&file)?;
    let entries = text
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok());
    // Claude Code's own lines (a usage limit, an API error) and a subagent's are not its report (/code-review of PR 1635)
    let its = |e: &Value| {
        e["type"] == "assistant"
            && e["message"]["model"] != "<synthetic>"
            && !truthy(&e["isApiErrorMessage"])
            && !truthy(&e["isSidechain"])
    };
    let report = entries.filter(its).find_map(|e| {
        let content = &e["message"]["content"];
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }
        let texts: Vec<&str> = (content.as_array()?.iter())
            .filter(|c| c["type"] == "text")
            .filter_map(|c| c["text"].as_str())
            .collect();
        (!texts.is_empty()).then(|| texts.join("\n"))
    })?;
    Some(report.chars().take(REPORT).collect())
}

/// A file's last `TAIL` bytes, its first cut line dropped.
fn tail(file: &PathBuf) -> Option<String> {
    let mut f = std::fs::File::open(file).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut bytes = vec![];
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    Some(match from {
        0 => text,
        _ => text
            .split_once('\n')
            .map_or(String::new(), |(_, rest)| rest.to_string()),
    })
}

/// `herdr pane list`, each pane's thread from the start line naming its Claude session, else the last naming its pane, else
/// its folder.
fn panes(starts: &[Value], run: &Runner, repo: &Path) -> Result<String, String> {
    let said: Value = serde_json::from_str(&run("herdr", &["pane", "list"], repo)?)
        .map_err(|e| format!("herdr pane list: {e}"))?;
    let panes = said["result"]["panes"]
        .as_array()
        .ok_or("herdr pane list: no panes")?;
    let thread_of = |key: &str, v: &str| {
        let found = starts.iter().rev().find(|e| !v.is_empty() && e[key] == v);
        found.and_then(|e| e["thread"].as_str()).map(String::from)
    };
    let lines = panes.iter().map(|p| {
        let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
        let pane = s("pane_id");
        let kind = p["agent"].as_str().unwrap_or("shell");
        let session = p["agent_session"]["value"].as_str().unwrap_or("");
        let thread = thread_of("id", session)
            .or_else(|| thread_of("pane", &pane))
            // a shell split beside a task's session: the thread last started in its folder
            .or_else(|| thread_of("cwd", p["cwd"].as_str().unwrap_or("")))
            .filter(|t| !t.starts_with("pane:"))
            .unwrap_or_else(|| "no task".into());
        format!("{pane} {kind} {} {thread} {}", s("agent_status"), s("cwd"))
    });
    Ok(lines.collect::<Vec<_>>().join("\n"))
}

fn prs(run: &Runner, repo: &Path) -> Result<String, String> {
    let args = [
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        "100",
        "--json",
        "number,title,headRefName,url",
    ];
    let said: Value =
        serde_json::from_str(&run("gh", &args, repo)?).map_err(|e| format!("gh pr list: {e}"))?;
    let prs = said.as_array().ok_or("gh pr list: no list")?.iter();
    let s = |p: &Value, k: &str| p[k].as_str().unwrap_or("").to_string();
    let lines = prs.map(|p| {
        format!(
            "#{} {} ({}) {}",
            p["number"],
            s(p, "title"),
            s(p, "headRefName"),
            s(p, "url")
        )
    });
    Ok(lines.collect::<Vec<_>>().join("\n"))
}

/// Each worktree whose head holds commits on no remote, or whose files hold changes not committed: a merged branch's
/// commits are not counted, its uncommitted files are.
fn unpushed(run: &Runner, repo: &Path) -> Result<String, String> {
    let list = run("git", &["worktree", "list", "--porcelain"], repo)?;
    // a branch pushed then deleted on the remote is a squash-merged PR (or one closed): its commits sit on no remote, yet
    // they are no work in flight; files changed since still are (/code-review of PR 1635)
    let refs = [
        "for-each-ref",
        "--format=%(refname:short) %(upstream:track)",
        "refs/heads",
    ];
    let refs = run("git", &refs, repo)?;
    let gone: Vec<&str> = (refs.lines())
        .filter_map(|l| l.strip_suffix(" [gone]"))
        .collect();
    let trees: Vec<(PathBuf, String, String)> = list
        .split("\n\n")
        .filter_map(|block| {
            let field = |k: &str| block.lines().find_map(|l| l.strip_prefix(k)).map(str::trim);
            let path = PathBuf::from(field("worktree ")?);
            let branch = field("branch refs/heads/")
                .unwrap_or("detached")
                .to_string();
            Some((path, field("HEAD ")?.to_string(), branch))
        })
        // the tool's own checkouts (live builds, the pre-commit's staged tree, a reviewer's lane) hold no task's work
        .filter(|(path, _, _)| {
            let p = path.to_string_lossy();
            path.is_dir() && !p.contains("/.claude/run/") && !p.contains("/.git/")
        })
        .collect();
    // ponytail: one thread per worktree, some fifty today; a pool if there come to be hundreds
    let lines: Vec<Option<String>> = std::thread::scope(|scope| {
        let gone = &gone;
        let looks = trees.iter().map(|(path, head, branch)| {
            scope.spawn(move || {
                let count = |r: Result<String, String>| r.map(|t| t.trim().to_string());
                let commits = match gone.contains(&branch.as_str()) {
                    true => Ok("0".into()),
                    false => count(run(
                        "git",
                        &["rev-list", "--count", head, "--not", "--remotes"],
                        repo,
                    )),
                };
                let changes = run(
                    "git",
                    &["--no-optional-locks", "status", "--porcelain"],
                    path,
                )
                .map(|t| t.lines().count());
                let (commits, changes) = match (commits, changes) {
                    (Ok(c), Ok(n)) if c == "0" && n == 0 => return None,
                    (Ok(c), Ok(n)) => (
                        format!("{c} commits on no remote"),
                        format!("{n} files changed"),
                    ),
                    (c, n) => (
                        c.unwrap_or_else(|e| e),
                        n.map_or_else(|e| e, |n| format!("{n} files changed")),
                    ),
                };
                Some(format!(
                    "{} ({branch}): {commits}, {changes}",
                    path.display()
                ))
            })
        });
        let looks: Vec<_> = looks.collect();
        looks
            .into_iter()
            .map(|l| l.join().unwrap_or(None))
            .collect()
    });
    Ok(lines.into_iter().flatten().collect::<Vec<_>>().join("\n"))
}

const USAGE: &str = "dash orch takeover";

fn takeover_run(host: &dyn dash_core::Host, args: &[&str], out: &mut dyn Write) {
    let home = match (args, host.home()) {
        ([], Ok(home)) => home,
        ([], Err(e)) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
        _ => {
            eprintln!("usage: {USAGE}");
            std::process::exit(2)
        }
    };
    // the caller's main checkout, as the hook's CLAUDE_PROJECT_DIR is: klar's orchestrator takes over klar's thread
    let cwd = std::env::current_dir().unwrap_or_else(|_| host.repo());
    let common = live(
        "git",
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        &cwd,
    );
    let repo = (common.ok())
        .and_then(|c| Path::new(c.trim()).parent().map(Path::to_path_buf))
        .unwrap_or_else(|| host.repo());
    let _ = write!(out, "{}", of_repo(&home, &repo, &live));
}

dash_core::submit! { dash_core::Command { usage: USAGE, words: &["orch", "takeover"], run: Some(takeover_run) } }

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// Done-when 2 of orch-takeover-state: the predecessor's last report, every herdr pane, Claude or shell, with its task,
    /// the open PRs, the worktrees with work on no remote, his questions and his open comments.
    /// A long part keeps its first lines and says how many more, so the parts after it still reach the start's context.
    #[dash_test]
    fn a_long_part_is_cut_and_says_how_many_lines_more() {
        let many: String = (0..30).map(|n| format!("pane {n}\n")).collect();
        let cut = indent(Ok(many));
        assert_eq!(cut.lines().count(), PART_LINES + 1, "{cut}");
        assert!(
            cut.ends_with("(5 more: `dash orch takeover` prints them all)\n"),
            "{cut}"
        );
        assert_eq!(indent(Ok("one\n".into())), "   one\n");
    }

    #[dash_test]
    fn the_takeover_prints_what_the_predecessor_had_in_flight() {
        let dir = TestDir::new("takeover");
        let home = dir.0.join("home");
        let repo = dir.0.join("site_perso");
        let wt = dir.0.join("wt");
        for d in [
            home.join(".claude/run"),
            home.join(".claude/projects/-x"),
            repo.join(".git/pre-commit-staged"),
            wt.clone(),
            dir.0.join("merged"),
            dir.0.join("kept"),
        ] {
            std::fs::create_dir_all(d).expect("dirs");
        }
        let threads = [
            r#"{"ev":"start","thread":"orchestrator:site_perso","id":"old-1","pane":"w1:p1"}"#,
            r#"{"ev":"start","thread":"task-a","id":"sa","pane":"w2:p1","cwd":"/wt/a"}"#,
            r#"{"ev":"start","thread":"orchestrator:site_perso","id":"new-2","predecessor":"old-1","pane":"w3:p1"}"#,
        ];
        std::fs::write(home.join(".claude/run/threads.jsonl"), threads.join("\n"))
            .expect("threads");
        let transcript = [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"an earlier report"}]}}"#,
            r#"{"type":"user","message":{"content":"go on"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Bottom line: PR 12 waits on CI."}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"a subagent's"}]}}"#,
            r#"{"type":"assistant","isApiErrorMessage":true,"message":{"model":"<synthetic>","content":[{"type":"text","text":"Claude AI usage limit reached"}]}}"#,
        ];
        std::fs::write(
            home.join(".claude/projects/-x/old-1.jsonl"),
            transcript.join("\n"),
        )
        .expect("transcript");
        let wt_s = wt.display().to_string();
        let worktrees = format!(
            "worktree {}\nHEAD aaa\nbranch refs/heads/main\n\nworktree {wt_s}\nHEAD bbb\nbranch refs/heads/task/a\n\nworktree /gone\nHEAD ccc\n\nworktree {}/.git/pre-commit-staged\nHEAD ddd\n\nworktree {}\nHEAD eee\nbranch refs/heads/task/merged\n\nworktree {}\nHEAD fff\nbranch refs/heads/task/kept\n",
            repo.display(),
            repo.display(),
            dir.0.join("merged").display(),
            dir.0.join("kept").display()
        );
        let run = move |bin: &str, args: &[&str], dir: &Path| -> Result<String, String> {
            let words = format!("{bin} {}", args.join(" "));
            Ok(match words.as_str() {
                "herdr pane list" => r#"{"result":{"panes":[
                    {"pane_id":"w2:p1","agent":"claude","agent_status":"working","cwd":"/wt/a","agent_session":{"value":"sa"}},
                    {"pane_id":"w2:p2","agent_status":"unknown","cwd":"/wt/a"},
                    {"pane_id":"w2:p3","agent_status":"unknown","cwd":"/wt/b"}]}}"#.into(),
                w if w.starts_with("gh pr list") => {
                    r#"[{"number":12,"title":"[feat][task-a] A","headRefName":"task/a","url":"https://x/12"}]"#.into()
                }
                "git worktree list --porcelain" => worktrees.clone(),
                w if w.starts_with("git for-each-ref") => "main \ntask/a [ahead 2]\ntask/merged [gone]\ntask/kept [gone]\n".into(),
                "git rev-list --count aaa --not --remotes" => "0\n".into(),
                "git rev-list --count bbb --not --remotes" => "2\n".into(),
                "git --no-optional-locks status --porcelain" if dir.ends_with("wt") || dir.ends_with("kept") => " M a.rs\n".into(),
                "git --no-optional-locks status --porcelain" => String::new(),
                w if w.ends_with("dash asks") => "1. Approve the draft on /d/x".into(),
                w if w.ends_with("dash queue") => Err("the app is down".to_string())?,
                w => Err(format!("unexpected: {w}"))?,
            })
        };
        let text = of_repo(&home, &repo, &run);
        for want in [
            "Takeover of orchestrator:site_perso",
            "   old-1, its last report:\n   | Bottom line: PR 12 waits on CI.\n",
            "   w2:p1 claude working task-a /wt/a\n",
            "   w2:p2 shell unknown task-a /wt/a\n",
            "   w2:p3 shell unknown no task /wt/b\n",
            "   #12 [feat][task-a] A (task/a) https://x/12\n",
            &format!("   {wt_s} (task/a): 2 commits on no remote, 1 files changed\n"),
            "5. His questions, waiting on him (dash asks):\n   1. Approve the draft on /d/x\n",
            "6. His open comments (dash queue):\n   not read: the app is down\n",
            "(task/kept): 0 commits on no remote, 1 files changed\n",
            "once the predecessor's turn ends, `dash orch takeover` reads it all again",
        ] {
            assert!(text.contains(want), "{want:?} missing from:\n{text}");
        }
        assert!(
            !text.contains("an earlier report")
                && !text.contains("usage limit")
                && !text.contains("a subagent's")
                && !text.contains("(main)")
                && !text.contains("staged")
                && !text.contains("merged"),
            "{text}"
        );
        assert_eq!(context(text.clone()), text);
        let long = context("x".repeat(CONTEXT + 50));
        assert!(
            long.starts_with(&"x".repeat(CONTEXT))
                && long.ends_with("`dash orch takeover` prints it whole)\n")
                && !long.contains(&"x".repeat(CONTEXT + 1)),
            "{long}"
        );
        let none = super::text(
            &home,
            &repo,
            "orchestrator:x",
            None,
            &[],
            &|_: &str, _: &[&str], _: &Path| Ok(String::new()),
        );
        assert!(
            none.contains("1. The predecessor:\n   none recorded"),
            "{none}"
        );
        // bin/dash hands the words to the Rust binary only when it holds the usage bin/dash greps for (/code-review of PR 1635)
        let Some(top) = dash_meter::monorepo() else {
            return;
        };
        let wrapper =
            std::fs::read_to_string(top.join("private/dashboard/bin/dash")).expect("bin/dash");
        assert!(
            wrapper.contains(&format!("\"orch takeover\") known='{USAGE}'")),
            "bin/dash does not route orch takeover to Rust: copy USAGE into its case"
        );
    }
}
