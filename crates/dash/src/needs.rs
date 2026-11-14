//! /needs-you: what waits on him, the needs-you view's rows, then the documents in review, the docs-to-review view's, both
//! section 6's dataflow views (crates/views), never a page-local read (task rust-needs-you-view-page; @nha, 2026-11-09:
//! « render via the system the RFC describe »). The page follows both views as /tasks follows the board's: each diff of
//! either draws its one part again (`NEEDS`), on the web through `?view=patches` and in the window through its one stream.
use crate::pages::{Asked, Page, Reads};
use crate::tasks::{Board, Follow};
use dash_comments::threads::shown_on;
use dash_surface::Drawing;
use dash_views::rules::Out;
use serde_json::{Value, json};

pub const PAGE: &str = "/needs-you";

/// The part of the page that follows the views, drawn again whole at each diff: a page of rows, cheap to draw.
const ELEMENT: &str = "needs";

fn whole(_: &dash_views::Diff) -> Vec<String> {
    vec![ELEMENT.into()]
}

/// /needs-you follows the needs-you and docs-to-review views. ponytail: a thread a view, so a commit changing both (a doc
/// entering review) sends the part twice; one subscription to both views if that redraw ever shows.
pub static NEEDS: Follow = Follow {
    page: PAGE,
    stream: "/needs-you?view=patches",
    views: &["needs", "docs"],
    whole: ELEMENT,
    ids: whole,
};

/// Where a needs-you row opens, by its key: a task's page, a draft's, a document at /v/, a question at its anchor there.
fn href(key: &str) -> Option<String> {
    let (kind, rest) = key.split_once(' ')?;
    Some(match kind {
        "task" => format!("/tasks/{rest}"),
        "draft" => format!("/drafts/{rest}"),
        "doc" => shown_on(rest),
        "question" => {
            let (rel, anchor) = rest.split_once('#').unwrap_or((rest, ""));
            format!("{}#{anchor}", shown_on(rel))
        }
        _ => return None,
    })
}

/// A table of `rows` under `title`, or the words saying it is empty.
fn section(
    elements: &mut serde_json::Map<String, Value>,
    id: &str,
    title: &str,
    table: Value,
    empty: &str,
) {
    let rows = format!("{id}-rows");
    let none = table["props"]["rows"].as_array().is_none_or(Vec::is_empty);
    let body = match none {
        true => json!({"type": "text", "props": {"text": empty}}),
        false => table,
    };
    elements.insert(rows.clone(), body);
    elements.insert(
        id.into(),
        json!({"type": "section", "props": {"title": title}, "children": [rows]}),
    );
}

/// The page's tree from the two views' rows. A document in review is listed once, under Documents to review with its day and
/// the decision it asks; the needs-you table holds every other thing waiting on him.
pub fn tree(needs: &[Out], docs: &[Out]) -> String {
    let mut elements = serde_json::Map::new();
    let needs_rows: Vec<Value> = (needs.iter())
        .filter(|(_, _, kind, _)| kind != "review")
        .map(|(_, key, kind, what)| json!({"cells": [what, kind], "href": href(key)}))
        .collect();
    let docs_rows: Vec<Value> = (docs.iter())
        .map(|(_, rel, title, said)| {
            let (since, decision) = said.split_once('\n').unwrap_or((said, ""));
            json!({"cells": [title, since, decision], "href": shown_on(rel)})
        })
        .collect();
    let table = |columns: &[&str], rows| json!({"type": "table", "props": {"columns": columns, "rows": rows}});
    section(
        &mut elements,
        "needs-you",
        "Needs you",
        table(&["What", "Kind"], needs_rows),
        "Nothing waits on you.",
    );
    section(
        &mut elements,
        "docs-review",
        "Documents to review",
        table(&["Document", "In review since", "Decision"], docs_rows),
        "No document in review.",
    );
    elements.insert(
        ELEMENT.into(),
        json!({"type": "live", "props": {"view": "needs"}, "children": ["needs-you", "docs-review"]}),
    );
    json!({"root": ELEMENT, "elements": elements}).to_string()
}

fn drawing(board: &Board) -> Result<Drawing, String> {
    let tree = tree(&board.view("needs")?, &board.view("docs")?);
    Ok(Drawing::Tree {
        title: "Needs you".into(),
        tree,
    })
}

fn serve(a: &Asked) -> crate::pages::Response {
    a.form(a.board::<Board>().map(drawing))
}

inventory::submit! {
    Page { path: PAGE, step: "route_needs_you", reads: Reads::Board, redraw: true, serve }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// A page in review, its question to discuss: the HTML lives in a fixture (dash-gate's the_dash_crate_holds_no_html).
    const REVIEW: &str = include_str!("../tests/fixtures/review-doc.html");

    /// Done-when 2 of rust-needs-you-view-page: the docs-to-review view reads the docs log's records (docs.sqlite). A page in
    /// review read into the store, as /v/ and the docs pass do, is listed by the view and on /needs-you with its day, and its
    /// question in Needs you; a page past review leaves the view.
    #[dash_test]
    fn a_doc_in_review_on_the_docs_log_is_listed_by_the_docs_view() {
        let dir = TestDir::new("needs-docs");
        let (repo, data) = (dir.0.join("repo"), dir.0.join("data"));
        std::fs::create_dir_all(repo.join("private/docs")).expect("repo");
        std::fs::create_dir_all(&data).expect("data");
        unsafe { std::env::set_var("DASH_WORKTREES", &dir.0) };
        let plan = "private/docs/plan.html";
        std::fs::write(repo.join(plan), REVIEW).expect("the page");
        let board = Board::open(&data).expect("the board");
        assert_eq!(board.view("docs"), Ok(vec![]));

        crate::docs_log::opened(&data, &repo, plan)
            .expect("read in")
            .expect("a doc");
        let listed = (
            "docs".to_string(),
            plan.to_string(),
            "The plan".to_string(),
            "2026-11-01\nAccept it, or comment where it falls short".to_string(),
        );
        assert_eq!(board.view("docs"), Ok(vec![listed.clone()]));
        let text = crate::pages::drawn(false, false, Some(drawing(&board))).2;
        assert!(
            text.contains("The plan\t2026-11-01\tAccept it, or comment where it falls short\t/v/private/docs/plan.html"),
            "{text}"
        );
        assert!(
            text.contains("Answer a question in plan.html: Which store?\tquestion\t/v/private/docs/plan.html#q1"),
            "its question in Needs you: {text}"
        );

        std::fs::write(
            repo.join(plan),
            REVIEW.replace(r#" aria-current="step">Review"#, ">Review"),
        )
        .expect("past review");
        crate::docs_log::opened(&data, &repo, plan)
            .expect("read again")
            .expect("a doc");
        let docs = board.view("docs").expect("rows");
        assert!(docs.iter().all(|(_, rel, ..)| rel != plan), "{docs:?}");
    }
}
