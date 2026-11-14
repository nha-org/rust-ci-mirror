//! A task worktree's two dash MCP servers (task worktree-surfaces; @nha, 2026-10-20: « have two local MCPs: one which is
//! bound to the local commands of the dash, which runs in memory locally in the worktree and another which is the main MCP
//! which talks to main »; « they must not get confused »). rfc-rust-unified's #worktree-surfaces holds the decision.
//! - `dash mcp --scratch --data <live>`, named dash-scratch: this checkout's own `dash` on a scratch copy of the live data,
//!   made at the first call and removed at the session's end. No word that reports or leaves the Mac runs there, and the
//!   older app it would reach answers on no port: its writes reach no one.
//! - `dash mcp --main <checkout> [--allow <tools>]`, named dash-main: the main app's commands a task may send, its allowlist
//!   alone listed, any other name refused.
//! - `dash mcp config <worktree> <checkout> <folder>`: both, each behind `dash mcp-proxy --watch`, in `<folder>/mcp.json`,
//!   which herdr/session.sh hands a worktree's Claude with --mcp-config.
use dash_core::registry::{Contribution, Registry, Tool};
use serde_json::{Value, json};
use std::cell::OnceCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCRATCH: &str = "dash-scratch";
pub const MAIN: &str = "dash-main";
/// What every scratch answer opens with, and its server's instructions.
pub const SCRATCH_SAYS: &str = "dash-scratch: this worktree's dash on a scratch copy of the live data; its writes reach no one.";
pub const MAIN_SAYS: &str =
    "dash-main: the main app his dashboard runs on; each call here reaches it.";

/// The commands that run on the scratch copy: those that read or write the data folder alone, never a process, a pane, a
/// push or a send. An allowlist, so a command added later stays refused until named here (/code-review: a list of refused
/// words failed open, `launchd run` and `claude --in-pane` were missing from it).
const ALLOWED: &[&str] = &[
    "help",
    "menu",
    "extensions",
    "ingest",
    "statements",
    "chances",
    "alerts",
    "latest",
    "audit",
    "draft show",
    "draft rates",
    "draft new",
    "draft import",
    "doc extract",
    "doc save",
    "doc move",
    "kb move",
    "comment move",
    "code map",
    "task show",
    "task style",
    "task rfc-check",
    "roadmap show",
    "inbox ls",
    "inbox rules",
    "orch next",
    "scripts count",
    "gate firings",
    "folds",
];

/// A port nothing listens on: the older app, which a Rust command asks through bin/dash, answers nowhere from scratch.
const NOWHERE: &str = "http://127.0.0.1:9";
const LIMIT: Duration = Duration::from_secs(120);

/// Why `words` may not run on the scratch copy, if it may not: a command off the allowlist, or a data folder other than
/// the copy.
pub fn refused(words: &[String]) -> Option<String> {
    let said = format!("{} ", words.join(" "));
    if !ALLOWED.iter().any(|w| said.starts_with(&format!("{w} "))) {
        let first = words.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
        return Some(format!(
            "dash {first} is not run on the scratch copy, which runs only {}; dash-main carries what a task may send",
            ALLOWED.join(", ")
        ));
    }
    let data = words
        .windows(2)
        .find(|w| w[0] == "--data" && w[1] != "scratch");
    data.map(|w| {
        format!(
            "--data {}: only --data scratch, the copy, is read here",
            w[1]
        )
    })
}

/// The folder of a scratch copy that is this process's: `dash-scratch-<pid>`.
fn scratch_name(pid: u32) -> String {
    format!("dash-scratch-{pid}")
}

/// The scratch copy: a repository root of its own, made at the first call, removed when the server ends. Each command
/// runs with `DASH_REPO` on it, so every path a command derives from the repository (the live data, the older app's
/// database, the journals) lands in the copy, never in the checkout.
struct Scratch {
    live: PathBuf,
    exe: PathBuf,
    copy: OnceCell<Result<PathBuf, String>>,
}

impl Scratch {
    /// The scratch repository's root. A copy a killed server left (its pid gone) is removed first.
    fn root(&self) -> Result<&Path, String> {
        let made = self.copy.get_or_init(|| {
            let tmp = std::env::temp_dir();
            prune(&tmp);
            let root = tmp.join(scratch_name(std::process::id()));
            crate::data::copy(&self.live, &data_of(&root)).map(|_| root)
        });
        made.as_deref().map_err(Clone::clone)
    }
}

/// Where the copy's data sits in its root, where `pages::live_data` looks under `DASH_REPO`.
fn data_of(root: &Path) -> PathBuf {
    root.join("private/live/rust/data")
}

