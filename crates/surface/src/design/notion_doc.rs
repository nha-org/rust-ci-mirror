//! The one document model, a canvist_model document (task rust-doc-model-canvist; @nha, 2026-11-09: « I suggest using
//! https://canvist.dev/docs/crates/canvist_model as the data structure if that makes sense. »): the Notion tab's page and a
//! message draft alike (rust/CLAUDE.md rule 7, task rust-one-doc-component; @nha, 2026-11-09: « we need to have _one_ model,
//! _one_ component that can turn into views depending on the UI. »). Its blocks keep stable ids, so a comment can anchor on
//! (block id, byte range), both ends mapped through each commit; it saves as ProseMirror JSON. One component draws it,
//! `documents::DocView`, and its text form is read off that HTML. The tab still draws LINES: its comments move onto
//! DocView with task editor-library.
use super::notion::LINES;
use canvist_model::{Attrs, BlockSpec, Document, JsonError, MarkSpec, Schema};
use serde_json::{Value, json};
use std::sync::Arc;

/// A page's title, then bullets: an `item` holds its `line` of text and the items under it, so moving a bullet moves
/// what hangs from it. A message's `section` holds its public `line` and its private `note`, why it says that: only lines
/// are ever sent. A link keeps its target in `href`, as ProseMirror's link mark does.
fn schema() -> Arc<Schema> {
    let schema = Schema::builder()
        .block(BlockSpec::text("title").groups(&["block"]))
        .block(BlockSpec::container("item", &["line", "item"]).groups(&["block"]))
        .block(BlockSpec::container("section", &["line", "note"]).groups(&["block"]))
        .block(BlockSpec::text("line"))
        .block(BlockSpec::text("note"))
        .mark(MarkSpec::new("link").attrs(Attrs::new().with("href", "")))
        .build()
        .expect("a title, items, sections, lines, notes and a link mark are a valid schema");
    Arc::new(schema)
}

fn line(words: &str, link: bool) -> Value {
    let mut text = json!({ "type": "text", "text": words });
    if link {
        text["marks"] = json!([{ "type": "link", "attrs": { "href": "" } }]);
    }
    json!({ "type": "line", "content": [text] })
}

/// A message as it is saved: each section its public words and its private note (empty: none).
pub(crate) fn message(sections: &[(&str, &str)]) -> Value {
    let section = |&(words, why): &(&str, &str)| {
        let mut content = vec![line(words, false)];
        if !why.is_empty() {
            content.push(json!({ "type": "note", "content": [{ "type": "text", "text": why }] }));
        }
        json!({ "type": "section", "content": content })
    };
    json!({ "type": "doc", "content": sections.iter().map(section).collect::<Vec<_>>() })
}

/// An older draft read into the model: each paragraph of its text a section, with no note, since none was kept.
pub(crate) fn from_text(text: &str) -> Value {
    let paragraphs: Vec<(&str, &str)> = (text.split("\n\n").map(str::trim))
        .filter(|p| !p.is_empty())
        .map(|p| (p, ""))
        .collect();
    message(&paragraphs)
}

/// The tab's page as it is saved: ProseMirror JSON, the d0 line the title, each d1 an item, each d2 an item inside the
/// d1 above it.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "its first reader is the editor, task editor-library"
    )
)]
pub(crate) fn seed() -> Value {
    let mut content: Vec<Value> = vec![];
    for &(depth, words, link) in &LINES {
        let item = json!({ "type": "item", "content": [line(words, link)] });
        match (depth, content.last_mut()) {
            ("d0", _) => content
                .push(json!({ "type": "title", "content": [{ "type": "text", "text": words }] })),
            ("d2", Some(parent)) if parent["type"] == "item" => parent["content"]
                .as_array_mut()
                .expect("an item holds an array")
                .push(item),
            _ => content.push(item),
        }
    }
    json!({ "type": "doc", "content": content })
}

