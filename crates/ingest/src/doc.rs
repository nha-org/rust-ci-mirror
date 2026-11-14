//! rfc-rust-unified section 20i: one extraction core, two doors (task rust-doc-core; @nha, 2026-11-11: « this will be the
//! entrypoint for the flare features into the rust dash »). A reader turns a source's bytes into Flare's elements; the core
//! makes them a `canvist_model` document, the one model of rust/CLAUDE.md rule 7. `dash doc extract` projects that document to
//! Flare's items, its text; `dash doc import` keeps it whole, images and diagrams in it. A reader registers itself with
//! `inventory::submit!`, as pages do: a new format is a reader, never a change here.
//!
//! Ported from graal's Clojure, which stays the reference: `lib/chunker.clj` (`chunk`), `lib/docling/markdown.clj`
//! (`markdown`, `render_with_refs`) and `lib/extraction/util.clj` (`make_title`, `chunk->item`, `prepend-preface`).
use base64::Engine;
use canvist_model::{Attrs, BlockSpec, Document, MarkSpec, Schema, Value as Attr};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// What a block is, as Flare's `:label` names it, with what that kind alone carries.
#[derive(Clone, Debug, PartialEq)]
pub enum Label {
    /// An h1: level 0 of the heading stack.
    Title,
    /// An h2 to h6: levels 1 to 5.
    SectionHeader(u8),
    Text,
    ListItem,
    Code,
    /// `text` holds its cells joined, « a | b » a line.
    Table(Table),
    /// One point of a section's writer notes (`details.wn`): kept apart from the body, which it explains.
    WriterNote,
    /// A fold (`<details>`): `text` its summary, the line shown closed, `inside` what opens under it.
    Fold {
        inside: Vec<Element>,
    },
    /// An image: `src` as the source names it, a `data:` URI once imported; `text` its alt.
    Picture {
        src: String,
    },
    /// A drawing kept as its own markup, an SVG; `text` the words it shows.
    Diagram {
        svg: String,
    },
    /// An RFC's stage, the rfc skill's `ol.stages`: its steps in order and the one it is at, each before it reached.
    Stage {
        steps: Vec<String>,
        current: String,
        /// its `aria-label`, the doc's own word for it (« Étape » in a French doc)
        label: String,
    },
}

/// A chip inside a block's words, the rfc skill's `span.chip`: its tone (« ok », « warn », « bad », or none) and its words'
/// byte range in that text.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub tone: String,
    pub span: (usize, usize),
}

/// A link inside a block's text. `span`: its anchor's byte range in that text, when the reader placed it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ref {
    pub anchor: String,
    pub href: String,
    pub span: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// Its cells, a row each.
    pub rows: Vec<Vec<String>>,
    /// Each row's ids in the source, as `Element::anchor` holds them (`<tr id="p-cap">`), empty for none: as many as `rows`.
    pub row_ids: Vec<String>,
    /// An RFC's decision (the rfc skill's `table.decision`), its options the rows.
    pub decision: Option<Decision>,
    /// Each cell's links, by row then cell, as a text block's `refs` (a glossary's « Jev », a task's id): empty when no cell
    /// holds one.
    pub links: Vec<Vec<Vec<Ref>>>,
    /// Each cell's chips, by row then cell, as `links`: empty when no cell holds one.
    pub chips: Vec<Vec<Vec<Chip>>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decision {
    /// Its chip: « decided », « yours ».
    pub status: String,
    /// Its chip's tone: « ok », « warn », « bad », or none.
    pub tone: String,
    /// The question and the call.
    pub caption: String,
}

impl Table {
    /// Rows with no ids and no decision, as a PDF's table reads.
    pub fn plain(rows: Vec<Vec<String>>) -> Self {
        let row_ids = vec![String::new(); rows.len()];
        Table {
            rows,
            row_ids,
            decision: None,
            links: vec![],
            chips: vec![],
        }
    }

    /// Its words, « a | b » a row, a row a line.
    pub fn text(&self) -> String {
        let rows: Vec<String> = self.rows.iter().map(|r| r.join(" | ")).collect();
        rows.join("\n")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub label: Label,
    pub text: String,
    pub refs: Vec<Ref>,
    /// The chips in its words, in order.
    pub chips: Vec<Chip>,
    /// Its page, for a format that has pages.
    pub page: Option<u32>,
    /// The source's ids a link's `#target` lands on here, space-separated as HTML's `class`: an HTML element's own, the
    /// ids inside its words, and a section's, figure's or list's for the first block it holds.
    pub anchor: Option<String>,
}

impl Element {
    pub fn new(label: Label, text: &str) -> Self {
        Element {
            label,
            text: text.to_string(),
            refs: vec![],
            chips: vec![],
            page: None,
            anchor: None,
        }
    }

