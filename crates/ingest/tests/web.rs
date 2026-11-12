//! graal's `test/hello_world/lib/extraction/web_test.clj`, one test per deftest and the same inputs and expectations, plus
//! `liteparse_corpus_test.clj`'s HTML and SVG cases on its fixtures (rfc-rust-unified section 20i: « a reader is ported with
//! its tests »). Items go through the whole core, the document and back, as `dash doc extract` does.
use dash_ingest::doc::{Element, Item, Label, Parsed, from_document, items, to_document};
use dash_ingest::html::{read_html, read_svg};
use dash_meter::memory::TestBudget;

const ARTICLE_HTML: &str = concat!(
    "<html><head>",
    "<title>Test Doc</title>",
    "<meta property=\"og:description\" content=\"OG description\"/>",
    "</head><body>",
    "<article>",
    "<h1>The Doc Title</h1>",
    "<p>Intro paragraph one.</p>",
    "<h2>First Section</h2>",
    "<p>First section body.</p>",
    "<ul><li>alpha</li><li>beta</li></ul>",
    "<h2>Second Section</h2>",
    "<p>Second section body.</p>",
    "<table><tr><th>X</th><th>Y</th></tr><tr><td>1</td><td>2</td></tr></table>",
    "</article></body></html>"
);

fn elements(html: &str) -> Vec<Element> {
    read_html(html).elements
}

/// web.clj's `extract-html-items`, through the document.
fn extract(parsed: Parsed) -> Vec<Item> {
    let doc = to_document(&parsed).expect("the reader's elements fit the schema");
    items(&from_document(&doc))
}

fn extract_html(html: &str) -> Vec<Item> {
    extract(read_html(html))
}

#[test]
fn html_to_structured_elements() {
    let _budget = TestBudget::new();
    let els = elements(ARTICLE_HTML);
    // h1 → :title (level 0); h2 → :section-header level=1
    assert_eq!(
        (&els[0].label, els[0].text.as_str()),
        (&Label::Title, "The Doc Title")
    );
    let header = |t: &str| els.iter().find(|e| e.text == t).map(|e| e.label.clone());
    assert_eq!(header("First Section"), Some(Label::SectionHeader(1)));
    assert_eq!(header("Second Section"), Some(Label::SectionHeader(1)));
    // paragraphs become :text
    assert!(
        els.iter()
            .any(|e| e.label == Label::Text && e.text == "First section body.")
    );
    // ul/ol expands to one :list-item per li
    let list: Vec<&str> = (els.iter())
        .filter(|e| e.label == Label::ListItem)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(list, ["alpha", "beta"]);
    // table captured with :rows for structured access
    let table = els.iter().find_map(|e| match &e.label {
        Label::Table(t) => Some(t.rows.clone()),
        _ => None,
    });
    assert_eq!(
        table,
        Some(vec![
            vec!["X".to_string(), "Y".into()],
            vec!["1".into(), "2".into()]
        ])
    );
}

#[test]
fn extract_yields_chunks_per_section() {
    let _budget = TestBudget::new();
    // h1 + 2× h2 → 3 chunks (intro + section A + section B)
    let items = extract_html(ARTICLE_HTML);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        ["The Doc Title", "First Section", "Second Section"],
        "intro chunk titled by the doc title"
    );
    // second-section chunk contains the table-rendered text
    assert!(
        items[2].fts.contains("X | Y") && items[2].fts.contains("1 | 2"),
        "{}",
        items[2].fts
    );
}

#[test]
fn empty_html_degrades_gracefully() {
    let _budget = TestBudget::new();
    // no body content + OG description → single description-only item
    let html = "<html><head><meta property=\"og:description\" content=\"only OG\"/></head><body></body></html>";
    let items = extract_html(html);
    assert_eq!(
        items.iter().map(|i| i.fts.as_str()).collect::<Vec<_>>(),
        ["only OG"]
    );
    // fully empty html → no items
    let none = extract_html("<html><body></body></html>");
    assert!(none.is_empty(), "an empty page indexes nothing: {none:?}");
}

#[test]
fn nested_blocks_not_double_counted() {
    let _budget = TestBudget::new();
    // <article><p>foo</p></article> → one :text item, not two
    let els = elements("<html><body><article><p>only para</p></article></body></html>");
    assert_eq!(
        els.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
        ["only para"]
    );
}

