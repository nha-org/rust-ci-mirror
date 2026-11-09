//! The rule that keeps pages out of the dash crate until rust-ui-tree settles the rendering. Reads strings with graph's lexer.
use crate::graph::{strings, tokens};
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

/// The components' file: the one place a page's button, field, card, badge, avatar or tabs is drawn (task
/// rust-design-components; @nha, 2026-11-09: « We want all components to be dual-use »).
pub const COMPONENTS: &str = "crates/surface/src/components.rs";
/// /design's own mock screens, which draw each state by hand to show it: the showcase, not a page.
pub const SHOWCASE: [&str; 4] = [
    "crates/surface/src/design/families.rs",
    "crates/surface/src/design/linear.rs",
    "crates/surface/src/design/mod.rs",
    "crates/surface/src/design/notion.rs",
];

/// Where a file's top-level test module starts.
const TESTS: &str = "\n#[cfg(test)]\nmod ";

/// The strings of the const `name` in `source`: its `[..]` up to the first `];`.
fn listed(source: &str, name: &str) -> Vec<String> {
    let Some(at) = source.find(&format!("const {name}:")) else {
        return vec![];
    };
    let list = &source[at..];
    strings(&list[..list.find("];").unwrap_or(list.len())])
}

/// A `class:` attribute's value: up to its `,` or the `}` that closes its element, braces and strings counted.
fn class_expr(rest: &str) -> &str {
    let (mut depth, mut quoted) = (0usize, false);
    for (i, ch) in rest.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            _ if quoted => {}
            '{' => depth += 1,
            '}' if depth == 0 => return &rest[..i],
            '}' => depth -= 1,
            ',' if depth == 0 => return &rest[..i],
            _ => {}
        }
    }
    rest
}

/// Each place a page draws what a component covers: an rsx element of COMPONENTS' OWNED_TAGS (`button {` after a `{`, `}`
/// or `,`), or a `class:` whose expression (up to its `,` or closing `}`) holds one of its OWNED_CLASSES in any literal, an
/// `if`'s branches included (/code-review: `class: if a { "li-card x" } else { "li-card" }` passed). Every crate's .rs is
/// read but COMPONENTS and SHOWCASE.
/// HTML written in a string may hold no button, input or textarea tag. Code after a top-level `#[cfg(test)]` module, a
/// tests/ folder and `//` lines are not read. ponytail: it misses a class built at run time (`format!`, a variable) and the
/// rust/ui-spike workspace, a spike outside crates/; an rsx parse when one slips past.
pub fn page_local_markup(root: &Path) -> Vec<String> {
    let owned = std::fs::read_to_string(root.join(COMPONENTS)).unwrap_or_default();
    let (tags, classes) = (
        listed(&owned, "OWNED_TAGS"),
        listed(&owned, "OWNED_CLASSES"),
    );
    if tags.is_empty() || classes.is_empty() {
        return vec![format!(
            "{COMPONENTS}: no OWNED_TAGS or OWNED_CLASSES to read"
        )];
    }
    let mut found = vec![];
    for file in rust_files(&root.join("crates")) {
        let name = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        // a tests/ folder is test code whole: its HTML is no page
        if name == COMPONENTS || SHOWCASE.contains(&name.as_str()) || name.contains("/tests/") {
            continue;
        }
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(e) => {
                found.push(format!("{name}: {e}"));
                continue;
            }
        };
        // the code before its top-level test module: a test's own markup is no page (/code-review)
        let body = &source[..source.find(TESTS).unwrap_or(source.len())];
        // HTML in a string: a button or a field written by hand, never the components'
        let written = strings(body)
            .iter()
            .flat_map(|s| html_tags(s))
            .filter(|t| tags.iter().any(|o| t[1..].eq_ignore_ascii_case(o)))
            .count();
        if written > 0 {
            found.push(format!(
                "{name}: {written} button, input or textarea in strings"
            ));
        }
        let code = tokens(body);
        for w in code.windows(3) {
            if ["{", "}", ","].contains(&w[0].as_str()) && tags.contains(&w[1]) && w[2] == "{" {
                found.push(format!("{name}: {} {{", w[1]));
            }
        }
        let lines = body.lines().filter(|l| !l.trim_start().starts_with("//"));
        for (line, at) in lines.flat_map(|l| l.match_indices("class:").map(move |(at, _)| (l, at)))
        {
            let expr = class_expr(&line[at + 6..]);
            if let Some(c) = (expr.split('"').skip(1).step_by(2))
                .flat_map(str::split_whitespace)
                .find(|c| classes.iter().any(|o| o == c))
            {
                found.push(format!("{name}: class: \"{c}\""));
            }
        }
    }
    found
}

/// Words no page says: a section still read is a skeleton that fills itself (surface's components::Skeleton), so a page never
/// sends him to a reload (task rust-skeleton-live; @nha, 2026-11-09: « Reading every access: the next reload shows it. does
/// not make sense. Also reloading doesn't do anything. »).
/// The ways seen of saying it, so the same promise in other words fails too (/code-review): a word alone (`reload`) would
/// refuse the refill script's own `location.reload()`.
pub const NEVER_ON_A_PAGE: [&str; 6] = [
    "next reload",
    "after a reload",
    "reload the page",
    "reload to",
    "next refresh",
    "refresh the page",
];

/// Each string of a crate's code holding a phrase of NEVER_ON_A_PAGE, in any case and across a line break: a page's words
/// are its code's strings. Every crate's .rs is read but this gate's own; test code is left out as `page_local_markup`
/// leaves it. ponytail: a phrase built from parts at run time (`format!("next {}", ..)`) passes; a render of each page's
/// states when one slips.
pub fn reload_promised(root: &Path) -> Vec<String> {
    let mut found = vec![];
    for file in rust_files(&root.join("crates")) {
        let name = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        if name.starts_with("crates/gate/") || name.contains("/tests/") {
            continue;
        }
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(e) => {
                found.push(format!("{name}: {e}"));
                continue;
            }
        };
        let body = &source[..source.find(TESTS).unwrap_or(source.len())];
        for said in strings(body) {
            // a string continued over lines (`\` at a line's end) reads as one line
            let said = said.replace("\\\n", " ").to_lowercase();
            let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
            for phrase in NEVER_ON_A_PAGE.iter().filter(|p| said.contains(*p)) {
                found.push(format!("{name}: « {phrase} »"));
            }
        }
    }
    found
}