/// A saved page, checked against the tab's schema. `client` names who edits: canvist packs it into every block id it makes,
/// so two editors of one page each need their own. Saving is canvist's own `Document::to_json`.
pub(crate) fn load(saved: &Value, client: u32) -> Result<Document, JsonError> {
    Document::from_json(schema(), client, saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::{View, view_forms};
    use canvist_model::{Bias, MarkSet, Point};
    use dash_meter::memory::TestBudget;

    /// The text an agent reads of it: its Notion view's, read off the HTML.
    fn text(doc: &Document) -> String {
        view_forms(doc.to_json(), View::Notion).1
    }

    #[test]
    fn the_tab_page_loads_edits_and_saves_through_canvist() {
        let _budget = TestBudget::new();
        let mut doc = load(&seed(), 1).expect("the seed fits the schema");
        let before = text(&doc);
        assert!(
            before.starts_with("2026:\nOne tree, two renderers\nthe window and the web"),
            "{before}"
        );
        assert_eq!(before.lines().count(), LINES.len());

        // An edit: words typed at the end of the « Memory » bullet, its block id kept.
        let memory = doc
            .text_blocks()
            .find(|b| b.inline().is_some_and(|t| t.as_str() == "Memory"))
            .map(|b| b.id())
            .expect("the Memory bullet");
        let mut edit = doc.transaction();
        edit.insert_text(Point::new(memory, 6), " budget", MarkSet::empty())
            .expect("an insert inside the block");
        let commit = edit.commit();
        let edited = text(&doc);
        assert!(edited.contains("\nMemory budget\n"), "{edited}");

        // A comment's ends follow the edit: one on « Memory » keeps its end before the typed words.
        assert_eq!(
            commit.map(Point::new(memory, 6), Bias::Before).point,
            Point::new(memory, 6)
        );
        assert_eq!(
            commit.map(Point::new(memory, 6), Bias::After).point,
            Point::new(memory, 13)
        );

        // Saved and loaded back by a second editor: the same text, the same block id, the link and its href kept.
        let saved = doc.to_json();
        let again = load(&saved, 2).expect("a saved page loads");
        assert_eq!(text(&again), edited);
        assert!(
            again.block(memory).is_some(),
            "the block id survives a save"
        );
        let first_line = &saved["content"][1]["content"][0];
        assert_eq!(first_line["type"], "line");
        assert_eq!(first_line["content"][0]["marks"][0]["type"], "link");
        assert_eq!(first_line["content"][0]["marks"][0]["attrs"]["href"], "");

        // A subitem moves with its bullet: « Fonts » holds its two lines.
        let fonts = &saved["content"][4];
        assert_eq!(fonts["content"][0]["content"][0]["text"], "Fonts");
        assert_eq!(fonts["content"].as_array().map(Vec::len), Some(3));

        // Every commit carries its undo.
        doc.apply_steps(commit.undo_steps()).expect("undo");
        assert_eq!(text(&doc), before);
    }

    #[test]
    fn the_text_form_is_derived_from_the_document() {
        let _budget = TestBudget::new();
        let mut doc = load(&seed(), 1).expect("the seed fits the schema");
        let page = "2026:\n\
                    One tree, two renderers\n\
                    the window and the web draw the same components\n\
                    theme.css generated from design.json, shared by both\n\
                    each behaviour is checked against the live document\n\
                    Memory\n\
                    Frame time\n\
                    Fonts\n\
                    one face a window\n\
                    matches on family name, then falls back\n\
                    Selection\n";
        assert_eq!(text(&doc), page);

        // Derived, never stored: an edit to the document is in the next text form.
        let selection = doc
            .text_blocks()
            .last()
            .map(|b| b.id())
            .expect("a last line");
        let mut edit = doc.transaction();
        edit.delete_text(selection, 0..9)
            .expect("a delete inside the block");
        edit.insert_text(Point::new(selection, 0), "Cursor", MarkSet::empty())
            .expect("an insert at the start");
        let _ = edit.commit();
        assert_eq!(text(&doc), page.replace("\nSelection", "\nCursor"));
    }

    /// Done-when 1 of rust-one-doc-component: a draft is a document of the same schema, its sections' public words and
    /// private notes kept through a save; an older draft's text reads into it, a paragraph a section, no note.
    #[test]
    fn a_draft_is_a_document_of_public_sections_and_private_notes() {
        let _budget = TestBudget::new();
        let doc = load(
            &message(&[
                ("Bonjour Ana,", ""),
                (
                    "Une question courte.",
                    "She asked for a rate on the call: answer it first.",
                ),
            ]),
            1,
        )
        .expect("a message fits the schema");
        let saved = doc.to_json();
        let again = load(&saved, 2).expect("a saved message loads");
        let kinds: Vec<(&str, &str)> = again
            .text_blocks()
            .map(|b| {
                let kind = &*again.schema().block(b.kind()).name;
                (kind, b.inline().map_or("", |t| t.as_str()))
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("line", "Bonjour Ana,"),
                ("line", "Une question courte."),
                ("note", "She asked for a rate on the call: answer it first."),
            ]
        );
        assert_eq!(saved["content"][1]["type"], "section");

        // An older draft: its text as /d/<id> gave it, a paragraph's line breaks kept.
        let old = load(
            &from_text("Bonjour Ana,\n\nUne question\ncourte.\n\n\nNicolas"),
            1,
        )
        .expect("an older draft reads into the model");
        let lines: Vec<&str> = old
            .text_blocks()
            .map(|b| b.inline().map_or("", |t| t.as_str()))
            .collect();
        assert_eq!(lines, ["Bonjour Ana,", "Une question\ncourte.", "Nicolas"]);
        assert_eq!(old.children(None).len(), 3, "a section a paragraph");
    }
}
