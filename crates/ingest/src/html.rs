//! The HTML and SVG readers, ported from graal's `lib/extraction/web.clj` (Jsoup there, html5ever through dom_query here):
//! a page's main content as Flare's elements, in document order. Beyond Flare, so `dash doc import` keeps them: an image or
//! an inline SVG inside a paragraph, a list item or a figure becomes its own block after it, and an SVG at block level is a
//! diagram block holding its markup, its words its text (Flare read those words as loose text).
use crate::doc::{Chip, Decision, Element, Label, Meta, Parsed, Reader, Ref, Table};
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
                // an inline SVG is its own diagram block, writer notes their own blocks: their words read there once
                if tag == "svg" || is_notes(&c) {
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

/// Where `target`'s words start in `text_of(el)`, at most one collapsed space early: the words before it read as `text_of`
/// reads them. A search from there binds a chip or a link to its own words, never to the same words earlier in the block
/// (/code-review of PR 1592: `open questions <span class="chip">open</span>` marked the first « open »).
fn place_of(el: &NodeRef, target: &NodeRef, text: &str) -> usize {
    fn go(n: &NodeRef, target: &NodeRef, acc: &mut String) -> bool {
        for c in n.children() {
            if c.id == target.id {
                return true;
            }
            if c.is_text() {
                push_collapsed(acc, &c.text());
            } else if c.is_element() {
                let tag = name(&c);
                if tag == "svg" || is_notes(&c) {
                    continue;
                }
                let spaced = SPACED.contains(&tag.as_str());
                if spaced || tag == "br" {
                    push_collapsed(acc, " ");
                }
                if go(&c, target, acc) {
                    return true;
                }
                if spaced {
                    push_collapsed(acc, " ");
                }
            }
        }
        false
    }
    let mut acc = String::new();
    go(el, target, &mut acc);
    let mut at = acc.trim_start().len().saturating_sub(1).min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
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
    // a link in the block's own writer notes is theirs: their block reads it
    for a in descendants(el, &["a"]).into_iter().filter(|a| own(el, a)) {
        let Some(href) = a.attr("href").map(|h| h.to_string()) else {
            continue;
        };
        let anchor = text_of(&a);
        if anchor.trim().is_empty() || href.trim().is_empty() {
            continue;
        }
        let from = cursor.max(place_of(el, &a, &text));
        let span = text[from..]
            .find(&anchor)
            .map(|i| from + i)
            .or_else(|| text[cursor..].find(&anchor).map(|i| cursor + i))
            .map(|i| (i, i + anchor.len()));
        if let Some((_, end)) = span {
            cursor = end;
        }
        out.push(Ref { anchor, href, span });
    }
    out
}

/// A chip's tone, its class beside `chip` (the rfc skill's `ok`, `warn`, `bad`), none when it has none.
fn tone_of(chip: &NodeRef) -> String {
    let class = chip
        .attr("class")
        .map(|c| c.to_string())
        .unwrap_or_default();
    let tone = class
        .split_whitespace()
        .find(|k| ["ok", "warn", "bad"].contains(k));
    tone.unwrap_or_default().to_string()
}

/// Each `span.chip` in the block's words, its tone and its words' place in the block's text, found left to right as
/// `block_refs` finds a link's.
fn block_chips(el: &NodeRef) -> Vec<Chip> {
    let text = text_of(el);
    let mut cursor = 0;
    let mut out = vec![];
    let chips = descendants(el, &["span"]).into_iter();
    for c in chips.filter(|c| has_class(c, "chip") && own(el, c)) {
        let words = text_of(&c);
        let from = cursor.max(place_of(el, &c, &text));
        let Some(at) = (!words.is_empty())
            .then(|| text[from..].find(&words).map(|i| from + i))
            .flatten()
        else {
            continue;
        };
        let span = (at, at + words.len());
        cursor = span.1;
        out.push(Chip {
            tone: tone_of(&c),
            span,
        });
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
    let shown = !src.is_empty() || !alt.is_empty();
    // a drawing the design system drew as an image of itself (dash-surface's SvgDrawing, one that could run code) is the
    // diagram it was: a save's export reads back to the same block
    let drawing = (has_class(n, "diagram"))
        .then(|| src.strip_prefix("data:image/svg+xml,").map(percent_decoded))
        .flatten();
    let label = match drawing {
        Some(svg) => Label::Diagram { svg },
        None => Label::Picture { src },
    };
    shown.then(|| Element {
        anchor: figure_anchor(n),
        ..Element::new(label, &alt)
    })
}

/// `%XX` read back to its byte, as a `data:` URI writes one; a bad escape stays as written.
fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        let hex = (b.get(i + 1..i + 3))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn diagram(n: &NodeRef) -> Element {
    let svg = n.html().to_string();
    Element {
        anchor: figure_anchor(n),
        ..Element::new(Label::Diagram { svg }, &svg_words(n))
    }
}

/// The images and inline SVGs inside a block, each its own element: an SVG's insides are not read again.
fn media(el: &NodeRef) -> Vec<Element> {
    // an SVG's insides are its own; the block's writer notes read their media themselves (`notes_inside`)
    let apart = |d: &NodeRef| {
        (d.ancestors(None).iter())
            .take_while(|a| a.id != el.id)
            .any(|a| name(a) == "svg" || is_notes(a))
    };
    let found = descendants(el, &["img", "svg"])
        .into_iter()
        .filter(|d| !apart(d));
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
    (!text.is_empty()).then(|| Element {
        anchor: inner_anchor(el),
        chips: block_chips(el),
        ..Element::new(label, &text).with_refs(block_refs(el))
    })
}

fn has_class(n: &NodeRef, class: &str) -> bool {
    n.attr("class")
        .is_some_and(|c| c.split_whitespace().any(|k| k == class))
}

/// Its id, a link's `#target`. An element that makes no block of its own (a section, a figure, a div) gives its id to
/// the first block made inside it that has none (`anchored`).
fn anchor_of(el: &NodeRef) -> Option<String> {
    el.attr("id")
        .map(|v| v.to_string())
        .filter(|v| !v.is_empty())
}

/// Whether `d`, inside the block `el`, is part of its words: not an image, an SVG or the block's own writer notes, nor
/// inside one, each read as a block of its own.
fn own(el: &NodeRef, d: &NodeRef) -> bool {
    let apart = |n: &NodeRef| matches!(name(n).as_str(), "svg" | "img") || is_notes(n);
    !apart(d)
        && (d.ancestors(None).iter())
            .take_while(|a| a.id != el.id)
            .all(|a| !apart(a))
}

/// Ids as a block's `anchor` holds them: each one a link lands on, space-separated as HTML's `class` (an id holds no space).
fn joined(ids: impl IntoIterator<Item = String>) -> Option<String> {
    let ids: Vec<String> = ids.into_iter().collect();
    (!ids.is_empty()).then(|| ids.join(" "))
}

/// A text block's or a row's ids: its own, then each inside its words (a `<span id>` in a point, a `<td id>` in a row).
fn inner_anchor(el: &NodeRef) -> Option<String> {
    let inside = (el.descendants().into_iter())
        .filter(|d| d.is_element() && own(el, d))
        .filter_map(|d| anchor_of(&d));
    joined(anchor_of(el).into_iter().chain(inside))
}

/// An image's or a diagram's id, and its figure's: a figure inside a point makes no block of its own.
fn figure_anchor(n: &NodeRef) -> Option<String> {
    let figure = n.parent().filter(|p| name(p) == "figure");
    joined(
        anchor_of(n)
            .into_iter()
            .chain(figure.and_then(|f| anchor_of(&f))),
    )
}

/// The rfc skill's writer notes, `details.wn`.
fn is_notes(n: &NodeRef) -> bool {
    name(n) == "details" && has_class(n, "wn")
}

/// A section's writer notes, read as any body is, its « Writer notes » summary dropped: each text block a writer note (a
/// point, a paragraph, a listing), a table or a diagram kept as it is.
fn writer_notes(details: &NodeRef) -> Vec<Element> {
    let (mut run, mut out) = (String::new(), vec![]);
    (details.children().iter())
        .filter(|c| name(c) != "summary")
        .for_each(|c| walk(c, &mut run, &mut out));
    flush(&mut run, &mut out);
    for e in &mut out {
        // a heading inside the notes is the notes', never a section of the body
        if matches!(
            e.label,
            Label::Title | Label::SectionHeader(_) | Label::Text | Label::ListItem | Label::Code
        ) {
            e.label = Label::WriterNote;
        }
    }
    anchored(details, out)
}

/// A fold (`<details>`): its summary's words, and what it holds read as any body is. One with nothing inside is its summary
/// alone, a paragraph.
fn fold(details: &NodeRef) -> Vec<Element> {
    let summary = (details.element_children().into_iter()).find(|c| name(c) == "summary");
    let words = summary.as_ref().map(text_of).unwrap_or_default();
    let (mut run, mut inside) = (String::new(), vec![]);
    (details.children().iter())
        .filter(|c| name(c) != "summary")
        .for_each(|c| walk(c, &mut run, &mut inside));
    flush(&mut run, &mut inside);
    let made = match (inside.is_empty(), words.is_empty()) {
        (true, true) => vec![],
        (true, false) => vec![Element::new(Label::Text, &words)],
        (false, _) => vec![Element::new(Label::Fold { inside }, &words)],
    };
    anchored(details, made)
}

/// The writer notes inside a block (a point's own notes), each read once, after the block: never in its words.
fn notes_inside(el: &NodeRef) -> Vec<Element> {
    // only the outermost: a notes fold inside another is read with it
    let outermost = |d: &NodeRef| {
        (d.ancestors(None).iter())
            .take_while(|a| a.id != el.id)
            .all(|a| !is_notes(a))
    };
    (el.descendants().into_iter())
        .filter(|d| d.is_element() && is_notes(d) && outermost(d))
        .flat_map(|d| writer_notes(&d))
        .collect()
}

/// `n`'s id added to the first of its blocks, where a link to `n` lands, unless one of them holds it already.
fn anchored(n: &NodeRef, mut made: Vec<Element>) -> Vec<Element> {
    let holds = |e: &Element, id: &str| (e.anchor.iter()).any(|a| a.split(' ').any(|i| i == id));
    if let Some(id) = anchor_of(n)
        && !made.iter().any(|e| holds(e, &id))
        && let Some(first) = made.first_mut()
    {
        first.anchor = joined([id].into_iter().chain(first.anchor.take()));
    }
    made
}

/// A table read whole: its rows with their ids, and for an RFC's decision its chip and question.
fn table_of(el: &NodeRef) -> Vec<Element> {
    let trs = descendants(el, &["tr"]);
    let cells: Vec<Vec<NodeRef>> = (trs.iter())
        .map(|tr| descendants(tr, &["th", "td"]))
        .collect();
    let rows: Vec<Vec<String>> = (cells.iter())
        .map(|r| r.iter().map(text_of).collect())
        .collect();
    let links = (cells.iter())
        .map(|r| r.iter().map(block_refs).collect())
        .collect();
    let chips = (cells.iter())
        .map(|r| r.iter().map(block_chips).collect())
        .collect();
    let row_ids = (trs.iter())
        .map(|tr| inner_anchor(tr).unwrap_or_default())
        .collect();
    let decision = has_class(el, "decision").then(|| {
        let Some(caption) = descendants(el, &["caption"]).into_iter().next() else {
            return Decision::default();
        };
        let chip = (descendants(&caption, &["span"]).into_iter()).find(|s| has_class(s, "chip"));
        let status = chip.as_ref().map(text_of).unwrap_or_default();
        let tone = chip.as_ref().map(tone_of).unwrap_or_default();
        // the chip's words once, in `status`: the chip itself leaves the caption, never a word of the question
        if let Some(chip) = chip {
            chip.remove_from_parent();
        }
        let caption = text_of(&caption);
        Decision {
            status,
            tone,
            caption,
        }
    });
    let t = Table {
        rows,
        row_ids,
        decision,
        links: crate::doc::linked(links),
        chips: crate::doc::linked(chips),
    };
    let text = t.text();
    match (t.rows.is_empty(), &t.decision) {
        // a decision written before its options: its question alone, a paragraph
        (true, Some(d)) => {
            let words = format!("{} {}", d.status, d.caption);
            vec![Element {
                anchor: anchor_of(el),
                ..Element::new(Label::Text, words.trim())
            }]
        }
        _ if text.trim().is_empty() => vec![],
        _ => vec![Element {
            anchor: anchor_of(el),
            ..Element::new(Label::Table(t), &text)
        }],
    }
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
                .then(|| Element {
                    anchor: anchor_of(el),
                    ..Element::new(Label::Code, &raw_text(el))
                })
                .into_iter()
                .collect()
        }
        "ol" if has_class(el, "stages") => vec![stage_of(el)],
        // web.clj's `list-elements`: one item per direct li, a nested list read as its parent's words
        "ul" | "ol" => (el.element_children().iter())
            .filter(|li| name(li) == "li")
            .flat_map(|li| {
                let made = texted(Label::ListItem, li).into_iter().chain(media(li));
                made.chain(notes_inside(li)).collect::<Vec<_>>()
            })
            .collect(),
        "table" => table_of(el),
        "img" => picture(el).into_iter().collect(),
        "svg" => return vec![diagram(el)],
        _ => vec![],
    };
    if !matches!(tag.as_str(), "ul" | "ol" | "img") {
        out.extend(media(el));
        out.extend(notes_inside(el));
    }
    anchored(el, out)
}

