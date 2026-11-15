//! A task worktree's two dash MCP servers (task worktree-surfaces): their config, their lists and refusals, the scratch
//! copy's life, and a reload on one connection, each through the binary as Claude Code runs it.
use dash_meter::{TestDir, dash_test};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

/// A fake main checkout: its bin/dash prints the words it got and its stdin, and its live data holds one file.
fn checkout(dir: &Path) -> PathBuf {
    let root = dir.join("main");
    let bin = root.join("private/dashboard/bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let stand_in =
        dash_meter::stand_in("#!/bin/sh\necho \"main got: $*\"\n[ \"$4\" != - ] || cat\n");
    std::os::unix::fs::symlink(stand_in, bin.join("dash")).expect("bin/dash");
    let data = root.join("private/live/rust/data");
    std::fs::create_dir_all(&data).expect("data");
    std::fs::write(data.join("note.txt"), "live").expect("a live file");
    root
}

/// One MCP client on a child's stdio: each request answered in turn, notifications kept apart.
struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    next: u64,
    notes: Vec<Value>,
}

impl Client {
    fn start(args: &[&str], tmp: &Path) -> Self {
        let mut child = Command::new(DASH)
            .args(args)
            .env("TMPDIR", tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dash");
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            input,
            output,
            next: 1,
            notes: vec![],
        }
    }