    /// Its refs, without a target that runs code or carries a payload (util.clj's `unsafe-href`).
    pub fn with_refs(mut self, refs: Vec<Ref>) -> Self {
        self.refs = refs.into_iter().filter(|r| !unsafe_href(&r.href)).collect();
        self
    }
}

fn unsafe_href(href: &str) -> bool {
    let h = href.trim_start().to_ascii_lowercase();
    ["javascript:", "vbscript:", "data:"]
        .iter()
        .any(|s| h.starts_with(s))
}

/// What the source says about itself: an HTML page's og or `<title>`, a PDF's info dictionary as `preface`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    /// One line put before the first item's text (util.clj's `prepend-preface`).
    pub preface: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub meta: Meta,
    pub elements: Vec<Element>,
}

/// One format's reader: chosen by the file's extension, else by `sniff` on its first bytes.
pub struct Reader {
    pub format: &'static str,
    pub extensions: &'static [&'static str],
    pub sniff: fn(&[u8]) -> bool,
    pub read: fn(&[u8]) -> Result<Parsed, String>,
}

inventory::collect!(Reader);

/// The reader for this file, by its extension first.
pub fn reader_for(path: &Path, bytes: &[u8]) -> Result<&'static Reader, String> {
    let ext = (path.extension())
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    // in format order, never link order: which sniff is tried first is then the same on every build
    let readers = || {
        let mut all: Vec<&'static Reader> = inventory::iter::<Reader>.into_iter().collect();
        all.sort_by_key(|r| r.format);
        all.into_iter()
    };
    (readers().find(|r| r.extensions.contains(&ext.as_str())))
        .or_else(|| readers().find(|r| (r.sniff)(bytes)))
        .ok_or_else(|| {
            let known: Vec<&str> = readers().map(|r| r.format).collect();
            format!(
                "{}: no reader for it (readers: {})",
                path.display(),
                known.join(", ")
            )
        })
}

/// A source read by its reader: its format's name and what it holds.
pub fn parse(path: &Path) -> Result<(&'static str, Parsed), String> {
    let bytes = crate::read_file(&path.to_string_lossy())?;
    let reader = reader_for(path, &bytes)?;
    // A reader built on a library that panics on a malformed file (pdf-extract has about a hundred unwraps) fails this one
    // file with an error, as its contract says, never the whole command (/code-review on PR 1435).
    let read = std::panic::catch_unwind(|| (reader.read)(&bytes))
        .unwrap_or_else(|_| Err(format!("the {} reader failed on it", reader.format)));
    let parsed = read.map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((reader.format, parsed))
}

// ---------------------------------------------------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------------------------------------------------

/// Every block a reader's elements become. A text block keeps its `page` (0: none) and the links its reader could not
/// place in its words, `refs`, each an [anchor, href] pair; a placed link is a `link` mark, as ProseMirror's.
fn schema() -> Arc<Schema> {
    static SCHEMA: OnceLock<Arc<Schema>> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            let text = |name: &str| {
                BlockSpec::text(name).attrs(
                    Attrs::new()
                        .with("page", 0i64)
                        .with("anchor", "")
                        .with("refs", Vec::<Attr>::new()),
                )
            };
            let leaf = |name: &str, keys: &[&str]| {
                let attrs = keys
                    .iter()
                    .fold(Attrs::new().with("page", 0i64), |a, k| a.with(k, ""));
                BlockSpec::leaf(name).attrs(attrs)
            };
            let schema = Schema::builder()
                .block(leaf("meta", &["title", "description", "image", "preface"]))
                .block(text("title"))
                .block(
                    BlockSpec::text("heading").attrs(
                        Attrs::new()
                            .with("level", 1i64)
                            .with("page", 0i64)
                            .with("anchor", "")
                            .with("refs", Vec::<Attr>::new()),
                    ),
                )
                .block(text("paragraph"))
                .block(text("list_item"))
                .block(text("code"))
                .block(text("writer_note"))
                .block(BlockSpec::container("fold", &["block"]).attrs(at_fold()))
                // `kind` « decision » for an RFC's decision, its `status` chip and `caption` question
                .block(
                    BlockSpec::container("table", &["row"]).attrs(
                        ["anchor", "kind", "status", "tone", "caption"]
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
                .block(leaf("stage", &["steps", "current", "label", "anchor"]))
                .mark(MarkSpec::new("link").attrs(Attrs::new().with("href", "")))
                .mark(MarkSpec::new("chip").attrs(Attrs::new().with("tone", "")))
                .build()
                .expect("the document's blocks and its link mark are a valid schema");
            Arc::new(schema)
        })
        .clone()
}

