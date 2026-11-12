//! The PDF reader, its text layer: ported from graal's `lib/extraction/pdf.clj` (PDFium there, pdf-extract over lopdf here,
//! pure Rust so the Linux CI builds it). A page's text blocks are paragraphs, a bookmark's title on its page a heading, the
//! page's link annotations a `Links:` line opening it, and the `/Info` dictionary the document's one-line preface. OCR,
//! tables, figures and running headers and footers need PDFium's layout and are task rust-pdf-ocr-tables. A reader alone:
//! the core and the two commands take it as they are (rfc-rust-unified section 20i). A crate of its own, out of dash-ingest
//! (task rust-heavy-crates-split, rfc-rust-unified#cache): a heavy dependency in a leaf crate, so a change elsewhere
//! never rebuilds or tests it (@nha, 2026-11-11: « we can then skip CI if the crate doesn't change »).
use dash_ingest::doc::{Element, Label, Meta, Parsed, Reader};
use pdf_extract::{Dictionary, Document, Object, ObjectId};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// pdf.clj's `parse-pdf-date`: `D:YYYYMMDD…` as `YYYY-MM-DD`, anything else none.
pub fn parse_pdf_date(s: &str) -> Option<String> {
    let digits = s.strip_prefix("D:")?.get(..8)?;
    digits
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| format!("{}-{}-{}", &digits[..4], &digits[4..6], &digits[6..]))
}

/// pdf.clj's `render-info-preface`: « PDF · Title · Author · date · Subject · Keywords », the producer or creator only when
/// none of those four is there, one line whatever the values hold; none when only dates are.
pub fn render_info_preface(info: &BTreeMap<String, String>) -> Option<String> {
    let get = |k: &str| info.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let date = (get("CreationDate").and_then(parse_pdf_date))
        .or_else(|| get("ModDate").and_then(parse_pdf_date));
    let any_text = ["Title", "Author", "Subject", "Keywords"]
        .iter()
        .any(|k| get(k).is_some());
    let fallback = (!any_text)
        .then(|| get("Producer").or(get("Creator")))
        .flatten();
    if !any_text && fallback.is_none() {
        return None;
    }
    let mut parts = vec!["PDF".to_string()];
    parts.extend(
        [get("Title"), get("Author")]
            .into_iter()
            .flatten()
            .map(str::to_string),
    );
    parts.extend(date);
    parts.extend(
        [get("Subject"), get("Keywords"), fallback]
            .into_iter()
            .flatten()
            .map(str::to_string),
    );
    let one_line = |p: &String| p.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(parts.iter().map(one_line).collect::<Vec<_>>().join(" · "))
}

/// pdf.clj's `strip-info-preface`: the text without the `PDF · …` line put before the first page.
pub fn strip_info_preface(s: &str) -> &str {
    (s.strip_prefix("PDF · "))
        .and_then(|rest| rest.split_once("\n\n"))
        .filter(|(line, _)| !line.contains('\n'))
        .map_or(s, |(_, body)| body)
}

/// A bookmark: its title, its depth from 1, its page from 1 (lopdf's numbering, where PDFium's is from 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub title: String,
    pub level: usize,
    pub page: u32,
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// pdf.clj's `outline-headings`: on a bookmark's page, a text element that is its title becomes a heading; one that opens
/// with its title and runs on into a sentence (a capital, a digit, a bracket or a quote next) splits into the two.
pub fn outline_headings(
    outline: &[Bookmark],
    by_page: BTreeMap<u32, Vec<Element>>,
) -> BTreeMap<u32, Vec<Element>> {
    // each title with its bookmark's depth, so « 3.1 Data » stays under « 3 Method » (pdf.clj made every one level 1)
    let mut titles: BTreeMap<u32, Vec<(String, u8)>> = BTreeMap::new();
    for b in outline {
        let t = squash(&b.title);
        let page = titles.entry(b.page).or_default();
        if t.chars().count() >= 3 && !page.iter().any(|(seen, _)| *seen == t) {
            page.push((t, b.level.clamp(1, 5) as u8));
        }
    }
    let promote = |page: u32, el: Element| -> Vec<Element> {
        let text = squash(&el.text);
        let lower: Vec<char> = text.to_lowercase().chars().collect();
        let opens = |t: &&(String, u8)| {
            let t: Vec<char> = t.0.to_lowercase().chars().collect();
            lower.len() >= t.len() && lower[..t.len()] == t[..]
        };
        let title = (el.label == Label::Text)
            .then(|| titles.get(&page))
            .flatten()
            .and_then(|ts| ts.iter().filter(opens).max_by_key(|t| t.0.chars().count()));
        let Some((title, level)) = title else {
            return vec![el];
        };
        let n = title.chars().count();
        let head: String = text.chars().take(n).collect();
        let rest: String = text.chars().skip(n).collect::<String>().trim().to_string();
        let runs_on = rest
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_numeric() || "([“\"".contains(c));
        match (rest.is_empty(), runs_on) {
            (true, _) => vec![Element {
                label: Label::SectionHeader(*level),
                ..el
            }],
            (false, true) => vec![
                Element {
                    label: Label::SectionHeader(*level),
                    text: head.trim().to_string(),
                    ..el.clone()
                },
                Element { text: rest, ..el },
            ],
            _ => vec![el],
        }
    };
    if titles.is_empty() {
        return by_page;
    }
    (by_page.into_iter())
        .map(|(page, els)| {
            (
                page,
                els.into_iter().flat_map(|e| promote(page, e)).collect(),
            )
        })
        .collect()
}

