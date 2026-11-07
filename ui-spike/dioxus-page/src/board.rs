//! Linear-like cards. Dragging is mouse events, not HTML5 drag: Blitz fires no drag event, and dioxus-native's
//! convert_drag_data is unimplemented!.
use crate::{Avatar, Badge, Button, Dialog, Input};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
struct Issue {
    id: u32,
    title: String,
    who: &'static str,
    label: &'static str,
    tone: &'static str,
}

/// Pixels a press must travel to be a drag: a hand moves one or two during a click.
const DRAG_FROM: f64 = 4.0;
const COLUMNS: [&str; 4] = ["Backlog", "Todo", "In Progress", "Done"];

fn issues() -> [Vec<Issue>; 4] {
    let issue = |id, title: &str, who, label, tone| Issue { id, title: title.into(), who, label, tone };
    [
        vec![
            issue(101, "Trace the window's memory", "NH", "rust", "tone-primary"),
            issue(102, "Draw theme.css in Blitz", "CL", "ui", "tone-secondary"),
            issue(103, "Port the board's view", "OP", "rust", "tone-primary"),
        ],
        vec![
            issue(104, "One tree, two renderers", "PM", "rfc", "tone-outline"),
            issue(105, "Drag a card between columns", "CL", "ui", "tone-secondary"),
        ],
        vec![issue(106, "Fix the blank screenshot", "CL", "bug", "tone-destructive")],
        vec![issue(107, "Scroll 100,000 rows", "NH", "ui", "tone-secondary")],
    ]
}

#[component]
pub(crate) fn Board() -> Element {
    let mut columns = use_signal(issues);
    let mut lifted = use_signal(|| None::<u32>);
    let mut moved = use_signal(|| false);
    let mut at = use_signal(|| (0.0, 0.0));
    let mut pressed_at = use_signal(|| (0.0, 0.0));
    let mut over_column = use_signal(|| None::<usize>);
    let mut before = use_signal(|| None::<u32>);
    let mut drops = use_signal(|| 0);
    let mut html5 = use_signal(|| 0);
    let mut editing = use_signal(|| None::<(u32, String)>);
    let lifted_now = lifted();
    let title_of = move |id: u32| {
        columns.read().iter().flatten().find(|i| i.id == id).map_or(String::new(), |i| i.title.clone())
    };
    let mut drop_in = move |to: usize| {
        let Some(id) = lifted() else { return };
        if !moved() {
            return;
        }
        let mut all = columns.write();
        let Some(from) = all.iter().position(|c| c.iter().any(|i| i.id == id)) else { return };
        let i = all[from].iter().position(|x| x.id == id).expect("the card is in its column");
        let card = all[from].remove(i);
        let slot = before().and_then(|b| all[to].iter().position(|x| x.id == b)).unwrap_or(all[to].len());
        all[to].insert(slot, card);
        drops += 1;
    };
    let mut ended = move || {
        lifted.set(None);
        over_column.set(None);
        before.set(None);
    };
    rsx! {
        div { class: "screen",
            onmousemove: move |e: MouseEvent| {
                if lifted.peek().is_none() {
                    return;
                }
                // Released outside this screen, where no mouseup reaches it.
                if e.held_buttons().is_empty() {
                    ended();
                    return;
                }
                let p = e.client_coordinates();
                let (from, to) = (*pressed_at.peek(), (p.x, p.y));
                at.set(to);
                if (to.0 - from.0).hypot(to.1 - from.1) >= DRAG_FROM {
                    moved.set(true);
                }
            },
            onmouseup: move |_| ended(),
            div { class: "bar",
                span { class: "bar-title", "Board" }
                span { id: "drops", class: "meta", "moved: {drops}" }
                span { id: "html5", class: "meta", draggable: "true", ondragstart: move |_| html5 += 1, "HTML5 drag starts: {html5}" }
            }
            div { class: "columns",
                for (c, name) in COLUMNS.into_iter().enumerate() {
                    div { id: "col-{c}", key: "{name}",
                        class: if over_column() == Some(c) { "column over" } else { "column" },
                        onmouseenter: move |_| if lifted.peek().is_some() { over_column.set(Some(c)) },
                        onmouseup: move |_| drop_in(c),
                        div { class: "column-head", "{name}", span { class: "meta", "{columns.read()[c].len()}" } }
                        for issue in columns.read()[c].iter().cloned() {
                            div { id: "card-{issue.id}", key: "{issue.id}",
                                class: if lifted_now == Some(issue.id) { "issue lifted" } else { "issue" },
                                onmousedown: move |e: MouseEvent| {
                                    // Never prevent_default here: in Blitz it also cancels the press's own bookkeeping, so
                                    // the next move reads as a selection drag and the click is lost. user-select holds it.
                                    let p = e.client_coordinates();
                                    at.set((p.x, p.y));
                                    pressed_at.set((p.x, p.y));
                                    moved.set(false);
                                    lifted.set(Some(issue.id));
                                },
                                onmouseenter: move |_| if lifted.peek().is_some_and(|l| l != issue.id) { before.set(Some(issue.id)) },
                                onmouseleave: move |_| before.set(None),
                                // Released on the card it pressed: ends here, before the column's mouseup would move it.
                                onmouseup: move |_| if lifted.peek().is_some_and(|l| l == issue.id) { lifted.set(None) },
                                onclick: {
                                    let title = issue.title.clone();
                                    move |_| if !moved() { editing.set(Some((issue.id, title.clone()))) }
                                },
                                div { class: "issue-top",
                                    span { class: "meta", "AAA-{issue.id}" }
                                    Avatar { initials: issue.who }
                                }
                                div { class: "issue-title", "{issue.title}" }
                                div { class: "issue-foot", Badge { class: issue.tone, "{issue.label}" } }
                            }
                        }
                    }
                }
            }
        }
        if let Some(id) = lifted_now.filter(|_| moved()) {
            div { id: "ghost", class: "ghost", style: "left:{at().0 - 100.0}px;top:{at().1 - 20.0}px", "{title_of(id)}" }
        }
        if let Some((id, title)) = editing() {
            Dialog { title: "Edit AAA-{id}", onclose: move |_| editing.set(None),
                div { class: "field",
                    Input { id: "issue-title-input", value: title.clone(), oninput: move |v| editing.set(Some((id, v))) }
                }
                div { class: "dialog-actions",
                    Button { id: "issue-cancel", class: "button quiet", onclick: move |_| editing.set(None), "Cancel" }
                    Button { id: "issue-save",
                        onclick: move |_| {
                            if let Some((id, title)) = editing() {
                                for issue in columns.write().iter_mut().flatten() {
                                    if issue.id == id {
                                        issue.title = title.clone();
                                    }
                                }
                            }
                            editing.set(None);
                        },
                        "Save"
                    }
                }
            }
        }
    }
}
