//! The HTML and SVG readers, ported from graal's `lib/extraction/web.clj` (Jsoup there, html5ever through dom_query here):
//! a page's main content as Flare's elements, in document order. Beyond Flare, so `dash doc import` keeps them: an image or
//! an inline SVG inside a paragraph, a list item or a figure becomes its own block after it, and an SVG at block level is a
//! diagram block holding its markup, its words its text (Flare read those words as loose text).
use crate::doc::{Element, Label, Meta, Parsed, Reader, Ref};
use dom_query::{Document, NodeRef};

/// web.clj's `block-selector`: the tags read as one block each.
/// `figure` is a container here, where web.clj read it as one picture: its code, table or text are read as anywhere else.
const BLOCKS: [&str; 14] = [
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "blockquote",
    "pre",
    "ul",
    "ol",
    "table",
    "img",
    "svg",
];

/// web.clj's `container-tags`: they separate text without being a block, so text right inside one is a paragraph.
const CONTAINERS: [&str; 26] = [
    "div",
    "figure",
    "section",
    "article",
    "main",
    "td",
    "th",
    "dd",
    "dt",
    "dl",
    "details",
    "summary",
    "form",
    "center",
    "address",
    "figcaption",
    "label",
    "fieldset",
    "legend",
    "li",
    "tr",
    "tbody",
    "thead",
    "body",
    "noscript",
    "aside",
];

/// The tags Jsoup's `Element.text()` puts a space around, so two cells or paragraphs never run together.
const SPACED: [&str; 38] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "nav",
    "noscript",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

fn name(n: &NodeRef) -> String {
    n.node_name().map(|t| t.to_string()).unwrap_or_default()
}

fn push_collapsed(acc: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_whitespace() {
            if !acc.is_empty() && !acc.ends_with(' ') {
                acc.push(' ');
            }
        } else {
            acc.push(c);
        }
    }
}

/// Jsoup's `Element.text()`: its words, whitespace collapsed to one space, a space at each block's edge and each `br`.
pub fn text_of(n: &NodeRef) -> String {
    fn go(n: &NodeRef, acc: &mut String) {
        for c in n.children() {
            if c.is_text() {
                push_collapsed(acc, &c.text());
            } else if c.is_element() {
                let tag = name(&c);
                // an inline SVG is its own diagram block, its words read there once
                if tag == "svg" {
                    continue;
                }
                let spaced = SPACED.contains(&tag.as_str());
                if spaced || tag == "br" {
                    push_collapsed(acc, " ");
                }
                go(&c, acc);
                if spaced {
                    push_collapsed(acc, " ");
                }
            }
        }
    }
    let mut acc = String::new();
    go(n, &mut acc);
    acc.trim().to_string()
}

fn descendants<'a>(n: &NodeRef<'a>, tags: &[&str]) -> Vec<NodeRef<'a>> {
    (n.descendants().into_iter())
        .filter(|d| d.is_element() && tags.contains(&name(d).as_str()))
        .collect()
}

/// web.clj's `block-refs`: each `a[href]` with its anchor's place in the block's text, found left to right so a repeated
/// anchor binds to its own link.
fn block_refs(el: &NodeRef) -> Vec<Ref> {
    let text = text_of(el);
    let mut cursor = 0;
    let mut out = vec![];
    for a in descendants(el, &["a"]) {
        let Some(href) = a.attr("href").map(|h| h.to_string()) else {
            continue;
        };
        let anchor = text_of(&a);
        if anchor.trim().is_empty() || href.trim().is_empty() {
            continue;
        }
        let span = text[cursor..]
            .find(&anchor)
            .map(|i| (cursor + i, cursor + i + anchor.len()));
        if let Some((_, end)) = span {
            cursor = end;
        }
        out.push(Ref { anchor, href, span });
    }
    out
}

/// web.clj's `extract-svg-items`' words: `<title>`, `<desc>` and each `<text>`, in document order.
fn svg_words(n: &NodeRef) -> String {
    let words: Vec<String> = (descendants(n, &["title", "desc", "text"]).iter())
        .map(|t| text_of(t))
        .filter(|t| !t.is_empty())
        .collect();
    words.join("\n")
}

