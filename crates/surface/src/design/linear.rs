//! Linear-like cards, laid out after private/evidence/linear/board-1512.jpg, on design.json's tokens. Its issues are made up:
//! rust/ rides to the CI mirror, and the evidence's titles are his work. Dragging is mouse events, as in board.rs.
use super::families::{CHAT, DOC, DOTS, INBOX, LINK, PERSON, PLUS, SEARCH, icon, status};
use crate::components::{Avatar, Badge};
use blitz_dom::BaseDocument;
use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
struct Card {
    id: u32,
    title: &'static str,
    tag: &'static str,
}

const DRAG_FROM: f64 = 4.0;
const COLUMNS: [(&str, &str); 2] = [("Todo", "todo"), ("In Progress", "started")];

/// .li-column's gap.
const CARD_GAP: f64 = 8.0;

/// Where a dropped card lands, first in its column: (left, top, width), read from the laid-out head above it.
fn first_slot(doc: &BaseDocument, column: usize) -> Option<(f64, f64, f64)> {
    let head = super::families::rect(doc, &format!("#li-head-{column}"))?;
    Some((head.0, head.1 + head.3 + CARD_GAP, head.2))
}

/// The cards a drop moved besides the dropped one: those under its old place, and all under its new one.
#[derive(Clone, Copy, PartialEq)]
struct Shift {
    from: (usize, usize),
    to: usize,
    by: f64,
}

impl Shift {
    /// How far this card now starts from its new place, which is where the drop found it.
    fn of(self, column: usize, index: usize) -> Option<f64> {
        if column == self.to && index > 0 {
            Some(-self.by)
        } else if column == self.from.0 && index >= self.from.1 {
            Some(self.by)
        } else {
            None
        }
    }
}

/// A title's two lines by characters, as text-overflow and line-clamp draw no ellipsis in Blitz: it does not follow the
/// card's width, so a window narrower than Linear's would wrap to a third line.
fn clamped(title: &str) -> String {
    const LINE_CLAMP: usize = 60;
    if title.chars().count() <= LINE_CLAMP {
        return title.into();
    }
    let cut: String = title.chars().take(LINE_CLAMP).collect();
    format!(
        "{}...",
        cut.rsplit_once(' ')
            .map_or(cut.as_str(), |(whole, _)| whole)
    )
}

fn columns() -> [Vec<Card>; 2] {
    let c = |id, title, tag| Card { id, title, tag };
    [
        vec![
            c(
                478,
                "A store file edited outside the app is caught, never silently synced",
                "dashboard",
            ),
            c(
                476,
                "The precommit passes at any load, and each slow test is named with its own timing line",
                "dashboard",
            ),
            c(
                473,
                "The one poll classes every pane: working, blocked, done, shell, crashed",
                "orchestration",
            ),
            c(
                472,
                "Every decision page reads clear enough to decide in one pass",
                "dashboard",
            ),
            c(
                469,
                "A command lists every open step that waits, task or not",
                "dashboard",
            ),
        ],
        vec![
            c(
                457,
                "Publish the three project pages and their videos on the site",
                "site",
            ),
            c(
                455,
                "The window's memory traced to its largest three owners",
                "rust",
            ),
            c(
                451,
                "Every button that writes answers in under a second, with visible feedback",
                "dashboard",
            ),
            c(
                449,
                "Scoring credits what a person can do, not only what they did",
                "dashboard",
            ),
            c(
                447,
                "A reviewer started outside the check claims its PR, so no second one starts",
                "orchestration",
            ),
        ],
    ]
}

const SIDE: [(&str, &str, &str); 5] = [
    ("Inbox", INBOX, ""),
    ("Reviews", LINK, "14"),
    ("My issues", PERSON, ""),
    ("Agent", CHAT, ""),
    ("Drafts", DOC, "1"),
];
const TEAM: [(&str, bool); 6] = [
    ("Home", false),
    ("Issues", true),
    ("Cycles", false),
    ("Current", false),
    ("Upcoming", false),
    ("Views", false),
];

