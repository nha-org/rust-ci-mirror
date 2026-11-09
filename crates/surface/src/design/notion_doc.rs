//! The Notion tab's page as a canvist_model document (task rust-doc-model-canvist; @nha, 2026-11-09: « I suggest using
//! https://canvist.dev/docs/crates/canvist_model as the data structure if that makes sense. »). Its blocks keep stable ids,
//! so a comment can anchor on (block id, byte range), both ends mapped through each commit; it saves as ProseMirror JSON,
//! and its text form is derived from the blocks, never kept beside them. The tab still draws LINES: the editor that draws
//! this document is task editor-library's.
use super::notion::LINES;
use canvist_model::{Attrs, BlockSpec, Document, JsonError, MarkSpec, Schema};
use serde_json::{Value, json};
use std::sync::Arc;

/// The page's title, then bullets: an `item` holds its `line` of text and the items under it, so moving a bullet moves
/// what hangs from it. A link keeps its target in `href`, as ProseMirror's link mark does.
fn schema() -> Arc<Schema> {
    let schema = Schema::builder()
        .block(BlockSpec::text("title").groups(&["block"]))
        .block(BlockSpec::container("item", &["line", "item"]).groups(&["block"]))
        .block(BlockSpec::text("line"))
        .mark(MarkSpec::new("link").attrs(Attrs::new().with("href", "")))
        .build()
        .expect("a title, items holding lines and items, and a link mark are a valid schema");
    Arc::new(schema)
}

fn line(words: &str, link: bool) -> Value {
    let mut text = json!({ "type": "text", "text": words });
    if link {
        text["marks"] = json!([{ "type": "link", "attrs": { "href": "" } }]);
    }
    json!({ "type": "line", "content": [text] })
}

/// The tab's page as it is saved: ProseMirror JSON, the d0 line the title, each d1 an item, each d2 an item inside the
/// d1 above it.
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

/// The text form agents read: a line a text block, two spaces for each item it sits under, past the first.
pub(crate) fn text(doc: &Document) -> String {
    doc.text_blocks()
        .map(|block| {
            let depth = std::iter::successors(block.parent(), |&p| doc.block(p)?.parent()).count();
            let bullet = if depth == 0 {
                String::new()
            } else {
                format!("{}- ", "  ".repeat(depth - 1))
            };
            format!("{bullet}{}", block.inline().map_or("", |t| t.as_str()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvist_model::{Bias, MarkSet, Point};
    use dash_meter::memory::TestBudget;

    #[test]
    fn the_tab_page_loads_edits_and_saves_through_canvist() {
        let _budget = TestBudget::new();
        let mut doc = load(&seed(), 1).expect("the seed fits the schema");
        let before = text(&doc);
        assert!(
            before.starts_with("2026:\n- One tree, two renderers\n  - the window and the web"),
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
        assert!(edited.contains("\n- Memory budget\n"), "{edited}");

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
                    - One tree, two renderers\n  \
                    - the window and the web draw the same components\n  \
                    - theme.css generated from design.json, shared by both\n  \
                    - each behaviour is checked against the live document\n\
                    - Memory\n\
                    - Frame time\n\
                    - Fonts\n  \
                    - one face a window\n  \
                    - matches on family name, then falls back\n\
                    - Selection";
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
        assert_eq!(text(&doc), page.replace("- Selection", "- Cursor"));
    }
}
