//! What each view says, row by row, and the plain recompute every engine is checked against (section 6). The rules are
//! rust-dataflow-spike's, measured there against dash's own lists, on the tables the op log carries here: `tasks` (a run
//! as the store keeps it), `listed` (the list as its last sync wrote it), `drafts` (the drafts plugin's records) and `docs`.
//! `docs` is docs.sqlite's, each page of private/docs with its file's text as `body` (dash's docs.rs, task
//! docs-table-lags-files). ponytail: acts, handoff steps and mail are not on the log, so needs-you has none of them until
//! they are.
use dash_core::Value;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

/// A view's row: its view (board, needs, docs), its key, then two fields.
pub type Out = (String, String, String, String);

/// The views, by the name their rows carry.
pub const VIEWS: [&str; 3] = ["board", "needs", "docs"];

/// What a table the views read holds for its fields, each field as the op log last wrote it.
pub type Cells = BTreeMap<String, Value>;

/// The tables the views read, and the fields of each: no other op reaches the dataflow. `gone` is a removal (oplog.rs).
pub const TABLES: [(&str, &[&str]); 5] = [
    ("tasks", &["status", "outcome", "said", "waits", "gone"]),
    ("listed", &["title", "pos", "waits", "gone"]),
    ("moved", &["column", "gone"]),
    ("drafts", &["record", "gone"]),
    ("docs", &["body", "stage", "gone"]),
];

/// An input row, decoded from its cells: what the dataflow holds, never a document's body.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Input {
    /// `status` « open » with none written, as the store reads a row a page of a batch split.
    Run {
        id: String,
        status: String,
        outcome: Option<String>,
        said: String,
        waits: Vec<String>,
    },
    Listed {
        id: String,
        title: String,
        pos: Option<i64>,
        waits: Vec<String>,
    },
    /// The column he dragged a card to on /tasks (task rust-board-linear-order).
    Moved { id: String, column: String },
    Draft {
        id: String,
        status: String,
        receiver: String,
    },
    Doc {
        rel: String,
        facts: DocFacts,
        questions: Vec<(String, String)>,
        stage: Option<String>,
    },
}

fn text(cells: &Cells, field: &str) -> Option<String> {
    match cells.get(field) {
        Some(Value::Text(t)) => Some(t.clone()),
        Some(Value::Int(i)) => Some(i.to_string()),
        _ => None,
    }
}

/// A row from its cells, `None` once removed or when its record cannot be read.
pub fn decode(table: &str, row: &str, cells: &Cells) -> Option<Input> {
    if !matches!(cells.get("gone"), None | Some(Value::Null | Value::Int(0))) {
        return None;
    }
    let id = row.to_string();
    let word = |field: &str| text(cells, field).unwrap_or_default();
    match table {
        // the store's own reads: `standing_in` splits a run's waits on spaces, `listed_in` the list's on lines
        "tasks" => Some(Input::Run {
            id,
            status: text(cells, "status").unwrap_or_else(|| "open".into()),
            outcome: text(cells, "outcome"),
            said: word("said"),
            waits: word("waits").split_whitespace().map(String::from).collect(),
        }),
        "listed" => Some(Input::Listed {
            id,
            title: word("title"),
            pos: match cells.get("pos") {
                Some(Value::Int(p)) => Some(*p),
                _ => None,
            },
            waits: word("waits").lines().map(String::from).collect(),
        }),
        "moved" => Some(Input::Moved {
            id,
            column: text(cells, "column").filter(|c| BOARD.contains(&c.as_str()))?,
        }),
        "drafts" => {
            let record: serde_json::Value = serde_json::from_str(&word("record")).ok()?;
            let field = |f: &str| record[f].as_str().unwrap_or_default().to_string();
            Some(Input::Draft {
                id,
                status: field("status"),
                receiver: field("receiver"),
            })
        }
        "docs" => {
            let body = word("body");
            Some(Input::Doc {
                facts: doc_facts(&id, &body),
                questions: questions(&body),
                rel: id,
                stage: text(cells, "stage"),
            })
        }
        _ => None,
    }
}

/// The board's column for a task of Rust's list, as /tasks names it: its run claimed, in progress; offered by the list with
/// every wait merged, to do; anything else not finished, backlog. A finished run is off the board. A card he moved sits where
/// he put it unless a run claims it: a live run is in progress whatever the card said (`recompute`).
pub fn column(claimed: bool, offered: bool, unmet: bool) -> &'static str {
    let [in_progress, todo, backlog] = COLUMNS;
    if claimed {
        in_progress
    } else if offered && !unmet {
        todo
    } else {
        backlog
    }
}