    fn ask(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.input.as_mut().expect("open"), "{line}").expect("a request");
        loop {
            let mut text = String::new();
            assert!(
                self.output.read_line(&mut text).expect("a line") > 0,
                "ended before answering {method}"
            );
            let answer: Value = serde_json::from_str(&text).expect("JSON");
            match answer.get("id") {
                Some(_) => return answer,
                None => self.notes.push(answer),
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.ask(
            "tools/call",
            serde_json::json!({"name": tool, "arguments": arguments}),
        )
    }

    /// Its input closed, its end waited for.
    fn end(mut self) -> std::process::ExitStatus {
        drop(self.input.take());
        let mut rest = String::new();
        let _ = std::io::Read::read_to_string(&mut self.output, &mut rest);
        self.child.wait().expect("its end")
    }
}

fn names(list: &Value) -> Vec<String> {
    (list["result"]["tools"].as_array().expect("tools").iter())
        .map(|t| t["name"].as_str().expect("a name").to_string())
        .collect()
}

fn said(answer: &Value) -> String {
    answer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn copies(tmp: &Path) -> Vec<String> {
    let mut names: Vec<String> = (std::fs::read_dir(tmp).expect("tmp").flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("dash-scratch-"))
        .collect();
    names.sort();
    names
}

/// Done-when 1 and 2: the config names two servers, the scratch one's name, instructions and answers say its writes reach
/// no one, it lists no reporting tool and refuses one asked through it; the main one lists only the task's allowlist and
/// refuses any other tool.
#[dash_test]
fn a_worktree_gets_a_scratch_server_and_a_main_server_on_its_allowlist() {
    let dir = TestDir::new("surfaces-two");
    let main = checkout(&dir.0);
    let worktree = dir.0.join("wt");
    let out = Command::new(DASH)
        .args(["mcp", "config"])
        .args([&worktree, &main, &dir.0.join("run")])
        .output()
        .expect("dash mcp config");
    assert!(out.status.success(), "{out:?}");
    let path = String::from_utf8(out.stdout).expect("utf-8");
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(path.trim()).expect("mcp.json"))
            .expect("JSON");
    let servers = config["mcpServers"].as_object().expect("servers");
    assert_eq!(
        servers.keys().collect::<Vec<_>>(),
        ["dash-main", "dash-scratch"]
    );
    let args = |name: &str| servers[name]["args"].to_string();
    assert!(
        args("dash-scratch").contains("wt/rust/target/debug/dash\",\"mcp\",\"--scratch\""),
        "{}",
        args("dash-scratch")
    );
    assert!(
        args("dash-main").contains("\"--main\""),
        "{}",
        args("dash-main")
    );

    let tmp = TestDir::new("surfaces-two-tmp");
    let data = main.join("private/live/rust/data");
    let mut scratch = Client::start(
        &["mcp", "--scratch", "--data", data.to_str().expect("utf-8")],
        &tmp.0,
    );
    let hello = scratch.ask("initialize", serde_json::json!({}));
    assert_eq!(hello["result"]["serverInfo"]["name"], "dash-scratch");
    assert!(
        hello["result"]["instructions"]
            .as_str()
            .expect("instructions")
            .contains("writes reach no one")
    );
    assert_eq!(
        names(&scratch.ask("tools/list", serde_json::json!({}))),
        ["dash"]
    );
    let report = scratch.call(
        "dash",
        serde_json::json!({"args": ["task", "report", "x", "r.json"]}),
    );
    assert_eq!(report["result"]["isError"], true, "{report}");
    assert!(
        said(&report).starts_with("dash-scratch:")
            && said(&report).contains("is not run on the scratch copy"),
        "{report}"
    );
    let ran = scratch.call(
        "dash",
        serde_json::json!({"args": ["extensions", "--data", "scratch"]}),
    );
    assert_eq!(ran["result"]["isError"], false, "{ran}");
    assert!(
        said(&ran).starts_with("dash-scratch:") && said(&ran).contains("writes reach no one"),
        "{ran}"
    );
    assert!(scratch.end().success());

    let mut main_server = Client::start(&["mcp", "--main", main.to_str().expect("utf-8")], &tmp.0);
    let hello = main_server.ask("initialize", serde_json::json!({}));
    assert_eq!(hello["result"]["serverInfo"]["name"], "dash-main");
    assert_eq!(
        names(&main_server.ask("tools/list", serde_json::json!({}))),
        ["request", "task_report", "task_show"]
    );
    let shown = main_server.call("task_show", serde_json::json!({"id": "worktree-surfaces"}));
    assert!(
        said(&shown).contains("main got: task show worktree-surfaces"),
        "{shown}"
    );
    let outside = main_server.call(
        "comment_answer",
        serde_json::json!({"id": "c1", "text": "hi"}),
    );
    assert_eq!(outside["error"]["code"], -32602, "{outside}");
    assert!(
        outside["error"]["message"]
            .as_str()
            .expect("why")
            .contains("no tool comment_answer"),
        "{outside}"
    );
    assert!(main_server.end().success());
}

/// Done-when 3: behind its proxy and the config's cache, the scratch server starts at the first call, which makes the copy,
/// and both are gone when the session's input ends.
#[dash_test]
fn the_scratch_copy_is_made_at_the_first_call_and_gone_at_the_end() {
    let dir = TestDir::new("surfaces-life");
    let main = checkout(&dir.0);
    let tmp = TestDir::new("surfaces-life-tmp");
    let data = main.join("private/live/rust/data");
    // the cache `dash mcp config` writes: the list, the name and the instructions, served before any start
    let made = Command::new(DASH)
        .args(["mcp", "config"])
        .args([&dir.0.join("wt"), &main, &dir.0.join("run")])
        .status()
        .expect("dash mcp config");
    assert!(made.success());
    let cache = dir.0.join("run/scratch-tools.json");
    let (cache, data) = (
        cache.to_str().expect("utf-8"),
        data.to_str().expect("utf-8"),
    );
    let args = [
        "mcp-proxy",
        "--cache",
        cache,
        "--",
        DASH,
        "mcp",
        "--scratch",
        "--data",
        data,
    ];
    let mut client = Client::start(&args, &tmp.0);
    let hello = client.ask("initialize", serde_json::json!({}));
    assert_eq!(
        hello["result"]["serverInfo"]["name"], "dash-scratch",
        "{hello}"
    );
    let says = hello["result"]["instructions"].as_str().unwrap_or_default();
    assert!(says.contains("writes reach no one"), "{hello}");

    assert_eq!(
        names(&client.ask("tools/list", serde_json::json!({}))),
        ["dash"]
    );
    assert!(copies(&tmp.0).is_empty(), "nothing started before a call");
    let ran = client.call(
        "dash",
        serde_json::json!({"args": ["extensions", "--data", "scratch"]}),
    );
    assert_eq!(ran["result"]["isError"], false, "{ran}");
    let made = copies(&tmp.0);
    assert_eq!(made.len(), 1, "{made:?}");
    let copied = tmp.0.join(&made[0]).join("private/live/rust/data/note.txt");
    assert_eq!(std::fs::read_to_string(copied).expect("the copy"), "live");
    assert!(client.end().success());
    assert!(copies(&tmp.0).is_empty(), "the copy outlived the session");
    assert_eq!(
        std::fs::read_to_string(Path::new(data).join("note.txt")).expect("live"),
        "live"
    );
}

/// The largest resident size of any child this process waited for, in bytes (macOS reports bytes).
fn children_peak() -> i64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) };
    unsafe { usage.assume_init() }.ru_maxrss
}

