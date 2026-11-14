//! What waits on him read from the older app's file while a writer holds it (task rust-handoff-port): its own file, as
//! dash-gate keeps the store's source free of `Instant`.
use dash_meter::{TestDir, dash_test};
use dash_store::Connection;
use dash_store::older_handoff::{Places, report, today};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The older app's tables, as much of them as the report reads.
const OLDER: &str = "
  PRAGMA journal_mode = WAL;
  CREATE TABLE collections (name TEXT PRIMARY KEY, kind TEXT NOT NULL, rows_at TEXT DEFAULT '[]' NOT NULL, shell TEXT);
  CREATE TABLE handoff_steps (collection TEXT, pos INTEGER, key TEXT, id TEXT, name TEXT, stage TEXT, data TEXT NOT NULL,
    body TEXT, types TEXT, PRIMARY KEY (collection, pos));
  CREATE TABLE drafts (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT,
    \"to\" TEXT, created TEXT);
  CREATE TABLE leads (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT);
  CREATE TABLE blocks (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT,
    short TEXT, short_en TEXT);
  CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, owner TEXT, data TEXT);
  CREATE TABLE task_events (id INTEGER PRIMARY KEY AUTOINCREMENT, task TEXT NOT NULL, status TEXT NOT NULL, at TEXT, data TEXT);
  CREATE TABLE records (collection TEXT, pos INTEGER, key TEXT, id TEXT, data TEXT);
  CREATE TABLE comments (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, file TEXT, text TEXT,
    date TEXT, quote TEXT);
  CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, data TEXT, body TEXT);
  CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
  CREATE TABLE contacts (id TEXT, name TEXT);
  CREATE TABLE companies (id TEXT, name TEXT);";

/// handoff.md's shell and three steps: one its draft's send closed, one his now, one not before 2027; a pending draft with
/// new wording and the new block it uses, one sent, a comment of his waiting on Claude.
fn rows(today: &str) -> String {
    let shell =
        serde_json::to_string("# Passation\n\n## À toi\n\u{E002}\n\u{E002}\n\u{E002}\n\u{E002}\n")
            .expect("a shell");
    format!(
        "INSERT INTO collections VALUES ('handoff.md', 'handoff', '[]', '{shell}');
  INSERT INTO handoff_steps VALUES ('handoff.md', 0, NULL, 'envoie', NULL, 'open', 'null',
    '- [⟦stage⟧] **Envoie la relance.** [le brouillon](/d/sent-one)' || char(10) ||
    '  {{\"task\":\"t1\",\"check\":{{\"draft\":\"sent-one\"}},\"judged\":\"{today}\"}}', NULL);
  INSERT INTO handoff_steps VALUES ('handoff.md', 1, NULL, 'signe', NULL, 'open', 'null',
    '- [⟦stage⟧] **Signe le contrat.** [Le PDF](https://x.io/c.pdf)' || char(10) ||
    '  {{\"task\":\"t1\",\"until\":\"2099-01-01\",\"judged\":\"{today}\"}}', NULL);
  INSERT INTO handoff_steps VALUES ('handoff.md', 2, NULL, 'cree', NULL, 'open', 'null',
    '- [⟦stage⟧] **Crée le certificat.** [Le portail](https://apple.com)' || char(10) ||
    '  {{\"task\":\"t1\",\"from\":\"2099-01-04\",\"judged\":\"{today}\"}}', NULL);
  INSERT INTO handoff_steps VALUES ('handoff.md', 3, NULL, 'relis', NULL, 'open', 'null',
    '- [⟦stage⟧] **Relis la PR.** [la PR](https://github.com/x/pull/1)' || char(10) ||
    '  {{\"task\":\"t2\",\"until\":\"2099-01-01\",\"judged\":\"{today}\"}}', NULL);
  INSERT INTO drafts VALUES ('drafts', 0, 'new-one.md', 'new-one', 'pending',
    '{{\"id\":\"⟦col⟧\",\"to\":\"⟦col⟧\",\"created\":\"⟦col⟧\",\"status\":\"⟦stage⟧\",\"channel\":\"linkedin\",\"new-parts\":[\"fresh\"]}}',
    'Bonjour, voici une phrase toute neuve.', 'Ana (Acme)', '2026-11-01');
  INSERT INTO drafts VALUES ('drafts', 1, 'sent-one.md', 'sent-one', 'sent',
    '{{\"id\":\"⟦col⟧\",\"status\":\"⟦stage⟧\",\"created\":\"⟦col⟧\"}}', 'Relance.', NULL, '2026-10-30');
  INSERT INTO blocks VALUES ('blocks.edn', 0, NULL, 'fresh', 'new', '{{}}', NULL, 'Une phrase toute neuve.', NULL);
  INSERT INTO tasks VALUES ('tasks.edn', 0, 't1', 'open', 'nicolas', '{{}}');
  INSERT INTO tasks VALUES ('tasks.edn', 1, 't2', 'done', 'nicolas', '{{\"review\":true}}');
  INSERT INTO task_events (task, status, at) VALUES ('t2', 'done', '2026-11-13');
  INSERT INTO comments VALUES ('comments', 0, 'c1', 'open', '{{\"replies\":[]}}', 'private/docs/x.html', 'Pourquoi ?',
    '2026-11-12', 'la phrase');"
    )
}

