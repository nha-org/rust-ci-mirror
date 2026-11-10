//! A page's words as a client that is no browser reads them, one line a block (rust/CLAUDE.md rule 4: the text is read off the
//! page's own HTML, never written twice). Its own module so any part may read a page's words without the menu: documents did,
//! and forms draws the documents' threads, a module cycle otherwise (task rust-comment-answer-on-page).

/// Kept out of the text: styling, scripts, drawings, what a page marks `aria-hidden` (a repeat, such as /design's dark cell)
/// and what it marks `hidden`, which no browser or window draws (the Notion tab's closed menus and its script's protos).
const SKIPPED: [&str; 5] = ["style", "script", "svg", "head", "template"];
/// Each starts a line of the text.
const BLOCKS: [&str; 24] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    // a table's line (its count, order and next page) is its own, never run into its head (task design-agent-forms)
    "caption",
    "dd",
    "div",
    "dl",
    "dt",
    "footer",
    // a form is a block: two forms' buttons never run together (task rust-draft-approve)
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "li",
    "nav",
    "p",
    "section",
    "tr",
];
/// Each a separate item on its line: a tab before and after it. A time too, so an event's words never run into its date
/// (/code-review of PR 1250).
const ITEMS: [&str; 6] = ["a", "button", "td", "th", "option", "time"];
/// Each a separate word: a space before and after it. A key too, so a chord's keys never run together (`⌘ K`).
const WORDS: [&str; 3] = ["span", "label", "kbd"];
const VOID: [&str; 9] = [
    "area", "br", "col", "hr", "img", "input", "link", "meta", "wbr",
];

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => match name.strip_prefix('#') {
            Some(hex) if hex.starts_with(['x', 'X']) => u32::from_str_radix(&hex[1..], 16)
                .ok()
                .and_then(char::from_u32),
            Some(dec) => dec.parse().ok().and_then(char::from_u32),
            None => None,
        },
    }
}

/// The words of an HTML page, one line a block, in reading order: what a client that is no browser reads.
pub fn text_of(html: &str) -> String {
    read(html, false)
}

/// Every word the page shows, an `aria-hidden` repeat included: what a window of the same component draws.
pub fn every_word(html: &str) -> String {
    read(html, true)
}

fn read(html: &str, hidden_too: bool) -> String {
    let (mut out, mut open, mut hidden_at) = (String::new(), Vec::<String>::new(), None::<usize>);
    let mut rest = html;
    let pre = |open: &[String]| open.iter().any(|t| t == "pre");
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            push_text(&mut out, rest, false, hidden_at.is_some());
            break;
        };
        push_text(&mut out, &rest[..lt], pre(&open), hidden_at.is_some());
        rest = &rest[lt..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        // a '<' with no '>' after it opens no tag: the rest is text
        let Some(close) = rest.find('>') else {
            push_text(&mut out, rest, pre(&open), hidden_at.is_some());
            break;
        };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        if BLOCKS.contains(&name.as_str()) || name == "pre" {
            out.push('\n');
        } else if ITEMS.contains(&name.as_str()) && !out.ends_with(['\n', '\t']) && !out.is_empty()
        {
            // a word's space inside the item gives way to the tab
            if out.ends_with(' ') {
                out.pop();
            }
            out.push('\t');
        } else if WORDS.contains(&name.as_str())
            && !out.ends_with(char::is_whitespace)
            && !out.is_empty()
        {
            out.push(' ');
        }
        if closing {
            if let Some(at) = open.iter().rposition(|t| *t == name) {
                open.truncate(at);
                if hidden_at.is_some_and(|h| at <= h) {
                    hidden_at = None;
                }
            }
            continue;
        }
        if SKIPPED.contains(&name.as_str()) && !tag.ends_with('/') {
            // a raw-text element's body is no markup: skip straight to its end tag
            let close = format!("</{name}");
            rest = rest
                .to_ascii_lowercase()
                .find(&close)
                .map_or("", |at| &rest[at..]);
            continue;
        }
        if VOID.contains(&name.as_str()) || tag.ends_with('/') {
            continue;
        }
        let unseen = unquoted(tag)
            .split_whitespace()
            .skip(1)
            .any(|a| a == "hidden" || a.starts_with("hidden="));
        let repeat = !hidden_too && tag.contains(r#"aria-hidden="true""#);
        if hidden_at.is_none() && (unseen || repeat) {
            hidden_at = Some(open.len());
        }
        open.push(name);
    }
    let lines = out
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty());
    lines.map(|l| format!("{l}\n")).collect()
}

/// A tag with its quoted values taken out: a word « hidden » in `class="card hidden"` is no attribute (/code-review).
fn unquoted(tag: &str) -> String {
    let mut quote = None;
    tag.chars()
        .filter(|&c| match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                false
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                false
            }
            None => true,
        })
        .collect()
}

fn push_text(out: &mut String, raw: &str, pre: bool, hidden: bool) {
    if hidden {
        return;
    }
    let mut decoded = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        decoded += &rest[..amp];
        rest = &rest[amp..];
        match rest[1..]
            .find(';')
            .filter(|&semi| semi < 10)
            .and_then(|semi| Some((semi, entity(&rest[1..semi + 1])?)))
        {
            Some((semi, c)) => {
                decoded.push(c);
                rest = &rest[semi + 2..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded += rest;
    if pre {
        *out += &decoded;
        return;
    }
    let mut collapsed = String::with_capacity(decoded.len());
    for c in decoded.chars() {
        if !c.is_whitespace() {
            collapsed.push(c);
        } else if !collapsed.ends_with(' ') {
            collapsed.push(' ');
        }
    }
    *out += if out.is_empty() || out.ends_with(char::is_whitespace) {
        collapsed.trim_start()
    } else {
        &collapsed
    };
}
