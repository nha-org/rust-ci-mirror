//! The one document model, a canvist_model document (task rust-doc-model-canvist; @nha, 2026-11-09: « I suggest using
//! https://canvist.dev/docs/crates/canvist_model as the data structure if that makes sense. »): the Notion tab's page and a
//! message draft alike (rust/CLAUDE.md rule 7, task rust-one-doc-component; @nha, 2026-11-09: « we need to have _one_ model,
//! _one_ component that can turn into views depending on the UI. »). Its blocks keep stable ids, so a comment can anchor on
//! (block id, byte range), both ends mapped through each commit; it saves as ProseMirror JSON. One component draws it,
//! `documents::DocView`, and its text form is read off that HTML. The tab still draws LINES: its comments move onto
//! DocView with task editor-library. /today is one too: an item a line, `outline`.
use canvist_model::{Attrs, BlockSpec, Document, JsonError, MarkSpec, Schema};
use serde_json::{Value, json};
use std::sync::Arc;

/// The Notion tab's bullets: (depth, words, whether they link). Here, not in notion.rs, so this model reads no component and
/// the design system's fields may read it (components::DocField; a module loop otherwise).
pub(crate) const LINES: [(&str, &str, bool); 11] = [
    ("d0", "2026:", false),
    ("d1", "One tree, two renderers", true),
    (
        "d2",
        "the window and the web draw the same components",
        false,
    ),
    (
        "d2",
        "theme.css generated from design.json, shared by both",
        false,
    ),
    (
        "d2",
        "each behaviour is checked against the live document",
        false,
    ),
    ("d1", "Memory", false),
    ("d1", "Frame time", false),
    ("d1", "Fonts", true),
    ("d2", "one face a window", false),
    ("d2", "matches on family name, then falls back", false),
    ("d1", "Selection", false),
];
/// Under the bullets, the blocks a page holds besides lines (task rust-doc-rich-blocks), drawn by DocView from
/// `rich` below: a paragraph, its middle words bold,
pub(crate) const BOLD: (&str, &str, &str) = (
    "Each frame has a budget of ",
    "16 ms",
    ", measured in the window and on the web.",
);
/// a code block,
pub(crate) const CODE: &str = "cargo nextest run -p dash-ui frame_time\n  --run-ignored all";
/// and an image: frame times under the budget's line, an SVG data URL, so the page needs no file.
pub(crate) const FRAMES: &str = "data:image/svg+xml;charset=utf-8,%3Csvg xmlns='http://www.w3.org/2000/svg' \
    viewBox='0 0 240 60' width='240' height='60'%3E%3Cpath d='M0 20H240' stroke='%23d97706' stroke-dasharray='4 3'/%3E\
    %3Cpolyline fill='none' stroke='%232563eb' stroke-width='2' points='0,40 20,36 40,42 60,30 80,38 100,24 120,34 140,40 \
    160,28 180,36 200,32 220,38 240,34'/%3E%3C/svg%3E";
pub(crate) const FRAMES_ALT: &str = "Frame time over the last 120 frames";

/// A page's title, then bullets: an `item` holds its `line` of text and the items under it, so moving a bullet moves
/// what hangs from it; its `dom` is its DOM id, and `reading` draws it as the design system's loading state, which the
/// page's LIVE_JS fills in place under that id. A message's `section` holds its public `line` and its private `note`, why it says that: only lines
/// are ever sent. A link keeps its target in `href`, as ProseMirror's link mark does.
///
/// A document `dash doc import` read (dash_ingest::doc, task editor-in-rfcs) holds that reader's blocks too: its meta, title
/// and headings, paragraphs, list items, listings, writer notes, tables with their rows and cells, images and diagrams, each
/// with its source `anchor` ids. One schema holds both, so DocView draws either (rust/CLAUDE.md rule 7: one model); dash's
/// test `an_imported_doc_loads_in_the_notion_view_unchanged` holds the two definitions to agree.
fn schema() -> Arc<Schema> {
    // once a process: every field and document drawn loads through it (/code-review: a build per render)
    static SCHEMA: std::sync::OnceLock<Arc<Schema>> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(built).clone()
}

