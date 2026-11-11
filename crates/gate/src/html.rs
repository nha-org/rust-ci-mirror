//! The rule that keeps pages out of the dash crate until rust-ui-tree settles the rendering. Reads strings with graph's lexer.
use crate::graph::{Lexeme, lexed, strings, tokens};
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
    // every crate but the design system's: an HTML string anywhere else is a page drawn outside it, a mail's body included
    // (rust-mail-parity's html.rs passed this gate when it read crates/dash alone, and the layer judge held it). A crate's
    // tests/ folder is test code: its fixtures hold a mail's HTML as the server sends it.
    let files = std::fs::read_dir(root.join("crates"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|crate_dir| !crate_dir.ends_with("surface"))
        .flat_map(|crate_dir| rust_files(&crate_dir))
        .filter(|f| {
            f.starts_with(root.join("crates/dash"))
                || !f.components().any(|c| c.as_os_str() == "tests")
        })
        .filter(|f| !HTML_HELD.iter().any(|held| f.ends_with(held)));
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

/// The files outside crates/surface that held HTML strings when html_in_strings widened to every crate (rust-mail-parity,
/// 2026-11-11): each moves into the design system in its own task, and this list only shrinks. A file not named here fails.
pub const HTML_HELD: [&str; 7] = [
    "crates/ui/src/lib.rs",
    "crates/launcher/src/brief.rs",
    "crates/views/src/tests.rs",
    "crates/views/src/rules.rs",
    "crates/core/src/tasks.rs",
    "crates/comments/src/lib.rs",
    "crates/site/src/main.rs",
];

/// The components' root module, which only declares their folder: the one place a page's button, field, card, badge, avatar
/// or tabs is drawn is that folder (task rust-design-components; @nha, 2026-11-09: « We want all components to be dual-use »).
pub const COMPONENTS: &str = "crates/surface/src/components.rs";
/// The folder of the components, each in a module of its own (task rust-components-register).
pub const COMPONENT_MODULES: &str = "crates/surface/src/components/";

/// What one component registers (surface's crate::component), read from its `inventory::submit!` block's text.
#[derive(Debug, PartialEq)]
pub struct Registered {
    pub file: String,
    pub slug: String,
    pub classes: Vec<String>,
    pub tags: Vec<String>,
}

/// The strings of `field: &[..]` in a registration, up to its `]`.
fn field_list(block: &str, field: &str) -> Vec<String> {
    let Some(at) = block.find(&format!("{field}: &[")) else {
        return vec![];
    };
    let list = &block[at..];
    strings(&list[..list.find(']').unwrap_or(list.len())])
}

/// Each component registered in COMPONENTS and the files under COMPONENT_MODULES: a block from `inventory::submit!` to the
/// next line opening on `}`, as rustfmt writes an item, that builds a `Component`.
pub fn registrations(root: &Path) -> Vec<Registered> {
    let mut files = vec![root.join(COMPONENTS)];
    let modules = root.join(COMPONENT_MODULES);
    if modules.is_dir() {
        files.extend(rust_files(&modules));
    }
    let mut found = vec![];
    for file in files {
        let name = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        let source = std::fs::read_to_string(&file).unwrap_or_default();
        for (at, _) in source.match_indices("inventory::submit!") {
            let rest = &source[at..];
            let block = &rest[..rest.find("\n}").unwrap_or(rest.len())];
            if !block.contains("Component {") {
                continue;
            }
            let slug = field_list(&block.replacen("slug: \"", "slug: &[\"", 1), "slug");
            found.push(Registered {
                file: name.clone(),
                slug: slug.into_iter().next().unwrap_or_default(),
                classes: field_list(block, "classes"),
                tags: field_list(block, "tags"),
            });
        }
    }
    found
}

/// A component's own file: COMPONENTS or one under COMPONENT_MODULES.
fn is_component_module(name: &str) -> bool {
    name == COMPONENTS || name.starts_with(COMPONENT_MODULES)
}
/// /design's own mock screens, which draw each state by hand to show it: the showcase, not a page.
pub const SHOWCASE: [&str; 4] = [
    "crates/surface/src/design/families.rs",
    "crates/surface/src/design/linear.rs",
    "crates/surface/src/design/mod.rs",
    "crates/surface/src/design/notion.rs",
];

/// Where a file's top-level test module starts.
const TESTS: &str = "\n#[cfg(test)]\nmod ";

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

/// Each place a page draws what a component covers: an rsx element of a registered component's tags (`button {` after a
/// `{`, `}` or `,`), or a `class:` whose expression (up to its `,` or closing `}`) holds one of their classes in any literal,
/// an `if`'s branches included (/code-review: `class: if a { "li-card x" } else { "li-card" }` passed). Every crate's .rs is
/// read but the components' own modules and SHOWCASE.
/// HTML written in a string may hold no button, input or textarea tag. Code after a top-level `#[cfg(test)]` module, a
/// tests/ folder and `//` lines are not read. ponytail: it misses a class built at run time (`format!`, a variable) and the
/// rust/ui-spike workspace, a spike outside crates/; an rsx parse when one slips past.
pub fn page_local_markup(root: &Path) -> Vec<String> {
    let registered = registrations(root);
    let tags: Vec<String> = registered.iter().flat_map(|r| r.tags.clone()).collect();
    let classes: Vec<String> = registered.iter().flat_map(|r| r.classes.clone()).collect();
    if tags.is_empty() || classes.is_empty() {
        return vec![format!(
            "{COMPONENTS}: no component registers a class or a tag"
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
        if is_component_module(&name)
            || SHOWCASE.contains(&name.as_str())
            || name.contains("/tests/")
        {
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

/// Whether this code submits a `Component` to inventory: `submit! {` then a path ending in `Component {`.
fn registers(body: &str) -> bool {
    let t = tokens(body);
    (0..t.len()).any(|i| {
        t[i] == "submit"
            && t.get(i + 1).is_some_and(|n| n == "!")
            && t.get(i + 2).is_some_and(|n| n == "{")
            && (i + 3..t.len())
                .find(|&j| t[j] == "{")
                .is_some_and(|j| t[j - 1] == "Component")
    })
}

/// Each list of `body` that names two components, two registrations, as `<file>: <name> names a, b`, in their module too: a
/// module holding several is where two tasks adding a part each would meet (/code-review of 114248eb4). A list is an array
/// literal, its brackets
/// after neither a name nor a `)` or `]` (that is an index); `#[cfg(test)]` items are skipped whole. It goes by the
/// `const`, `static` or `let` before it in its block, else « a list in <its fn> ».
fn lists_naming_components<'a>(
    name: &str,
    body: &str,
    owner: &dyn Fn(&str) -> Option<&'a Registered>,
) -> Vec<String> {
    let lexemes = lexed(body);
    let code = |i: usize| match lexemes.get(i) {
        Some(Lexeme::Code(c)) => c.as_str(),
        _ => "",
    };
    let is_name = |c: &str| {
        c.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_')
            && !["in", "return", "break", "mut"].contains(&c)
    };
    let (mut found, mut i) = (vec![], 0);
    // each open `[`: whether it opened a list; the outermost list's name and what it named
    let (mut open, mut binding, mut function): (Vec<bool>, Option<String>, String) =
        (vec![], None, String::new());
    let (mut list, mut named): (String, Vec<&Registered>) = (String::new(), vec![]);
    while i < lexemes.len() {
        match &lexemes[i] {
            Lexeme::Str(s) if open.contains(&true) => {
                if let Some(r) = owner(s)
                    && !named.iter().any(|n| n.slug == r.slug)
                {
                    named.push(r);
                }
            }
            Lexeme::Str(_) => {}
            Lexeme::Code(c) => match c.as_str() {
                "#" if (1..7)
                    .map(|k| code(i + k))
                    .eq(["[", "cfg", "(", "test", ")", "]"]) =>
                {
                    // past the item: its `;`, or its body's closing brace
                    let mut j = i + 7;
                    while j < lexemes.len() && !["{", ";"].contains(&code(j)) {
                        j += 1;
                    }
                    let mut depth = 0;
                    while j < lexemes.len() {
                        match code(j) {
                            "{" => depth += 1,
                            "}" if depth == 1 => break,
                            "}" => depth -= 1,
                            ";" if depth == 0 => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    i = j;
                }
                "fn" if open.is_empty() && is_name(code(i + 1)) => {
                    function = code(i + 1).to_string()
                }
                "const" | "static" | "let" if open.is_empty() => {
                    let at = if code(i + 1) == "mut" { i + 2 } else { i + 1 };
                    binding = (is_name(code(at)) && code(at) != "fn").then(|| code(at).to_string());
                }
                "{" | "}" | ";" if open.is_empty() => binding = None,
                "[" => {
                    let before = code(i.wrapping_sub(1));
                    let is_list = !(is_name(before) || before == ")" || before == "]");
                    if is_list && !open.contains(&true) {
                        list = binding
                            .clone()
                            .unwrap_or_else(|| format!("a list in {function}"));
                    }
                    open.push(is_list);
                }
                "]" => {
                    let closed_a_list = open.pop() == Some(true);
                    if closed_a_list && !open.contains(&true) {
                        if named.len() >= 2 {
                            let slugs: Vec<&str> = named.iter().map(|r| r.slug.as_str()).collect();
                            found.push(format!("{name}: {list} names {}", slugs.join(", ")));
                        }
                        named.clear();
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    found
}

/// In the design system (crates/surface/src), outside tests: each array literal, a `const`'s, a `let`'s or one inline, that
/// names two components by slug or class (`.button` too), and each component registered outside COMPONENTS and
/// COMPONENT_MODULES, which `registrations` would not read. A list of components is what two tasks that each add one both
/// edit, so each registers its own instead (task rust-components-register; rust-ui-controls and rust-timeline-component met
/// 3 conflicts in the catalog, OWNED_CLASSES and components.rs). One name is no shared list: `form` and `button` are HTML's
/// tags as much as components. A list is named by the `const`, `static` or `let` before it in its block.
/// ponytail: a `match` on slugs is not read, nor another crate, whose lists of words (`list`, `mail`) meet the slugs by
/// chance. A parse of the item if one slips past.
pub fn shared_component_lists(root: &Path) -> Vec<String> {
    let registered = registrations(root);
    let owner = |s: &str| {
        let s = s.trim_start_matches('.');
        let named = |r: &&Registered| r.slug == s || r.classes.iter().any(|c| c == s);
        registered.iter().find(named)
    };
    let mut found = vec![];
    for file in rust_files(&root.join("crates/surface/src")) {
        let name = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        let source = std::fs::read_to_string(&file).unwrap_or_default();
        let body = &source[..source.find(TESTS).unwrap_or(source.len())];
        if !is_component_module(&name) && registers(body) {
            found.push(format!(
                "{name}: registers a component outside {COMPONENTS} and {COMPONENT_MODULES}"
            ));
        }
        found.extend(lists_naming_components(&name, body, &owner));
    }
    found
}

/// The tasks board's drawing: components only (task rust-board-linear-order).
pub const BOARD: &str = "crates/surface/src/board.rs";

/// Each HTML element the tasks board writes itself, rsx (`div {` after a `{`, `}`, `,`, `(` or `[`; a string between is no
/// token) or a tag in a string: the board composes components alone, so a part it lacks goes to COMPONENTS first (@nha,
/// 2026-11-09: « does not follow the design system completely it seems ? let's not have drift. Shared components. »). A missing
/// file is a finding: the rule never passes empty.
pub fn board_markup(root: &Path) -> Vec<String> {
    let source = match std::fs::read_to_string(root.join(BOARD)) {
        Ok(source) => source,
        Err(e) => return vec![format!("{BOARD}: {e}")],
    };
    let body = &source[..source.find(TESTS).unwrap_or(source.len())];
    let code = tokens(body);
    let elements = code.windows(3).filter(|w| {
        ["{", "}", ",", "(", "["].contains(&w[0].as_str())
            && HTML_TAGS.contains(&w[1].as_str())
            && w[2] == "{"
    });
    let mut found: Vec<String> = elements.map(|w| format!("{BOARD}: {} {{", w[1])).collect();
    let written = strings(body).iter().flat_map(|s| html_tags(s)).count();
    if written > 0 {
        found.push(format!("{BOARD}: {written} HTML tags in strings"));
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

/// The files every feature edited while each page was a line in them (task rust-registry-no-hubs: main.rs changed 31 times
/// from 2026-11-08 to 11-10, PRs 1282 and 1284 conflicted there): main.rs names no page, tasks.rs none of another module's.
pub const HUBS: [&str; 2] = ["crates/dash/src/main.rs", "crates/dash/src/tasks.rs"];

/// Each page path a `Page { .., path: "<path>", .. }` registration in this code holds, its fields in any order (a page the
/// rule failed to read is a hub free to name it), a `/*` cut to its prefix's `/`.
fn pages_registered(body: &str) -> Vec<String> {
    let mut found = vec![];
    for (at, _) in body.match_indices("Page {") {
        let rest = &body[at + "Page {".len()..];
        let scope = &rest[..rest.find("Page {").unwrap_or(rest.len())];
        let Some(path_at) = scope.find("path:") else {
            continue;
        };
        let Some(rest) = scope[path_at + "path:".len()..]
            .trim_start()
            .strip_prefix('"')
        else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            found.push(rest[..end].trim_end_matches('*').to_string());
        }
    }
    found
}

/// Each page path a hub of `HUBS` names in its code, test code left out (task rust-registry-no-hubs; @nha, 2026-11-09: « is it
/// an orchestration failure or is it because the architecture is not decoupled enough yet ? »): a page registers its route in
/// its own module (crates/dash/src/pages.rs), so a new page adds no line to a hub but its `mod`. main.rs names no page and
/// registers none; tasks.rs names only its own. `/` alone is left out, a separator as much as Home's path.
pub fn hub_named_pages(root: &Path) -> Vec<String> {
    let mut registered: Vec<(String, String)> = vec![];
    let mut hubs: Vec<(String, Vec<String>)> = vec![];
    for file in rust_files(&root.join("crates/dash/src")) {
        let name = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(e) => return vec![format!("{name}: {e}")],
        };
        let body = &source[..source.find(TESTS).unwrap_or(source.len())];
        for path in pages_registered(body).into_iter().filter(|p| p != "/") {
            registered.push((path, name.clone()));
        }
        if HUBS.contains(&name.as_str()) {
            hubs.push((name, strings(body)));
        }
    }
    let mut found = vec![];
    for (hub, named) in &hubs {
        for (path, owner) in &registered {
            if owner == hub && hub == HUBS[0] {
                found.push(format!(
                    "{hub} registers {path}: register it in its page's module"
                ));
            } else if owner != hub && named.contains(path) {
                found.push(format!(
                    "{hub} names {path}, the page of {owner}: route it there"
                ));
            }
        }
    }
    found.sort();
    found.dedup();
    found
}
