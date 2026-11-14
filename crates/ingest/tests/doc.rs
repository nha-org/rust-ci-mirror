//! rfc-rust-unified section 20i's core (task rust-doc-core): a source read into one canvist_model document by the reader its
//! format registered, and that document's two doors.
use base64::Engine;
use dash_ingest::doc::{self, Element, Label, Reader, from_document, to_document};
use dash_meter::TestDir;
use dash_meter::dash_test;
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
#[dash_test]
fn a_docs_page_reads_into_one_canvist_document() {
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
#[dash_test]
fn readers_register_per_format_html_first() {
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
#[dash_test]
fn import_keeps_images_and_diagrams_intact() {
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
#[dash_test]
fn extract_reads_a_diagram_by_its_words() {
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
#[dash_test]
fn a_reader_that_panics_fails_its_file_only() {
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
#[dash_test]
fn an_html_page_with_an_svg_sniffs_as_html() {
    let page = std::fs::read(fixture("with-media/page.html")).expect("the page");
    assert_eq!(
        doc::reader_for(Path::new("no-extension"), &page).map(|r| r.format),
        Ok("html")
    );
}

/// /code-review on PR 1435: an image path leaving the source's folder is kept as written, its file never read in.
#[dash_test]
fn import_never_reads_an_image_outside_the_source_folder() {
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
#[dash_test]
fn an_anchor_never_splits_another_links_url() {
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

/// rfc-inbox.html, an RFC of private/docs, imported: the saved document and the elements it reads back to.
fn rfc() -> (Value, Vec<dash_ingest::doc::Element>) {
    let saved = doc::import(&fixture("rfc-inbox.html")).expect("the RFC imports");
    let schema_checked =
        dash_ingest::doc::to_document(&doc::parse(&fixture("rfc-inbox.html")).expect("reads").1)
            .expect("fits the schema");
    assert_eq!(
        schema_checked.to_json(),
        saved,
        "import is the document the reader's elements make"
    );
    (saved, from_document(&schema_checked).elements)
}

fn words_of(block: &Value) -> String {
    (block["content"].as_array().into_iter().flatten())
        .filter_map(|t| t["text"].as_str())
        .collect()
}

/// Task editor-in-rfcs, done-when 1, sections: each `<section id>` keeps its id on its heading, so `/v/<doc>#intent` and a
/// link `#d-cap` still land; a point's own id (`r-1-1`) is kept on its list item.
#[dash_test]
fn import_keeps_each_section_and_its_anchor() {
    let (saved, _) = rfc();
    let sections: Vec<(String, String)> = (of_type(&saved, "heading").iter())
        .map(|h| {
            (
                h["attrs"]["anchor"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                words_of(h),
            )
        })
        .collect();
    assert_eq!(sections[0], ("intent".into(), "1. Intent".into()));
    assert_eq!(
        sections[1],
        ("mirror".into(), "2. The mirror of comments".into())
    );
    let anchored = sections.iter().filter(|(a, _)| !a.is_empty()).count();
    assert_eq!(
        anchored, 10,
        "the RFC's ten sections, each its id: {sections:?}"
    );
    let point = of_type(&saved, "list_item")
        .into_iter()
        .find(|b| b["attrs"]["anchor"] == "r-1-1");
    assert!(
        point.is_some_and(|p| words_of(&p).starts_with("Only what needs him.")),
        "a rule's id stays on its point"
    );
}

/// Done-when 1, writer notes: a section's `details.wn` points are writer_note blocks, its « Writer notes » summary
/// dropped, never body paragraphs; extract still reads them, as list items.
#[dash_test]
fn import_keeps_writer_notes_apart_from_the_body() {
    let (saved, elements) = rfc();
    let notes = of_type(&saved, "writer_note");
    assert_eq!(
        notes.len(),
        23,
        "every point of the ten sections' writer notes"
    );
    assert!(
        words_of(&notes[0]).starts_with("His brief, 2026-11-11, verbatim:"),
        "{}",
        notes[0]
    );
    let all = blocks(&saved);
    let intent = all
        .iter()
        .position(|b| b["attrs"]["anchor"] == "intent")
        .expect("section 1");
    assert_eq!(
        all[intent + 1]["type"],
        "writer_note",
        "the notes open their section, as in the RFC"
    );
    assert!(
        !all.iter()
            .any(|b| b["type"] == "paragraph" && words_of(b) == "Writer notes"),
        "the fold's summary is no paragraph"
    );
    assert!(
        elements
            .iter()
            .any(|e| e.label == Label::WriterNote && e.text.contains("Intent only"))
    );
    let items = doc::extract(&fixture("rfc-inbox.html")).expect("extract").1;
    assert!(
        items
            .iter()
            .any(|i| i.fts.contains("- Intent only (rfc skill, rule 2)")),
        "search still reads them"
    );
}

/// Done-when 1, decision tables: `table.decision` keeps its anchor, its chip as `status`, its question as `caption`, and
/// each option a row, as a decision and not a plain table.
#[dash_test]
fn import_keeps_a_decision_table_whole() {
    let (saved, elements) = rfc();
    let decisions: Vec<Value> = of_type(&saved, "table")
        .into_iter()
        .filter(|t| t["attrs"]["kind"] == "decision")
        .collect();
    assert_eq!(decisions.len(), 1, "rfc-inbox holds one decision");
    let d = &decisions[0];
    assert_eq!(d["attrs"]["anchor"], "d-cap");
    assert_eq!(d["attrs"]["status"], "decided");
    assert_eq!(
        d["attrs"]["caption"],
        "How many asks should /inbox show at once? A, at most 7 (@nha, 2026-11-11: « 7 s fine. »)"
    );
    let rows = d["content"].as_array().expect("its rows");
    assert_eq!(rows.len(), 4, "the header and options A, B, C");
    assert_eq!(
        words_of(&rows[1]["content"][0]),
        "A. At most 7, the rest wait their turn"
    );
    let decided = |e: &&dash_ingest::doc::Element| match &e.label {
        Label::Table(t) => {
            t.decision.as_ref().is_some_and(|d| d.status == "decided") && t.rows.len() == 4
        }
        _ => false,
    };
    assert!(
        elements.iter().any(|e| decided(&e)),
        "it reads back as a decision"
    );
    let row =
        (d["content"].as_array().into_iter().flatten()).find(|r| r["attrs"]["anchor"] == "p-cap");
    assert!(row.is_none(), "p-cap is a row of another table");
    let p_cap = (of_type(&saved, "table").into_iter())
        .flat_map(|t| t["content"].as_array().cloned().unwrap_or_default())
        .find(|r| r["attrs"]["anchor"] == "p-cap");
    assert!(
        p_cap.is_some_and(|r| words_of(&r["content"][0]).starts_with("How many")),
        "a row keeps its id, a link's #p-cap"
    );
    let plain = of_type(&saved, "table")
        .iter()
        .filter(|t| t["attrs"]["kind"] != "decision")
        .count();
    assert_eq!(plain, 8, "its other eight tables stay plain tables");
}

/// Done-when 1, diagrams: each `figure.diagram`'s SVG is a diagram block holding its markup whole, its words its text,
/// its caption the paragraph after it.
#[dash_test]
fn import_keeps_every_rfc_diagram_as_its_svg() {
    let (saved, _) = rfc();
    let diagrams = of_type(&saved, "diagram");
    assert_eq!(diagrams.len(), 3, "the RFC's three figures");
    let svg = diagrams[0]["attrs"]["svg"].as_str().expect("its markup");
    assert!(
        svg.starts_with("<svg viewBox=\"0 0 720 230\"") && svg.ends_with("</svg>"),
        "{svg}"
    );
    assert!(
        svg.contains("<marker id=\"arrow\""),
        "its defs kept, so its arrows draw"
    );
    assert!(
        diagrams[0]["attrs"]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Check pass"))
    );
    let all = blocks(&saved);
    let at = all
        .iter()
        .position(|b| b == &diagrams[0])
        .expect("in order");
    assert!(
        words_of(&all[at + 1]).starts_with("The check pass, in accent,"),
        "its figcaption follows it"
    );
}

/// /code-review of done-when 1: every id lands on a block, whatever holds it (a section with no heading of its own, a
/// figure, a div, a list, a listing), and writer notes are read whole, wherever they sit and whatever they hold.
#[dash_test]
fn every_id_and_every_writer_note_survives_the_import() {
    let html = r#"<!doctype html><body>
<section id="fees"><div class="tw"><table><tr id="r1"><td>Stripe</td><td>1.5%</td></tr></table></div></section>
<figure class="diagram" id="f-flow"><svg viewBox="0 0 10 10"><text>flow</text></svg><figcaption>The flow.</figcaption></figure>
<div id="lede"><p>Opening words.</p></div>
<ol id="steps"><li>One <details class="wn"><summary>Writer notes</summary><ul><li>why <a href="/x">one</a></li></ul></details></li></ol>
<pre id="cfg">x = 1</pre>
<details class="wn"><summary>Writer notes</summary><p><b>Still to read</b> by Friday.</p><pre># one claim a line</pre></details>
<table class="decision" id="d-late"><caption>Cap on asks <span class="chip ok">decided</span></caption></table>
</body>"#;
    let els = dash_ingest::html::read_html(html).elements;
    let at = |id: &str| {
        els.iter()
            .find(|e| (e.anchor.iter()).any(|a| a.split(' ').any(|i| i == id)))
            .map(|e| (&e.label, e.text.as_str()))
    };
    assert!(
        matches!(at("fees"), Some((Label::Table(_), _))),
        "a section's id with no heading: {els:#?}"
    );
    assert!(
        matches!(at("f-flow"), Some((Label::Diagram { .. }, "flow"))),
        "a figure's id on its diagram"
    );
    assert_eq!(at("lede").map(|a| a.1), Some("Opening words."));
    assert_eq!(
        at("steps").map(|a| a.1),
        Some("One"),
        "a list's id on its first point, its notes out of its words"
    );
    let point = els.iter().find(|e| e.text == "One").expect("the point");
    let note = els.iter().find(|e| e.text == "why one").expect("its note");
    assert!(
        point.refs.is_empty(),
        "its notes' link is theirs: {point:?}"
    );
    assert_eq!(note.refs[0].href, "/x");
    assert_eq!(at("cfg").map(|a| a.1), Some("x = 1"));
    let notes: Vec<&str> = (els.iter())
        .filter(|e| e.label == Label::WriterNote)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        notes,
        ["why one", "Still to read by Friday.", "# one claim a line"],
        "a point's notes, a paragraph, a listing"
    );
    assert!(
        !els.iter().any(|e| e.text.contains("Writer notes")),
        "no summary read"
    );
    let Label::Table(t) = &els
        .iter()
        .find(|e| matches!(e.label, Label::Table(_)))
        .expect("a table")
        .label
    else {
        unreachable!()
    };
    assert_eq!(t.row_ids, ["r1"]);
    assert_eq!(
        at("d-late").map(|a| (a.0.clone(), a.1)),
        Some((Label::Text, "decided Cap on asks")),
        "a decision with no options yet: its question once, chip first"
    );
}

/// The second /code-review pass: an SVG's own ids stay its own, a heading or an image in writer notes is the notes', a
/// container's id and its first block's both land, a chip leaves the caption alone, a decision needs no caption, and a
/// quoted article inside the one article is not a second page.
#[dash_test]
fn ids_and_notes_keep_to_their_own_block() {
    let html = r#"<!doctype html><body><article>
<section id="intent"><h2 id="intent-h">Intent</h2><p>See <svg><defs><marker id="arrowhead"/></defs><text>a</text></svg> it.</p></section>
<ul><li>Point <details class="wn"><summary>Writer notes</summary><h3>Sources</h3><figure><svg><text>n</text></svg></figure></details></li></ul>
<table class="decision" id="d-y"><caption>Is the call yours to make? <span class="chip warn">yours</span></caption><tr><td>A</td></tr></table>
<table class="decision" id="d-n"><tr><td>Option A</td></tr></table>
<article class="tweet"><p>Quoted card.</p></article>
</article><p>Outside the post.</p></body>"#;
    let els = dash_ingest::html::read_html(html).elements;
    let anchor = |text: &str| {
        els.iter()
            .find(|e| e.text == text)
            .and_then(|e| e.anchor.clone())
    };
    assert_eq!(
        anchor("Intent").as_deref(),
        Some("intent intent-h"),
        "both ids on the heading: {els:#?}"
    );
    assert_eq!(
        anchor("See it."),
        None,
        "an SVG's marker id is the SVG's: {els:#?}"
    );
    let notes: Vec<&Element> = els
        .iter()
        .filter(|e| e.label == Label::WriterNote)
        .collect();
    assert_eq!(
        notes.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
        ["Sources"],
        "a heading in the notes is a note"
    );
    let diagrams = els
        .iter()
        .filter(|e| matches!(e.label, Label::Diagram { .. }))
        .count();
    assert_eq!(
        diagrams, 2,
        "the paragraph's SVG and the notes' SVG, each once"
    );
    let decision = |id: &str| {
        let e = els
            .iter()
            .find(|e| e.anchor.as_deref() == Some(id))
            .expect(id);
        match &e.label {
            Label::Table(t) => t.decision.clone(),
            _ => None,
        }
    };
    let yours = decision("d-y").expect("a decision");
    assert_eq!(
        (yours.status.as_str(), yours.caption.as_str()),
        ("yours", "Is the call yours to make?")
    );
    assert!(
        decision("d-n").is_some(),
        "a decision table with no caption is still a decision"
    );
    assert!(els.iter().any(|e| e.text == "Quoted card."));
    assert!(
        !els.iter().any(|e| e.text == "Outside the post."),
        "the one article is the page"
    );
}

/// /code-review of fa5aa6443: a drawing the design system drew as an image of itself (one that could run code) reads back as
/// the diagram it was, its markup whole, so a save's export loses no drawing.
#[dash_test]
fn a_drawing_drawn_as_an_image_reads_back_as_its_diagram() {
    let svg = r#"<svg onload="x()"><text>a &amp; b</text></svg>"#;
    let encoded: String = (svg.bytes())
        .map(|b| match b.is_ascii_alphanumeric() {
            true => (b as char).to_string(),
            false => format!("%{b:02X}"),
        })
        .collect();
    let html = format!(
        r#"<!doctype html><body><img class="diagram" id="d1" src="data:image/svg+xml,{encoded}" alt="a and b"><img src="p.png" alt="a photo"></body>"#
    );
    let els = dash_ingest::html::read_html(&html).elements;
    assert_eq!(els[0].label, Label::Diagram { svg: svg.into() }, "{els:?}");
    assert_eq!(
        (els[0].text.as_str(), els[0].anchor.as_deref()),
        ("a and b", Some("d1"))
    );
    assert_eq!(
        els[1].label,
        Label::Picture {
            src: "p.png".into()
        },
        "any other image stays one"
    );
}

/// Task rust-doc-view-parity, done-when 3: an empty link to a point (`<a class="pt" href="#p-…"></a>`) reads its point's
/// number, as shell.js writes it in a browser: a list's from its `start`, a table's `p-` rows from 1. One whose point is gone
/// keeps no words and makes no link; so /v/ draws each point link, which dash parity found missing against :7799.
#[dash_test]
fn a_point_link_reads_its_point_s_number() {
    let html = r##"<main><ol start="3"><li id="p-a">A</li><li id="p-b">B</li></ol>
<table><tr><th>Q</th></tr><tr id="p-x"><td>X</td></tr><tr id="p-y"><td>Y</td></tr></table>
<p>See <a class="pt" href="#p-b"></a>, <a class="pt" href="#p-y"></a> and <a class="pt" href="#p-gone"></a>.</p></main>"##;
    let els = dash_ingest::html::read_html(html).elements;
    let see = els
        .iter()
        .find(|e| e.text.starts_with("See"))
        .expect("the paragraph");
    assert_eq!(see.text, "See 4, 2 and .");
    let links: Vec<(&str, &str)> = see
        .refs
        .iter()
        .map(|r| (r.anchor.as_str(), r.href.as_str()))
        .collect();
    assert_eq!(links, [("4", "#p-b"), ("2", "#p-y")]);
}