/// Done-when 4: on one connection, a call, a new build of the watched server, a call: the second is the new server's, the
/// client is told the list changed, and the peak memory of the proxy and its servers stays under 64 MB. Both servers.
#[dash_test]
fn both_servers_reload_between_two_calls_on_one_connection() {
    let dir = TestDir::new("surfaces-reload");
    let main = checkout(&dir.0);
    let tmp = TestDir::new("surfaces-reload-tmp");
    let data = main.join("private/live/rust/data");
    // the watched build: a script that logs each start and runs the real server, rewritten as a cargo build rewrites dash
    let build = |name: &str, version: u32, server: &[&str]| {
        let path = dir.0.join(format!("{name}.sh"));
        let log = dir.0.join(format!("{name}.starts"));
        let text = format!(
            "#!/bin/sh\necho {version} >> '{}'\nexec '{DASH}' {}\n",
            log.display(),
            server.join(" ")
        );
        std::fs::write(&path, text).expect("the build");
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        (path, log)
    };
    let data_word = data.to_str().expect("utf-8");
    let main_word = main.to_str().expect("utf-8");
    let cases: [(&str, Vec<&str>, &str, Value); 2] = [
        (
            "scratch",
            vec!["mcp", "--scratch", "--data", data_word],
            "dash",
            serde_json::json!({"args": ["extensions", "--data", "scratch"]}),
        ),
        (
            "main",
            vec!["mcp", "--main", main_word],
            "task_show",
            serde_json::json!({"id": "t1"}),
        ),
    ];
    for (name, server, tool, arguments) in cases {
        let (path, log) = build(name, 1, &server);
        let cache = dir.0.join(format!("{name}.json"));
        let (p, c) = (
            path.to_str().expect("utf-8"),
            cache.to_str().expect("utf-8"),
        );
        let mut client = Client::start(
            &["mcp-proxy", "--cache", c, "--watch", p, "--", "sh", p],
            &tmp.0,
        );
        client.ask("initialize", serde_json::json!({}));
        let first = client.call(tool, arguments.clone());
        assert_eq!(first["result"]["isError"], false, "{name}: {first}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        build(name, 2, &server);
        let second = client.call(tool, arguments.clone());
        assert_eq!(second["result"]["isError"], false, "{name}: {second}");
        let starts = std::fs::read_to_string(&log).expect("its starts");
        assert_eq!(
            starts, "1\n2\n",
            "{name}: the second call ran on the new build"
        );
        let changed = client
            .notes
            .iter()
            .filter(|n| n["method"] == "notifications/tools/list_changed")
            .count();
        assert_eq!(changed, 1, "{name}: {:?}", client.notes);
        assert!(client.end().success());
        assert!(
            copies(&tmp.0).is_empty(),
            "{name}: a copy outlived its server"
        );
    }
    let peak = children_peak();
    eprintln!(
        "peak resident size of the proxies and their servers: {} MB",
        peak / 1_048_576
    );
    assert!(peak < 64 * 1_048_576, "{peak} bytes");
}