/// A fold's attrs: its summary, its page and its ids.
fn at_fold() -> Attrs {
    Attrs::new()
        .with("page", 0i64)
        .with("anchor", "")
        .with("summary", "")
}

/// A block's words as ProseMirror text nodes, each placed link a `link` mark, each chip a `chip` mark with its tone.
fn inline(text: &str, refs: &[Ref], chips: &[Chip]) -> (Vec<Value>, Vec<Value>) {
    let fits = |s: usize, e: usize| {
        s < e && e <= text.len() && text.is_char_boundary(s) && text.is_char_boundary(e)
    };
    let mut placed: Vec<(usize, usize, &str)> = (refs.iter())
        .filter_map(|r| r.span.map(|(s, e)| (s, e, r.href.as_str())))
        .filter(|&(s, e, _)| fits(s, e))
        .collect();
    placed.sort();
    placed.dedup_by(|b, a| b.0 < a.1); // overlapping spans: the first one wins
    let mut toned: Vec<(usize, usize, &str)> = (chips.iter())
        .map(|c| (c.span.0, c.span.1, c.tone.as_str()))
        .filter(|&(s, e, _)| fits(s, e))
        .collect();
    toned.sort();
    toned.dedup_by(|b, a| b.0 < a.1);
    let unplaced = (refs.iter())
        .filter(|r| {
            !placed
                .iter()
                .any(|&(s, e, h)| r.span == Some((s, e)) && r.href == h)
        })
        .map(|r| json!([r.anchor, r.href]))
        .collect();
    // cut at each link's and chip's edges; a piece carries the marks whose span holds it
    let mut cuts: Vec<usize> = [0, text.len()].into_iter().collect();
    for &(s, e, _) in placed.iter().chain(&toned) {
        cuts.extend([s, e]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let within = |all: &[(usize, usize, &'_ str)], a: usize, b: usize| {
        (all.iter())
            .find(|&&(s, e, _)| s <= a && b <= e)
            .map(|t| t.2.to_string())
    };
    let nodes = (cuts.windows(2))
        .filter(|w| w[0] < w[1])
        .map(|w| {
            let mut node = json!({ "type": "text", "text": &text[w[0]..w[1]] });
            let mut marks = vec![];
            if let Some(href) = within(&placed, w[0], w[1]) {
                marks.push(json!({ "type": "link", "attrs": { "href": href } }));
            }
            if let Some(tone) = within(&toned, w[0], w[1]) {
                marks.push(json!({ "type": "chip", "attrs": { "tone": tone } }));
            }
            if !marks.is_empty() {
                node["marks"] = Value::Array(marks);
            }
            node
        })
        .collect();
    (nodes, unplaced)
}

fn block(e: &Element) -> Value {
    let page = i64::from(e.page.unwrap_or(0));
    let anchor = e.anchor.clone().unwrap_or_default();
    let text_block = |kind: &str, mut attrs: Value| {
        let (content, unplaced) = inline(&e.text, &e.refs, &e.chips);
        attrs["page"] = json!(page);
        attrs["anchor"] = json!(anchor);
        attrs["refs"] = Value::Array(unplaced);
        let mut b = json!({ "type": kind, "attrs": attrs });
        if !content.is_empty() {
            b["content"] = Value::Array(content);
        }
        b
    };
    match &e.label {
        Label::Title => text_block("title", json!({})),
        Label::SectionHeader(level) => text_block("heading", json!({ "level": level })),
        Label::Text => text_block("paragraph", json!({})),
        Label::ListItem => text_block("list_item", json!({})),
        Label::Code => text_block("code", json!({})),
        Label::WriterNote => text_block("writer_note", json!({})),
        Label::Table(t) => {
            let mut attrs = json!({ "page": page, "anchor": anchor });
            if let Some(d) = &t.decision {
                attrs["kind"] = json!("decision");
                attrs["status"] = json!(d.status);
                attrs["tone"] = json!(d.tone);
                attrs["caption"] = json!(d.caption);
            }
            table(t, attrs)
        }
        Label::Picture { src } => {
            json!({ "type": "image", "attrs": { "src": src, "alt": e.text, "page": page, "anchor": anchor } })
        }
        Label::Diagram { svg } => {
            json!({ "type": "diagram", "attrs": { "svg": svg, "text": e.text, "page": page, "anchor": anchor } })
        }
        Label::Stage {
            steps,
            current,
            label,
        } => {
            json!({ "type": "stage", "attrs": { "steps": steps.join("\n"), "current": current, "label": label, "page": page, "anchor": anchor } })
        }
        Label::Fold { inside } => {
            let content: Vec<Value> = inside.iter().map(block).collect();
            json!({ "type": "fold", "attrs": { "summary": e.text, "page": page, "anchor": anchor }, "content": content })
        }
    }
}

fn table(t: &Table, attrs: Value) -> Value {
    // a cell's placed links are marks on its words; one its reader could not place is dropped, a cell keeping no refs
    let cell = |c: &String, refs: &[Ref], chips: &[Chip]| match inline(c, refs, chips).0 {
        words if words.is_empty() => json!({ "type": "cell" }),
        words => json!({ "type": "cell", "content": words }),
    };
    let rows: Vec<Value> = (t.rows.iter().enumerate())
        .map(|(i, r)| {
            let anchor = t.row_ids.get(i).map_or("", String::as_str);
            let refs =
                |j: usize| (t.links.get(i).and_then(|l| l.get(j))).map_or(&[][..], Vec::as_slice);
            let chips =
                |j: usize| (t.chips.get(i).and_then(|l| l.get(j))).map_or(&[][..], Vec::as_slice);
            let cells: Vec<Value> = r
                .iter()
                .enumerate()
                .map(|(j, c)| cell(c, refs(j), chips(j)))
                .collect();
            json!({ "type": "row", "attrs": { "anchor": anchor }, "content": cells })
        })
        .collect();
    json!({ "type": "table", "attrs": attrs, "content": rows })
}

/// Cells' links (or chips) as a table keeps them: none at all is no list, so a table read twice compares equal.
pub fn linked<T>(links: Vec<Vec<Vec<T>>>) -> Vec<Vec<Vec<T>>> {
    match links.iter().flatten().all(Vec::is_empty) {
        true => vec![],
        false => links,
    }
}

/// What `dash doc import` reads of a file, raised whenever it reads more, so every document a store kept under an older
/// reader reads in again (2: an RFC's stage, chips and point numbers, task rust-doc-view-parity; a reader bump left them off every /v/ page
/// until its file changed).
pub const READER: u32 = 2;

/// A file's bytes as a fingerprint under this READER: equal bytes, equal print, until it rises. Not a secret, only a change seen.
pub fn print_of(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}-r{READER}", h.finish())
}

/// Whether `kept`, a store's print of a file, is of the bytes `now` prints but under an older READER: read it in again.
pub fn older_reader(kept: &str, now: &str) -> bool {
    kept != now && kept.split("-r").next() == now.split("-r").next()
}

/// `file` imported and passed by `check`, or, when this READER refuses the bytes `now` prints, `kept`'s doc if an older
/// READER printed them (/code-review of PR 1592).
pub fn read_in(
    file: &Path,
    check: fn(&Value) -> Result<Value, String>,
    kept: Option<(Value, String)>,
    now: &str,
) -> Result<Value, String> {
    older_doc(import(file).and_then(|d| check(&d).map(|_| d)), kept, now)
}

/// `read`, or, when it is a refusal of the bytes `now` prints, `kept`'s doc if an older READER printed them (`kept`: a doc
/// and its print).
pub fn older_doc(
    read: Result<Value, String>,
    kept: Option<(Value, String)>,
    now: &str,
) -> Result<Value, String> {
    match (read, kept) {
        (Err(_), Some((doc, file))) if older_reader(&file, now) => Ok(doc),
        (read, _) => read,
    }
}

/// A stage block, its words « Stage: Accepted (Draft, Review, Accepted, Shipped) » as Flare reads it.
pub fn stage(steps: Vec<String>, current: String, label: String) -> Element {
    // a doc's own word for it, « Stage » when it names none (/code-review of PR 1592: a French doc's « Étape » was rewritten)
    let label = if label.is_empty() {
        "Stage".to_string()
    } else {
        label
    };
    let text = format!("{label}: {current} ({})", steps.join(", "));
    Element::new(
        Label::Stage {
            steps,
            current,
            label,
        },
        &text,
    )
}

/// The document a reader's output makes, checked against the schema.
pub fn to_document(parsed: &Parsed) -> Result<Document, String> {
    let m = &parsed.meta;
    let mut content = vec![];
    if *m != Meta::default() {
        let s = |v: &Option<String>| v.clone().unwrap_or_default();
        content.push(json!({ "type": "meta", "attrs": { "title": s(&m.title),
            "description": s(&m.description), "image": s(&m.image), "preface": s(&m.preface) } }));
    }
    content.extend(parsed.elements.iter().map(block));
    let saved = json!({ "type": "doc", "content": content });
    Document::from_json(schema(), 0, &saved).map_err(|e| format!("not a document: {e}"))
}

/// A text block's words, its links and its chips, as `words` reads them back.
type Words = (String, Vec<Ref>, Vec<Chip>);

fn words(block: &Value) -> Words {
    let mut text = String::new();
    let mut refs: Vec<Ref> = vec![];
    let mut chips: Vec<Chip> = vec![];
    for node in block["content"].as_array().into_iter().flatten() {
        let t = node["text"].as_str().unwrap_or_default();
        let start = text.len();
        text.push_str(t);
        let mark = |kind: &str, key: &str| {
            (node["marks"].as_array().into_iter().flatten())
                .find(|m| m["type"] == kind)
                .map(|m| m["attrs"][key].as_str().unwrap_or_default().to_string())
        };
        // a link cut in pieces by a chip's edge is one link again, as a chip cut by a link's is below
        match (mark("link", "href"), refs.last_mut()) {
            (Some(href), Some(last))
                if last.span.is_some_and(|s| s.1 == start) && last.href == href =>
            {
                last.anchor.push_str(t);
                last.span = last.span.map(|s| (s.0, text.len()));
            }
            (Some(href), _) => refs.push(Ref {
                anchor: t.to_string(),
                href,
                span: Some((start, text.len())),
            }),
            (None, _) => {}
        }
        match (mark("chip", "tone"), chips.last_mut()) {
            (Some(tone), Some(last)) if last.span.1 == start && last.tone == tone => {
                last.span.1 = text.len()
            }
            (Some(tone), _) => chips.push(Chip {
                tone,
                span: (start, text.len()),
            }),
            (None, _) => {}
        }
    }
    for pair in block["attrs"]["refs"].as_array().into_iter().flatten() {
        let s = |i: usize| pair[i].as_str().unwrap_or_default().to_string();
        refs.push(Ref {
            anchor: s(0),
            href: s(1),
            span: None,
        });
    }
    (text, refs, chips)
}

/// A document read back into elements: what `extract` projects, the same whichever reader made it.
pub fn from_document(doc: &Document) -> Parsed {
    let mut parsed = Parsed::default();
    read_blocks(&doc.to_json()["content"], &mut parsed);
    parsed
}

/// Saved blocks read into `parsed`'s elements, in order, a fold's into its own.
fn read_blocks(blocks: &Value, parsed: &mut Parsed) {
    for b in blocks.as_array().into_iter().flatten() {
        let attr = |k: &str| b["attrs"][k].as_str().unwrap_or_default().to_string();
        let some = |k: &str| Some(attr(k)).filter(|v| !v.is_empty());
        let page = b["attrs"]["page"]
            .as_u64()
            .filter(|&p| p > 0)
            .map(|p| p as u32);
        let anchor = some("anchor");
        let (text, refs, chips) = words(b);
        let label = match b["type"].as_str().unwrap_or_default() {
            "meta" => {
                parsed.meta = Meta {
                    title: some("title"),
                    description: some("description"),
                    image: some("image"),
                    preface: some("preface"),
                };
                continue;
            }
            "title" => Label::Title,
            "heading" => Label::SectionHeader(b["attrs"]["level"].as_u64().unwrap_or(1) as u8),
            "list_item" => Label::ListItem,
            "code" => Label::Code,
            "writer_note" => Label::WriterNote,
            "fold" => {
                let mut inside = Parsed::default();
                read_blocks(&b["content"], &mut inside);
                let e = Element::new(
                    Label::Fold {
                        inside: inside.elements,
                    },
                    &attr("summary"),
                );
                parsed.elements.push(Element { page, anchor, ..e });
                continue;
            }
            "table" => {
                let saved_rows = b["content"].as_array().cloned().unwrap_or_default();
                let cells: Vec<Vec<Words>> = (saved_rows.iter())
                    .map(|r| {
                        (r["content"].as_array().into_iter().flatten())
                            .map(words)
                            .collect()
                    })
                    .collect();
                let rows = (cells.iter())
                    .map(|r| r.iter().map(|c| c.0.clone()).collect())
                    .collect();
                let links = linked(
                    (cells.iter())
                        .map(|r| r.iter().map(|c| c.1.clone()).collect())
                        .collect(),
                );
                let chips = linked(
                    cells
                        .into_iter()
                        .map(|r| r.into_iter().map(|c| c.2).collect())
                        .collect(),
                );
                let row_ids = (saved_rows.iter())
                    .map(|r| {
                        r["attrs"]["anchor"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string()
                    })
                    .collect();
                let decision = (attr("kind") == "decision").then(|| Decision {
                    status: attr("status"),
                    tone: attr("tone"),
                    caption: attr("caption"),
                });
                let t = Table {
                    rows,
                    row_ids,
                    decision,
                    links,
                    chips,
                };
                let text = t.text();
                parsed.elements.push(Element {
                    page,
                    anchor,
                    ..Element::new(Label::Table(t), &text)
                });
                continue;
            }
            "image" => {
                let e = Element::new(Label::Picture { src: attr("src") }, &attr("alt"));
                parsed.elements.push(Element { page, anchor, ..e });
                continue;
            }
            "diagram" => {
                let e = Element::new(Label::Diagram { svg: attr("svg") }, &attr("text"));
                parsed.elements.push(Element { page, anchor, ..e });
                continue;
            }
            "stage" => {
                let steps: Vec<String> = attr("steps").lines().map(str::to_string).collect();
                let e = stage(steps, attr("current"), attr("label"));
                parsed.elements.push(Element { page, anchor, ..e });
                continue;
            }
            _ => Label::Text,
        };
        parsed.elements.push(Element {
            label,
            text,
            refs,
            chips,
            page,
            anchor,
        });
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Door 1: extract, the document as Flare's items
// ---------------------------------------------------------------------------------------------------------------------

/// One indexable item, as Flare's extract prints it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub title: String,
    pub fts: String,
    pub page: Option<u32>,
}

/// chunker.clj's `default-max-chars`: about 512 tokens.
pub const MAX_CHARS: usize = 2048;
const ELLIPSIS: &str = " [...]";

/// util.clj's `make-preview` at 70: whitespace collapsed, cut at a space near the end, else hard.
pub fn make_title(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let target = 70;
    if flat.chars().count() <= target {
        return flat;
    }
    let snippet: String = flat.chars().take(target).collect();
    let cut = match snippet.rfind(' ') {
        Some(i) if i > 0 && snippet[..i].chars().count() >= target - ELLIPSIS.len() => {
            &snippet[..i]
        }
        _ => &snippet,
    };
    format!("{}{ELLIPSIS}", cut.trim())
}

/// markdown.clj's `find-anchor`: the first `anchor` at or after `from` that cuts no word.
fn find_anchor(s: &str, anchor: &str, from: usize) -> Option<usize> {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let (first, last) = (anchor.chars().next(), anchor.chars().last());
    let mut from = from;
    while let Some(i) = s
        .get(from..)
        .and_then(|rest| rest.find(anchor))
        .map(|i| i + from)
    {
        let before = s[..i].chars().last();
        let after = s[i + anchor.len()..].chars().next();
        if !(word(first) && word(before)) && !(word(last) && word(after)) {
            return Some(i);
        }
        from = i + s[i..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// markdown.clj's `render-with-refs`: each link as `[anchor](href)`, at its span, else at its anchor's first place, else
/// listed after the text as `(see: href …)`.
pub fn render_with_refs(text: &str, refs: &[Ref]) -> String {
    let valid = |r: &&Ref| {
        r.span.is_some_and(|(s, e)| {
            s <= e && e <= text.len() && text.is_char_boundary(s) && text.is_char_boundary(e)
        })
    };
    // Every link is placed on the words first, then all are written in one pass: an anchor searched for never lands inside
    // a link already written, its URL included (markdown.clj searched the text with the links in it).
    let mut spans: Vec<(usize, usize, &str)> = (refs.iter().filter(valid))
        .map(|r| {
            r.span
                .map(|(s, e)| (s, e, r.href.as_str()))
                .expect("filtered on its span")
        })
        .collect();
    let mut from = 0;
    let mut placed: Vec<&str> = vec![];
    let without: Vec<&Ref> = refs.iter().filter(|r| !valid(r)).collect();
    for r in &without {
        if placed.contains(&r.href.as_str()) || r.anchor.trim().is_empty() {
            continue;
        }
        let mut at = from;
        while let Some(i) = find_anchor(text, &r.anchor, at) {
            let end = i + r.anchor.len();
            if spans.iter().all(|&(s, e, _)| end <= s || e <= i) {
                spans.push((i, end, &r.href));
                from = end;
                placed.push(&r.href);
                break;
            }
            at = i + text[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans.sort();
    spans.dedup_by(|b, a| b.0 < a.1);
    let mut out = text.to_string();
    for &(s, e, href) in spans.iter().rev() {
        out = format!("{}[{}]({href}){}", &out[..s], &out[s..e], &out[e..]);
    }
    let mut missing: Vec<&str> = vec![];
    for r in &without {
        if !placed.contains(&r.href.as_str()) && !missing.contains(&r.href.as_str()) {
            missing.push(&r.href);
        }
    }
    if !missing.is_empty() {
        out = format!("{out} (see: {})", missing.join(" "));
    }
    out
}

fn marked(prefix: &str, suffix: &str, text: &str) -> String {
    match text.trim().is_empty() {
        true => String::new(),
        false => format!("{prefix}{text}{suffix}"),
    }
}

/// markdown.clj's `gfm-table`: the first row the header, every row as wide as the widest.
fn gfm_table(rows: &[Vec<String>]) -> String {
    let esc = |s: &str| {
        s.replace('|', "\\|")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let ncol = rows.iter().map(Vec::len).max().unwrap_or(0);
    let row = |r: &Vec<String>| {
        let cells: Vec<String> = (0..ncol)
            .map(|i| esc(r.get(i).map_or("", String::as_str)))
            .collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut lines = vec![row(&rows[0]), format!("|{}", " --- |".repeat(ncol))];
    lines.extend(rows[1..].iter().map(row));
    lines.join("\n")
}

/// markdown.clj's `element->markdown`.
pub fn markdown(e: &Element) -> String {
    let linked = || render_with_refs(&e.text, &e.refs);
    match &e.label {
        Label::Text => linked(),
        Label::Title => marked("# ", "", &linked()),
        Label::SectionHeader(_) => marked("## ", "", &linked()),
        Label::ListItem => marked("- ", "", &linked()),
        // a note's listing keeps its lines inside its point
        Label::WriterNote => marked("- ", "", &linked().replace('\n', "\n  ")),
        Label::Code => marked("```\n", "\n```", &e.text),
        Label::Table(t) => {
            let body = match (t.rows.is_empty(), e.text.trim().is_empty()) {
                (false, _) => gfm_table(&t.rows),
                (true, true) => String::new(),
                (true, false) => format!("```\n{}\n```", e.text),
            };
            match &t.decision {
                // a decision's question leads its options
                Some(d) => format!("{} {}\n\n{body}", d.status, d.caption)
                    .trim()
                    .to_string(),
                None => body,
            }
        }
        Label::Picture { .. } | Label::Diagram { .. } | Label::Stage { .. } => e.text.clone(),
        Label::Fold { inside } => {
            let all: Vec<String> = std::iter::once(e.text.clone())
                .chain(inside.iter().map(markdown))
                .collect();
            all.into_iter()
                .filter(|m| !m.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
        }
    }
}

/// A heading's level on the stack: a title 0, a section header its own.
fn level(e: &Element) -> Option<u8> {
    match e.label {
        Label::Title => Some(0),
        Label::SectionHeader(l) => Some(l),
        _ => None,
    }
}

struct Chunk {
    text: String,
    headings: Vec<String>,
    page: Option<u32>,
}

/// chunker.clj's `chunk-elements`: a body is cut at each heading, at `MAX_CHARS`, and, for a format with pages, at each
/// new page, so an item never holds two pages.
fn chunk(elements: &[Element]) -> Vec<Chunk> {
    let mut stack: Vec<(u8, String)> = vec![];
    let mut out = vec![];
    let mut body: Vec<&Element> = vec![];
    let flush = |body: &mut Vec<&Element>, stack: &[(u8, String)], out: &mut Vec<Chunk>| {
        let mut batch: Vec<String> = vec![];
        let mut len = 0;
        let mut page = None;
        let mut emit = |batch: &mut Vec<String>, page: Option<u32>| {
            let text = batch.join("\n\n");
            if !text.trim().is_empty() {
                let headings = stack.iter().map(|(_, h)| h.clone()).collect();
                out.push(Chunk {
                    text,
                    headings,
                    page,
                });
            }
            batch.clear();
        };
        for e in body.drain(..) {
            let s = markdown(e);
            if s.trim().is_empty() {
                continue;
            }
            let sep = if batch.is_empty() { 0 } else { 2 };
            if !batch.is_empty() && len + sep + s.chars().count() > MAX_CHARS {
                emit(&mut batch, page);
                len = 0;
            }
            let sep = if batch.is_empty() { 0 } else { 2 };
            len += sep + s.chars().count();
            page = page.or(e.page);
            batch.push(s);
        }
        emit(&mut batch, page);
    };
    for e in elements {
        if let Some(l) = level(e) {
            flush(&mut body, &stack, &mut out);
            stack.retain(|(k, _)| *k < l);
            stack.push((l, e.text.clone()));
        } else {
            if body.last().is_some_and(|last| last.page != e.page) {
                flush(&mut body, &stack, &mut out);
            }
            body.push(e);
        }
    }
    flush(&mut body, &stack, &mut out);
    out
}

/// The document's items, as Flare indexes them (web.clj's `extract-html-items`, made the rule for every format): a chunk
/// under its heading path; with none, one item from its description, else its headings, else its title; the preface first.
pub fn items(parsed: &Parsed) -> Vec<Item> {
    let Meta {
        title,
        description,
        preface,
        ..
    } = &parsed.meta;
    let chunks = chunk(&parsed.elements);
    let mut items: Vec<Item> = if chunks.is_empty() {
        let headings: Vec<&str> = (parsed.elements.iter())
            .filter(|e| level(e).is_some() && !e.text.trim().is_empty())
            .map(|e| e.text.as_str())
            .collect();
        let headings = headings.join("\n");
        let text = [description.as_deref(), Some(&headings), title.as_deref()]
            .into_iter()
            .flatten()
            .find(|t| !t.trim().is_empty());
        text.map(|t| Item {
            title: title.clone().unwrap_or_else(|| make_title(t)),
            fts: t.to_string(),
            page: None,
        })
        .into_iter()
        .collect()
    } else {
        (chunks.into_iter())
            .map(|c| {
                // util.clj's `chunk->item`: the heading path goes into the text, which nothing else carries
                let mut fts = match c.headings.is_empty() {
                    true => c.text.clone(),
                    false => format!("{}\n\n{}", c.headings.join("\n"), c.text),
                };
                let mut item_title = c
                    .headings
                    .last()
                    .cloned()
                    .unwrap_or_else(|| make_title(&c.text));
                if c.headings.is_empty() {
                    if let Some(t) = title {
                        item_title = t.clone();
                    }
                    if let Some(d) = description {
                        fts = format!("{d}\n\n{fts}");
                    }
                }
                Item {
                    title: item_title,
                    fts,
                    page: c.page,
                }
            })
            .collect()
    };
    if let Some(p) = preface.as_deref().filter(|p| !p.is_empty()) {
        match items.first_mut() {
            Some(first) => first.fts = format!("{p}\n\n{}", first.fts),
            None => items.push(Item {
                title: make_title(p),
                fts: p.to_string(),
                page: None,
            }),
        }
    }
    items
}

/// `dash doc extract`: the source read into the document, then projected to its items.
pub fn extract(path: &Path) -> Result<(&'static str, Vec<Item>), String> {
    let (format, parsed) = parse(path)?;
    let doc = to_document(&parsed)?;
    Ok((format, items(&from_document(&doc))))
}

// ---------------------------------------------------------------------------------------------------------------------
// Door 2: import, the document whole
// ---------------------------------------------------------------------------------------------------------------------

/// An image file's bytes as a `data:` URI, when `src` names a file beside the source: the document then holds the image,
/// not a path that breaks once it moves. A web address, a server path or a file not there is kept as written.
/// Only a file inside the source's own folder: a page's `../../Pictures/passport.jpg` is never read into a document that
/// may be shared (/code-review on PR 1435). A `?v=2` or `#part` after the name is not part of it.
fn embedded(src: &str, beside: &Path) -> Option<String> {
    let src = src.split(['?', '#']).next()?;
    let kind = match src.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    if src.contains(':') || src.starts_with('/') {
        return None;
    }
    let folder = beside.canonicalize().ok()?;
    let file = folder
        .join(src)
        .canonicalize()
        .ok()
        .filter(|f| f.starts_with(&folder))?;
    let bytes = crate::read_file(&file.to_string_lossy()).ok()?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{kind};base64,{data}"))
}

/// `dash doc import`: the source as a document of the dash, saved as canvist saves it, every image and diagram in it.
pub fn import(path: &Path) -> Result<Value, String> {
    let (_, mut parsed) = parse(path)?;
    let beside = path.parent().unwrap_or(Path::new("."));
    for e in &mut parsed.elements {
        if let Label::Picture { src } = &mut e.label
            && let Some(data) = embedded(src, beside)
        {
            *src = data;
        }
    }
    Ok(to_document(&parsed)?.to_json())
}
