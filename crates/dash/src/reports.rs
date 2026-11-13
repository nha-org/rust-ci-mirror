//! `dash task report <id> <file>`'s proof check, in Rust before the older app files the record (`Dashboard.Orch.Reports`
//! keeps the store, the Knowledge gate and its own shape checks): each done-when part a done report claims quotes its own proof (task
//! report-proof-quotes; @nha, 2026-10-26: « It needs to be convincing. »; 4 PM rounds lost on 2026-11-10 to reports that
//! named a test per part and quoted no assertion).

//! The pure check is `dash_launcher::report`; this module reads the record and the stores it judges against.

use dash_launcher::report::{Moves, problems, refusal, shape, text_of};
pub use dash_launcher::report::{from_merged, parts};
use serde_json::Value;

/// `dash task report <id> <src> [--read|--unread]` from bin/dash, read as the older app reads it (`Orch.CLI`): `src` is the
/// record itself when it opens with « { », else a file from `cwd`, « ~ » his home. The refusal when the report's proofs fall
/// short, its shape problems first, else none and the older app files it. A source it cannot read or parse, or a task with
/// no row, goes on to the older app, which names that problem itself.
/// `adds`: the adds Rust's task store holds whose form opens with a prefix, `SqliteTasks::adds_from` (task move-names-task).
pub fn bounced(
    args: &[String],
    cwd: &str,
    db: &std::path::Path,
    adds: &dyn Fn(&str) -> Result<Vec<dash_store::tasks::Copy>, String>,
) -> Option<String> {
    let (id, record) = record(args, cwd)?;
    let filed = dash_store::older::brief(db, &id).ok()??;
    // the folders and the store are read only for an outcome that moves a part (/code-review)
    let waiver = dash_launcher::brief::waiver();
    let moving = waiver.says(&text_of(&record["outcome"]), waiver.verbs[1]);
    let root = worktree(cwd).filter(|_| moving);
    let copies = root
        .as_ref()
        .map_or(Ok(vec![]), |root| adds(&crate::pages::form_of(root, &[])));
    // a store that does not read adds none: a move then names its task's id
    let added = root.map_or(vec![], |root| {
        session_adds(&copies.unwrap_or_default(), &root)
    });
    let known = |w: &str| dash_store::older::brief(db, w).is_ok_and(|f| f.is_some());
    let moves = Moves {
        task: &id,
        known: &known,
        added: &added,
    };
    let proofs = problems(&filed.done_when, &record, &moves);
    if proofs.is_empty() {
        return None;
    }
    refusal(&[shape(&record), proofs].concat()).map(|why| format!("{id}: report refused, {why}"))
}

/// The worktree `cwd` sits in: the nearest folder holding `.git`, `cwd` itself included.
fn worktree(cwd: &str) -> Option<std::path::PathBuf> {
    let cwd = std::path::Path::new(cwd);
    (cwd.is_absolute())
        .then(|| cwd.ancestors().find(|a| a.join(".git").exists()))
        .flatten()
        .map(std::path::Path::to_path_buf)
}

/// The tasks the session in worktree `root` filed with dash task add, each its id and :why (its `--why`, or the text a « - »
/// read from stdin), from `copies`, the store's adds whose form opens with `root`: an add run in `root` or a folder inside it
/// (« root-other » is another worktree).
fn session_adds(
    copies: &[dash_store::tasks::Copy],
    root: &std::path::Path,
) -> Vec<(String, String)> {
    let root = root.display().to_string();
    copies
        .iter()
        .filter_map(|c| {
            let cwd = crate::pages::param(&c.form, "cwd").unwrap_or_default();
            let args = crate::pages::args_of(&c.form);
            let inside = cwd == root || cwd.starts_with(&format!("{root}/"));
            let at = args.iter().position(|a| a == "--why")?;
            let why = match args.get(at + 1)?.as_str() {
                "-" => crate::pages::param(&c.form, "stdin").unwrap_or_default(),
                why => why.to_string(),
            };
            inside.then(|| (c.task.clone(), why))
        })
        .collect()
}

/// `dash task report`'s task id and record, or none for any other command or a source that does not read.
fn record(args: &[String], cwd: &str) -> Option<(String, Value)> {
    let (id, src) = match args {
        [_, _, id, src] => (id, src),
        [_, _, id, src, read] if ["--read", "--unread"].contains(&read.as_str()) => (id, src),
        _ => return None,
    };
    if (args[0].as_str(), args[1].as_str()) != ("task", "report") {
        return None;
    }
    let text = match src.trim_start().starts_with('{') {
        true => src.clone(),
        false => std::fs::read_to_string(expanded(src, cwd)?).ok()?,
    };
    Some((id.clone(), serde_json::from_str(&text).ok()?))
}