/// Every `dash-scratch-<pid>` in `tmp` whose process is gone: a server killed outright never removed its copy.
fn prune(tmp: &Path) {
    for entry in std::fs::read_dir(tmp).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let pid = name
            .strip_prefix("dash-scratch-")
            .and_then(|p| p.parse::<i32>().ok());
        // ESRCH: no such process; EPERM means one lives under another user
        let gone = pid.is_some_and(|p| {
            let asked = unsafe { libc::kill(p, 0) };
            asked != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        });
        if gone {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(Ok(copy)) = self.copy.get() {
            let _ = std::fs::remove_dir_all(copy);
        }
    }
}

struct ScratchDash(std::rc::Rc<Scratch>);

impl Tool for ScratchDash {
    fn description(&self) -> &str {
        "Runs this worktree's own dash command on a scratch copy of the live data: its writes reach no one. `args`: the \
         words after dash; `--data scratch` names the copy, and paths the command derives from the repository land in it too. \
         Only commands that read or write the data folder run; any other is refused, naming them (dash-main carries what a \
         task may send)."
    }

    fn input_schema(&self) -> &str {
        r#"{"type":"object","properties":{"args":{"type":"array","items":{"type":"string"}}},"required":["args"]}"#
    }

    fn call(&self, arguments: &str) -> Result<String, String> {
        let said = |text: String| format!("{SCRATCH_SAYS}\n{text}");
        let asked: Value = serde_json::from_str(arguments).map_err(|e| said(e.to_string()))?;
        let words: Vec<String> = (asked["args"].as_array().into_iter().flatten())
            .map(|w| w.as_str().map(String::from).ok_or("args: words"))
            .collect::<Result<_, _>>()
            .map_err(|e| said(e.to_string()))?;
        if let Some(why) = refused(&words) {
            return Err(said(why));
        }
        let root = self.0.root().map_err(|e| said(e.clone()))?;
        let folder = data_of(root).display().to_string();
        let mut args: Vec<&str> = words.iter().map(String::as_str).collect();
        for i in 1..args.len() {
            if args[i - 1] == "--data" && args[i] == "scratch" {
                args[i] = &folder;
            }
        }
        let root = root.display().to_string();
        let env = [
            ("DASHBOARD_URL", NOWHERE),
            ("DASH_RESTART_TRIES", "0"),
            ("DASH_REPO", root.as_str()),
        ];
        let exe = self.0.exe.display().to_string();
        let (status, out, err) = dash_process::answer_fed(&exe, &args, None, &env, None, LIMIT)
            .map_err(|e| said(e.why))?;
        let text = said(format!("{out}{}", err.trim()));
        match status.success() {
            true => Ok(text),
            false => Err(format!("{text}\n({status})")),
        }
    }
}

/// Each report's own file, removed once its call returns (/code-review: one name a server, kept).
static REPORTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// bin/dash's words, and its stdin if any.
type Words = (Vec<String>, Option<String>);

/// One tool of the main app: its name, what it does, its schema, and its bin/dash words (and stdin) from its arguments.
struct MainTool {
    name: &'static str,
    description: &'static str,
    schema: &'static str,
    words: fn(&Value) -> Result<Words, String>,
}

fn text(asked: &Value, key: &str) -> Result<String, String> {
    asked[key]
        .as_str()
        .map(String::from)
        .ok_or(format!("{key}: a text"))
}

