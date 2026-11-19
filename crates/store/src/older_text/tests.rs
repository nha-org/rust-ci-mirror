//! The store text reader, held to the older app's bytes on a planted file: each kind of collection, its columns, types and
//! markers, as `Dashboard.Store.text/2` and `files/1` print them (the live parity run in PR of task rust-forwarded-next).
use super::*;
use crate::older::marked;
use dash_meter::dash_test;

const COL_: &str = "⟦col⟧";

/// A file of the older app's schema, the tables these collections live in, with one collection of each kind.
fn planted() -> Connection {
    let db = Connection::open_in_memory().expect("a file in memory");
    let step = encode(&Json::Str(format!("# Passation\n\n{STEP}\n\nend\n")), false);
    let sql = marked(&format!(
        r#"
        CREATE TABLE collections (name TEXT, kind TEXT, rows_at TEXT, shell TEXT);
        CREATE TABLE records (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, types TEXT,
          body TEXT, title TEXT);
        CREATE TABLE tasks (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, types TEXT,
          body TEXT, title TEXT, priority INTEGER, review INTEGER);
        CREATE TABLE leads (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, types TEXT,
          body TEXT, company TEXT, from_france_ok INTEGER);
        CREATE TABLE drafts (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, types TEXT,
          body TEXT, cold INTEGER);
        CREATE TABLE handoff_steps (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL,
          types TEXT, body TEXT);
        CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, at TEXT, data TEXT, body TEXT);
        CREATE TABLE links ("from" TEXT, kind TEXT, "to" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
        CREATE TABLE companies (id TEXT, name TEXT);
        CREATE TABLE contacts (id TEXT, name TEXT);
        INSERT INTO companies VALUES ('acme', 'Acme');
        INSERT INTO contacts VALUES ('sam', 'Sam Doe');
        INSERT INTO collections VALUES ('profile.edn', 'file', '[]', '{{}}'), ('glossary.edn', 'file', '[]', '{{"~#map":[]}}'),
          ('tasks.edn', 'file', '["tasks"]', '{{"ordered":"2026-10-08","tasks":[]}}'), ('opportunities', 'dir', '[]', 'null'),
          ('drafts', 'drafts', '[]', 'null'), ('handoff.md', 'handoff', '[]', '{step}'),
          ('outbound.jsonl', 'jsonl', '[]', 'null'), ('postings', 'notes', '[]', 'null');
        INSERT INTO records (collection, pos, key, data, types) VALUES
          ('profile.edn', 0, 'rate', '{{"day":650,"unit":"eur","ratio":1.5e-5}}', '{{"key":"k","data":{{"o":{{"unit":"k"}}}}}}'),
          ('profile.edn', 1, 'name', '"~tilde"', '{{"key":"k"}}'),
          ('glossary.edn', 0, 'term', '"a word"', NULL);
        INSERT INTO tasks VALUES ('tasks.edn', 0, NULL, 'a', 'open',
          '{{"id":"{COL_}","title":"{COL_}","status":"⟦stage⟧","priority":"{COL_}","review":"{COL_}","labels":["x"],"blocked-by":"⟦@waits-on/0⟧ and ⟦m:companies:acme⟧"}}',
          '{{"data":{{"o":{{"status":"k","labels":{{"set":null}}}}}}}}', NULL, 'Alpha', 2, 1);
        INSERT INTO links VALUES ('tasks:a', 'waits-on', 'tasks:b', 0);
        INSERT INTO leads VALUES ('opportunities', 0, 'acme-ai.edn', 'acme-ai', NULL,
          '{{"id":"{COL_}","company":"{COL_}","timeline":"⟦events⟧","fit":0.92}}', NULL, NULL, '⟦m:companies:acme⟧', NULL);
        INSERT INTO events VALUES ('timeline:acme-ai', 0, NULL, 'leads:acme-ai', NULL, '{{"date":"2026-11-01"}}', NULL);
        INSERT INTO drafts VALUES ('drafts', 0, 'x.md', 'x', NULL, '{{"to":"Sam","cold":"{COL_}"}}', NULL,
          'Hello ⟦m:contacts:sam⟧' || char(10), 0);
        INSERT INTO handoff_steps VALUES ('handoff.md', 0, NULL, 's1', 'open', 'null', NULL,
          '- [⟦stage⟧] **Do it**' || char(10) || '  dash x');
        INSERT INTO events VALUES ('outbound.jsonl', 0, NULL, NULL, NULL, '{{"a":1}}', '{{"a": 1}}'),
          ('outbound.jsonl', 1, NULL, NULL, NULL, '{{"n":1000.0,"s":"tab\there","c":"\u001b","e":"\u00e9"}}', NULL),
          ('postings', 0, 'p.md', NULL, NULL, NULL, '# Ad' || char(10)), ('postings', 1, 'a.md', NULL, NULL, NULL, '# A');
        "#
    ));
    db.execute_batch(&sql).expect("the planted file");
    db
}