#[test]
fn hyperlinks_attach_as_refs() {
    let _budget = TestBudget::new();
    let html = concat!(
        "<html><body><article>",
        "<h2><a href=\"https://example.com/h\">Heading link</a></h2>",
        "<p>See <a href=\"https://example.com/docs\">the docs</a>",
        " or <a href=\"https://example.com/blog\">our blog</a>.</p>",
        "<ul>",
        "<li>plain item</li>",
        "<li><a href=\"https://example.com/a\">linked</a> item</li>",
        "</ul>",
        "</article></body></html>"
    );
    let els = elements(html);
    let pairs = |e: &Element| {
        e.refs
            .iter()
            .map(|r| (r.anchor.clone(), r.href.clone()))
            .collect::<Vec<_>>()
    };
    // section header carries a single ref with offsets
    let header = els
        .iter()
        .find(|e| matches!(e.label, Label::SectionHeader(_)))
        .expect("the h2");
    assert_eq!(
        pairs(header),
        [("Heading link".into(), "https://example.com/h".into())]
    );
    assert!(header.refs[0].span.is_some(), "the ref has its offsets");
    // paragraph with two links carries refs in document order
    let p = els
        .iter()
        .find(|e| e.label == Label::Text)
        .expect("the paragraph");
    assert_eq!(
        pairs(p),
        [
            (
                "the docs".to_string(),
                "https://example.com/docs".to_string()
            ),
            ("our blog".into(), "https://example.com/blog".into())
        ]
    );
    let at = |i: usize| p.refs[i].span.map(|(s, e)| &p.text[s..e]);
    assert_eq!((at(0), at(1)), (Some("the docs"), Some("our blog")));
    // plain list-item has no refs; linked list-item carries refs
    let item = |t: &str| {
        els.iter()
            .find(|e| e.label == Label::ListItem && e.text.contains(t))
            .expect("the item")
    };
    assert!(
        item("plain item").refs.is_empty(),
        "a plain item has no refs"
    );
    assert_eq!(
        pairs(item("linked")),
        [("linked".into(), "https://example.com/a".into())]
    );
}

#[test]
fn hyperlinks_disambiguate_repeated_anchor() {
    let _budget = TestBudget::new();
    let html = "<p>click <a href=\"https://example.com/1\">here</a> or <a href=\"https://example.com/2\">here</a>.</p>";
    // two <a>s with identical anchor text bind to their own href via offsets
    let p = &elements(html)[0];
    let hrefs: Vec<&str> = p.refs.iter().map(|r| r.href.as_str()).collect();
    assert_eq!(hrefs, ["https://example.com/1", "https://example.com/2"]);
    assert_ne!(
        p.refs[0].span, p.refs[1].span,
        "offsets point to different places of « here »"
    );
    // render-with-refs splices both into the right positions
    let fts = &extract_html(html)[0].fts;
    assert!(
        fts.contains("[here](https://example.com/1)")
            && fts.contains("[here](https://example.com/2)"),
        "{fts}"
    );
    assert_eq!(
        fts.matches("[here]").count(),
        2,
        "both substitutions present: {fts}"
    );
}

#[test]
fn hyperlinks_render_as_gfm_links() {
    let _budget = TestBudget::new();
    let html = "<html><body><article><h1>Doc</h1><p>See <a href=\"https://example.com/docs\">the docs</a>.</p></article></body></html>";
    let fts: Vec<String> = extract_html(html).into_iter().map(|i| i.fts).collect();
    assert!(
        fts.join("\n")
            .contains("[the docs](https://example.com/docs)"),
        "{fts:?}"
    );
}

#[test]
fn text_outside_blocks_is_read() {
    let _budget = TestBudget::new();
    // learn-from-mineru: text right inside a div, cell or noscript was lost
    let els = elements(concat!(
        "<html><body><div>Order 4417 shipped <b>today</b>.</div>",
        "<div><p>para</p>tail text</div>",
        "<noscript>Price without JavaScript: 42 EUR</noscript></body></html>"
    ));
    let texts: Vec<&str> = els.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Order 4417 shipped today.",
            "para",
            "tail text",
            "Price without JavaScript: 42 EUR"
        ]
    );
}