/// A card's parts: the column draws them, and the ghost draws the same so its landing shows no seam.
fn card_body(card: Card, kind: &'static str) -> Element {
    let labels = [card.tag.to_string()];
    face(
        &format!("AAA-{}", card.id),
        card.title,
        "",
        kind,
        &labels,
        None,
        true,
        rsx! { "Created Oct 23" },
    )
}

/// A card's face, this page's and /tasks' board's (task rust-board-linear; @nha, 2026-11-09: « /tasks?project=site_perso ->
/// needs to follow the linear-like design for the cards »): its id, who it is assigned to (none: the empty person), its
/// status and title, the answer under it when it has one (/comments' Answered), its labels, then one line. `kind`: a status
/// `status` draws.
#[allow(clippy::too_many_arguments)]
pub(crate) fn face(
    key: &str,
    title: &str,
    answer: &str,
    kind: &'static str,
    labels: &[String],
    assignee: Option<&str>,
    urgent: bool,
    line: Element,
) -> Element {
    let who = assignee.map(|a| {
        (
            a,
            a.chars().next().unwrap_or('?').to_uppercase().to_string(),
        )
    });
    rsx! {
        div { class: "li-id", "{key}" }
        match who {
            Some((name, initial)) => rsx! { div { class: "li-assign", title: "{name}", Avatar { initials: initial, look: "li-who" } } },
            None => rsx! { div { class: "li-assign", title: "No assignee", {icon(PERSON)} } },
        }
        div { class: "li-title", {status(kind)} span { "{clamped(title)}" } }
        if !answer.is_empty() {
            div { class: "li-answer", "{answer}" }
        }
        div { class: "li-tags",
            if urgent {
                div { class: "li-urgent", "!" }
            }
            for label in labels {
                Badge { look: "li-tag", "{label}" }
            }
        }
        div { class: "meta", {line} }
    }
}