fn picture(n: &NodeRef) -> Option<Element> {
    let src = n.attr("src").map(|s| s.to_string()).unwrap_or_default();
    let alt = n
        .attr("alt")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    (!src.is_empty() || !alt.is_empty()).then(|| Element::new(Label::Picture { src }, &alt))
}

fn diagram(n: &NodeRef) -> Element {
    Element::new(
        Label::Diagram {
            svg: n.html().to_string(),
        },
        &svg_words(n),
    )
}

/// The images and inline SVGs inside a block, each its own element: an SVG's insides are not read again.
fn media(el: &NodeRef) -> Vec<Element> {
    let inside_svg = |d: &NodeRef| {
        d.ancestors(None)
            .iter()
            .any(|a| name(a) == "svg" && a.id != el.id)
    };
    let found = descendants(el, &["img", "svg"])
        .into_iter()
        .filter(|d| !inside_svg(d));
    found
        .filter_map(|d| {
            if name(&d) == "svg" {
                Some(diagram(&d))
            } else {
                picture(&d)
            }
        })
        .collect()
}

fn texted(label: Label, el: &NodeRef) -> Option<Element> {
    let text = text_of(el);
    (!text.is_empty()).then(|| Element::new(label, &text).with_refs(block_refs(el)))
}

/// web.clj's `element->item`, one tag's elements.
fn items_of(el: &NodeRef) -> Vec<Element> {
    let tag = name(el);
    let mut out: Vec<Element> = match tag.as_str() {
        "h1" => texted(Label::Title, el).into_iter().collect(),
        "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag[1..].parse::<u8>().unwrap_or(2) - 1;
            texted(Label::SectionHeader(level), el)
                .into_iter()
                .collect()
        }
        "p" | "blockquote" => texted(Label::Text, el).into_iter().collect(),
        "pre" => {
            let text = text_of(el);
            (!text.is_empty())
                .then(|| Element::new(Label::Code, &raw_text(el)))
                .into_iter()
                .collect()
        }
        // web.clj's `list-elements`: one item per direct li, a nested list read as its parent's words
        "ul" | "ol" => (el.element_children().iter())
            .filter(|li| name(li) == "li")
            .flat_map(|li| texted(Label::ListItem, li).into_iter().chain(media(li)))
            .collect(),
        "table" => {
            let rows: Vec<Vec<String>> = (descendants(el, &["tr"]).iter())
                .map(|tr| descendants(tr, &["th", "td"]).iter().map(text_of).collect())
                .collect();
            let text = rows
                .iter()
                .map(|r| r.join(" | "))
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty())
                .then(|| Element::new(Label::Table(rows), &text))
                .into_iter()
                .collect()
        }
        "img" => picture(el).into_iter().collect(),
        "svg" => return vec![diagram(el)],
        _ => vec![],
    };
    if !matches!(tag.as_str(), "ul" | "ol" | "img") {
        out.extend(media(el));
    }
    out
}

/// A `pre`'s text as written, its lines kept (Jsoup's `text()` of a `pre` keeps them too), trimmed.
fn raw_text(el: &NodeRef) -> String {
    el.text().trim().to_string()
}

