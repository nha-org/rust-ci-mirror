use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "switch",
        classes: &["switch", "switch-input", "switch-track", "switch-thumb"],
        tags: &[],
        name: "Switch",
        used: "An on or off choice in a form: a tap flips it, the form sends it, no script. No page holds one yet.",
        example: Some(|_| {
            rsx! {
                Switch { id: "c-switch-on", name: "notify", checked: true, "Notify me" }
                Switch { id: "c-switch-off", name: "digest", checked: false, "Daily digest" }
            }
        }),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// An on or off choice: dioxus-components' Switch (notice in NOTICE-dioxus-components), its track and thumb kept, its button
/// and hidden input made one checkbox with `role="switch"`, so a form sends it and a tap flips it with no script, on the web
/// and in the window. `children` are its words, the whole of it one label. ponytail: its text is its words alone, not its
/// state; a state word when a page's agent needs it.
#[component]
pub fn Switch(
    #[props(into)] id: String,
    name: &'static str,
    checked: bool,
    #[props(default)] disabled: bool,
    children: Element,
) -> Element {
    // a present `checked` or `disabled`, even « false », sets it: only a set one carries one
    let (checked, disabled) = (checked.then_some(true), disabled.then_some(true));
    rsx! {
        label { class: "switch",
            input { id, class: "switch-input", r#type: "checkbox", role: "switch", name, checked, disabled }
            span { class: "switch-track", "aria-hidden": "true", span { class: "switch-thumb" } }
            {children}
        }
    }
}
