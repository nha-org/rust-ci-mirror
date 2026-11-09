use super::*;
use dash_core::{FieldWrite, Value};
use dash_meter::memory::TestBudget;

fn op(stamp: i64, table: &str, row: &str, field: &str, value: Value) -> Op {
    let write = FieldWrite {
        table: table.into(),
        row: row.into(),
        field: field.into(),
        value,
    };
    Op {
        stamp,
        writer: "test".into(),
        write,
    }
}

fn t(s: &str) -> Value {
    Value::Text(s.into())
}

/// A commit at a time, each op stamped after the last.
struct Log {
    engine: Engine,
    stamp: i64,
}

impl Log {
    fn new() -> Self {
        Self {
            engine: Engine::new(),
            stamp: 0,
        }
    }

    fn commit(&mut self, writes: &[(&str, &str, &str, Value)]) -> Diff {
        let ops: Vec<Op> = writes
            .iter()
            .map(|(table, row, field, value)| {
                self.stamp += 1;
                op(self.stamp, table, row, field, value.clone())
            })
            .collect();
        self.engine.commit(&ops)
    }

    fn view(&self, view: &str) -> Vec<Out> {
        self.engine.rows(view)
    }
}

fn row(view: &str, key: &str, a: &str, b: &str) -> Out {
    (view.into(), key.into(), a.into(), b.into())
}

const REVIEW_DOC: &str = r#"<title>The plan</title><meta name="crm:review-since" content="2026-11-01T10:00">
<ol class="stages"><li aria-current="step">Review</li></ol>
<p id="q1">Which store? <span class="chip warn">to discuss</span></p>"#;

#[test]
fn the_board_puts_each_task_in_its_column_and_moves_it_on_each_commit() {
    let _budget = TestBudget::new();
    let mut log = Log::new();
    log.commit(&[
        ("listed", "a", "title", t("Task a")),
        ("listed", "a", "pos", Value::Int(0)),
        ("listed", "b", "title", t("Task b")),
        ("listed", "b", "pos", Value::Int(1)),
        ("listed", "b", "waits", t("a")),
    ]);
    assert_eq!(
        log.view("board"),
        [
            row("board", "a", "todo", "Task a"),
            row("board", "b", "backlog", "Task b")
        ]
    );
    let diff = log.commit(&[("tasks", "a", "status", t("claimed"))]);
    assert_eq!(
        diff,
        [
            (row("board", "a", "in-progress", "Task a"), 1),
            (row("board", "a", "todo", "Task a"), -1)
        ]
    );
    log.commit(&[
        ("tasks", "a", "status", t("finished")),
        ("tasks", "a", "outcome", t("merged")),
    ]);
    assert_eq!(
        log.view("board"),
        [row("board", "b", "todo", "Task b")],
        "a's wait is merged"
    );
    log.commit(&[
        ("listed", "b", "pos", Value::Null),
        ("tasks", "c", "status", t("open")),
    ]);
    assert_eq!(
        log.view("board"),
        [row("board", "c", "backlog", "")],
        "b unlisted and no run: off the board; c a run the list does not offer"
    );
}