/// Done-when 1: each kind of collection prints the older app's bytes (a keyword map, a tagged map, tasks under their key,
/// a lead with its timeline, a draft with its message, handoff.md's steps, a journal, a note); its columns read back, its
/// tags put back and its markers replaced.
#[dash_test]
fn store_text_prints_each_kind_as_the_older_app_did() {
    let db = planted();
    let text = |name: &str, key: Option<&str>| super::text(&db, name, key).expect(name);
    assert_eq!(
        text("profile.edn", None),
        "{\n  \"rate\": {\n    \"day\": 650,\n    \"unit\": \"~:eur\",\n    \"ratio\": 1.5e-5\n  },\n  \"name\": \"~~tilde\"\n}\n"
    );
    assert_eq!(
        text("glossary.edn", None),
        "{\n  \"~#map\": [\n    [\n      \"term\",\n      \"a word\"\n    ]\n  ]\n}\n"
    );
    assert_eq!(
        text("tasks.edn", None),
        "{\n  \"ordered\": \"2026-10-08\",\n  \"tasks\": [\n    {\n      \"id\": \"a\",\n      \"title\": \"Alpha\",\n      \
         \"status\": \"~:open\",\n      \"priority\": 2,\n      \"review\": true,\n      \"labels\": {\n        \"~#set\": [\n          \
         \"x\"\n        ]\n      },\n      \"blocked-by\": \"b and Acme\"\n    }\n  ]\n}\n"
    );
    assert_eq!(
        text("opportunities", Some("acme-ai.edn")),
        "{\n  \"id\": \"acme-ai\",\n  \"company\": \"Acme\",\n  \"timeline\": [\n    {\n      \"date\": \"2026-11-01\"\n    }\n  ],\n  \
         \"fit\": 0.92\n}\n"
    );
    assert_eq!(
        text("drafts", None),
        "{\n  \"to\": \"Sam\",\n  \"cold\": false\n}\n---\nHello Sam Doe\n"
    );
    assert_eq!(
        text("handoff.md", None),
        "# Passation\n\n- [ ] **Do it**\n  dash x\n\nend\n"
    );
    assert_eq!(
        text("outbound.jsonl", None),
        "{\"a\": 1}\n{\"n\":1.0e3,\"s\":\"tab\\there\",\"c\":\"\\u001B\",\"e\":\"é\"}\n"
    );
    assert_eq!(text("postings", Some("p.md")), "# Ad\n");
    assert_eq!(text("nothing", None), "");
}

