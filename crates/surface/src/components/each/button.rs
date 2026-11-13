use super::parts::joined;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "button",
        classes: &["button"],
        tags: &["button"],
        name: "Button",
        used: "The search bar's button, the menu's compass (icon), the comment box's Send and Cancel.",
        example: Some(|_| rsx! { Button { "Search" } Button { look: "quiet", r#type: "button", "Cancel" } Button { look: "icon", r#type: "button", aria_label: "Menu", "☰" } }),
        menu: true,
        heroui: Some(("button", "restyled", "Petal")),
        ..Component::BARE
    }
}

/// A button: it submits the form around it unless given `r#type: "button"`. Any button attribute passes through.
#[component]
pub fn Button(
    #[props(default)] look: &'static str,
    #[props(extends = GlobalAttributes, extends = button)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! { button { class: joined("button", look), ..attributes, {children} } }
}

inventory::submit! {
    Component {
        slug: "button-link",
        classes: &[],
        tags: &[],
        name: "ButtonLink",
        used: "A link drawn as a button: an action that opens a page, as a draft's Change and Cancel.",
        example: Some(|_| rsx! { ButtonLink { look: "quiet", href: "/drafts/a-first?edit=1", "Change" } }),
        ..Component::BARE
    }
}

/// A link drawn as a button: an action that opens a page, a draft's Change and its Cancel (task rust-draft-state-edit).
#[component]
pub fn ButtonLink(
    #[props(default)] look: &'static str,
    #[props(into)] href: String,
    children: Element,
) -> Element {
    rsx! { a { class: joined("button", look), href, {children} } }
}