#[test]
fn needs_you_holds_blocked_runs_pending_drafts_and_docs_in_review() {
    let _budget = TestBudget::new();
    let mut log = Log::new();
    log.commit(&[
        ("tasks", "a", "status", t("finished")),
        ("tasks", "a", "outcome", t("blocked")),
        (
            "tasks",
            "a",
            "said",
            t("\nApprove section 23 on /approve\nmore"),
        ),
        (
            "drafts",
            "d1",
            "record",
            t(r#"{"status": "pending", "receiver": "Ana"}"#),
        ),
        ("drafts", "d2", "record", t("not json")),
        ("docs", "private/docs/plan.html", "body", t(REVIEW_DOC)),
    ]);
    assert_eq!(
        log.view("needs"),
        [
            row(
                "needs",
                "doc private/docs/plan.html",
                "review",
                "Review the document The plan in plan.html"
            ),
            row("needs", "draft d1", "draft", "Approve the draft to Ana"),
            row(
                "needs",
                "question private/docs/plan.html#q1",
                "question",
                "Answer a question in plan.html: Which store?"
            ),
            row("needs", "task a", "task", "Approve section 23 on /approve"),
        ]
    );
    assert_eq!(
        log.view("docs"),
        [row(
            "docs",
            "private/docs/plan.html",
            "The plan",
            "2026-11-01\nAccept it, or comment where it falls short"
        )]
    );
    log.commit(&[
        ("tasks", "a", "status", t("open")),
        (
            "drafts",
            "d1",
            "record",
            t(r#"{"status": "approved", "receiver": "Ana"}"#),
        ),
        ("docs", "private/docs/plan.html", "stage", t("accepted")),
    ]);
    assert_eq!(
        log.view("needs"),
        [row(
            "needs",
            "question private/docs/plan.html#q1",
            "question",
            "Answer a question in plan.html: Which store?"
        )],
        "a reopened, d1 approved, the doc's stage past review: its question stays"
    );
    assert!(log.view("docs").is_empty());
    log.commit(&[("docs", "private/docs/plan.html", "gone", Value::Int(1))]);
    assert!(
        log.view("needs").is_empty(),
        "a removed doc takes its question"
    );
}

/// Seeded: the same commits each run, so a failure names its seed and commit.
struct Seed(u64);

impl Seed {
    fn pick(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as usize % n
    }
}

fn random_write(seed: &mut Seed) -> (String, String, String, Value) {
    let ids = ["a", "b", "c", "d"];
    let id = ids[seed.pick(ids.len())].to_string();
    let words = ["", "a", "b c", "d\na", "Approve it\nnow"];
    let word = t(words[seed.pick(words.len())]);
    let (table, field, value) = match seed.pick(12) {
        0 => (
            "tasks",
            "status",
            t(["open", "claimed", "finished"][seed.pick(3)]),
        ),
        1 => (
            "tasks",
            "outcome",
            [t("merged"), t("blocked"), t("done"), Value::Null][seed.pick(4)].clone(),
        ),
        2 => ("tasks", "said", word),
        3 => ("tasks", "waits", word),
        4 => ("listed", "title", word),
        5 => (
            "listed",
            "pos",
            [Value::Int(0), Value::Int(3), Value::Null][seed.pick(3)].clone(),
        ),
        6 => ("listed", "waits", word),
        7 => {
            let status = ["pending", "approved"][seed.pick(2)];
            let record = format!(r#"{{"status": "{status}", "receiver": "{id}"}}"#);
            ("drafts", "record", t(&record))
        }
        8 => (
            "docs",
            "body",
            [t(REVIEW_DOC), t("<title>Old</title>"), t("")][seed.pick(3)].clone(),
        ),
        9 => (
            "docs",
            "stage",
            [t("review"), t("done"), Value::Null][seed.pick(3)].clone(),
        ),
        10 => (
            ["tasks", "listed", "drafts", "docs"][seed.pick(4)],
            "gone",
            [Value::Int(1), Value::Null][seed.pick(2)].clone(),
        ),
        _ => ("tasks", "pane", t("a field no view reads")),
    };
    (table.into(), id, field.into(), value)
}

/// Done-when 2: after every commit, each view's rows are what a plain recompute of the tables says, read from its own replay
/// of the ops, never the engine's cells.
#[test]
fn each_view_agrees_with_a_plain_recompute_after_every_commit() {
    let _budget = TestBudget::new();
    for start in 0..20u64 {
        let mut seed = Seed(start);
        let mut log = Log::new();
        let mut tables: BTreeMap<(String, String), Cells> = BTreeMap::new();
        for commit in 0..60 {
            let writes: Vec<_> = (0..1 + seed.pick(5))
                .map(|_| random_write(&mut seed))
                .collect();
            for (table, row, field, value) in &writes {
                let cells = tables.entry((table.clone(), row.clone())).or_default();
                cells.insert(field.clone(), value.clone());
            }
            let refs: Vec<(&str, &str, &str, Value)> = writes
                .iter()
                .map(|(a, b, c, v)| (a.as_str(), b.as_str(), c.as_str(), v.clone()))
                .collect();
            log.commit(&refs);
            let inputs: Vec<Input> = tables
                .iter()
                .filter_map(|((t, r), c)| decode(t, r, c))
                .collect();
            let plain = recompute(inputs.iter());
            if let Some(wrong) = disagreement(log.engine.weights(), &plain) {
                panic!("seed {start}, commit {commit}: {wrong}");
            }
            for view in VIEWS {
                let ours = log.view(view);
                let theirs: Vec<Out> = plain.iter().filter(|o| o.0 == view).cloned().collect();
                assert_eq!(ours, theirs, "seed {start}, commit {commit}, {view}");
            }
        }
        assert_eq!(log.engine.disagreement(), None);
    }
}

/// The views after a commit that fills all three, and a row of `view` to plant a wrong diff on.
fn filled(view: &str) -> (Log, Out) {
    let mut log = Log::new();
    log.commit(&[
        ("listed", "a", "title", t("Task a")),
        ("listed", "a", "pos", Value::Int(0)),
        ("tasks", "b", "status", t("finished")),
        ("tasks", "b", "outcome", t("blocked")),
        ("docs", "plan.html", "body", t(REVIEW_DOC)),
    ]);
    assert_eq!(
        log.engine.disagreement(),
        None,
        "the check passes before the plant"
    );
    let row = log
        .view(view)
        .into_iter()
        .next()
        .expect("a row of the view");
    (log, row)
}

/// Each way a diff goes wrong on `view`'s row: one too many, one missing, held twice, the wrong column or words.
fn planted_wrong_diffs_fail(view: &str) {
    for plant in 0..4 {
        let (log, row) = filled(view);
        let mut rows = log.engine.weights().clone();
        match plant {
            0 => {
                rows.insert(
                    (row.0.clone(), "a key no table holds".into(), row.2, row.3),
                    1,
                );
            }
            1 => {
                rows.remove(&row);
            }
            2 => {
                *rows.get_mut(&row).expect("its row") += 1;
            }
            _ => {
                rows.remove(&row);
                rows.insert(
                    (row.0, row.1, row.2, "words the rules never wrote".into()),
                    1,
                );
            }
        }
        let inputs = log.engine.inputs();
        let found = disagreement(&rows, &recompute(inputs.iter()));
        assert!(found.is_some(), "{view}: plant {plant} passed the check");
    }
}

#[test]
fn a_planted_wrong_diff_on_the_board_fails_the_check() {
    let _budget = TestBudget::new();
    planted_wrong_diffs_fail("board");
}

#[test]
fn a_planted_wrong_diff_on_needs_you_fails_the_check() {
    let _budget = TestBudget::new();
    planted_wrong_diffs_fail("needs");
}

#[test]
fn a_planted_wrong_diff_on_docs_to_review_fails_the_check() {
    let _budget = TestBudget::new();
    planted_wrong_diffs_fail("docs");
}

/// Done-when 3: a subscriber gets the view's rows at once, then each commit's diff to that view alone.
#[test]
fn a_subscriber_gets_a_views_rows_then_each_diff() {
    let _budget = TestBudget::new();
    let mut log = Log::new();
    log.commit(&[
        ("listed", "a", "title", t("Task a")),
        ("listed", "a", "pos", Value::Int(0)),
    ]);
    let (tx, rx) = channel();
    log.engine.subscribe("board", tx);
    assert_eq!(
        rx.try_recv(),
        Ok(vec![(row("board", "a", "todo", "Task a"), 1)])
    );
    log.commit(&[("drafts", "d", "record", t(r#"{"status": "pending"}"#))]);
    assert!(
        rx.try_recv().is_err(),
        "a commit that leaves the board alone sends it nothing"
    );
    log.commit(&[("listed", "a", "title", t("Task A"))]);
    assert_eq!(
        rx.try_recv(),
        Ok(vec![
            (row("board", "a", "todo", "Task A"), 1),
            (row("board", "a", "todo", "Task a"), -1)
        ])
    );
    drop(rx);
    log.commit(&[("listed", "a", "pos", Value::Null)]);
    assert!(
        log.engine.subscribers.is_empty(),
        "a subscriber gone is dropped"
    );
}

/// The engine on its thread, fed from the log before each read, a page at a time.
#[test]
fn views_read_the_log_before_each_answer() {
    let _budget = TestBudget::new();
    let log = std::sync::Arc::new(Mutex::new(vec![
        op(1, "listed", "a", "title", t("Task a")),
        op(2, "listed", "a", "pos", Value::Int(0)),
    ]));
    let shared = log.clone();
    let source: Source = Box::new(move |after| {
        let ops = shared.lock().expect("the log");
        let newer = ops
            .iter()
            .filter(|o| after.is_none_or(|(s, w)| (o.stamp, o.writer.as_str()) > (s, w)));
        // a page of one op: the views read until a page comes back empty
        Ok(newer.take(1).cloned().collect())
    });
    let views = Views::start(source).expect("started");
    assert_eq!(
        views.rows("board"),
        Ok(vec![row("board", "a", "todo", "Task a")])
    );
    let diffs = views.subscribe("board").expect("subscribed");
    assert_eq!(
        diffs.recv(),
        Ok(vec![(row("board", "a", "todo", "Task a"), 1)])
    );
    log.lock()
        .expect("the log")
        .push(op(3, "tasks", "a", "status", t("claimed")));
    // nobody asks for rows: the subscriber alone has the views follow the log
    assert_eq!(
        diffs.recv_timeout(FOLLOW * 5),
        Ok(vec![
            (row("board", "a", "in-progress", "Task a"), 1),
            (row("board", "a", "todo", "Task a"), -1)
        ])
    );
    assert_eq!(
        views.rows("board"),
        Ok(vec![row("board", "a", "in-progress", "Task a")])
    );
}
