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
        terminal: Some(crate::component::Terminal::Drawn(|_| {
            let (works, four, rust) = (badge("chip", "works"), badge("count", "4"), badge("li-tag", "rust"));
            ratatui::text::Line::from(vec![works, " ".into(), four, " ".into(), rust]).into()
        })),
        ..Component::BARE
    }
}

/// A short label: a state, a count, a tag.
#[component]
pub fn Badge(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { span { class: joined("badge", look), {children} } }
}

/// A badge in the terminal, its look as a style: a chip bold, a count dim, a tag in brackets (task rust-tui-tracer).
pub fn badge(look: &str, words: &str) -> ratatui::text::Span<'static> {
    use ratatui::style::Stylize;
    match look {
        "chip" => words.to_string().bold(),
        "count" => words.to_string().dim(),
        _ => format!("[{words}]").into(),
    }
}
