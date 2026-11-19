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
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A field of several lines: a draft's « Request corrections », its text when he changes it (`text`, the words it opens on).
/// The document component's reduced mode draws it, `text` its document (DocField).
#[component]
pub fn Textarea(
    #[props(default, into)] text: String,
    #[props(extends = GlobalAttributes, extends = textarea)] attributes: Vec<Attribute>,
) -> Element {
    let doc = crate::design::notion_doc::message(&[(&text, "")]);
    super::doc_field::doc_field(doc, true, attributes)
}