/// The board's columns: a page maps its own status words from these.
pub const COLUMNS: [&str; 3] = ["in-progress", "todo", "backlog"];

/// The board's columns left to right, as Linear's (task rust-board-linear-order; @nha, 2026-11-09: « wrong order »): the
/// columns a card may be dragged to. In Review is never computed here: a moved card or a page's own reading puts one there.
pub const BOARD: [&str; 4] = ["backlog", "todo", "in-progress", "in-review"];

pub fn board_row(id: &str, column: &str, title: &str) -> Out {
    ("board".into(), id.into(), column.into(), title.into())
}

pub fn needs(key: String, kind: &str, what: String) -> Out {
    ("needs".into(), key, kind.into(), what)
}

/// A run that ended blocked waits on him until the list opens it again: its first line of words says for what.
pub fn blocked_row(id: &str, said: &str) -> Out {
    let ask = said.lines().map(str::trim).find(|l| !l.is_empty());
    let what = ask.map_or_else(|| format!("Unblock task {id}"), str::to_string);
    needs(format!("task {id}"), "task", what)
}

pub fn is_blocked(status: &str, outcome: Option<&str>) -> bool {
    status == "finished" && outcome == Some("blocked")
}

pub fn draft_row(id: &str, to: &str) -> Out {
    needs(
        format!("draft {id}"),
        "draft",
        format!("Approve the draft to {to}"),
    )
}

/// A document's facts, read once from its body so the dataflow never holds the body.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DocFacts {
    pub title: String,
    pub since: String,
    pub decision: String,
    pub claimed: bool,
}

/// In review: its stage says so, or with no stage its page claims it.
pub fn in_review(stage: Option<&str>, claimed: bool) -> bool {
    stage.map_or(claimed, |s| s == "review")
}

static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<title>([^<]+)</title>").unwrap());
static STAGES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<ol class="stages"[^>]*>(.*?)</ol>"#).unwrap());
static CURRENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"aria-current="step"[^>]*>\s*(\w+)"#).unwrap());
static SINCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<meta name="crm:review-since" content="([^"]+)""#).unwrap());
static DECISION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<meta name="crm:decision" content="([^"]+)""#).unwrap());
static CHIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span class="chip warn">to discuss</span>"#).unwrap());
static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bid="([^"]+)""#).unwrap());
static NUMERIC_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#(x[0-9a-fA-F]+|\d+);").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static DEFAULT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bdefault\b").unwrap());
static NO_DEFAULT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bno default\b").unwrap());

fn first(re: &Regex, text: &str) -> String {
    re.captures(text)
        .map_or(String::new(), |c| c[1].trim().to_string())
}