#[test]
fn script_links_are_not_refs() {
    let _budget = TestBudget::new();
    let html = "<html><body><p><a href=\"javascript:alert(1)\">run</a> and <a href=\"https://x.org\">go</a></p></body></html>";
    let hrefs: Vec<String> = elements(html)[0]
        .refs
        .iter()
        .map(|r| r.href.clone())
        .collect();
    assert_eq!(hrefs, ["https://x.org"]);
}

#[test]
fn katex_formulas_are_read_once() {
    let _budget = TestBudget::new();
    let els = elements(concat!(
        "<html><body><p>Energy <span class=\"katex\"><span class=\"katex-mathml\"><math><semantics>",
        "<mrow><mi>E</mi><mo>=</mo><mi>m</mi><msup><mi>c</mi><mn>2</mn></msup></mrow>",
        "<annotation encoding=\"application/x-tex\">E=mc^2</annotation></semantics></math></span>",
        "<span class=\"katex-html\" aria-hidden=\"true\"><span class=\"mord mathnormal\">E</span>",
        "<span class=\"mrel\">=</span><span class=\"mord mathnormal\">m</span><span class=\"mord\">c</span>",
        "<span class=\"msupsub\">2</span></span></span> holds.</p></body></html>"
    ));
    let text = &els[0].text;
    assert_eq!(
        text.matches("E=mc^2").count(),
        1,
        "the TeX source is kept once: {text}"
    );
    assert!(
        !text.contains("E = m c 2"),
        "the hidden layout copy is gone: {text}"
    );
    assert!(
        text.matches("E=mc2").count() < 2,
        "the hidden layout copy is gone: {text}"
    );
}

fn fixture(rel: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/liteparse/{rel}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// liteparse_corpus_test.clj's `snapshot-text`: each item under its locator, here none, as graal's snapshots hold them.
fn snapshot(items: &[Item]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|i| format!("### nil\n{}", i.fts))
        .collect();
    parts.join("\n\n")
}

/// `fast-output-matches-snapshot` on hello.html: a heading-only page indexes its heading (compare-liteparse).
#[test]
fn a_heading_only_page_matches_its_snapshot() {
    let _budget = TestBudget::new();
    let items = extract_html(&fixture("hello.html"));
    assert_eq!(snapshot(&items), fixture("snapshot/hello.html.txt"));
}

/// `svg-text-is-indexed` and `fast-output-matches-snapshot` on sample.svg: an SVG's words, its geometry not.
#[test]
fn svg_text_is_indexed() {
    let _budget = TestBudget::new();
    let items = extract(read_svg(&fixture("sample.svg")));
    assert!(
        items[0].fts.contains("Love comes through the stomach."),
        "{items:?}"
    );
    assert_eq!(snapshot(&items), fixture("snapshot/sample.svg.txt"));
}

/// /code-review on PR 1435: a figure's code or table is read, not only its picture and caption.
#[test]
fn a_figure_keeps_its_code_and_caption() {
    let _budget = TestBudget::new();
    let els = elements("<figure><pre>let x = 1;</pre><figcaption>Listing 1</figcaption></figure>");
    let texts: Vec<(&Label, &str)> = els.iter().map(|e| (&e.label, e.text.as_str())).collect();
    assert_eq!(
        texts,
        [(&Label::Code, "let x = 1;"), (&Label::Text, "Listing 1")]
    );
}

/// /code-review on PR 1435: an article's own header keeps its h1; a site header outside it still goes.
#[test]
fn an_article_header_keeps_its_title() {
    let _budget = TestBudget::new();
    let html = "<body><header>Site menu</header><article><header><h1>Post title</h1></header><p>Body.</p></article></body>";
    let els = elements(html);
    assert_eq!(
        (&els[0].label, els[0].text.as_str()),
        (&Label::Title, "Post title")
    );
    assert!(
        els.iter().all(|e| !e.text.contains("Site menu")),
        "the site header is gone: {els:?}"
    );
}

/// /code-review on PR 1435: an inline SVG's words are its diagram's, not also the paragraph's.
#[test]
fn an_inline_svg_is_read_once() {
    let _budget = TestBudget::new();
    let els = elements("<p>See <svg><text>Revenue 2025</text></svg></p>");
    let texts: Vec<&str> = els.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["See", "Revenue 2025"]);
    assert!(matches!(els[1].label, Label::Diagram { .. }), "{els:?}");
}
