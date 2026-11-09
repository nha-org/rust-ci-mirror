//! The components every page draws with, each one of dioxuslabs.com/components ported to design.json's tokens (@nha,
//! 2026-11-09: « our design page should have https://dioxuslabs.com/components or equivalents either use or port what we
//! need », « We want all components to be dual-use »). Dual-use: each is a Dioxus element, so the window draws it and the
//! web's server render is its HTML, whose text form (crate::forms::text_of) is what an agent reads. /design?tab=components
//! (design/catalog.rs) shows each one's two forms side by side. `look` picks a component's look: its CSS class, never written by a page.
//! The gate (dash-gate's `page_local_markup`) refuses OWNED_TAGS and OWNED_CLASSES anywhere but here.
use dioxus::prelude::*;

/// The components' looks, which every page and /design load, and the menu scopes (crate::forms).
pub const CSS: &str = include_str!("components.css");

/// The elements a component here draws: a page writes `Button {}`, never `button {}`.
pub const OWNED_TAGS: [&str; 3] = ["button", "input", "textarea"];
/// The classes a component here draws, each a look a page passes as `look`.
pub const OWNED_CLASSES: [&str; 14] = [
    "textarea",
    "card",
    "description",
    "item",
    "access",
    "badge",
    "chip",
    "count",
    "li-tag",
    "avatar",
    "li-who",
    "button",
    "input",
    "tabs",
];

/// A component's own class, then its look's, if any.
fn joined(base: &str, look: &str) -> String {
    match look {
        "" => base.to_string(),
        look => format!("{base} {look}"),
    }
}

/// A card: a link when it has `href`.
#[component]
pub fn Card(
    #[props(into)] id: String,
    #[props(default)] href: Option<String>,
    children: Element,
) -> Element {
    match href {
        Some(href) => rsx! { a { id, class: "card", href, {children} } },
        None => rsx! { div { id, class: "card", {children} } },
    }
}

/// One row of a list: /tools' accesses. `status` is `data-status`, which a look's colours read (components.css).
#[component]
pub fn Item(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default, into)] status: Option<String>,
    children: Element,
) -> Element {
    rsx! { div { id, class: joined("item", look), "data-status": status, {children} } }
}

#[component]
pub fn CardTitle(children: Element) -> Element {
    rsx! { b { class: "title", {children} } }
}

#[component]
pub fn CardDescription(children: Element) -> Element {
    rsx! { span { class: "description", {children} } }
}

/// A short label: a state, a count, a tag.
#[component]
pub fn Badge(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { span { class: joined("badge", look), {children} } }
}

/// Someone's initials in a circle.
#[component]
pub fn Avatar(#[props(into)] initials: String, #[props(default)] look: &'static str) -> Element {
    rsx! { span { class: joined("avatar", look), "{initials}" } }
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

/// A field: any input attribute passes through (`r#type`, `name`, `placeholder`, `hidden`).
#[component]
pub fn Input(
    #[props(extends = GlobalAttributes, extends = input)] attributes: Vec<Attribute>,
) -> Element {
    rsx! { input { class: "input", ..attributes } }
}

/// A field of several lines: the compass's comment box.
#[component]
pub fn Textarea(
    #[props(extends = GlobalAttributes, extends = textarea)] attributes: Vec<Attribute>,
) -> Element {
    rsx! { textarea { class: "textarea", ..attributes } }
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