/// A meta's content as written: NeedsYou.meta does not trim it.
fn raw(re: &Regex, text: &str) -> String {
    re.captures(text)
        .map_or(String::new(), |c| c[1].to_string())
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Docs.html_unescape, on what a meta's content can hold.
pub fn html_unescape(text: &str) -> String {
    let numeric = NUMERIC_ENTITY.replace_all(text, |c: &regex::Captures| {
        let code = match c[1].strip_prefix('x') {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => c[1].parse(),
        };
        code.ok()
            .and_then(char::from_u32)
            .map_or(c[0].to_string(), String::from)
    });
    numeric
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// NeedsYou.day: the date a `crm:review-since` starts with, empty when malformed.
fn day(text: &str) -> String {
    let date = text.get(..10).unwrap_or("");
    let shape = date.bytes().enumerate().all(|(i, b)| match i {
        4 | 7 => b == b'-',
        _ => b.is_ascii_digit(),
    });
    if shape && date.len() == 10 {
        date.to_string()
    } else {
        String::new()
    }
}

const DEFAULT_DECISION: &str = "Accept it, or comment where it falls short";

pub fn doc_facts(rel: &str, body: &str) -> DocFacts {
    let title = first(&TITLE, body);
    let claimed = STAGES.captures(body).is_some_and(|ol| {
        let label = first(&CURRENT, &ol[1]);
        label == "Review" || label == "Relecture"
    });
    DocFacts {
        title: if title.is_empty() {
            basename(rel).into()
        } else {
            title
        },
        since: day(&html_unescape(&raw(&SINCE, body))),
        decision: match raw(&DECISION, body) {
            d if d.is_empty() => DEFAULT_DECISION.to_string(),
            d => html_unescape(&d),
        },
        claimed,
    }
}

/// NeedsYou.asks for « to discuss » rows: `(anchor, text)`, keyed by the anchor.
pub fn questions(body: &str) -> Vec<(String, String)> {
    if !body.contains("chip warn\">to discuss") {
        return vec![];
    }
    let mut anchor = String::new();
    let mut found = vec![];
    for line in body.lines() {
        let own = ID.captures(line).map(|c| c[1].to_string());
        if CHIP.is_match(line) && !(DEFAULT.is_match(line) && !NO_DEFAULT.is_match(line)) {
            let stripped = line.replace("to discuss", " ");
            let text = TAG.replace_all(&stripped, " ");
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            found.push((own.clone().unwrap_or_else(|| anchor.clone()), text));
        }
        if let Some(c) = ID.captures_iter(line).last() {
            anchor = c[1].to_string();
        }
    }
    found
}

pub fn question_row(rel: &str, anchor: &str, text: &str) -> Out {
    needs(
        format!("question {rel}#{anchor}"),
        "question",
        format!("Answer a question in {}: {text}", basename(rel)),
    )
}

pub fn docs_row(rel: &str, f: &DocFacts) -> Out {
    (
        "docs".into(),
        rel.into(),
        f.title.clone(),
        format!("{}\n{}", f.since, f.decision),
    )
}

pub fn review_row(rel: &str, f: &DocFacts) -> Out {
    let what = if f.title == basename(rel) {
        format!("Review the document {}", f.title)
    } else {
        format!("Review the document {} in {}", f.title, basename(rel))
    };
    needs(format!("doc {rel}"), "review", what)
}

/// The three views from whole tables, with no dataflow: the check on the engine.
pub fn recompute<'a>(rows: impl Iterator<Item = &'a Input>) -> BTreeSet<Out> {
    let (mut runs, mut listed, mut moved) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut out = BTreeSet::new();
    for row in rows {
        match row {
            Input::Run {
                id,
                status,
                outcome,
                said,
                waits,
            } => {
                if is_blocked(status, outcome.as_deref()) {
                    out.insert(blocked_row(id, said));
                }
                runs.insert(id.as_str(), (status.as_str(), outcome.as_deref(), waits));
            }
            Input::Listed {
                id,
                title,
                pos,
                waits,
            } => {
                listed.insert(id.as_str(), (title.as_str(), pos.is_some(), waits));
            }
            Input::Moved { id, column } => {
                moved.insert(id.as_str(), column.as_str());
            }
            Input::Draft {
                id,
                status,
                receiver,
            } if status == "pending" => {
                out.insert(draft_row(id, receiver));
            }
            Input::Draft { .. } => {}
            Input::Doc {
                rel,
                facts,
                questions,
                stage,
            } => {
                for (anchor, text) in questions {
                    out.insert(question_row(rel, anchor, text));
                }
                if in_review(stage.as_deref(), facts.claimed) {
                    out.insert(docs_row(rel, facts));
                    out.insert(review_row(rel, facts));
                }
            }
        }
    }
    let merged = |id: &str| {
        runs.get(id)
            .is_some_and(|r| r.0 == "finished" && r.1 == Some("merged"))
    };
    let ids: BTreeSet<&str> = runs.keys().chain(listed.keys()).copied().collect();
    for id in ids {
        let run = runs.get(id);
        if run.is_some_and(|r| r.0 == "finished") {
            continue;
        }
        let list = listed.get(id);
        let offered = list.is_some_and(|l| l.1);
        if !offered && run.is_none() {
            continue;
        }
        let waits = run
            .map(|r| r.2)
            .into_iter()
            .chain(list.map(|l| l.2))
            .flatten();
        let unmet = waits.into_iter().any(|w| !merged(w));
        let claimed = run.is_some_and(|r| r.0 == "claimed");
        let title = list.map_or("", |l| l.0);
        let at = match moved.get(id) {
            Some(to) if !claimed => to,
            _ => column(claimed, offered, unmet),
        };
        out.insert(board_row(id, at, title));
    }
    out
}

/// What is wrong with a view's rows against the plain recompute: a row missing, one too many, or one held other than once.
pub fn disagreement(rows: &BTreeMap<Out, i64>, plain: &BTreeSet<Out>) -> Option<String> {
    let mut wrong = vec![];
    for (row, weight) in rows {
        if *weight != 1 {
            wrong.push(format!("held {weight} times: {row:?}"));
        } else if !plain.contains(row) {
            wrong.push(format!("not in the recompute: {row:?}"));
        }
    }
    for row in plain.iter().filter(|r| !rows.contains_key(*r)) {
        wrong.push(format!("missing: {row:?}"));
    }
    (!wrong.is_empty()).then(|| wrong.join("\n"))
}