#[component]
pub(crate) fn Linear() -> Element {
    // No document on the web's server render: the cards draw, and only the window's drag moves them.
    let document = use_hook(|| CopyValue::new(try_consume_context::<Rc<RefCell<BaseDocument>>>()));
    let mut cards = use_signal(columns);
    let mut lifted = use_signal(|| None::<u32>);
    let mut moved = use_signal(|| false);
    let mut pressed = use_signal(|| (0.0, 0.0));
    let mut at = use_signal(|| (0.0, 0.0));
    let mut over = use_signal(|| None::<usize>);
    // A card let go: the ghost stays, its transition carries it to its slot and then hides it, as the card itself appears
    // there (design.css). The next press removes it.
    let mut settling = use_signal(|| None::<(Card, (f64, f64, f64))>);
    let mut shift = use_signal(|| None::<Shift>);
    let mut drop_in = move |to: usize| {
        let Some(id) = lifted() else { return };
        if !moved() {
            return;
        }
        let mut all = cards.write();
        let Some(from) = all.iter().position(|c| c.iter().any(|i| i.id == id)) else {
            return;
        };
        if from == to {
            return;
        }
        let i = all[from]
            .iter()
            .position(|x| x.id == id)
            .expect("the card is in its column");
        let card = all[from].remove(i);
        all[to].insert(0, card);
        let Some(doc) = document.read().clone() else {
            return;
        };
        let doc = doc.borrow();
        // No read means no flight and no easing, never a lost drop. Still the old layout: the render comes after this.
        settling.set(first_slot(&doc, to).map(|slot| (card, slot)));
        shift.set(
            super::families::rect(&doc, &format!("#li-card-{id}")).map(|r| Shift {
                from: (from, i),
                to,
                by: r.3 + CARD_GAP,
            }),
        );
    };
    let ghost = lifted()
        .filter(|_| moved())
        .and_then(|id| cards.read().iter().flatten().find(|c| c.id == id).copied());
    let carried = ghost
        .map(|c| (c, (at().0 - 150.0, at().1 - 20.0, 300.0), "li-ghost"))
        .or_else(|| settling().map(|(c, to)| (c, to, "li-ghost settling")));
    rsx! {
        div { class: "li",
            onmousemove: move |e: MouseEvent| {
                if lifted.peek().is_none() {
                    return;
                }
                if e.held_buttons().is_empty() {
                    lifted.set(None);
                    over.set(None);
                    return;
                }
                let p = e.client_coordinates();
                at.set((p.x, p.y));
                let from = *pressed.peek();
                if (p.x - from.0).hypot(p.y - from.1) >= DRAG_FROM {
                    moved.set(true);
                }
            },
            onmouseup: move |_| {
                lifted.set(None);
                over.set(None);
            },
            div { class: "li-side",
                div { class: "li-work", div { class: "li-mark", "AA" } "aaa" span { class: "spring" } {icon(SEARCH)} }
                for (name, d, n) in SIDE {
                    div { key: "{name}", class: "li-item", {icon(d)} span { class: "ds-grow", "{name}" } span { class: "meta", "{n}" } }
                }
                div { class: "li-head", "Your teams" }
                for (name, on) in TEAM {
                    div { key: "t-{name}", class: if on { "li-item sub on" } else { "li-item sub" }, "{name}" }
                }
            }
            div { class: "li-main",
                div { class: "li-top", b { "Aaa" } span { class: "meta", "›" } b { "Issues" } span { class: "spring" } {icon(PLUS)} "New issue" }
                div { class: "li-bar",
                    span { class: "li-chip on", "Active" } span { class: "li-chip", "Backlog" } span { class: "li-chip", "All issues" }
                    span { class: "spring" }
                    div { class: "li-round", {icon(DOTS)} }
                }
                div { class: "li-columns",
                    for (c, (name, kind)) in COLUMNS.into_iter().enumerate() {
                        div { id: "li-col-{c}", key: "{name}",
                            class: if over() == Some(c) { "li-column over" } else { "li-column" },
                            onmouseenter: move |_| if lifted.peek().is_some() { over.set(Some(c)) },
                            onmouseleave: move |_| over.set(None),
                            onmouseup: move |_| drop_in(c),
                            div { id: "li-head-{c}", class: "li-column-head", {status(kind)} "{name}" span { id: "li-count-{c}", class: "meta", "{cards.read()[c].len()}" } span { class: "spring" } {icon(DOTS)} {icon(PLUS)} }
                            for (i, card) in cards.read()[c].iter().copied().enumerate() {
                                div { id: "li-card-{card.id}", key: "{card.id}",
                                    class: if lifted() == Some(card.id) { "li-card lifted" } else if settling().is_some_and(|(c, _)| c.id == card.id) { "li-card arriving" } else if shift().and_then(|s| s.of(c, i)).is_some() { "li-card shifted" } else { "li-card" },
                                    style: shift().and_then(|s| s.of(c, i)).map(|by| format!("--shift:{by}px")),
                                    onmousedown: move |e: MouseEvent| {
                                        let p = e.client_coordinates();
                                        at.set((p.x, p.y));
                                        pressed.set((p.x, p.y));
                                        moved.set(false);
                                        settling.set(None);
                                        shift.set(None);
                                        lifted.set(Some(card.id));
                                    },
                                    {card_body(card, kind)}
                                }
                            }
                        }
                    }
                    div { class: "li-hidden",
                        div { class: "meta", "Hidden columns" }
                        div { class: "li-hidden-row", {status("review")} span { class: "ds-grow", "In Review" } span { class: "meta", "0" } }
                    }
                }
            }
        }
        // One element carried and settling, so the style changes on it and its transition runs.
        if let Some((card, (x, y, w), class)) = carried {
            div { id: "li-ghost", class, style: "left:{x}px;top:{y}px;width:{w}px",
                {card_body(card, COLUMNS[cards.read().iter().position(|c| c.contains(&card)).expect("the card is in a column")].1)}
            }
        }
    }
}
