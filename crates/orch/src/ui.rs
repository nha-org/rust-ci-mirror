//! Orch's UI components (ARCHITECTURE.md section 2 point 5): trees of the catalog's elements on the SDK alone, which the
//! design system draws in every form (task rust-orch-ports). rust-move-orch grows /tasks here.
use dash_core::{Answer, Asked, BOARD, Drawing, Row, Tree, View, json};

/// /tasks' board from the board view's rows, each `(board, id, column, title)`: a column per status, a row per task in it.
pub fn tasks(rows: &[Row]) -> Drawing {
    let mut tree = Tree::default();
    let mut groups = vec![];
    for column in BOARD {
        let mut ids = vec![];
        for (_, id, _, title) in rows.iter().filter(|r| r.2 == column) {
            let props =
                json!({"id": id, "title": title, "status": column, "says": "", "comments": 0});
            ids.push(format!("row-{id}"));
            tree.add(&format!("row-{id}"), "issue-row", props, &[]);
        }
        groups.push(format!("group-{column}"));
        tree.add(
            &format!("group-{column}"),
            "issue-group",
            json!({"status": column}),
            &ids,
        );
    }
    tree.add("board", "board", json!({"view": "board"}), &groups);
    tree.add("title", "heading", json!({"text": "Tasks"}), &[]);
    tree.add(
        "page",
        "stack",
        json!(null),
        &["title".into(), "board".into()],
    );
    tree.drawing("Tasks", "page")
}

/// /tasks' page: the board view read through the core's views port, drawn by the host in the form asked.
pub fn tasks_page(a: &Asked, views: &dyn View) -> Answer {
    a.form(Some(views.rows("board").map(|rows| tasks(&rows))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::{OpLog, WallClock};
    use dash_meter::dash_test;
    use std::path::{Path, PathBuf};

    /// The board view's stand-in: its rows, or why it has none.
    struct Board(Result<Vec<Row>, String>);

    impl View for Board {
        fn rows(&self, view: &str) -> Result<Vec<Row>, String> {
            assert_eq!(view, "board");
            self.0.clone()
        }
    }

    struct Bare;

    impl WallClock for Bare {
        fn now_ms(&self) -> i64 {
            0
        }
    }

    impl dash_core::Host for Bare {
        fn repo(&self) -> PathBuf {
            "/repo".into()
        }
        fn data(&self) -> PathBuf {
            "/data".into()
        }
        fn home(&self) -> Result<PathBuf, String> {
            Err("none".into())
        }
        fn arm(&self, _: &Path, _: &str) {}
        fn log(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
            Err("none".into())
        }
    }

    fn row(id: &str, column: &str, title: &str) -> Row {
        ("board".into(), id.into(), column.into(), title.into())
    }

    /// Done-when 1 and 2 of rust-orch-ports: orch, which takes dash-core alone, reads the board view through the core's port
    /// and its page answers a /tasks screen as a `Drawing::Tree` built on the SDK, no design system taken.
    #[dash_test]
    fn the_tasks_page_reads_the_board_view_and_answers_a_tree() {
        let a = Asked {
            path: "/tasks",
            query: "",
            html: true,
            held: None,
            host: &Bare,
        };
        let views = Board(Ok(vec![
            row("a", "todo", "Task A"),
            row("b", "in-review", "Task B"),
        ]));
        let Answer::Drawn(Drawing::Tree { title, tree }) = tasks_page(&a, &views) else {
            panic!("a tree")
        };
        assert_eq!(title, "Tasks");
        let tree: serde_json::Value = serde_json::from_str(&tree).expect("json");
        let el = &tree["elements"];
        assert_eq!(tree["root"], "page");
        assert_eq!(el["group-todo"]["children"], json!(["row-a"]));
        assert_eq!(el["group-in-review"]["children"], json!(["row-b"]));
        assert_eq!(el["row-a"]["props"]["title"], "Task A");
        assert_eq!(el["board"]["children"].as_array().map(Vec::len), Some(4));
        let failed = tasks_page(&a, &Board(Err("the views thread ended".into())));
        assert!(
            matches!(failed, Answer::Body(s, _, e) if s.starts_with("500") && e == "the views thread ended")
        );
    }
}
