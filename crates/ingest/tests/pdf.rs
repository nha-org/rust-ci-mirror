//! graal's `test/hello_world/lib/extraction/pdf_test.clj`, its text-layer deftests, on its fixtures (task rust-doc-core,
//! done-when 5; the orchestrator's answer: OCR, tables, figures, furniture and the garbled check are rust-pdf-ocr-tables).
//! `fast-mode-pdf-still-emits-info-preface` is `info_dict_preface_on_first_page`: Rust's reader has one mode, the text layer.
use dash_ingest::doc::{self, Element, Item, Label, Reader};
use dash_ingest::pdf::{
    Bookmark, outline, outline_headings, page_links, parse_pdf_date, render_info_preface,
    strip_info_preface,
};
use dash_meter::memory::TestBudget;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pdf")
        .join(name)
}

/// A real docling-corpus paper: page 1 has the DOI as a link annotation and the authors' `mailto:` ones.
const WITH_LINKS: &str = "2206.01062.pdf";

fn items(name: &str) -> Vec<Item> {
    doc::extract(&fixture(name)).expect("the PDF reads").1
}

fn page_texts(name: &str) -> Vec<String> {
    let bytes = std::fs::read(fixture(name)).expect("the PDF");
    pdf_extract::extract_text_from_mem_by_pages(&bytes).expect("its text layer")
}

fn lopdf(name: &str) -> pdf_extract::Document {
    pdf_extract::Document::load(fixture(name)).expect("the PDF loads")
}

#[test]
fn info_dict_preface_on_first_page() {
    let _budget = TestBudget::new();
    let items = items(WITH_LINKS);
    let page1 = &items
        .iter()
        .find(|i| i.page == Some(1))
        .expect("page 1")
        .fts;
    assert!(page1.starts_with("PDF · "), "{page1}");
    assert!(
        page1.contains("DocLayNet"),
        "Title from /Info dict: {page1}"
    );
    assert!(
        page1.contains("Birgit Pfitzmann"),
        "Author from /Info dict: {page1}"
    );
    assert!(
        page1.contains("2022-06-03"),
        "CreationDate decoded to yyyy-mm-dd: {page1}"
    );
}

#[test]
fn parse_pdf_date_shapes() {
    let _budget = TestBudget::new();
    // PDF date parser handles common shapes
    assert_eq!(
        parse_pdf_date("D:20220603003241Z").as_deref(),
        Some("2022-06-03")
    );
    assert_eq!(
        parse_pdf_date("D:20240930120000-04'00'").as_deref(),
        Some("2024-09-30")
    );
    assert_eq!(
        parse_pdf_date("D:20230410120000+00'00'").as_deref(),
        Some("2023-04-10")
    );
    // malformed input → nil
    assert_eq!(parse_pdf_date(""), None);
    assert_eq!(parse_pdf_date("not a date"), None);
}