/// An RFC's stage (`ol.stages`): each step's words, and the one at `aria-current`, else the last one done.
fn stage_of(el: &NodeRef) -> Element {
    let lis: Vec<NodeRef> = (el.element_children().into_iter())
        .filter(|li| name(li) == "li")
        .collect();
    let steps: Vec<String> = lis.iter().map(text_of).collect();
    let at = (lis.iter().position(|li| li.attr("aria-current").is_some()))
        .or_else(|| lis.iter().rposition(|li| has_class(li, "done")));
    let current = at.map(|i| steps[i].clone()).unwrap_or_default();
    Element {
        anchor: anchor_of(el),
        ..crate::doc::stage(
            steps,
            current,
            el.attr("aria-label")
                .map(|l| l.trim().to_string())
                .unwrap_or_default(),
        )
    }
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

/// Loose text gathered so far, as one paragraph.
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

/// One node of the page's content read into its elements, in order: a block its own, a container's text a paragraph.
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
        } else if is_notes(n) {
            flush(run, out);
            out.extend(writer_notes(n));
        } else if tag == "details" {
            flush(run, out);
            out.extend(fold(n));
        } else if tag == "br" {
            run.push('\n');
        } else if CONTAINERS.contains(&tag.as_str()) {
            flush(run, out);
            let mut made = vec![];
            n.children().iter().for_each(|c| walk(c, run, &mut made));
            flush(run, &mut made);
            out.extend(anchored(n, made));
        } else {
            n.children().iter().for_each(|c| walk(c, run, out));
        }
    }
}

