use super::parts::joined;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "badge",
        classes: &["badge", "chip", "count", "li-tag"],
        tags: &[],
        name: "Badge",
        used: "An access's state, a column's count, a card's labels.",
        example: Some(|_| {
            rsx! { Badge { look: "chip", "works" } Badge { look: "count", "4" } Badge { look: "li-tag", "rust" } }
        }),
        heroui: Some(("chip", "restyled", "badge")),
        ..Component::BARE
    }
}

/// A short label: a state, a count, a tag.
#[component]
pub fn Badge(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { span { class: joined("badge", look), {children} } }
}
