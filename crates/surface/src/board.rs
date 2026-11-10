//! The tasks board's drawing (task rust-board-linear-order; @nha, 2026-11-09: « does not follow the design system completely
//! it seems ? let's not have drift. Shared components. »): each element a board holds, drawn with the design system's
//! components alone, crate::components and design/linear.rs's card face. dash-gate's `board_markup` refuses any HTML element
//! written here: a part the board lacks is a component first.
use crate::components::{Board, Card, CardLine, Column};
use crate::design::families::status as icon;
use crate::design::linear;
use crate::documents::IssueStatus;
use dioxus::prelude::*;

pub(crate) fn board(id: &str, children: Element) -> Element {
    rsx! { Board { id, {children} } }
}

pub(crate) fn column(id: &str, at: &IssueStatus, count: usize, children: Element) -> Element {
    rsx! { Column { id, status: at.class(), name: at.name(), count, icon: icon(at.kind()), {children} } }
}

/// A task's card. `movable`: his drag moves it to another column (a task Rust's own board view places).
pub(crate) struct Face<'a> {
    pub task: &'a str,
    pub title: &'a str,
    pub at: &'a IssueStatus,
    pub says: &'a str,
    pub comments: usize,
    pub said: Option<String>,
    pub href: String,
    pub labels: &'a [String],
    pub assignee: Option<&'a str>,
    pub movable: bool,
}

pub(crate) fn card(id: &str, f: Face) -> Element {
    let line = rsx! { CardLine { says: f.says, comments: f.comments, said: f.said } };
    rsx! {
        Card { id, look: "li-card", href: f.href, task: f.task.to_string(), draggable: f.movable,
            {linear::face(f.task, f.title, f.at.kind(), f.labels, f.assignee, false, line)}
        }
    }
}
