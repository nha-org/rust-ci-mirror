use super::avatar::Logo;
use super::card::CardTitle;
use super::item::Access;
use super::skeleton::Skeleton;
use crate::component::Component;
use dioxus::prelude::*;

/// A tool on /tools, a section of the list and no card (@nha, 2026-11-09: « commands tools, etc have no reason to use cards »): its
/// logo and name, then its accesses. While its provider answers, it is a Skeleton of `reading`
/// grey lines under its head (@nha, 2026-11-09: « cards greyed out show a loading state »).
#[component]
pub fn Tool(
    #[props(into)] id: String,
    #[props(into)] name: String,
    #[props(default)] reading: usize,
    children: Element,
) -> Element {
    let head = rsx! { p { class: "head", Logo { name: name.clone() } CardTitle { "{name}" } } };
    if reading == 0 {
        return rsx! { div { id, class: "tool", {head} {children} } };
    }
    rsx! { Skeleton { id, what: name, lines: reading, look: "tool", {head} } }
}

inventory::submit! {
    Component {
        slug: "live",
        classes: &["live", "tool"],
        tags: &[],
        name: "Live",
        used: "A part a skeleton in it refills whole: the tools page's themes, whose cards appear as each provider answers. With a view, a part drawn again at each diff of that view: /needs-you.",
        example: Some(|_| {
            rsx! {
                Live { id: "c-live",
                    Tool { id: "c-tool", name: "fly",
                        Access { id: "c-tool-item", status: "works", name: "fly" }
                    }
                    Tool { id: "c-live-reading", name: "fly", reading: 2 }
                }
            }
        }),
        ..Component::BARE
    }
}

/// A part of a page refilled whole while a skeleton in it is busy: /tools' themes, whose cards appear as providers answer.
/// `view`: the views' name it follows, its `data-view`: the page's LIVE_JS takes each diff of that view as a patch of this
/// part, drawn again whole (/needs-you, task rust-needs-you-view-page).
#[component]
pub fn Live(
    #[props(into)] id: String,
    #[props(default)] view: Option<String>,
    children: Element,
) -> Element {
    rsx! { div { id, class: "live", "data-view": view, {children} } }
}
