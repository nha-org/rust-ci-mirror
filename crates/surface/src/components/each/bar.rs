use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "bar",
        classes: &["bar"],
        tags: &[],
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A page's top line, Linear's view head (board_live.ex's `view_head`): its parts in a row, the first at the left, the rest
/// at the right end (the board's title, then its « + New issue »).
#[component]
pub fn Bar(#[props(into)] id: String, children: Element) -> Element {
    rsx! { div { id, class: "bar", {children} } }
}