fn meta(doc: &Document) -> Meta {
    let content = |sel: &str| {
        let v = doc
            .select(sel)
            .first()
            .attr("content")
            .map(|v| v.to_string())
            .unwrap_or_default();
        (!v.trim().is_empty()).then_some(v)
    };
    let title = doc.select("head > title").first().text().trim().to_string();
    Meta {
        title: content(r#"meta[property="og:title"]"#).or((!title.is_empty()).then_some(title)),
        description: content(r#"meta[property="og:description"]"#)
            .or(content(r#"meta[name="description"]"#)),
        image: content(r#"meta[property="og:image"]"#),
        preface: None,
    }
}

/// web.clj's `html->elements`: the main content (`article`, else `main`, else `body`, once navigation, scripts and
/// KaTeX's hidden copy are gone) walked in order, each block its elements, loose text in a container a paragraph.
pub fn read_html(html: &str) -> Parsed {
    let doc = Document::from(html);
    let meta = meta(&doc);
    doc.select("script, style, nav, footer, aside, .katex-html")
        .remove();
    // a site's header goes, an article's own (its h1, its byline) stays: web.clj dropped both
    for header in doc.select("header").nodes() {
        let held = |a: &NodeRef| matches!(name(a).as_str(), "article" | "main");
        if !header.ancestors(None).iter().any(held) {
            header.remove_from_parent();
        }
    }
    let content = ["article", "main", "body"]
        .iter()
        .map(|s| doc.select(s).first())
        .find(|s| s.exists());
    let mut out = vec![];
    let mut run = String::new();
    fn flush(run: &mut String, out: &mut Vec<Element>) {
        let lines: Vec<String> = (run.split('\n'))
            .map(|l| {
                l.split([' ', '\t', '\x0B', '\x0C', '\r'])
                    .filter(|w| !w.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let text = lines.join("\n").trim().to_string();
        run.clear();
        if !text.is_empty() {
            out.push(Element::new(Label::Text, &text));
        }
    }
    fn walk(n: &NodeRef, run: &mut String, out: &mut Vec<Element>) {
        if n.is_text() {
            let t = n.text();
            let mut collapsed = String::new();
            for c in t.chars() {
                match c.is_whitespace() {
                    true if collapsed.ends_with(' ') => {}
                    true => collapsed.push(' '),
                    false => collapsed.push(c),
                }
            }
            run.push_str(&collapsed);
        } else if n.is_element() {
            let tag = name(n);
            if BLOCKS.contains(&tag.as_str()) {
                flush(run, out);
                out.extend(items_of(n));
            } else if tag == "br" {
                run.push('\n');
            } else if CONTAINERS.contains(&tag.as_str()) {
                flush(run, out);
                n.children().iter().for_each(|c| walk(c, run, out));
                flush(run, out);
            } else {
                n.children().iter().for_each(|c| walk(c, run, out));
            }
        }
    }
    if let Some(content) = content.and_then(|s| s.nodes().first().cloned()) {
        content
            .children()
            .iter()
            .for_each(|c| walk(c, &mut run, &mut out));
        flush(&mut run, &mut out);
    }
    Parsed {
        meta,
        elements: out,
    }
}

/// A file's text: UTF-8, a byte-order mark dropped.
fn decoded(bytes: &[u8]) -> String {
    // ponytail: UTF-8 only; web.clj's `decode-html` also reads a <meta> charset (GBK, 1252), port it with a page that needs it
    String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).into_owned()
}

fn head_has(bytes: &[u8], needle: &str) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    head.contains(needle)
}

inventory::submit! {
    Reader {
        format: "html",
        extensions: &["html", "htm"],
        sniff: |b| head_has(b, "!doctype html"),
        read: |b| Ok(read_html(&decoded(b))),
    }
}

/// An SVG file: one diagram, its markup whole, its words what an index reads (web.clj's `extract-svg-items`). SVG is XML,
/// so the raster image path read nothing from it.
pub fn read_svg(svg: &str) -> Parsed {
    let doc = Document::from(svg);
    let words = doc
        .select("svg")
        .first()
        .nodes()
        .first()
        .map(svg_words)
        .unwrap_or_default();
    let elements = vec![Element::new(
        Label::Diagram {
            svg: svg.to_string(),
        },
        &words,
    )];
    Parsed {
        meta: Meta::default(),
        elements,
    }
}

inventory::submit! {
    Reader {
        format: "svg",
        extensions: &["svg"],
        // an HTML page holding an inline SVG names the namespace too: its doctype keeps it the HTML reader's
        sniff: |b| head_has(b, "http://www.w3.org/2000/svg") && !head_has(b, "!doctype html"),
        read: |b| Ok(read_svg(&decoded(b))),
    }
}
