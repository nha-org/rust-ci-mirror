//! `dash handoff` as sessions run it (task rust-handoff-port): bin/dash hands the words to this binary, which reads the older
//! app's file itself; `Dashboard.Handoff.report`, the older app's CLI clause, is never reached, and no older app need run.
use dash_meter::{TestDir, dash_test};
use std::process::Command;

/// The older app's tables the report reads, one open step and its shell.
const OLDER: &str = "
  CREATE TABLE collections (name TEXT PRIMARY KEY, kind TEXT NOT NULL, rows_at TEXT DEFAULT '[]' NOT NULL, shell TEXT);
  CREATE TABLE handoff_steps (collection TEXT, pos INTEGER, key TEXT, id TEXT, name TEXT, stage TEXT, data TEXT NOT NULL,
    body TEXT, types TEXT);
  CREATE TABLE drafts (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT);
  CREATE TABLE leads (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT);
  CREATE TABLE blocks (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT,
    short TEXT, short_en TEXT);
  CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, owner TEXT, data TEXT);
  CREATE TABLE task_events (id INTEGER PRIMARY KEY, task TEXT NOT NULL, status TEXT NOT NULL, at TEXT, data TEXT);
  CREATE TABLE records (collection TEXT, pos INTEGER, key TEXT, id TEXT, data TEXT);
  CREATE TABLE comments (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, file TEXT, text TEXT,
    date TEXT, quote TEXT);
  CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, data TEXT, body TEXT);
  CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
  CREATE TABLE contacts (id TEXT, name TEXT);
  CREATE TABLE companies (id TEXT, name TEXT);
  INSERT INTO collections VALUES ('handoff.md', 'handoff', '[]', '\"# Passation\\n\\n## À toi\\n\u{E002}\\n\"');
  INSERT INTO handoff_steps VALUES ('handoff.md', 0, NULL, 'signe', NULL, 'open', 'null',
    '- [⟦stage⟧] **Signe le contrat.** [Le PDF](https://x.io/c.pdf)' || char(10) ||
    '  {\"task\":\"t1\",\"until\":\"2099-01-01\",\"judged\":\"2099-01-01\"}', NULL);
  INSERT INTO tasks VALUES ('tasks.edn', 0, 't1', 'open', 'nicolas', '{}');";

/// Task rust-handoff-port, done-when 3: `dash handoff` answers from this binary with no older app reachable (its CLI and its
/// URL both dead), so `Dashboard.Handoff.report` is not what answers; bin/dash routes `dash handoff` and `--db` here and
/// only there, `count`, `rejudge` and `pages` staying the older app's.
#[dash_test]
fn dash_handoff_answers_from_rust_and_bin_dash_routes_it_here() {
    let dir = TestDir::new("handoff-cli");
    let db = dir.0.join("private/dashboard/dashboard_dev.db");
    std::fs::create_dir_all(db.parent().expect("its folder")).expect("folders");
    let conn = dash_store::Connection::open(&db).expect("db");
    conn.execute_batch(&dash_store::older::marked(OLDER))
        .expect("tables");
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_dash"))
            .arg("handoff")
            .args(args)
            .env("DASH_REPO", &dir.0)
            .env("HOME", &dir.0)
            // no older app: a call to it would fail, never answer these words
            .env("DASH_CLI", dir.0.join("no-older-app"))
            .env("DASHBOARD_URL", "http://127.0.0.1:9")
            .output()
            .expect("dash handoff");
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        (out.status.code(), text(&out.stdout), text(&out.stderr))
    };
    let (code, out, err) = run(&[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.starts_with("1 things wait on Nicolas. Page: http://100.72.158.115:7788/handoff."),
        "{out}"
    );
    assert!(
        out.contains(
            "## His three steps (À toi) (1)\n- Signe le contrat. [Le PDF](https://x.io/c.pdf)"
        ),
        "{out}"
    );
    let (code, _, err) = run(&["--db", db.to_str().expect("utf-8")]);
    assert_eq!(code, Some(0), "{err}");
    // the usage bin/dash greps the binary for, printed by this binary on other words
    let (code, _, err) = run(&["count"]);
    assert_eq!(code, Some(2));
    assert!(err.contains("dash handoff [--db"), "{err}");

    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let wrapper =
        std::fs::read_to_string(top.join("private/dashboard/bin/dash")).expect("bin/dash");
    assert!(
        wrapper.contains("\"handoff \"|\"handoff --db\") known=\"dash handoff [--db\""),
        "bin/dash does not route dash handoff to Rust"
    );
}
