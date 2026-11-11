use super::badge::Badge;
use super::parts::joined;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "list",
        classes: &[],
        tags: &[],
        name: "List",
        used: "The default list: home's suggestions, /inbox, /today, /agents, /tools, a row each with its title, its line and its link; a tree's « list » draws its cards this way.",
        example: Some(|_| {
            rsx! {
                div { id: "c-list", class: "list",
                    Item { id: "c-list-a", look: "issue-row", href: "/inbox".to_string(),
                        span { class: "title", "Inbox" }
                        span { class: "says", "2 wait on you" }
                    }
                    Item { id: "c-list-b", look: "issue-row",
                        span { class: "title", "Plan" }
                        span { class: "says", "now, next, later" }
                    }
                }
            }
        }),
        ..Component::BARE
    }
}

inventory::submit! {
    Component {
        slug: "item",
        classes: &["issue-row", "item", "access"],
        tags: &[],
        name: "Item",
        used: "The tools page's accesses, a row each; the tasks list's rows, look issue-row, each its task's link.",
        example: Some(|_| {
            rsx! {
                Access { id: "c-item", status: "works", name: "fly"
                }
                Item { id: "c-row", look: "issue-row", href: "/tasks/a".to_string(), task: "a", b { "a" } " Port the board" }
            }
        }),
        ..Component::BARE
    }
}

/// One row of a list: /tools' accesses, /tasks' rows (a link when it has `href`). `status` is `data-status`, which a look's
/// colours read (components.css); `task` is `data-task`, as Card's.
#[component]
pub fn Item(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default, into)] status: Option<String>,
    #[props(default)] href: Option<String>,
    #[props(default, into)] task: Option<String>,
    children: Element,
) -> Element {
    let class = joined("item", look);
    match href {
        Some(href) => {
            rsx! { a { id, class, "data-status": status, "data-task": task, href, {children} } }
        }
        None => rsx! { div { id, class, "data-status": status, "data-task": task, {children} } },
    }
}

/// One access on /tools: its state's pill, then its name with room between them (task design-access-pill; @nha,
/// 2026-11-09: « "works" pill eats the space next to fly and overlaps »), then what it used, why and where from. The design
/// page draws this same component, never a copy of its markup.
#[component]
pub fn Access(
    #[props(into)] id: String,
    #[props(into)] status: String,
    #[props(into)] name: String,
    #[props(default, into)] usage: String,
    #[props(default, into)] note: String,
    #[props(default, into)] place: String,
    #[props(default, into)] source: String,
) -> Element {
    rsx! {
        Item { id, look: "access", status: status.clone(),
            p { class: "head", Badge { look: "chip", "{status}" } b { class: "name", "{name}" } }
            p { class: "usage", "{usage}" }
            p { class: "note", "{note}" }
            p { class: "place", span { class: "source", "{source}" } " {place}" }
        }
    }
}
