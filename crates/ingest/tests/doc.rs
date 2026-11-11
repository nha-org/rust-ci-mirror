//! rfc-rust-unified section 20i's core (task rust-doc-core): a source read into one canvist_model document by the reader its
//! format registered, and that document's two doors.
use base64::Engine;
use dash_ingest::doc::{self, Label, Reader, from_document, to_document};
use dash_meter::TestDir;
use dash_meter::memory::TestBudget;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

/// with-media/page.html copied beside its PNG, fetched through the fixture cache from fixtures.txt (task
/// rust-heavy-crates-split: no binary fixture in git).
fn with_media(dir: &TestDir) -> PathBuf {
    let page = dir.0.join("page.html");
    std::fs::copy(fixture("with-media/page.html"), &page).expect("the page");
    std::fs::copy(png(), dir.0.join("dot.png")).expect("the PNG");
    page
}

fn png() -> PathBuf {
    dash_meter::fixtures::fixture(env!("CARGO_MANIFEST_DIR"), "hello.png")
}

fn blocks(saved: &Value) -> Vec<Value> {
    saved["content"].as_array().cloned().unwrap_or_default()
}

fn of_type(saved: &Value, kind: &str) -> Vec<Value> {
    blocks(saved)
        .into_iter()
        .filter(|b| b["type"] == kind)
        .collect()
}

/// Done-when 1: one private/docs page, supervisor-sandbox.html, read by the HTML reader into a canvist_model document whose
/// blocks are its title, headings, paragraphs, list items and its figure's image; and it reads back to the same elements.
#[test]
fn a_docs_page_reads_into_one_canvist_document() {
    let _budget = TestBudget::new();
    let (format, parsed) = doc::parse(&fixture("supervisor-sandbox.html")).expect("the page reads");
    assert_eq!(format, "html");
    let document = to_document(&parsed).expect("its elements fit the schema");
    let saved = document.to_json();
    assert_eq!(
        of_type(&saved, "meta")[0]["attrs"]["title"],
        "Sandbox and supervisor"
    );
    let title: Vec<String> = (of_type(&saved, "title").iter())
        .flat_map(|b| b["content"].as_array().cloned().unwrap_or_default())
        .map(|t| t["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        title.concat(),
        "Claude is free inside its box. What leaves it goes through you."
    );
    let headings = of_type(&saved, "heading").len();
    assert!(
        headings >= 5,
        "its h2 sections are heading blocks: {headings}"
    );
    assert!(
        of_type(&saved, "list_item").len() >= 5,
        "its points are list items"
    );
    let image = &of_type(&saved, "image")[0]["attrs"];
    assert_eq!(image["src"], "/f/private/docs/supervisor-sandbox.en.svg");
    assert!(
        image["alt"]
            .as_str()
            .is_some_and(|a| a.starts_with("The phone, the Mac")),
        "{image}"
    );
    assert_eq!(
        from_document(&document),
        parsed,
        "the document holds all the reader read, and gives it back"
    );
}

/// Readers register per format from their own module: the core lists none, and picks by extension, else by the bytes.
#[test]
fn readers_register_per_format_html_first() {
    let _budget = TestBudget::new();
    let formats: Vec<&str> = inventory::iter::<Reader>
        .into_iter()
        .map(|r| r.format)
        .collect();
    assert!(
        formats.contains(&"html") && formats.contains(&"svg"),
        "{formats:?}"
    );
    let page = std::fs::read(fixture("supervisor-sandbox.html")).expect("the page");
    let by_bytes = doc::reader_for(Path::new("no-extension"), &page).expect("sniffed");
    assert_eq!(by_bytes.format, "html");
    let refused = doc::reader_for(Path::new("x.bin"), b"\x00\x01").err();
    assert!(
        refused.is_some_and(|e| e.contains("no reader")),
        "an unknown format is refused by name"
    );
}

/// Done-when 3: `import` keeps an inline SVG whole as a diagram block and holds a local PNG as its own bytes; an image on
/// the web keeps its address.
#[test]
fn import_keeps_images_and_diagrams_intact() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("doc-with-media");
    let saved = doc::import(&with_media(&dir)).expect("the page imports");
    let diagram = &of_type(&saved, "diagram")[0]["attrs"];
    let svg = diagram["svg"].as_str().unwrap_or_default();
    let source = std::fs::read_to_string(fixture("with-media/page.html")).expect("the page");
    let drawn = source
        .lines()
        .find(|l| l.contains("viewBox"))
        .expect("the SVG line");
    // html5ever writes an empty SVG element out as an open and a close tag: the same drawing
    assert_eq!(
        svg,
        drawn.trim().replace("/>", "></rect>"),
        "the SVG's markup kept whole"
    );
    assert_eq!(
        diagram["text"], "Request flow\nclient",
        "its words, for extract"
    );
    let images = of_type(&saved, "image");
    let src = images[0]["attrs"]["src"].as_str().unwrap_or_default();
    let data = src
        .strip_prefix("data:image/png;base64,")
        .expect("the PNG held as a data URI");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .expect("base64");
    assert_eq!(bytes, std::fs::read(png()).expect("the PNG"));
    assert_eq!(images[0]["attrs"]["alt"], "A red dot");
    assert_eq!(images[1]["attrs"]["src"], "https://example.com/far.png");
    let caption = blocks(&saved)
        .into_iter()
        .any(|b| b["content"][0]["text"] == "The dot, as a PNG.");
    assert!(
        caption,
        "the figure's caption is a paragraph after its image"
    );
}

