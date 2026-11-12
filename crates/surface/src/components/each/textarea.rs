use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "textarea",
        classes: &["textarea"],
        tags: &["textarea"],
        name: "Textarea",
        used: "A draft's Request corrections; every compose box is the documents editor (Composer).",
        example: Some(|_| {
            rsx! { Textarea { name: "text", rows: "3", placeholder: "Your comment", required: true } }
        }),
        menu: true,
        ..Component::BARE
    }
}

/// A field of several lines: a draft's « Request corrections », its text when he changes it (`text`, the words it opens on).
#[component]
pub fn Textarea(
    #[props(default, into)] text: String,
    #[props(extends = GlobalAttributes, extends = textarea)] attributes: Vec<Attribute>,
) -> Element {
    // a parser drops one newline right after <textarea>: a text opening on a blank line keeps it (/code-review)
    let lead = if text.starts_with('\n') { "\n" } else { "" };
    rsx! { textarea { class: "textarea", ..attributes, "{lead}{text}" } }
}