fn built() -> Arc<Schema> {
    let at = || Attrs::new().with("page", 0i64).with("anchor", "");
    let text = |name: &str| {
        BlockSpec::text(name).attrs(at().with("refs", Vec::<canvist_model::Value>::new()))
    };
    let leaf = |name: &str, keys: &[&str]| {
        BlockSpec::leaf(name).attrs(
            keys.iter()
                .fold(Attrs::new().with("page", 0i64), |a, k| a.with(k, "")),
        )
    };
    let schema = Schema::builder()
        .block(text("title"))
        // a page's own words about itself, each attr set alone: no list of names (the gate reads one as components')
        .block(
            BlockSpec::leaf("meta").attrs(
                Attrs::new()
                    .with("page", 0i64)
                    .with("title", "")
                    .with("description", "")
                    .with("image", "")
                    .with("preface", ""),
            ),
        )
        .block(
            BlockSpec::text("heading").attrs(
                at().with("level", 1i64)
                    .with("refs", Vec::<canvist_model::Value>::new()),
            ),
        )
        .block(text("paragraph"))
        .block(text("list_item"))
        .block(text("code"))
        .block(text("writer_note"))
        .block(BlockSpec::container("fold", &["block"]).attrs(at().with("summary", "")))
        .block(
            BlockSpec::container("table", &["row"]).attrs(
                ["anchor", "kind", "status", "caption"]
                    .iter()
                    .fold(Attrs::new().with("page", 0i64), |a, k| a.with(k, "")),
            ),
        )
        .block(
            BlockSpec::container("row", &["cell"])
                .groups(&[])
                .attrs(Attrs::new().with("anchor", "")),
        )
        .block(BlockSpec::text("cell").groups(&[]))
        .block(leaf("image", &["src", "alt", "anchor"]))
        .block(leaf("diagram", &["svg", "text", "anchor"]))
        .block(
            BlockSpec::container("item", &["line", "item", "page"])
                .groups(&["block"])
                .attrs(Attrs::new().with("dom", "").with("reading", false)),
        )
        .block(BlockSpec::container("section", &["line", "note"]).groups(&["block"]))
        .block(BlockSpec::text("line"))
        .block(BlockSpec::text("note"))
        // a link to a page of ours, its title its text (components::PageLink; task design-doc-tree-menu)
        .block(
            BlockSpec::text("page").groups(&["block"]).attrs(
                Attrs::new()
                    .with("href", "")
                    .with("kind", "")
                    .with("stage", ""),
            ),
        )
        .mark(MarkSpec::new("link").attrs(Attrs::new().with("href", "")))
        // bold, named as ProseMirror's basic schema names it, so a saved page reads there unchanged
        .mark(MarkSpec::new("strong"))
        .build()
        .expect(
            "a title, items, sections, lines, notes, page links, a link and a bold mark are a valid schema",
        );
    Arc::new(schema)
}

fn line(words: &str, href: Option<&str>) -> Value {
    // ProseMirror holds no empty text node: a line not known yet is a line with no content
    if words.is_empty() {
        return json!({ "type": "line" });
    }
    let mut text = json!({ "type": "text", "text": words });
    if let Some(href) = href {
        text["marks"] = json!([{ "type": "link", "attrs": { "href": href } }]);
    }
    json!({ "type": "line", "content": [text] })
}

/// A message as it is saved: each section its public words and its private note (empty: none).
pub(crate) fn message(sections: &[(&str, &str)]) -> Value {
    let section = |&(words, why): &(&str, &str)| {
        let mut content = vec![line(words, None)];
        if !why.is_empty() {
            content.push(json!({ "type": "note", "content": [{ "type": "text", "text": why }] }));
        }
        json!({ "type": "section", "content": content })
    };
    json!({ "type": "doc", "content": sections.iter().map(section).collect::<Vec<_>>() })
}

/// An older draft read into the model: each paragraph of its text a section, with no note, since none was kept.
pub(crate) fn from_text(text: &str) -> Value {
    // a paragraph's indentation kept, the blank lines around it dropped: the words drawn as he wrote them
    let paragraphs: Vec<(&str, &str)> = (text.split("\n\n"))
        .map(|p| p.trim_matches(['\n', '\r']).trim_end())
        .filter(|p| !p.trim().is_empty())
        .map(|p| (p, ""))
        .collect();
    message(&paragraphs)
}

/// What a message sends: each section's public line, a blank line between, its private notes never (rust/CLAUDE.md rule 7:
/// « only a message's public part is ever sent »). A send reads this, never the saved document whole.
pub(crate) fn sent(saved: &Value) -> Result<String, JsonError> {
    let doc = load(saved, 0)?;
    let lines: Vec<&str> = (doc.text_blocks())
        .filter(|b| &*doc.schema().block(b.kind()).name == "line")
        .map(|b| b.inline().map_or("", |t| t.as_str()))
        .collect();
    Ok(lines.join("\n\n"))
}