fn older(dir: &TestDir, today: &str) -> PathBuf {
    let db = dir.0.join("dashboard_dev.db");
    let conn = Connection::open(&db).expect("db");
    conn.execute_batch(OLDER).expect("tables");
    conn.execute_batch(&dash_store::older::marked(&rows(today)))
        .expect("rows");
    db
}

fn places(dir: &TestDir, db: PathBuf) -> Places {
    Places {
        db,
        root: dir.0.clone(),
        repos: dir.0.join("repos"),
        home: dir.0.join("home"),
        runners: true,
    }
}

fn stage(db: &Path, pos: i64) -> (String, String) {
    let conn = Connection::open(db).expect("db");
    conn.query_row(
        "SELECT stage, body FROM handoff_steps WHERE pos = ?1",
        [pos],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .expect("the step")
}

/// Task rust-handoff-port, done-when 1: `dash handoff` (the text) and /handoff (the parts) render from Rust's read of the
/// store in under a second while another connection holds the store's write lock; the step a check closed stays off his
/// list, its tick left for a run with the lock free, which then ticks it with its evidence under it.
#[dash_test]
fn a_writer_holding_the_store_never_holds_the_handoff() {
    let dir = TestDir::new("handoff-writer");
    let today = today();
    let db = older(&dir, &today);
    let p = places(&dir, db.clone());
    let writer = Connection::open(&db).expect("a writer");
    writer
        .execute_batch("BEGIN IMMEDIATE; UPDATE tasks SET stage = 'open';")
        .expect("the write lock");
    let began = std::time::Instant::now();
    let r = report(&p, &today, true).expect("a report");
    let took = began.elapsed();
    assert!(
        took < Duration::from_secs(1),
        "the report took {took:?} behind a writer"
    );
    for words in [
        "4 things wait on Nicolas. Page: http://100.72.158.115:7788/handoff.",
        // t2 is done but its PR waits for his review: its step stays his (Tasks.in_review?)
        "## His three steps (À toi) (2)\n- Signe le contrat. [Le PDF](https://x.io/c.pdf)\n- Relis la PR.",
        "## Later (Plus tard) (1)\n- Crée le certificat. [Le portail](https://apple.com)  (from 2099-01-04)",
        "## Drafts to review (1)",
        "- new-one -> Ana (Acme)",
        "## Blocks to validate once (1)\n",
        "- fresh: Une phrase toute neuve.",
        "## Comments waiting on Claude (1): answer in the document, then resolve\n- private/docs/x.html: « la phrase » -> Pourquoi ?  [c1]",
        "WARNING: 1 met steps stay unticked (a writer held the store, or it is read-only here)",
    ] {
        assert!(r.text.contains(words), "« {words} » missing:\n{}", r.text);
    }
    assert!(
        !r.text.contains("Envoie la relance"),
        "a met step is never his:\n{}",
        r.text
    );
    assert_eq!((r.exit, r.unticked), (0, 1), "{}", r.text);
    assert_eq!(
        r.parts["count"], 4,
        "his two steps, the draft, its new block: {}",
        r.parts
    );
    assert_eq!(
        r.parts["drafts"][0]["href"], "/drafts/new-one",
        "{}",
        r.parts
    );
    assert_eq!(
        r.parts["tasks"][0]["title"], "Signe le contrat. Le PDF",
        "{}",
        r.parts
    );
    assert_eq!(stage(&db, 0).0, "open", "the writer kept the tick out");
    writer.execute_batch("COMMIT").expect("the lock let go");
    let r = report(&p, &today, true).expect("a report");
    assert_eq!(r.unticked, 0, "{}", r.text);
    let (stage, body) = stage(&db, 0);
    assert_eq!(stage, "done");
    assert!(
        body.ends_with(&format!(
            "\n  **Fait {today} :** le brouillon sent-one est sent"
        )),
        "{body}"
    );
}
