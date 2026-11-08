//! A page's three views at its one URL (rust/CLAUDE.md rule 4, task rust-design-page; @nha, 2026-11-08: « a toggle allows
//! seeing both (change the URL) or just side by side »): `?view=html`, `?view=text`, `?view=both`. The text is read off the
//! page's own HTML, never written twice (« make sure there is say html->text »), and every HTML view carries the toggle.

/// Kept out of the text: styling, scripts, drawings, and what a page marks `aria-hidden` (a repeat, such as /design's dark cell).
const SKIPPED: [&str; 4] = ["style", "script", "svg", "head"];
/// Each starts a line of the text.
const BLOCKS: [&str; 22] = [
    "address", "article", "aside", "blockquote", "br", "dd", "div", "dl", "dt", "footer", "h1", "h2", "h3", "h4", "h5",
    "h6", "header", "li", "nav", "p", "section", "tr",
];
/// Each a separate item on its line: a tab before and after it.
const ITEMS: [&str; 5] = ["a", "button", "td", "th", "option"];
/// Each a separate word: a space before and after it.
const WORDS: [&str; 2] = ["span", "label"];
const VOID: [&str; 9] = ["area", "br", "col", "hr", "img", "input", "link", "meta", "wbr"];

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => match name.strip_prefix('#') {
            Some(hex) if hex.starts_with(['x', 'X']) => u32::from_str_radix(&hex[1..], 16).ok().and_then(char::from_u32),
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
        } else if ITEMS.contains(&name.as_str()) && !out.ends_with(['\n', '\t']) && !out.is_empty() {
            // a word's space inside the item gives way to the tab
            if out.ends_with(' ') {
                out.pop();
            }
            out.push('\t');
        } else if WORDS.contains(&name.as_str()) && !out.ends_with(char::is_whitespace) && !out.is_empty() {
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
            rest = rest.to_ascii_lowercase().find(&close).map_or("", |at| &rest[at..]);
            continue;
        }
        if VOID.contains(&name.as_str()) || tag.ends_with('/') {
            continue;
        }
        if !hidden_too && hidden_at.is_none() && tag.contains(r#"aria-hidden="true""#) {
            hidden_at = Some(open.len());
        }
        open.push(name);
    }
    let lines = out.lines().map(str::trim_end).filter(|l| !l.trim().is_empty());
    lines.map(|l| format!("{l}\n")).collect()
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
        match rest[1..].find(';').filter(|&semi| semi < 10).and_then(|semi| Some((semi, entity(&rest[1..semi + 1])?))) {
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
    *out += if out.is_empty() || out.ends_with(char::is_whitespace) { collapsed.trim_start() } else { &collapsed };
}

/// The query with `view` set to this one, the rest kept.
pub fn href(path: &str, query: &str, view: &str) -> String {
    let kept: Vec<&str> = query.split('&').filter(|p| !p.is_empty() && !p.starts_with("view=")).collect();
    let mut q = kept.join("&");
    if !q.is_empty() {
        q.push('&');
    }
    format!("{path}?{q}view={view}")
}

/// A URL from the request, as an attribute's value: its quote or bracket never ends the attribute.
use crate::escaped as attr;

