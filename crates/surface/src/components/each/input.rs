use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "input",
        classes: &["input"],
        tags: &["input"],
        name: "Input",
        used: "The search bar's field, the menu's fold.",
        example: Some(|_| {
            rsx! { Input { r#type: "search", name: "q", placeholder: "Search tasks" } }
        }),
        menu: true,
        ..Component::BARE
    }
}

/// A field: any input attribute passes through (`r#type`, `name`, `placeholder`, `hidden`).
#[component]
pub fn Input(
    #[props(extends = GlobalAttributes, extends = input)] attributes: Vec<Attribute>,
) -> Element {
    rsx! { input { class: "input", ..attributes } }
}