fn info(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn render_info_preface_shapes() {
    let _budget = TestBudget::new();
    // rich /Info renders all human-readable fields in order
    let rich = info(&[
        ("Title", "Q3 Report"),
        ("Author", "Sarah"),
        ("Keywords", "revenue, growth"),
        ("CreationDate", "D:20240930120000Z"),
    ]);
    assert_eq!(
        render_info_preface(&rich).as_deref(),
        Some("PDF · Q3 Report · Sarah · 2024-09-30 · revenue, growth")
    );
    // producer-only fallback fires when no title/author/subject/keywords
    let producer = info(&[
        ("Producer", "Adobe Acrobat Pro"),
        ("CreationDate", "D:20230410120000Z"),
    ]);
    assert_eq!(
        render_info_preface(&producer).as_deref(),
        Some("PDF · 2023-04-10 · Adobe Acrobat Pro")
    );
    // one line even when a value holds newlines, so strip-info-preface finds it
    let preface =
        render_info_preface(&info(&[("Title", "Line one\n\nline two")])).expect("a preface");
    assert_eq!(preface, "PDF · Line one line two");
    assert_eq!(strip_info_preface(&format!("{preface}\n\nBody")), "Body");
    // CreationDate-only input → nil (not worth a preface); empty input → nil
    assert_eq!(
        render_info_preface(&info(&[("CreationDate", "D:20230410120000Z")])),
        None
    );
    assert_eq!(render_info_preface(&info(&[])), None);
}

#[test]
fn link_annotations_surface_on_page_1() {
    let _budget = TestBudget::new();
    let d = lopdf(WITH_LINKS);
    let texts = page_texts(WITH_LINKS);
    let links = page_links(&d, d.get_pages()[&1], &texts[0]);
    let doi = "https://doi.org/10.1145/3534678.3539043";
    // page 1 surfaces the DOI URL (deduped from N annotation rects), and the authors' mailto: annotations
    assert!(
        links.iter().any(|l| l == doi),
        "doi missing from links: {links:?}"
    );
    assert!(
        links.iter().any(|l| l.starts_with("mailto:")),
        "no mailto: in links: {links:?}"
    );
    // DOI appears in FTS so 'doi.org/...' queries hit, on a Links: line
    let items = items(WITH_LINKS);
    let page1 = &items
        .iter()
        .find(|i| i.page == Some(1))
        .expect("page 1")
        .fts;
    assert!(page1.contains(doi) && page1.contains("Links: "), "{page1}");
}

#[test]
fn pages_without_links_omit_link_metadata() {
    let _budget = TestBudget::new();
    let d = lopdf(WITH_LINKS);
    let texts = page_texts(WITH_LINKS);
    let free = (d.get_pages().into_iter())
        .find(|&(no, id)| no > 2 && page_links(&d, id, &texts[no as usize - 1]).is_empty())
        .map(|(no, _)| no)
        .expect("a page past 2 with no link annotation");
    let items = items(WITH_LINKS);
    let on_it: Vec<&Item> = items.iter().filter(|i| i.page == Some(free)).collect();
    assert!(!on_it.is_empty(), "page {free} has items");
    assert!(
        on_it.iter().all(|i| !i.fts.contains("Links: ")),
        "no Links: line on page {free}"
    );
}

fn text(t: &str) -> Element {
    Element::new(Label::Text, t)
}

#[test]
fn bookmarks_are_headings() {
    let _budget = TestBudget::new();
    let mark = |t: &str, page| Bookmark {
        title: t.into(),
        level: 1,
        page,
    };
    let outline_marks = [
        mark("Mortality  by age", 1),
        mark("Background", 1),
        mark("Annex", 2),
    ];
    let table = Element::new(
        Label::Table(vec![vec!["Background".into(), "1".into()]]),
        "Background | 1",
    );
    let by_page = BTreeMap::from([
        (
            1,
            vec![
                text("Mortality by age"),
                text("Mortality by age The average number of years a newborn lives."),
                text("Background information was scarce."),
                table.clone(),
            ],
        ),
        (2, vec![text("Mortality by age")]),
    ]);
    let out = outline_headings(&outline_marks, by_page.clone());
    let header = Element::new(Label::SectionHeader(1), "Mortality by age");
    // a text element that is a bookmark's title, whitespace aside, is a heading
    assert_eq!(out[&1][0], header);
    // a heading the layout merged into its paragraph splits off
    assert_eq!(
        out[&1][1..3],
        [
            header.clone(),
            text("The average number of years a newborn lives.")
        ]
    );
    // a paragraph that only begins with a bookmark's words, a table, and another page's title stay as they were
    assert_eq!(out[&1][3..], by_page[&1][2..]);
    assert_eq!(out[&2], by_page[&2]);

    // read from the PDF: WHO's 156 bookmarks; lopdf counts pages from 1, so PDFium's page 5 is 6 here
    let marks = outline(&lopdf("who_indicators.pdf"));
    assert_eq!(marks.len(), 156);
    assert_eq!(marks[0], mark("Acknowledgements", 6));
}

#[test]
fn fast_mode_on_image_only_returns_empty() {
    let _budget = TestBudget::new();
    // image-only PDF with no OCR → no page items (its text comes from OCR only, rust-pdf-ocr-tables)
    let items = items("image_only_sample.pdf");
    assert!(
        items.is_empty(),
        "no items from an image-only PDF without OCR: {items:?}"
    );
}

/// liteparse_corpus_test.clj's `unreadable`: a zero-byte file and a password-protected PDF fail loudly, never index as empty.
#[test]
fn unreadable_pdfs_fail_cleanly() {
    let _budget = TestBudget::new();
    for name in ["empty-file.pdf", "encrypted_pwd_is_hello.pdf"] {
        let e = doc::extract(&fixture(name)).err();
        assert!(
            e.as_deref()
                .is_some_and(|e| e.contains("not a readable PDF")),
            "{name}: {e:?}"
        );
    }
}

/// Done-when 4: the PDF reader registered itself; the core picks it by extension or by its `%PDF-` bytes, as it picks HTML.
#[test]
fn a_pdf_reader_plugs_in_beside_html() {
    let _budget = TestBudget::new();
    let formats: Vec<&str> = inventory::iter::<Reader>
        .into_iter()
        .map(|r| r.format)
        .collect();
    assert!(
        formats.contains(&"pdf") && formats.contains(&"html"),
        "{formats:?}"
    );
    let bytes = std::fs::read(fixture(WITH_LINKS)).expect("the PDF");
    assert_eq!(
        doc::reader_for(Path::new("no-extension"), &bytes).map(|r| r.format),
        Ok("pdf")
    );
    let saved = doc::import(&fixture(WITH_LINKS)).expect("a PDF imports through the same door");
    let blocks = saved["content"].as_array().cloned().unwrap_or_default();
    assert!(
        blocks.iter().any(|b| b["attrs"]["page"] == 9),
        "its blocks keep their pages"
    );
}

/// /code-review on PR 1435: a sub-bookmark is a heading one level down, so a chunk under « 3.1 Data » keeps « 3 Method »
/// in its heading path (pdf.clj made every bookmark level 1).
#[test]
fn a_sub_bookmark_keeps_its_depth() {
    let _budget = TestBudget::new();
    let marks = [
        Bookmark {
            title: "3 Method".into(),
            level: 1,
            page: 1,
        },
        Bookmark {
            title: "3.1 Data".into(),
            level: 2,
            page: 1,
        },
    ];
    let page = vec![
        text("3 Method"),
        text("3.1 Data"),
        text("Rows from the survey."),
    ];
    let out = outline_headings(&marks, BTreeMap::from([(1, page)]));
    let labels: Vec<&Label> = out[&1].iter().map(|e| &e.label).collect();
    assert_eq!(
        labels,
        [
            &Label::SectionHeader(1),
            &Label::SectionHeader(2),
            &Label::Text
        ]
    );
    let items = doc::items(&doc::Parsed {
        meta: Default::default(),
        elements: out[&1].clone(),
    });
    assert_eq!(
        items[0].fts, "3 Method\n3.1 Data\n\nRows from the survey.",
        "both headings in its path"
    );
}
