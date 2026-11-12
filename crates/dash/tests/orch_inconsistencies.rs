//! `dash orch --inconsistencies` as the orchestrator's prompt hook runs it (task rust-held-by-nobody, done-when 3): bin/dash's
//! post to the real `dash serve`, the older app's last view in its database, and a Rust run's claim on the real store. The
//! claimed PR's « held by nobody » line is dropped, the others kept, and the older app is never asked.
use dash_core::tasks::TaskStore;
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

const DASH: &str = env!("CARGO_BIN_EXE_dash");

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn nobody(n: u64, branch: &str) -> String {
    format!(
        "PR https://github.com/nha-org/site_perso/pull/{n} (site_perso, branch {branch}) is held by nobody: no task, reviewer, lane or merge on it"
    )
}

#[test]
fn the_prompt_hook_s_inconsistencies_come_from_rust_and_drop_a_claimed_pr() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("orch-inconsistencies");
    let claimed = nobody(1387, "task/rust-terminal-no-hc");
    let free = nobody(1449, "task/rust-window-compass");
    let no_pane = "task t has no pane: its claim's pane 9 is gone (last status none)";
    // the older app's last view, as Dashboard.Orch.View records it
    let db = dir.0.join("private/dashboard");
    std::fs::create_dir_all(&db).expect("its folder");
    let conn = dash_store::Connection::open(db.join("dashboard_dev.db")).expect("its db");
    let view = serde_json::json!({ "inconsistencies": [claimed, free, no_pane] }).to_string();
    conn.execute_batch(
        "CREATE TABLE orch_views (day TEXT PRIMARY KEY, at TEXT NOT NULL, view TEXT NOT NULL, count INTEGER NOT NULL)",
    )
    .expect("its table");
    conn.execute(
        "INSERT INTO orch_views VALUES ('2026-11-11', '2026-11-11T19:32:51.436221', ?1, 3)",
        [&view],
    )
    .expect("its view");
    drop(conn);
    let mut tasks = dash_store::tasks::open(&dir.0, &dir.0).expect("store");
    tasks
        .claim("rust-terminal-no-hc")
        .expect("a Rust run's claim");
    drop(tasks);

    let free_port = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = free_port.local_addr().expect("address").port();
    drop(free_port);
    let mut serve = Command::new(DASH);
    serve
        .args(["serve", "--data"])
        .arg(&dir.0)
        .args(["--port", &port.to_string()]);
    serve.env("HOME", &dir.0).env("DASH_WORKTREES", &dir.0);
    serve.env("DASH_REPO", &dir.0);
    let mut serve = Killed(serve.stdout(Stdio::piped()).spawn().expect("serve"));
    let mut line = String::new();
    let out = serve.0.stdout.take().expect("stdout");
    BufReader::new(out).read_line(&mut line).expect("port line");

    let answer = Command::new("curl")
        .args(["-sS", "-D", "-", "-H", "x-dashboard-cli: 1"])
        .args(["--data-urlencode", "cwd=/tmp"])
        .args(["--data-urlencode", "args[]=orch"])
        .args(["--data-urlencode", "args[]=--inconsistencies"])
        .arg(format!("http://127.0.0.1:{port}/dashboard/cli"))
        .output()
        .expect("curl");
    let answer = String::from_utf8_lossy(&answer.stdout);
    assert!(answer.contains("x-exit: 0\r\n"), "{answer}");
    let body = answer.split_once("\r\n\r\n").expect("a body").1;
    assert!(
        body.starts_with("Inconsistencies (2, as read 2026-11-11 19:32Z, "),
        "{body}"
    );
    assert!(body.contains(&format!("  {free}\n")), "{body}");
    assert!(body.contains(&format!("  {no_pane}\n")), "{body}");
    assert!(!body.contains("pull/1387"), "a claimed PR is held: {body}");
    println!("{body}");
}