/// A destination's page: an array opening with the page, or a name looked up in the catalog's `/Dests` or its `/Names` tree.
fn dest_page(
    doc: &Document,
    dest: &Object,
    named: &BTreeMap<Vec<u8>, Object>,
    pages: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    match doc.dereference(dest).ok()?.1 {
        Object::Array(a) => pages.get(&a.first()?.as_reference().ok()?).copied(),
        Object::String(name, _) | Object::Name(name) => {
            let target = doc.dereference(named.get(name)?).ok()?.1;
            let target = target
                .as_dict()
                .ok()
                .and_then(|d| d.get(b"D").ok())
                .unwrap_or(target);
            dest_page(doc, target, &BTreeMap::new(), pages)
        }
        _ => None,
    }
}

/// Every named destination: the catalog's `/Dests` dictionary and its `/Names` `/Dests` tree, leaves and kids. lopdf's own
/// `get_toc` resolves none, so WHO's 156 bookmarks read as none.
fn named_dests(doc: &Document) -> BTreeMap<Vec<u8>, Object> {
    let mut out = BTreeMap::new();
    let Ok(catalog) = doc.catalog() else {
        return out;
    };
    let dict = |o: &Object| {
        doc.dereference(o)
            .ok()
            .and_then(|(_, o)| o.as_dict().ok().cloned())
    };
    if let Some(d) = catalog.get(b"Dests").ok().and_then(dict) {
        out.extend(d.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let tree = catalog
        .get(b"Names")
        .ok()
        .and_then(dict)
        .and_then(|n| n.get(b"Dests").ok().and_then(dict));
    let mut todo: Vec<Dictionary> = tree.into_iter().collect();
    // a bound, so a tree whose kids loop back ends
    let mut nodes = 0;
    while let Some(node) = todo.pop() {
        nodes += 1;
        if nodes > 100_000 {
            break;
        }
        if let Ok(names) = node.get(b"Names").and_then(Object::as_array) {
            for pair in names.chunks(2) {
                if let [Object::String(k, _), v] = pair {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        if let Ok(kids) = node.get(b"Kids").and_then(Object::as_array) {
            todo.extend(kids.iter().filter_map(dict));
        }
    }
    out
}

/// The document's bookmarks, in order, each with its depth and page (pdf.clj reads them with PDFium's `outline`): none
/// when it has no outline. A bookmark whose page is not found is left out.
pub fn outline(doc: &Document) -> Vec<Bookmark> {
    let pages: BTreeMap<ObjectId, u32> = doc
        .get_pages()
        .into_iter()
        .map(|(no, id)| (id, no))
        .collect();
    let named = named_dests(doc);
    let first = |d: &Dictionary| d.get(b"First").ok().and_then(|f| f.as_reference().ok());
    let root = (doc.catalog().ok())
        .and_then(|c| c.get(b"Outlines").ok())
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok());
    let mut todo: Vec<(ObjectId, usize)> =
        root.and_then(first).map(|f| (f, 1)).into_iter().collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut out = vec![];
    while let Some((id, level)) = todo.pop() {
        let Some(item) = doc.get_dictionary(id).ok().filter(|_| seen.insert(id)) else {
            continue;
        };
        let action = item
            .get(b"A")
            .ok()
            .and_then(|a| doc.dereference(a).ok())
            .and_then(|(_, a)| a.as_dict().ok());
        let dest = item
            .get(b"Dest")
            .ok()
            .or_else(|| action.and_then(|a| a.get(b"D").ok()));
        let page = dest.and_then(|d| dest_page(doc, d, &named, &pages));
        let title = item.get(b"Title").ok().and_then(|t| text_of(doc, t));
        if let (Some(title), Some(page)) = (title, page) {
            out.push(Bookmark { title, level, page });
        }
        // a stack: the next sibling goes on first, so this item's children are read before it
        if let Ok(next) = item.get(b"Next").and_then(Object::as_reference) {
            todo.push((next, level));
        }
        if let Some(child) = first(item) {
            todo.push((child, level + 1));
        }
    }
    out
}

fn text_of(doc: &Document, o: &Object) -> Option<String> {
    let o = doc.dereference(o).ok()?.1;
    pdf_extract::decode_text_string(o)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The `/Info` dictionary's text fields.
pub fn info(doc: &Document) -> BTreeMap<String, String> {
    let dict = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok());
    (dict.into_iter().flat_map(|d| d.iter()))
        .filter_map(|(k, v)| Some((String::from_utf8_lossy(k).into_owned(), text_of(doc, v)?)))
        .collect()
}

/// pdfium_panama.clj's `safe-link`: a scheme the reader can go to.
fn safe_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:", "tel:", "ftp:"]
        .iter()
        .any(|s| lower.starts_with(s) && lower.len() > s.len())
}

/// What PDFium's `FPDFLink_LoadWebLinks` finds in a page's text: a web address, or a mail address as `mailto:`, its domain
/// ending in letters (pdfium_panama.clj's `plausible-link?`: a paper's `mAP@0.5-0.95` is no address).
pub fn text_links(text: &str) -> Vec<String> {
    static WEB: LazyLock<Regex> = LazyLock::new(|| {
        let web = r"\bhttps?://[^\s<>()\[\]{}]+";
        let mail = r"\b[a-z0-9._%+-]+@(?:[a-z0-9-]+\.)+[a-z]{2,}\b";
        Regex::new(&format!("(?i){web}|{mail}")).expect("a valid pattern")
    });
    let found = WEB
        .find_iter(text)
        .map(|m| m.as_str().trim_end_matches(['.', ',', ';', ':', '\'', '"']));
    found
        .map(|l| {
            if l.contains("://") {
                l.to_string()
            } else {
                format!("mailto:{l}")
            }
        })
        .collect()
}

/// pdf.clj's `page-links`: the distinct targets of a page's URI link annotations, then of the addresses in its text.
pub fn page_links(doc: &Document, page: ObjectId, text: &str) -> Vec<String> {
    let annotations = (doc
        .get_page_annotations(page)
        .unwrap_or_default()
        .into_iter())
    .filter_map(|annot| {
        let action = annot
            .get(b"A")
            .ok()
            .and_then(|a| doc.dereference(a).ok())
            .and_then(|(_, a)| a.as_dict().ok());
        action
            .and_then(|a| a.get(b"URI").ok())
            .and_then(|u| text_of(doc, u))
    });
    let mut links: Vec<String> = vec![];
    for link in annotations.chain(text_links(text)).filter(|l| safe_link(l)) {
        if !links.contains(&link) {
            links.push(link);
        }
    }
    links
}

/// A page's text blocks, a blank line between two, each one paragraph with its lines joined.
fn paragraphs(page_text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut block: Vec<&str> = vec![];
    for line in page_text.lines().map(str::trim).chain([""]) {
        if line.is_empty() {
            if !block.is_empty() {
                out.push(block.join(" "));
                block.clear();
            }
        } else {
            block.push(line);
        }
    }
    out
}

pub fn read_pdf(bytes: &[u8]) -> Result<Parsed, String> {
    let unreadable = |e: &dyn std::fmt::Display| format!("not a readable PDF: {e}");
    let mut doc = Document::load_mem(bytes).map_err(|e| unreadable(&e))?;
    if doc.is_encrypted() {
        doc.decrypt("").map_err(|e| unreadable(&e))?;
    }
    // Each page read on the one loaded document. pdf-extract's by-pages call parses the file again and stops at the first
    // page it cannot read, dropping every page after it with no error (/code-review on PR 1435): a page that fails names
    // itself here.
    let mut by_page: BTreeMap<u32, Vec<Element>> = BTreeMap::new();
    for (no, id) in doc.get_pages() {
        let mut page_text = String::new();
        pdf_extract::output_doc_page(
            &doc,
            &mut pdf_extract::PlainTextOutput::new(&mut page_text),
            no,
        )
        .map_err(|e| unreadable(&format!("page {no}: {e}")))?;
        let text = &page_text;
        let on_page = |label, text: &str| Element {
            page: Some(no),
            ..Element::new(label, text)
        };
        let links = page_links(&doc, id, text);
        let mut els: Vec<Element> = vec![];
        if !links.is_empty() {
            els.push(on_page(Label::Text, &format!("Links: {}", links.join(" "))));
        }
        els.extend(paragraphs(text).iter().map(|p| on_page(Label::Text, p)));
        by_page.insert(no, els);
    }
    let elements = outline_headings(&outline(&doc), by_page)
        .into_values()
        .flatten()
        .collect();
    let meta = Meta {
        preface: render_info_preface(&info(&doc)),
        ..Meta::default()
    };
    Ok(Parsed { meta, elements })
}

inventory::submit! {
    Reader {
        format: "pdf",
        extensions: &["pdf"],
        sniff: |b| b.starts_with(b"%PDF-"),
        read: read_pdf,
    }
}