/// Done-when 2's projection on the same page: an image's alt and a diagram's words are text, its markup and bytes are not.
#[test]
fn extract_reads_a_diagram_by_its_words() {
    let _budget = TestBudget::new();
    let (_, items) = doc::extract(&fixture("with-media/page.html")).expect("the page extracts");
    let all: Vec<&str> = items.iter().map(|i| i.fts.as_str()).collect();
    let text = all.join("\n");
    assert!(
        text.contains("Request flow\nclient") && text.contains("A red dot"),
        "{text}"
    );
    assert!(
        !text.contains("base64") && !text.contains("viewBox"),
        "{text}"
    );
    assert_eq!(items[0].title, "Two pictures");
    let labels: Vec<Label> = (doc::parse(&fixture("with-media/page.html"))
        .expect("read")
        .1
        .elements)
        .into_iter()
        .map(|e| e.label)
        .filter(|l| matches!(l, Label::Diagram { .. } | Label::Picture { .. }))
        .collect();
    assert_eq!(labels.len(), 3, "one diagram and two pictures: {labels:?}");
}

/// A reader that panics, as a library with unwraps can on a malformed file: the core turns it into this file's error.
struct Panics;
inventory::submit! {
    Reader { format: "zz-panics", extensions: &["panics"], sniff: |_| false, read: |_| panic!("a malformed file") }
}

/// /code-review on PR 1435: a reader's panic is an error naming the file, never the end of the command.
#[test]
fn a_reader_that_panics_fails_its_file_only() {
    let _budget = TestBudget::new();
    let _ = Panics;
    let dir = TestDir::new("doc-panics");
    let file = dir.0.join("odd.panics");
    std::fs::write(&file, b"x").expect("the file");
    let e = doc::parse(&file).err();
    assert!(
        e.as_deref()
            .is_some_and(|e| e.contains("the zz-panics reader failed")),
        "{e:?}"
    );
}

/// /code-review on PR 1435: an HTML page holding an inline SVG, with no extension, is still read as HTML, on every build.
#[test]
fn an_html_page_with_an_svg_sniffs_as_html() {
    let _budget = TestBudget::new();
    let page = std::fs::read(fixture("with-media/page.html")).expect("the page");
    assert_eq!(
        doc::reader_for(Path::new("no-extension"), &page).map(|r| r.format),
        Ok("html")
    );
}

/// /code-review on PR 1435: an image path leaving the source's folder is kept as written, its file never read in.
#[test]
fn import_never_reads_an_image_outside_the_source_folder() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("doc-outside");
    std::fs::create_dir_all(dir.0.join("page")).expect("folders");
    std::fs::copy(png(), dir.0.join("secret.png")).expect("a file outside");
    std::fs::copy(png(), dir.0.join("page/dot.png")).expect("a file inside");
    let page = dir.0.join("page/page.html");
    let html = "<!doctype html><p><img src=\"../secret.png\" alt=\"out\"><img src=\"dot.png?v=2\" alt=\"in\"></p>";
    std::fs::write(&page, html).expect("the page");
    let saved = doc::import(&page).expect("it imports");
    let images = of_type(&saved, "image");
    assert_eq!(
        images[0]["attrs"]["src"], "../secret.png",
        "the outside file is not read"
    );
    let inside = images[1]["attrs"]["src"].as_str().unwrap_or_default();
    assert!(
        inside.starts_with("data:image/png;base64,"),
        "a query after the name still embeds: {inside}"
    );
}

/// /code-review on PR 1435: a link placed by its anchor never lands inside a link already there, its URL included.
#[test]
fn an_anchor_never_splits_another_links_url() {
    let _budget = TestBudget::new();
    let text = "see site and example";
    let refs = [
        doc::Ref {
            anchor: "site".into(),
            href: "https://example.com/docs".into(),
            span: Some((4, 8)),
        },
        doc::Ref {
            anchor: "example".into(),
            href: "https://e.org".into(),
            span: None,
        },
    ];
    let out = doc::render_with_refs(text, &refs);
    assert_eq!(
        out,
        "see [site](https://example.com/docs) and [example](https://e.org)"
    );
}
