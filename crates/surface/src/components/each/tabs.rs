use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "tabs",
        classes: &["tabs"],
        tags: &[],
        name: "Tabs",
        used: "The tasks board's views; a tree's « links »: /mail's accounts and pager, /settings' download.",
        example: Some(|_| {
            rsx! {
                Tabs { id: "c-tabs", label: "Views",
                    Tab { href: "/tasks", current: true, "List" }
                    Tab { href: "/tasks?layout=board", current: false, "Board" }
                }
            }
        }),
        ..Component::BARE
    }
}

/// One view of several, each a link: the server render runs no handler, so a tab changes the URL.
#[component]
pub fn Tabs(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "tabs", "aria-label": label, {children} } }
}

#[component]
pub fn Tab(#[props(into)] href: String, current: bool, children: Element) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, "aria-current": on, {children} } }
}
