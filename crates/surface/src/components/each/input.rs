use crate::component::Component;
use dioxus::prelude::*;
use serde_json::Value;

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
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// Input types whose value is no words: drawn as they are, never read into a document.
const NOT_WORDS: [&str; 13] = [
    "hidden",
    "checkbox",
    "radio",
    "file",
    "date",
    "datetime-local",
    "time",
    "month",
    "week",
    "color",
    "range",
    "submit",
    "reset",
];

/// A field: any input attribute passes through (`r#type`, `name`, `placeholder`, `hidden`). The document component's reduced
/// mode draws it (DocField): a field of words holds its `value` as a document.
#[component]
pub fn Input(
    #[props(extends = GlobalAttributes, extends = input)] attributes: Vec<Attribute>,
) -> Element {
    let text = |a: &Attribute| match &a.value {
        dioxus::core::AttributeValue::Text(t) => Some(t.clone()),
        _ => None,
    };
    let kind = attributes.iter().find(|a| a.name == "type").and_then(text);
    if kind.is_some_and(|k| NOT_WORDS.contains(&k.as_str())) {
        return super::doc_field::doc_field(Value::Null, false, attributes);
    }
    let (value, attributes): (Vec<Attribute>, Vec<Attribute>) = attributes
        .into_iter()
        .partition(|a| a.name == "value" && text(a).is_some());
    // no value given: no document, so the field carries no `value` attribute, as before
    let doc = (value.first().and_then(text)).map_or(Value::Null, |words| {
        crate::design::notion_doc::message(&[(&words, "")])
    });
    super::doc_field::doc_field(doc, false, attributes)
}