/// The three views as links, the one shown marked. `target=_top`: in the side-by-side view's frame it changes the whole tab.
fn toggle(path: &str, query: &str, current: &str) -> String {
    let link = |view: &str, name: &str| {
        let on = if view == current { r#" aria-current="page""# } else { "" };
        format!(r#"<a href="{}" target="_top"{on}>{name}</a>"#, attr(&href(path, query, view)))
    };
    format!(
        r#"<nav id="view-toggle" aria-label="View" style="position:fixed;right:12px;bottom:12px;z-index:9;display:flex;gap:2px;padding:3px;border-radius:8px;background:#222c;font:12px system-ui,sans-serif">{}{}{}</nav><style>#view-toggle a{{color:#eee;text-decoration:none;padding:3px 9px;border-radius:6px}}#view-toggle a[aria-current]{{background:#fff3;color:#fff}}</style>"#,
        link("html", "HTML"),
        link("text", "Text"),
        link("both", "Both")
    )
}

/// An HTML page with the toggle at the end of its body.
pub fn with_toggle(html: &str, path: &str, query: &str) -> String {
    let bar = toggle(path, query, "html");
    match html.rfind("</body>") {
        Some(at) => format!("{}{bar}{}", &html[..at], &html[at..]),
        None => format!("{html}{bar}"),
    }
}

/// The page and its text side by side, each in a frame of its own URL. A link followed in the page's frame takes the text
/// frame and the address along, so the URL still names what both show; the frame's own toggle hides under this one.
pub fn both(path: &str, query: &str) -> String {
    let (html, text) = (attr(&href(path, query, "html")), attr(&href(path, query, "text")));
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Both views</title><style>body{{margin:0;display:flex;height:100vh}}iframe{{flex:1;border:0;min-width:0}}iframe+iframe{{border-left:1px solid #8884;background:#fff}}@media (max-width:700px){{body{{flex-direction:column}}iframe+iframe{{border-left:0;border-top:1px solid #8884}}}}</style></head><body><iframe id="view-html" title="HTML" src="{html}"></iframe><iframe id="view-text" title="Text" src="{text}"></iframe>{}<script>
const page=document.getElementById('view-html'),text=document.getElementById('view-text');
page.addEventListener('load',()=>{{const at=page.contentWindow.location,q=new URLSearchParams(at.search);
const bar=page.contentDocument.getElementById('view-toggle');if(bar)bar.style.display='none';
q.set('view','text');const t=at.pathname+'?'+q;if(!text.src.endsWith(t))text.src=t;
q.set('view','both');history.replaceState(null,'',at.pathname+'?'+q);}});
</script></body></html>"#,
        toggle(path, query, "both")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    #[test]
    fn the_text_is_the_page_s_words_one_line_a_block() {
        let _budget = TestBudget::new();
        let html = r#"<!doctype html><html><head><title>t</title><style>p > a { x }</style></head><body><h1>Rust &#38; tasks</h1><!--x--><div class="a">one <b>two</b><span>three</span></div><div aria-hidden="true">one <i>again</i></div><p>after<br>break &lt;ok&gt; &bogus;</p><svg><text>drawn</text></svg><pre>  kept
  as is</pre></body></html>"#;
        assert_eq!(text_of(html), "Rust & tasks\none two three\nafter\nbreak <ok> &bogus;\n  kept\n  as is\n");
        assert_eq!(text_of("<p>a</p>b <"), "a\nb <\n");
        assert_eq!(text_of("<p>a</p><STYLE>p{}</Style><p>after</p>"), "a\nafter\n");
        let bar = r#"<div><span>Design</span><a href="/">Layout</a><a href="/d"><span>Data</span></a> after</div><div><button>Primary</button><button>Approve</button></div>"#;
        assert_eq!(text_of(bar), "Design\tLayout\tData\tafter\nPrimary\tApprove\n");
    }

    #[test]
    fn each_view_s_link_keeps_the_query_and_sets_the_view() {
        let _budget = TestBudget::new();
        assert_eq!(href("/design", "tab=data&view=text", "both"), "/design?tab=data&view=both");
        assert_eq!(href("/", "", "text"), "/?view=text");
        let page = with_toggle("<html><body><p>x</p></body></html>", "/design", "tab=data");
        assert!(page.contains(r#"<a href="/design?tab=data&amp;view=text" target="_top">Text</a>"#), "{page}");
        assert!(page.contains(r#"<a href="/design?tab=data&amp;view=html" target="_top" aria-current="page">HTML</a>"#), "{page}");
        assert!(page.ends_with("</nav><style>#view-toggle a{color:#eee;text-decoration:none;padding:3px 9px;border-radius:6px}#view-toggle a[aria-current]{background:#fff3;color:#fff}</style></body></html>"), "{page}");
        assert!(!both("/d", r#"a="><script>"#).contains("<script>\""), "a query never ends the attribute");
        let both = both("/design", "tab=data");
        for part in [r#"src="/design?tab=data&amp;view=html""#, r#"src="/design?tab=data&amp;view=text""#, r#"aria-current="page">Both</a>"#] {
            assert!(both.contains(part), "{part} in {both}");
        }
    }
}
