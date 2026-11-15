//! The document component's reduced mode (task rust-doc-rich-blocks): a form field is a document of the one model
//! (rust/CLAUDE.md rule 7; @nha, 2026-11-09: « we need to have _one_ model, _one_ component that can turn into views depending
//! on the UI. »), drawn as the browser's own field, so a form posts it and the window types in it with no script. Textarea and
//! Input draw through it; dash-gate's `every_text_field_is_the_document_component` refuses a field drawn anywhere else.
use crate::design::notion_doc;
use dioxus::prelude::*;
use serde_json::Value;

/// One line (an `input`) or several (`lines`, a `textarea`), its value the words `doc` holds (notion_doc's `message`), the
/// public words a send reads; Null for a field whose value is no words (a checkbox, a hidden field), its `value` attribute
/// passed as it is. Listeners ride in its attributes as any other (`oninput`, `onkeydown`), heard in the window.
#[component]
pub fn DocField(
    #[props(default)] doc: Value,
    #[props(default)] lines: bool,
    #[props(extends = GlobalAttributes, extends = input)] attributes: Vec<Attribute>,
) -> Element {
    doc_field(doc, lines, attributes)
}

/// DocField for a component that collected its own attributes (Textarea's, Input's): a component takes no spread.
pub fn doc_field(doc: Value, lines: bool, attributes: Vec<Attribute>) -> Element {
    // a document's words, even one the model refuses (/code-review: an empty field saved over a draft); Null holds none
    let text = match &doc {
        Value::Null => None,
        doc => Some(notion_doc::sent(doc).unwrap_or_else(|_| notion_doc::words_in(doc))),
    };
    if lines {
        let text = text.unwrap_or_default();
        // a parser drops one newline right after <textarea>: a text opening on a blank line keeps it (/code-review)
        let lead = if text.starts_with('\n') { "\n" } else { "" };
        return rsx! { textarea { class: "textarea", ..attributes, "{lead}{text}" } };
    }
    // set whenever a document is given, empty too, so a field its owner empties in code is emptied (/code-review: the Notion
    // tab's comment box kept its words after Post); none for no document, an input of no words passing its own
    rsx! { input { class: "input", value: text, ..attributes } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Input, Textarea};
    use dash_meter::dash_test;

    fn html(e: Element) -> String {
        dioxus_ssr::render_element(e)
    }

    /// Done-when 2 of rust-doc-rich-blocks: Textarea (textarea.rs) and the menu's field (forms.rs, through Ask's Input) are the
    /// document component's reduced mode: each draws what DocField draws of its words as a document, the words as written.
    #[dash_test]
    fn textarea_and_the_menu_s_field_draw_the_document_component() {
        let words = "\nDear Ada,\n\n\n  the payment comes.";
        let doc = notion_doc::message(&[(words, "")]);
        let area = html(rsx! { Textarea { name: "body", text: words } });
        assert_eq!(
            area,
            html(rsx! { DocField { doc: doc.clone(), lines: true, name: "body" } })
        );
        assert!(
            area.ends_with(">\n\nDear Ada,\n\n\n  the payment comes.</textarea>"),
            "{area}"
        );

        let menu = crate::forms::with_menu("<html><body></body></html>", "/", "");
        let field =
            html(rsx! { DocField { name: "q", placeholder: "Search or ask", r#type: "search" } });
        assert_eq!(
            field,
            r#"<input class="input" name="q" placeholder="Search or ask" type="search"/>"#
        );
        assert!(menu.contains(&field), "{field} in {menu}");

        // an empty document still sets the value, so a field its owner empties in code is emptied (/code-review)
        assert_eq!(
            html(rsx! { DocField { doc: notion_doc::message(&[("", "")]), name: "q" } }),
            r#"<input class="input" value="" name="q"/>"#
        );
        // a document the model refuses still gives its words, never an empty field saved over them (/code-review)
        let odd = serde_json::json!({ "type": "doc", "content": [{ "type": "no-such-block", "content": [{ "type": "text", "text": "Dear Ada," }] }] });
        assert_eq!(
            html(rsx! { DocField { doc: odd, lines: true, name: "body" } }),
            r#"<textarea class="textarea" name="body">Dear Ada,</textarea>"#
        );
        // a field's words are its document's; a field of no words (hidden) passes its value as it is
        assert_eq!(
            html(rsx! { Input { r#type: "search", name: "q", value: "rate" } }),
            html(
                rsx! { DocField { doc: notion_doc::message(&[("rate", "")]), r#type: "search", name: "q" } }
            )
        );
        assert_eq!(
            html(rsx! { Input { r#type: "hidden", name: "tab", value: "data" } }),
            r#"<input class="input" name="tab" type="hidden" value="data"/>"#
        );
    }
}