/// Each empty link to a point (`<a class="pt" href="#p-…"></a>`, the rfc skill's) given its point's number as its words, as
/// shell.js's `number()` writes it in a browser: the point's place among its `<ol>`'s items from its start (counted down in a
/// reversed one), or among its table's `p-` rows from 1. A link whose point is gone keeps no words, and is read as none.
fn numbered(doc: &Document) {
    // each id's first node, read once: a target is looked up, never selected by a selector made of the page's words
    let mut ids = std::collections::HashMap::new();
    for n in doc.select("[id]").nodes() {
        ids.entry(n.attr("id").map(|i| i.to_string()).unwrap_or_default())
            .or_insert(*n);
    }
    for a in doc.select("a.pt").nodes() {
        let Some(id) = a
            .attr("href")
            .and_then(|h| h.strip_prefix('#').map(str::to_string))
        else {
            continue;
        };
        // words of its own stay; digits are a number an earlier read or save wrote, counted again (/code-review of PR 1592)
        let words = text_of(a).trim().to_string();
        if !words.is_empty() && !words.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Some((target, list)) = ids.get(&id).and_then(|t| Some((*t, t.parent()?))) else {
            continue;
        };
        let ol = name(&list) == "ol";
        let siblings: Vec<NodeRef> = (list.element_children().into_iter())
            .filter(|c| match ol {
                true => name(c) == "li",
                false => c.attr("id").is_some_and(|i| i.starts_with("p-")),
            })
            .collect();
        let Some(i) = siblings.iter().position(|c| c.id == target.id) else {
            continue;
        };
        let start = || {
            list.attr("start")
                .and_then(|s| s.trim().parse::<i64>().ok())
        };
        let n = match (ol, ol && list.attr("reversed").is_some()) {
            (_, true) => start().unwrap_or(siblings.len() as i64) - i as i64,
            (true, false) => start().unwrap_or(1) + i as i64,
            (false, _) => 1 + i as i64,
        };
        a.set_text(n.to_string());
    }
}