/// Done-when 1: a folder's files by name, sorted, a known folder with no rows `{}`, a one-file source none; the command's
/// refusals and usages as `Dashboard.CLI` gave them.
#[dash_test]
fn store_files_and_the_command_answer_as_the_older_app_did() {
    let db = planted();
    let files = super::files(&db, "postings")
        .expect("postings")
        .expect("a folder");
    assert_eq!(files.keys().collect::<Vec<_>>(), ["a.md", "p.md"]);
    assert_eq!(
        super::files(&db, "mail").expect("mail"),
        Some(BTreeMap::new())
    );
    assert_eq!(super::files(&db, "tasks.edn").expect("tasks"), None);
    assert_eq!(super::files(&db, "nothing").expect("nothing"), None);

    let dir = std::env::temp_dir().join(format!("older-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let file = dir.join("dashboard_dev.db");
    let _ = std::fs::remove_file(&file);
    db.execute("VACUUM INTO ?1", [file.to_string_lossy()])
        .expect("the file");
    let ask = |w: &[&str]| {
        answer(&file, &w.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .expect("an answer")
            .expect("Rust's")
    };
    let (o, e) = (String::new(), String::new());
    assert_eq!(
        ask(&["store", "files", "postings"]),
        (
            0,
            "{\n  \"a.md\": \"# A\",\n  \"p.md\": \"# Ad\\n\"\n}\n".into(),
            e.clone()
        )
    );
    assert_eq!(
        ask(&["store", "text", "postings/p.md"]),
        (0, "# Ad\n".into(), e.clone())
    );
    assert_eq!(
        ask(&["store", "text", "postings", "p.md"]),
        (0, "# Ad\n".into(), e.clone())
    );
    assert_eq!(
        ask(&["store", "text", "nothing"]),
        (
            1,
            o.clone(),
            "store text: the model holds no nothing\n".into()
        )
    );
    assert_eq!(
        ask(&["store", "text", "drafts", "y.md"]),
        (
            1,
            o.clone(),
            "store text: the model holds no drafts y.md\n".into()
        )
    );
    assert_eq!(
        ask(&["store", "files", "tasks.edn"]),
        (
            1,
            o.clone(),
            "store files: tasks.edn is no folder of the store\n".into()
        )
    );
    assert_eq!(
        ask(&["store", "text"]),
        (
            2,
            o.clone(),
            "usage: dash store text <path> | <folder> <file>\n".into()
        )
    );
    assert_eq!(
        ask(&["store", "files", "a", "b"]),
        (2, o, "usage: dash store files <folder>\n".into())
    );
    // a task show of other words is the older app's, with no reader failure logged
    let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(answer(&file, &words(&["task", "show"])), Ok(None));
    assert_eq!(
        answer(&file, &words(&["task", "show", "a", "--brief"])),
        Ok(None)
    );
    let _ = std::fs::remove_file(&file);
}

/// Done-when 1: a folder past 32 files prints its files sorted by name (his answer « Sorted is fine (Recommended) »),
/// where the older app's map past 32 keys came out in the BEAM's hash order.
#[dash_test]
fn a_folder_past_32_files_prints_them_sorted_by_name() {
    let db = planted();
    for i in 0..40 {
        db.execute(
            "INSERT INTO events (source, pos, key, body) VALUES ('postings', ?1, ?2, 'x')",
            rusqlite::params![i + 2, format!("f{:02}.md", 39 - i)],
        )
        .expect("a row");
    }
    let files = super::files(&db, "postings")
        .expect("postings")
        .expect("a folder");
    let names: Vec<_> = files.keys().cloned().collect();
    assert_eq!(names.len(), 42);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert_eq!((names[0].as_str(), names[41].as_str()), ("a.md", "p.md"));
}

/// `Store.locate/1`: a source's name, a folder's entry by its extension (never its template), a top-level data file, a
/// path read through « .. »; anything else is the path itself.
#[dash_test]
fn a_path_locates_its_source_as_the_older_app_did() {
    let at = |p: &str| locate(p);
    let some = |n: &str, f: &str| (n.to_string(), Some(f.to_string()));
    let none = |n: &str| (n.to_string(), None);
    assert_eq!(at("handoff.md"), none("handoff.md"));
    assert_eq!(at("live/comments.edn"), none("live/comments.edn"));
    assert_eq!(at("drafts/x.md"), some("drafts", "x.md"));
    assert_eq!(at("interview/calls/x.md"), some("interview/calls", "x.md"));
    assert_eq!(at("opportunities/x.edn"), some("opportunities", "x.edn"));
    assert_eq!(at("opportunities/x.md"), none("opportunities/x.md"));
    assert_eq!(at("drafts/_template.md"), none("drafts/_template.md"));
    assert_eq!(at("tasks.edn"), none("tasks.edn"));
    assert_eq!(at("drafts/../tasks.edn"), none("tasks.edn"));
    assert_eq!(at("mail-cursor.edn"), none("mail-cursor.edn"));
    // a known source reached through « . » or « .. » is that source (/code-review of PR 1604)
    assert_eq!(at("./handoff.md"), none("handoff.md"));
    assert_eq!(at("drafts/../outbound.jsonl"), none("outbound.jsonl"));
    assert_eq!(
        at("./live/task-journal.jsonl"),
        none("live/task-journal.jsonl")
    );
}

/// A step row no marker of handoff.md's shell places is an error, never a step dropped: the older app then answers, as its
/// `handoff_join` failed on leftovers (/code-review of PR 1604).
#[dash_test]
fn a_handoff_step_with_no_marker_is_an_error() {
    let db = planted();
    db.execute_batch(
        "INSERT INTO handoff_steps VALUES ('handoff.md', 1, NULL, 's2', 'open', 'null', NULL, '- [ ] **Two**');",
    )
    .expect("a second step");
    assert!(super::text(&db, "handoff.md", None).is_err());
}

/// Erlang's `float_to_binary(f, [:short])`, read from the older app's toolchain on 2026-11-14 for each of these.
#[dash_test]
fn floats_print_as_erlang_prints_them() {
    let cases = [
        (1000.0, "1.0e3"),
        (100.0, "100.0"),
        (0.0001, "0.0001"),
        (1.0e-5, "1.0e-5"),
        (0.0012, "0.0012"),
        (0.000123, "1.23e-4"),
        (12340.0, "12340.0"),
        (123400.0, "1.234e5"),
        (2.5e9, "2.5e9"),
        (1234567890123456.0, "1234567890123456.0"),
        (12345678901234567.0, "1.2345678901234568e16"),
        (5.0e-324, "5.0e-324"),
        (0.0019488, "0.0019488"),
        (-0.0, "-0.0"),
        (-1.5, "-1.5"),
    ];
    for (f, want) in cases {
        assert_eq!(erlang_float(f), want, "{f}");
    }
}

/// JSON read back as it was written: order, escapes, a surrogate pair, a big integer.
#[dash_test]
fn json_reads_and_prints_back_in_its_order() {
    let s = r#"{"z":1,"a":[true,null,"\ud83d\ude00\/"],"n":123456789012345678901234567890,"m":-0}"#;
    assert_eq!(
        encode(&parse(s).expect("JSON"), false),
        "{\"z\":1,\"a\":[true,null,\"😀/\"],\"n\":123456789012345678901234567890,\"m\":0}"
    );
    assert!(parse("{\"a\":1} x").is_err() && parse("[1,").is_err());
    assert!(parse("-").is_err() && parse("[--1]").is_err());
}
