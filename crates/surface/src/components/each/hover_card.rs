use super::badge::Badge;
use super::card::{CardDescription, CardTitle};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "hover_card",
        classes: &["hover-card", "hover-card-content"],
        tags: &[],
        name: "HoverCard",
        used: "A tasks board card: resting on it shows its task's title and why under it.",
        example: Some(|_| {
            rsx! {
                HoverCard { open: true,
                    content: rsx! { CardTitle { "Port the board" } CardDescription { "The old board drifts from the design system." } },
                    Badge { look: "li-tag", "AAA-12" }
                }
            }
        }),
        ..Component::BARE
    }
}

/// More about a thing, shown while the pointer rests on it or a key focuses it: dioxus-components' HoverCard (notice in
/// NOTICE-dioxus-components), its content drawn under its trigger after a short wait, by CSS alone, no script. Upstream's
/// `tooltip` role is left out: shown, the content is part of what holds it (a board card's link), which names it.
/// `children` are the trigger, `content` the card. Its content is `hidden` but on a hover, so the text, as a closed
/// DropdownMenu's, holds the trigger's words alone (a board card's face; its task's page holds the rest). `open` draws it
/// open, for the catalog.
#[component]
pub fn HoverCard(#[props(default)] open: bool, content: Element, children: Element) -> Element {
    let state = if open { "open" } else { "closed" };
    rsx! {
        div { class: "hover-card", "data-state": state,
            {children}
            div { class: "hover-card-content", "data-state": state, hidden: !open, {content} }
        }
    }
}