/// web.clj's `html->elements`: the main content (`article`, else `main`, else `body`, once navigation, scripts and
/// KaTeX's hidden copy are gone) walked in order, each block its elements, loose text in a container a paragraph.
pub fn read_html(html: &str) -> Parsed {
    let doc = Document::from(html);
    let meta = meta(&doc);
    numbered(&doc);
    // `.text-only`: a page of ours writes a link's target out for agents (dash-surface), never words of the page
    doc.select("script, style, nav, footer, aside, .katex-html, .text-only")
        .remove();
    // a site's header goes, an article's own (its h1, its byline) stays: web.clj dropped both
    for header in doc.select("header").nodes() {
        let held = |a: &NodeRef| matches!(name(a).as_str(), "article" | "main");
        if !header.ancestors(None).iter().any(held) {
            header.remove_from_parent();
        }
    }
    // one article is the page's content, an article inside it (a quoted card) its own; a page of several cards is read whole
    let articles: Vec<NodeRef> = (doc.select("article").nodes().iter())
        .filter(|a| !a.ancestors(None).iter().any(|x| name(x) == "article"))
        .cloned()
        .collect();
    let content = match articles.as_slice() {
        [one] => Some(*one),
        _ => (["main", "body"].iter()).find_map(|s| doc.select(s).nodes().first().cloned()),
    };
    let mut out = vec![];
    let mut run = String::new();
    if let Some(content) = content {
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