/// Every text node's words in a saved document, in order, a blank line between: a document the schema refuses still gives
/// back what it holds, never nothing (components::DocField draws a field of it).
pub(crate) fn words_in(saved: &Value) -> String {
    fn walk<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
        if let Some(t) = v["text"].as_str() {
            out.push(t);
        }
        v["content"]
            .as_array()
            .into_iter()
            .flatten()
            .for_each(|c| walk(c, out));
    }
    let mut out = vec![];
    walk(saved, &mut out);
    out.join("\n\n")
}

/// One line of an outline page: `says` its words, opening `href`, `what` the line under it; `id` its DOM id. Still
/// `reading`, it is drawn loading, its words not known yet.
pub struct Line<'a> {
    pub id: &'a str,
    pub says: &'a str,
    pub what: &'a str,
    pub href: Option<&'a str>,
    pub reading: bool,
}

/// A page of lines as it is saved, /today's: its title, then an item a line, what it is under it.
pub(crate) fn outline(title: &str, lines: &[Line]) -> Value {
    let item = |l: &Line| {
        let what = json!({ "type": "item", "content": [line(l.what, None)] });
        json!({ "type": "item", "attrs": { "dom": l.id, "reading": l.reading },
            "content": [line(l.says, l.href), what] })
    };
    let mut content =
        vec![json!({ "type": "title", "content": [{ "type": "text", "text": title }] })];
    content.extend(lines.iter().map(item));
    json!({ "type": "doc", "content": content })
}

/// The tab's blocks past its bullets, the ones a page holds besides lines (task rust-doc-rich-blocks): a paragraph with bold
/// words, a code block and an image, drawn by DocView under the bullets.
pub(crate) fn rich() -> Vec<Value> {
    let words = |text: &str| json!({ "type": "text", "text": text });
    let (before, bold, after) = BOLD;
    vec![
        json!({ "type": "paragraph", "content": [
            words(before),
            { "type": "text", "text": bold, "marks": [{ "type": "strong" }] },
            words(after),
        ] }),
        json!({ "type": "code", "content": [words(CODE)] }),
        json!({ "type": "image", "attrs": { "src": FRAMES, "alt": FRAMES_ALT } }),
    ]
}

/// The tab's page as it is saved: ProseMirror JSON, the d0 line the title, each d1 an item, each d2 an item inside the
/// d1 above it, then its rich blocks.
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
        let item = json!({ "type": "item", "content": [line(words, link.then_some(""))] });
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
    content.extend(rich());
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
    use dash_meter::dash_test;

    /// The text an agent reads of it: its Notion view's, read off the HTML.
    fn text(doc: &Document) -> String {
        view_forms(doc.to_json(), View::Notion).1
    }

    #[dash_test]
    fn the_tab_page_loads_edits_and_saves_through_canvist() {
        let mut doc = load(&seed(), 1).expect("the seed fits the schema");
        let before = text(&doc);
        assert!(
            before.starts_with("2026:\nOne tree, two renderers\nthe window and the web"),
            "{before}"
        );
        // its bullets, then the rich blocks: a paragraph, a code block of two lines between fences, an image
        assert_eq!(before.lines().count(), LINES.len() + 6);

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

    #[dash_test]
    fn the_text_form_is_derived_from_the_document() {
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
                    Selection\n\
                    Each frame has a budget of **16 ms**, measured in the window and on the web.\n\
                    ```\n\
                    cargo nextest run -p dash-ui frame_time\n  --run-ignored all\n\
                    ```\n\
                    ![Frame time over the last 120 frames]\n";
        assert_eq!(text(&doc), page);

        // Derived, never stored: an edit to the document is in the next text form.
        let selection = doc
            .text_blocks()
            .find(|b| b.inline().is_some_and(|t| t.as_str() == "Selection"))
            .map(|b| b.id())
            .expect("the last bullet");
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
    #[dash_test]
    fn a_draft_is_a_document_of_public_sections_and_private_notes() {
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

        // An older draft: its text as /drafts/<id> gave it, a paragraph's line breaks kept.
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

    /// Done-when 3 of rust-one-doc-component: only a draft's public sections are sent, never a private note; an older draft's
    /// text is sent as it was.
    #[dash_test]
    fn a_send_reads_the_public_lines_alone() {
        let draft = message(&[
            ("Bonjour Ana,", ""),
            ("Une question courte.", "She asked for a rate on the call."),
            ("Nicolas", "Signed as on every first message."),
        ]);
        assert_eq!(
            sent(&draft).expect("a message"),
            "Bonjour Ana,\n\nUne question courte.\n\nNicolas"
        );
        let old = "Bonjour Ana,\n\nUne question\ncourte.\n\nNicolas";
        assert_eq!(sent(&from_text(old)).expect("an older draft"), old);
    }
}