/// The lesson a report's feedback line carries, as a row of the watcher's wake (the older app's `Attention` lesson, task
/// rust-watch-port): none when it says « none » or nothing.
pub fn lesson(args: &[String], cwd: &str, now_ms: i64) -> Option<dash_comments::watch::Item> {
    use dash_comments::watch::{Item, Kind};
    let (id, record) = record(args, cwd)?;
    let said = record["feedback"]
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let bare = said.trim_end_matches('.').to_lowercase();
    if bare.is_empty() || bare == "none" {
        return None;
    }
    let key = format!("lesson {id} {said}");
    let title = format!("task {id}'s feedback, a lesson to fold");
    Some(Item::new(
        &key,
        Kind::Lesson,
        Some(&id),
        &title,
        &said,
        now_ms,
    ))
}

/// `src` as `Path.expand(src, cwd)` reads it: « ~ » his home, an absolute path as it is, else under `cwd`; none for a relative
/// path with no folder to read it from.
fn expanded(src: &str, cwd: &str) -> Option<std::path::PathBuf> {
    let home = || std::env::var("HOME").ok().map(std::path::PathBuf::from);
    match src.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => Some(home()?.join(&rest[1..])),
        _ if std::path::Path::new(src).is_absolute() => Some(src.into()),
        _ if cwd.is_empty() => None,
        _ => Some(std::path::Path::new(cwd).join(src)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use serde_json::json;

    /// Done-when 2 of rust-watch-port: a report's feedback is a lesson row of the watcher, « none » none.
    #[dash_test]
    fn a_report_s_feedback_is_a_lesson_for_the_watcher() {
        let args = |record: &str| {
            ["task", "report", "rust-x", record]
                .map(String::from)
                .to_vec()
        };
        let said = r#"{"kind": "done", "feedback": "herdr's  read source cost two turns."}"#;
        let row = lesson(&args(said), "", 5).expect("a lesson");
        assert_eq!(row.kind, dash_comments::watch::Kind::Lesson);
        assert_eq!(row.task.as_deref(), Some("rust-x"));
        assert_eq!(row.body, "herdr's read source cost two turns.");
        assert_eq!(row.title, "task rust-x's feedback, a lesson to fold");
        for none in [
            r#"{"feedback": "none"}"#,
            r#"{"feedback": "None."}"#,
            r#"{"feedback": " "}"#,
            r#"{"kind": "done"}"#,
        ] {
            assert_eq!(lesson(&args(none), "", 5), None, "{none}");
        }
        let other = ["task", "show", "rust-x", said].map(String::from).to_vec();
        assert_eq!(lesson(&other, "", 5), None);
    }

    fn report(checks: Value, not_checked: &str, after_merge: &str) -> Value {
        json!({"kind": "done", "outcome": "met", "checks": checks, "not_checked": not_checked,
               "after_merge": after_merge, "next": "none"})
    }

    /// Done-when 1 of move-names-task: the session's adds are those its worktree or a folder inside it ran, each with its
    /// :why, a « - » read from stdin; another worktree's and a refused add are not.
    #[dash_test]
    fn the_session_s_adds_are_its_worktree_s_with_their_why() {
        let dir = dash_meter::TestDir::new("reports-session-adds");
        let root = dir.0.join("rust-x");
        std::fs::create_dir_all(root.join("rust")).expect("the worktree");
        std::fs::write(root.join(".git"), "gitdir: elsewhere").expect(".git");
        assert_eq!(
            worktree(&root.join("rust").display().to_string()),
            Some(root.clone())
        );
        let mut store = dash_store::tasks::open(&dir.0, &dir.0).expect("the store");
        let add = |cwd: &std::path::Path, id: &str, why: &str| {
            crate::pages::form_of(cwd, &["task", "add", id, "--why", why, "--title", "T"])
        };
        store
            .file_copy(
                "from-root",
                "add",
                &add(&root, "from-root", "Part 2 of rust-x"),
            )
            .expect("filed");
        let stdin = add(&root.join("rust"), "from-inside", "-") + "&stdin=Part%201%20of%20rust-x";
        store
            .file_copy("from-inside", "add", &stdin)
            .expect("filed");
        store
            .file_copy(
                "other",
                "add",
                &add(&dir.0.join("rust-x-other"), "other", "w"),
            )
            .expect("filed");
        let refused = store
            .file_copy("refused", "add", &add(&root, "refused", "w"))
            .expect("filed");
        store.copy_ended(&refused, "refused: a dup").expect("ended");
        let copies = store
            .adds_from(&crate::pages::form_of(&root, &[]))
            .expect("the adds");
        assert_eq!(
            session_adds(&copies, &root),
            vec![
                ("from-root".to_string(), "Part 2 of rust-x".to_string()),
                ("from-inside".to_string(), "Part 1 of rust-x".to_string()),
            ]
        );
    }

    /// Done-when 1 through the front door: `dash task report <id> <file>`, its file read from the caller's folder and its
    /// done-when from the older app's row, is refused naming the part; a quoting one, a review's or a garbled file goes on.
    #[dash_test]
    fn dash_task_report_bounces_from_the_file_and_the_task_s_row() {
        let dir = dash_meter::TestDir::new("report-bounce");
        let db = dir.0.join("older.db");
        let tables = "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT,
              why TEXT, done_when TEXT, epic TEXT, plan_first INTEGER);
            CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, title TEXT, data TEXT NOT NULL);
            CREATE TABLE comments (pos INTEGER, id TEXT, stage TEXT, task TEXT, file TEXT, quote TEXT, text TEXT,
              data TEXT NOT NULL);
            INSERT INTO tasks VALUES ('tasks.edn', 0, 't', 'open', '{}', 'T', 'w', '1. It bounces 2. It passes', NULL, NULL);";
        let store = dash_store::Connection::open(&db).expect("db");
        let tasks =
            std::cell::RefCell::new(dash_store::tasks::open(&dir.0, &dir.0).expect("the store"));
        let adds = |prefix: &str| tasks.borrow().adds_from(prefix);
        store.execute_batch(tables).expect("tables");
        let write = |name: &str, r: Value| {
            std::fs::write(dir.0.join(name), r.to_string()).expect("a report")
        };
        write(
            "bare.json",
            report(
                json!([{"command": "c", "part": "1", "output": "PASS t"}, {"command": "c", "part": "2", "output": "assert!(b)"}]),
                "none",
                "none",
            ),
        );
        write(
            "quoted.json",
            report(
                json!([{"command": "c", "part": "1", "output": "assert!(a)"}, {"command": "c", "part": "2", "output": "assert!(b)"}]),
                "none",
                "none",
            ),
        );
        std::fs::write(dir.0.join("garbled.json"), "{").expect("garbled");
        let cwd = dir.0.display().to_string();
        let asked = |id: &str, file: &str| {
            bounced(
                &["task", "report", id, file].map(String::from),
                &cwd,
                &db,
                &adds,
            )
        };
        let asked_in = |id: &str, file: &str, cwd: &str| {
            bounced(
                &["task", "report", id, file].map(String::from),
                cwd,
                &db,
                &adds,
            )
        };
        let why = asked("t", "bare.json").expect("bounced");
        assert!(
            why.starts_with("t: report refused, part 1 (« It bounces ») quotes no assertion or command output of its own"),
            "{why}"
        );
        assert_eq!(asked("t", "quoted.json"), None);
        assert_eq!(asked("review-t", "bare.json"), None);
        assert_eq!(asked("t", "garbled.json"), None);
        // the older app's other forms (/code-review of ccdf1033e): « --read », the record inline, an absolute path
        let read = ["task", "report", "t", "bare.json", "--read"].map(String::from);
        assert!(bounced(&read, &cwd, &db, &adds).is_some());
        let bare = std::fs::read_to_string(dir.0.join("bare.json")).expect("bare");
        assert!(
            bounced(
                &["task", "report", "t", &format!("  {bare}")].map(String::from),
                "",
                &db,
                &adds
            )
            .is_some()
        );
        let absolute = dir.0.join("bare.json").display().to_string();
        assert!(
            bounced(
                &["task", "report", "t", &absolute].map(String::from),
                "",
                &db,
                &adds
            )
            .is_some()
        );
        assert_eq!(asked_in("t", "bare.json", ""), None);
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(
            expanded("~/r.json", "/w"),
            Some(std::path::Path::new(&home).join("r.json"))
        );
        // its shape problems come in the same refusal, never a second round
        let shapeless = json!({"kind": "done", "outcome": "met", "prs": "1414", "not_checked": "none", "next": "none",
                               "checks": [{"command": "c", "part": "1", "output": "PASS t"}, {"part": "2"}]});
        std::fs::write(dir.0.join("shapeless.json"), shapeless.to_string()).expect("shapeless");
        let why = asked("t", "shapeless.json").expect("bounced");
        assert!(
            why.starts_with("t: report refused, 4 problems: 1. prs is a list of strings"),
            "{why}"
        );
        assert!(
            why.contains("2. each check needs its command and its output"),
            "{why}"
        );
        assert_eq!(
            bounced(&["task", "show", "t"].map(String::from), &cwd, &db, &adds),
            None
        );
        // task move-names-task, done-when 1: part 2 moved by the dialog to a task this worktree's session filed, no id named
        let moved = json!({"kind": "done", "outcome": "met; part 2 moved to a new task by his answer « Move it »",
                           "after_merge": "none", "not_checked": "none",
                           "checks": [{"command": "c", "part": "1", "output": "assert!(a)"}]});
        std::fs::write(dir.0.join("moved.json"), moved.to_string()).expect("moved");
        let why = asked("t", "moved.json").expect("no session, no task filed");
        assert!(why.contains("is called moved to no task"), "{why}");
        std::fs::write(dir.0.join(".git"), "gitdir: elsewhere").expect(".git");
        let add = crate::pages::form_of(
            &dir.0.join("rust"),
            &["task", "add", "t-next", "--why", "Part 2 of t."],
        );
        tasks
            .borrow_mut()
            .file_copy("t-next", "add", &add)
            .expect("filed");
        assert_eq!(asked("t", "moved.json"), None);
    }
}