const MAIN_TOOLS: &[MainTool] = &[
    MainTool {
        name: "task_show",
        description: "The task's live row on the main app: dash task show <id>.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}"#,
        words: |a| Ok((vec!["task".into(), "show".into(), text(a, "id")?], None)),
    },
    MainTool {
        name: "task_report",
        description: "Files the task's report for the PM check: dash task report <id> <record>. `record`: the JSON \
                      record the brief asks for.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"},"record":{"type":"object"}},"required":["id","record"]}"#,
        words: |a| {
            let record = a
                .get("record")
                .filter(|r| r.is_object())
                .ok_or("record: an object")?;
            let file = std::env::temp_dir().join(format!(
                "dash-main-report-{}-{}.json",
                std::process::id(),
                REPORTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::write(&file, record.to_string()).map_err(|e| e.to_string())?;
            let file = file.display().to_string();
            Ok((
                vec!["task".into(), "report".into(), text(a, "id")?, file],
                None,
            ))
        },
    },
    MainTool {
        name: "request",
        description: "Files a request on another project's board: dash request <project> <title> --words <his words> \
                      --priority <1-4> and --later <why>, or --start when `later` is empty.",
        schema: r#"{"type":"object","properties":{"project":{"type":"string"},"title":{"type":"string"},"words":{"type":"string"},"priority":{"type":"integer","minimum":1,"maximum":4},"later":{"type":"string"}},"required":["project","title","words","priority"]}"#,
        words: |a| {
            let priority = a["priority"]
                .as_u64()
                .ok_or("priority: 1 to 4")?
                .to_string();
            let mut words = vec!["request".into(), text(a, "project")?, text(a, "title")?];
            words.extend([
                "--words".into(),
                text(a, "words")?,
                "--priority".into(),
                priority,
            ]);
            match a["later"].as_str().filter(|w| !w.is_empty()) {
                Some(why) => words.extend(["--later".into(), why.into()]),
                None => words.push("--start".into()),
            }
            Ok((words, None))
        },
    },
    MainTool {
        name: "comment_answer",
        description: "Answers one of his comments: dash comment answer <id> -, the text on stdin.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"},"text":{"type":"string"}},"required":["id","text"]}"#,
        words: |a| {
            let words = vec![
                "comment".into(),
                "answer".into(),
                text(a, "id")?,
                "-".into(),
            ];
            Ok((words, Some(text(a, "text")?)))
        },
    },
];

/// A task's allowlist when its launch names none.
pub const TASK_ALLOW: &str = "task_show,task_report,request";

struct MainCall {
    tool: &'static MainTool,
    dash: PathBuf,
}

impl Tool for MainCall {
    fn description(&self) -> &str {
        self.tool.description
    }

    fn input_schema(&self) -> &str {
        self.tool.schema
    }

    fn call(&self, arguments: &str) -> Result<String, String> {
        let asked: Value = serde_json::from_str(arguments).map_err(|e| e.to_string())?;
        let (words, stdin) = (self.tool.words)(&asked)?;
        let args: Vec<&str> = words.iter().map(String::as_str).collect();
        let dash = self.dash.display().to_string();
        let url = std::env::var("DASH_MAIN_URL").unwrap_or("http://127.0.0.1:7788".into());
        let env = [("DASHBOARD_URL", url.as_str()), ("DASH_RESTART_TRIES", "0")];
        let answered = dash_process::answer_fed(&dash, &args, None, &env, stdin.as_deref(), LIMIT)
            .map_err(|e| e.why);
        if self.tool.name == "task_report" {
            let _ = std::fs::remove_file(&words[3]);
        }
        let (status, out, err) = answered?;
        match status.success() {
            true => Ok(format!("{MAIN_SAYS}\n{out}")),
            false => Err(format!("{MAIN_SAYS}\n{out}{}\n({status})", err.trim())),
        }
    }
}

/// The scratch server's tools: the one `dash` tool on a copy of `live`, run by this binary.
pub fn scratch(live: &Path) -> Result<Registry, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let scratch = std::rc::Rc::new(Scratch {
        live: live.to_path_buf(),
        exe,
        copy: OnceCell::new(),
    });
    let mut registry = Registry::default();
    let tools: Vec<(String, Box<dyn Tool>)> = vec![("dash".into(), Box::new(ScratchDash(scratch)))];
    registry.add(
        SCRATCH,
        Contribution {
            tools,
            ..Default::default()
        },
    )?;
    Ok(registry)
}

