use super::form::Form;
use super::input::Input;
use super::kbd::{Kbd, KbdGroup};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "ask",
        classes: &["ask"],
        tags: &[],
        name: "Ask",
        used: "The menu's Search or ask field, beside the compass on every page: ⌘ K focuses it, Enter searches /search. Built on Input and Kbd.",
        example: Some(|_| rsx! { Ask { id: "c-ask" } }),
        library: Some("https://dioxuslabs.github.io/dioxus-components/component/?name=input"),
        menu: true,
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// The menu's « Search or ask » field, beside the compass on every page (task rust-notion-one-menu; @nha, 2026-11-10: « the
/// "ask" field is a good idea though it needs to live somewhere else »): it GETs /search on Enter, ⌘ K shown in it and
/// focusing it (forms.rs's NAV_JS). Its name is a word of the text, so an agent reads what the field is; the keys sit outside
/// its label, so a screen reader names it « Search or ask » alone (/code-review).
#[component]
pub fn Ask(#[props(into)] id: Option<String>) -> Element {
    rsx! {
        Form { id, role: "search", method: "get", action: "/search",
            div { class: "ask",
                label {
                    span { class: "sr-only", "Search or ask" }
                    Input { r#type: "search", name: "q", placeholder: "Search or ask" }
                }
                KbdGroup { Kbd { "⌘" } Kbd { "K" } }
            }
        }
    }
}
