//! The rule that keeps pages out of the dash crate until rust-ui-tree settles the rendering. Reads strings with graph's lexer.
use crate::graph::strings;
use crate::rust_files;
use std::path::Path;

/// HTML's living elements and the common SVG ones. `dir` is an obsolete one left out: `<dir>` is a usage line's placeholder. A
/// placeholder named as an element, `<path>`, fails loudly: write it PATH, no brackets. A missed tag would pass quietly.
const HTML_TAGS: [&str; 125] = [
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "circle",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "defs",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "ellipse",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "g",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "line",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "path",
    "picture",
    "polygon",
    "polyline",
    "pre",
    "progress",
    "q",
    "rect",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "template",
    "text",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "tspan",
    "u",
    "ul",
    "use",
    "var",
    "video",
    "wbr",
];

/// The HTML tags in this text, as written: `<td>`, `</TD`, `<a href`, `<!doctype`, `<!--`. `<dir>`, `<https:`, `x<!y` and a capital
/// single letter, the type parameter of `Foo<S>`, are none.
pub fn html_tags(text: &str) -> Vec<String> {
    let mut found = vec![];
    for (start, _) in text.match_indices('<') {
        let rest = &text[start + 1..];
        let name = rest.strip_prefix('/').unwrap_or(rest);
        let len = name.find(|c: char| !c.is_ascii_alphanumeric());
        let len = len.unwrap_or(name.len());
        let (word, after) = name.split_at(len);
        let lower = word.to_ascii_lowercase();
        let tag = (word.len() > 1 || word == lower)
            && HTML_TAGS.contains(&lower.as_str())
            && !after.starts_with(['-', '_', ':', '.']);
        let doctype = rest
            .get(..8)
            .is_some_and(|r| r.eq_ignore_ascii_case("!doctype"));
        if tag || doctype || rest.starts_with("!--") {
            let end = start + 1 + (rest.len() - after.len()).max(1);
            found.push(text[start..end].to_string());
        }
    }
    found
}

/// Each HTML tag in a string of the dash crate, or of the gate's own source, which writes that crate's /deps page in its build
/// script (@nha, 2026-11-06: « okay, remove please. I feel that we will go in the wrong direction. »). Rendering has one home,
/// crates/surface, until rust-ui-tree settles it. It reads string literals as written, so a tag put together, `format!("<{name}>")`
/// or `concat!`, an escaped `\x3c`, a `\` line break inside a tag, an obsolete element or a file taken by `include_str!` passes. A
/// file it cannot read is a finding, named.
pub fn html_in_strings(root: &Path) -> Vec<String> {
    let files = rust_files(&root.join("crates/dash"));
    let files = files
        .into_iter()
        .chain(rust_files(&root.join("crates/gate/src")));
    let mut found = vec![];
    for file in files {
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(e) => {
                found.push(format!("{}: {e}", file.display()));
                continue;
            }
        };
        let tags = strings(&source).into_iter().flat_map(|s| html_tags(&s));
        found.extend(tags.map(|tag| format!("{}: {tag}", file.display())));
    }
    found
}