/// The main server's tools: `allow`'s, each through `checkout`'s bin/dash. A name it does not know fails the start.
pub fn main(checkout: &Path, allow: &str) -> Result<Registry, String> {
    let dash = checkout.join("private/dashboard/bin/dash");
    let mut tools: Vec<(String, Box<dyn Tool>)> = vec![];
    for name in allow.split(',').filter(|n| !n.is_empty()) {
        let tool = MAIN_TOOLS.iter().find(|t| t.name == name).ok_or(format!(
            "--allow {name}: no such tool; dash-main knows {}",
            MAIN_TOOLS
                .iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        tools.push((
            name.into(),
            Box::new(MainCall {
                tool,
                dash: dash.clone(),
            }),
        ));
    }
    let mut registry = Registry::default();
    registry.add(
        MAIN,
        Contribution {
            tools,
            ..Default::default()
        },
    )?;
    Ok(registry)
}

/// A proxy's cache: the list served before the server's first start, and the name and instructions its initialize answers.
fn listed(name: &str, says: &str, registry: &Registry) -> Value {
    let mut cache = dash_surface::tools_list(registry);
    cache["serverInfo"] = json!({"name": name, "version": env!("CARGO_PKG_VERSION")});
    cache["instructions"] = json!(says);
    cache
}

/// `<folder>/mcp.json` for the worktree `worktree` of `checkout`, with each proxy's cache filled, so that no server starts
/// before its first call: dash-scratch runs the worktree's own build, dash-main and both proxies the checkout's live one.
/// Each proxy watches the binary it runs, so a new build serves the next call on the same connection.
pub fn config(worktree: &Path, checkout: &Path, folder: &Path) -> Result<PathBuf, String> {
    let live_dash = checkout.join("private/live/rust/dash");
    let own_dash = worktree.join("rust/target/debug/dash");
    let data = checkout.join("private/live/rust/data");
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let write = |name: &str, text: String| {
        let path = folder.join(name);
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok::<_, String>(path)
    };
    let proxy = |cache: &PathBuf, watch: &Path, server: Vec<String>| {
        let mut args: Vec<String> = ["mcp-proxy", "--cache"].map(String::from).into();
        args.push(cache.display().to_string());
        args.extend([
            "--watch".to_string(),
            watch.display().to_string(),
            "--".to_string(),
        ]);
        args.extend(server);
        json!({ "command": live_dash, "args": args })
    };
    let scratch_cache = write(
        "scratch-tools.json",
        listed(SCRATCH, SCRATCH_SAYS, &scratch(&data)?).to_string(),
    )?;
    let main_cache = write(
        "main-tools.json",
        listed(MAIN, MAIN_SAYS, &main(checkout, TASK_ALLOW)?).to_string(),
    )?;
    let s = |p: &Path| p.display().to_string();
    let scratch_server = vec![
        s(&own_dash),
        "mcp".into(),
        "--scratch".into(),
        "--data".into(),
        s(&data),
    ];
    let main_server = vec![
        s(&live_dash),
        "mcp".into(),
        "--main".into(),
        s(checkout),
        "--allow".into(),
        TASK_ALLOW.into(),
    ];
    let servers = json!({ "mcpServers": {
        SCRATCH: proxy(&scratch_cache, &own_dash, scratch_server),
        MAIN: proxy(&main_cache, &live_dash, main_server),
    }});
    write(
        "mcp.json",
        serde_json::to_string_pretty(&servers).map_err(|e| e.to_string())?,
    )
}

inventory::submit! { crate::usage::Command::usage("dash mcp --scratch --data <dir>") }
inventory::submit! { crate::usage::Command::usage("dash mcp --main <checkout> [--allow <tool,tool>]") }
inventory::submit! { crate::usage::Command::usage("dash mcp config <worktree> <checkout> <folder>") }

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Only the allowlist runs on scratch, and only on the copy: what reports, sends, starts a process, or names another data
    /// folder is refused; a command only starting like an allowed one is too.
    #[dash_test]
    fn scratch_runs_its_allowlist_on_the_copy_alone() {
        let w = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        for words in [
            "task report x f.json",
            "request klar t",
            "comment answer c1 -",
            "ship m f",
            "mail send d1",
            "launchd run --host h --repo r",
            "claude --in-pane --data scratch",
            "extensionsx --data scratch",
            "extensions --data /Users/x/repo/site_perso/private/live/rust/data",
        ] {
            assert!(refused(&w(words)).is_some(), "{words}");
        }
        for words in ["task show x", "extensions --data scratch", "help"] {
            assert!(refused(&w(words)).is_none(), "{words}");
        }
    }

    /// A copy whose server is gone is pruned, one whose server lives is kept.
    #[dash_test]
    fn a_copy_left_by_a_killed_server_is_pruned() {
        let tmp = dash_meter::TestDir::new("surfaces-prune");
        let mut gone = std::process::Command::new("true").spawn().expect("true");
        gone.wait().expect("its end");
        let (dead, alive) = (scratch_name(gone.id()), scratch_name(std::process::id()));
        for name in [&dead, &alive] {
            std::fs::create_dir_all(tmp.0.join(name)).expect("a copy");
        }
        prune(&tmp.0);
        assert!(!tmp.0.join(&dead).exists() && tmp.0.join(&alive).exists());
    }

    /// An allowlist names only known tools; the default lists exactly its three.
    #[dash_test]
    fn main_lists_its_allowlist_and_refuses_an_unknown_name() {
        let registry = main(Path::new("/nowhere"), TASK_ALLOW).expect("the default");
        let names: Vec<&str> = registry.tools.iter().map(|(n, _, _)| n).collect();
        assert_eq!(names, ["request", "task_report", "task_show"]);
        let e = main(Path::new("/nowhere"), "task_show,mail_send")
            .err()
            .expect("refused");
        assert!(e.contains("mail_send: no such tool"), "{e}");
    }
}
