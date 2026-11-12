//! Kbd and KbdGroup: rust-ui's Kbd (notice in NOTICE-rust-ui), which dioxus-components lacks. The first component in its own
//! module (task rust-components-register): its registration, its look and its code here, no other file naming it.
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "kbd",
        classes: &["kbd", "kbd-group"],
        tags: &["kbd"],
        name: "Kbd",
        used: "A key to press: ⌘ K in the menu's Search or ask field (Ask). Built on rust-ui's Kbd, which dioxus-components lacks.",
        example: Some(|_| rsx! { KbdGroup { Kbd { "⌘" } Kbd { "K" } } Kbd { "Esc" } }),
        library: Some("https://rust-ui.com/docs/components/kbd"),
        css: include_str!("kbd.css"),
        menu: true,
        ..Component::BARE
    }
}

/// A key to press: rust-ui's Kbd on this system's tokens, a small keycap no pointer selects.
#[component]
pub fn Kbd(children: Element) -> Element {
    rsx! { kbd { class: "kbd", {children} } }
}

/// Keys pressed together, as rust-ui's KbdGroup: `Kbd`s side by side.
#[component]
pub fn KbdGroup(children: Element) -> Element {
    rsx! { kbd { class: "kbd-group", {children} } }
}
