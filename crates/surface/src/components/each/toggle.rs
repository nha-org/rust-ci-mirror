use super::parts::{BOARD, LIST, icon};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "toggle",
        classes: &["toggle"],
        tags: &[],
        name: "Toggle",
        used: "One choice of several, the shown one pressed, each its icon beside its name: the tasks' Board, List and Graph views.",
        example: Some(|_| {
            rsx! {
                Toggle { id: "c-toggle", label: "Views",
                    ToggleOption { href: "/tasks", current: true, icon: icon(BOARD), "Board" }
                    ToggleOption { href: "/tasks?layout=list", current: false, icon: icon(LIST), "List" }
                }
            }
        }),
        menu: true,
        ..Component::BARE
    }
}

/// One choice of several, each a link, the shown one pressed: /design's « One of several » toggle group as a component
/// (task rust-tasks-views-toggle; @nha, 2026-11-09: « list/graph => use the design system toggle group, or better the logos etc
/// from the original design »). Its options are `ToggleOption`s.
#[component]
pub fn Toggle(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "toggle", "aria-label": label, {children} } }
}

/// One option of a `Toggle`: its icon beside its name, `aria-current` on the one shown.
#[component]
pub fn ToggleOption(
    #[props(into)] href: String,
    current: bool,
    icon: Element,
    children: Element,
) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, "aria-current": on, {icon} {children} } }
}
