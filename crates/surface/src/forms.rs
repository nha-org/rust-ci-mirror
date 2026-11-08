//! A page's three views at its one URL (rust/CLAUDE.md rule 4, task rust-design-page; @nha, 2026-11-08: « a toggle allows
//! seeing both (change the URL) or just side by side »): `?view=html`, `?view=text`, `?view=both`. The text is read off the
//! page's own HTML, never written twice (« make sure there is say html->text »), and every HTML view carries the toggle.

/// Kept out of the text: styling, scripts, drawings, and what a page marks `aria-hidden` (a repeat, such as /design's dark cell).
const SKIPPED: [&str; 4] = ["style", "script", "svg", "head"];
/// Each starts a line of the text.
const BLOCKS: [&str; 22] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "footer",
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
/// Each a separate item on its line: a tab before and after it.
const ITEMS: [&str; 5] = ["a", "button", "td", "th", "option"];
/// Each a separate word: a space before and after it.
const WORDS: [&str; 2] = ["span", "label"];
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
        if !hidden_too && hidden_at.is_none() && tag.contains(r#"aria-hidden="true""#) {
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

/// The query with `view` set to this one, the rest kept.
pub fn href(path: &str, query: &str, view: &str) -> String {
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("view="))
        .collect();
    let mut q = kept.join("&");
    if !q.is_empty() {
        q.push('&');
    }
    format!("{path}?{q}view={view}")
}

/// A URL from the request, as an attribute's value: its quote or bracket never ends the attribute.
use crate::escaped as attr;

/// The three views as links, the one shown marked: Human the HTML, Agent the text (@nha, 2026-11-08: « the menu should have a
/// toggle human/agent »). `target=_top`: in the side-by-side view's frame it changes the whole tab.
fn toggle(path: &str, query: &str, current: &str) -> String {
    let link = |view: &str, name: &str| {
        let on = if view == current {
            r#" aria-current="page""#
        } else {
            ""
        };
        format!(
            r#"<a href="{}" target="_top"{on}>{name}</a>"#,
            attr(&href(path, query, view))
        )
    };
    format!(
        r#"<nav id="view-toggle" aria-label="View">{}{}{}</nav><style>#view-toggle{{display:flex;gap:2px;padding:3px;border-radius:8px;background:#222c;font:12px system-ui,sans-serif}}#view-toggle a{{flex:1;text-align:center;color:#eee;text-decoration:none;padding:3px 9px;border-radius:6px}}#view-toggle a[aria-current]{{background:#fff3;color:#fff}}</style>"#,
        link("html", "Human"),
        link("text", "Agent"),
        link("both", "Both")
    )
}

/// The menu, Linear's sidebar (task rust-nav-board; @nha, 2026-11-08: « http://localhost:7788/ needs a menu »): each entry's
/// name, its URL, and the paths it stands for. Mail and the projects' boards are the older app's, reached through the front door.
const MENU: [(&str, &str, &[&str]); 6] = [
    ("Home", "/", &["/"]),
    ("Tasks", "/tasks", &["/tasks", "/graph", "/projects"]),
    ("Mail", "/mail", &["/mail"]),
    ("Tools", "/tools", &["/tools"]),
    ("Design", "/design", &["/design"]),
    ("Code", "/deps", &["/deps"]),
];
/// His projects under Tasks, each Rust's board kept to it, `/tasks?project=<name>` (task rust-nav-polish, the orchestrator's
/// answer, 2026-11-08: « each sidebar project opens Rust's board filtered to it »). ponytail: Dashboard.TaskList.projects/0's
/// names copied, read from the older app once a project is added more than once a quarter.
const PROJECTS: [&str; 4] = ["site_perso", "klar", "graal", "geodb-whatsnr"];

/// The sidebar: the menu, his projects under Tasks, the view toggle at its foot. Collapsed on a wide screen and a drawer on a
/// phone by one checkbox, so it needs no script; the script only remembers a wide screen's fold.
fn sidebar(path: &str, query: &str) -> String {
    // a page under an entry is the entry's: /tasks/<id> and /projects/<name> mark Tasks, a mail thread Mail
    let under = |p: &str| {
        path == p
            || (p != "/"
                && path
                    .strip_prefix(p)
                    .is_some_and(|rest| rest.starts_with('/')))
    };
    let here = |paths: &[&str]| paths.iter().any(|p| under(p));
    let mut items = String::new();
    for (name, url, paths) in MENU {
        let on = if here(paths) {
            r#" aria-current="page""#
        } else {
            ""
        };
        items += &format!(r#"<a href="{url}"{on}>{name}</a>"#);
        if name == "Tasks" {
            let shown =
                |p: &str| path == "/tasks" && query.split('&').any(|q| q == format!("project={p}"));
            let project = |p: &&str| {
                let on = if shown(p) || under(&format!("/projects/{p}")) {
                    r#" aria-current="page""#
                } else {
                    ""
                };
                format!(r#"<a href="/tasks?project={p}"{on}>{p}</a>"#)
            };
            let projects: String = PROJECTS.iter().map(project).collect();
            items += &format!(r#"<div class="nav-sub">{projects}</div>"#);
        }
    }
    format!(
        r#"<input type="checkbox" id="dash-nav-fold" hidden><label for="dash-nav-fold" id="dash-nav-open" title="Menu">&#9776;</label><aside id="dash-nav" aria-label="Menu"><div class="nav-head"><b>dash</b><label for="dash-nav-fold" title="Fold the menu">&#171;</label></div><nav id="dash-menu" aria-label="Pages">{items}</nav><div class="nav-foot">{}</div></aside><style>{NAV_CSS}</style><script>{NAV_JS}</script>"#,
        toggle(path, query, "html")
    )
}

/// Every id and rule starts `dash-`: the older app's pages wear the same sidebar, and its own layout has an `id="menu"`.
/// Wide: a fixed column, folded by the checkbox. Narrow (a phone): off screen until the checkbox opens it over the page. The
/// button that opens it sits bottom left, clear of a page's own header and its menu button, and the column over an older
/// page's own drawer (z-index 50 there).
const NAV_CSS: &str = "html{padding-left:220px}html:has(#dash-nav-fold:checked){padding-left:0}\
#dash-nav{position:fixed;top:0;bottom:0;left:0;width:220px;box-sizing:border-box;z-index:60;display:flex;flex-direction:column;gap:8px;padding:12px 10px;background:var(--color-base-100,#fafafa);color:var(--color-base-content,#222);border-right:1px solid #8883;font:13px Inter,system-ui,sans-serif}\
#dash-nav-fold:checked~#dash-nav{display:none}\
#dash-nav .nav-head{display:flex;justify-content:space-between;align-items:center;padding:2px 8px}\
#dash-nav label,#dash-nav-open{cursor:pointer;user-select:none}\
#dash-nav-open{display:none;position:fixed;bottom:12px;left:12px;z-index:61;padding:2px 9px;border-radius:6px;background:var(--color-base-100,#fafafa);border:1px solid #8883;font:16px system-ui,sans-serif}\
#dash-nav-fold:checked~#dash-nav-open{display:block}\
#dash-menu{display:flex;flex-direction:column;gap:1px;flex:1;overflow-y:auto}\
#dash-menu a{color:inherit;text-decoration:none;padding:5px 8px;border-radius:6px}\
#dash-menu a:hover{background:#8882}#dash-menu a[aria-current]{background:#8883;font-weight:600}\
#dash-menu .nav-sub{display:flex;flex-direction:column}#dash-menu .nav-sub a{padding-left:22px;opacity:.8}\
@media (max-width:700px){html,html:has(#dash-nav-fold:checked){padding-left:0}\
#dash-nav{display:none;box-shadow:0 0 24px #0005}#dash-nav-open{display:block}\
#dash-nav-fold:checked~#dash-nav{display:flex}#dash-nav-fold:checked~#dash-nav-open{display:none}}";
/// A wide screen's fold is kept across pages; a phone's drawer opens closed on each. The checkbox means folded when wide and
/// open when narrow, so crossing the width sets it afresh: a wide fold never becomes an open drawer (/code-review).
const NAV_JS: &str = "try{const f=document.getElementById('dash-nav-fold'),w=matchMedia('(min-width:701px)');\
const kept=()=>{try{return w.matches&&localStorage.getItem('dash-nav-folded')==='1'}catch(e){return false}};\
f.checked=kept();w.addEventListener('change',()=>{f.checked=kept()});\
f.addEventListener('change',()=>{try{if(w.matches)localStorage.setItem('dash-nav-folded',f.checked?'1':'0')}catch(e){}})}catch(e){}";

/// Where a page's body starts: just past its `<body>` tag, found in any case and as a whole tag name.
fn body_open(html: &str) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<body").map(|at| from + at) {
        let after = lower[at + 5..].chars().next();
        if after.is_some_and(|c| c == '>' || c == '/' || c.is_ascii_whitespace()) {
            return lower[at..].find('>').map(|end| at + end + 1);
        }
        from = at + 5;
    }
    None
}

/// An HTML page with the sidebar first in its body: every page carries the one menu. A page with no body tag gets it first.
pub fn with_menu(html: &str, path: &str, query: &str) -> String {
    dressed(html, sidebar(path, query))
}

/// An older page's own navigation goes whole, the sidebar the one menu (task rust-nav-polish: « An older-app page behind
/// the front door shows the sidebar only: its own top nav hidden »; the PM check, 2026-11-08: a second bar of older-app
/// controls is no « one menu »): the plain layout's header, the bell and the comment button with it, and the full
/// layout's side column, its grid then one column. The page's comment rail at its right edge stays: commenting needs no bar.
const OLDER_CSS: &str = "<style>#site-header,#site-nav,#site-nav-backdrop{display:none!important}\
.linear{grid-template-columns:minmax(0,1fr)!important}</style>";

fn dressed(html: &str, bar: String) -> String {
    match body_open(html) {
        Some(at) => format!("{}{bar}{}", &html[..at], &html[at..]),
        None => format!("{bar}{html}"),
    }
}

/// The body's length of an older answer the front door may dress, read off its head (the bytes before the blank line): a 200
/// of HTML with a Content-Length, neither compressed nor chunked, whose Content-Security-Policy neither sandboxes it nor
/// limits its scripts (the menu's own script would be refused there, and a sandboxed file is not a page of the app).
/// `None`: it goes on as it came. ponytail: a chunked page goes undressed; the older app sends its pages with a length today.
pub fn older_dressable(head: &str) -> Option<usize> {
    let mut lines = head.split("\r\n");
    if lines.next()?.split_whitespace().nth(1) != Some("200") {
        return None;
    }
    let headers: Vec<&str> = lines.collect();
    let named = |name: &str| {
        let value = headers.iter().find_map(|h| {
            h.split_once(':')
                .filter(|(n, _)| n.trim().eq_ignore_ascii_case(name))
        });
        value.map(|(_, v)| v.trim().to_ascii_lowercase())
    };
    let html = named("content-type").is_some_and(|t| t.starts_with("text/html"));
    let locked = named("content-security-policy").is_some_and(|p| {
        ["sandbox", "script-src", "default-src"]
            .iter()
            .any(|d| p.contains(d))
    });
    if !html
        || locked
        || named("content-encoding").is_some()
        || named("transfer-encoding").is_some()
    {
        return None;
    }
    named("content-length")?.parse().ok()
}

/// The older app's answer to a browser's page, as the front door hands it on (task rust-nav-board; the orchestrator's answer,
/// 2026-11-08: « Older pages too »): the menu first in its body, or, `text` asked, its words. Its validators go: they name
/// the page as the older app sent it. `None`: not one `older_dressable` takes, or no body tag, and it goes on as it came.
pub fn older_page(head: &str, body: &str, path: &str, query: &str, text: bool) -> Option<Vec<u8>> {
    if older_dressable(head)? != body.len() {
        return None;
    }
    let (kind, body) = if text {
        ("text/plain; charset=utf-8", text_of(body))
    } else {
        body_open(body)?;
        (
            "text/html; charset=utf-8",
            dressed(body, sidebar(path, query) + OLDER_CSS),
        )
    };
    let drop = [
        "content-type",
        "content-length",
        "connection",
        "etag",
        "last-modified",
        "content-md5",
    ];
    let kept: String = (head.split("\r\n").skip(1))
        .filter(|h| {
            !drop.iter().any(|d| {
                h.split(':')
                    .next()
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case(d))
            })
        })
        .map(|h| format!("{h}\r\n"))
        .collect();
    let head = format!(
        "HTTP/1.1 200 OK\r\n{kept}Content-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    Some([head.into_bytes(), body.into_bytes()].concat())
}

/// The page and its text side by side, each in a frame of its own URL. A link followed in the page's frame takes the text
/// frame and the address along, so the URL still names what both show. The frame's sidebar, its toggle with it, goes: its
/// links would open inside the frame, and this page's own toggle stands for it.
pub fn both(path: &str, query: &str) -> String {
    let (html, text) = (
        attr(&href(path, query, "html")),
        attr(&href(path, query, "text")),
    );
    // here the toggle floats: this page has no sidebar of its own, its frame's does
    let bar = format!(
        r#"<div style="position:fixed;right:12px;bottom:12px;z-index:9">{}</div>"#,
        toggle(path, query, "both")
    );
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Both views</title><style>body{{margin:0;display:flex;height:100vh}}iframe{{flex:1;border:0;min-width:0}}iframe+iframe{{border-left:1px solid #8884;background:#fff}}@media (max-width:700px){{body{{flex-direction:column}}iframe+iframe{{border-left:0;border-top:1px solid #8884}}}}</style></head><body><iframe id="view-html" title="HTML" src="{html}"></iframe><iframe id="view-text" title="Text" src="{text}"></iframe>{}<script>
const page=document.getElementById('view-html'),text=document.getElementById('view-text');
page.addEventListener('load',()=>{{const at=page.contentWindow.location,q=new URLSearchParams(at.search);
const d=page.contentDocument;for(const id of['dash-nav','dash-nav-open'])d.getElementById(id)?.remove();d.documentElement.style.padding='0';
q.set('view','text');const t=at.pathname+'?'+q;if(!text.src.endsWith(t))text.src=t;
q.set('view','both');history.replaceState(null,'',at.pathname+'?'+q);}});
</script></body></html>"#,
        bar
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
        assert_eq!(
            text_of(html),
            "Rust & tasks\none two three\nafter\nbreak <ok> &bogus;\n  kept\n  as is\n"
        );
        assert_eq!(text_of("<p>a</p>b <"), "a\nb <\n");
        assert_eq!(
            text_of("<p>a</p><STYLE>p{}</Style><p>after</p>"),
            "a\nafter\n"
        );
        let bar = r#"<div><span>Design</span><a href="/">Layout</a><a href="/d"><span>Data</span></a> after</div><div><button>Primary</button><button>Approve</button></div>"#;
        assert_eq!(
            text_of(bar),
            "Design\tLayout\tData\tafter\nPrimary\tApprove\n"
        );
    }

    #[test]
    fn each_view_s_link_keeps_the_query_and_sets_the_view() {
        let _budget = TestBudget::new();
        assert_eq!(
            href("/design", "tab=data&view=text", "both"),
            "/design?tab=data&view=both"
        );
        assert_eq!(href("/", "", "text"), "/?view=text");
        let page = with_menu("<html><body><p>x</p></body></html>", "/design", "tab=data");
        assert!(
            page.contains(r#"<a href="/design?tab=data&amp;view=text" target="_top">Agent</a>"#),
            "{page}"
        );
        assert!(page.contains(r#"<a href="/design?tab=data&amp;view=html" target="_top" aria-current="page">Human</a>"#), "{page}");
        assert!(
            page.starts_with(r#"<html><body><input type="checkbox" id="dash-nav-fold" hidden>"#),
            "first in the body: {page}"
        );
        assert!(page.ends_with("<p>x</p></body></html>"), "{page}");
    }

    /// Done-when 1 of rust-nav-board: one menu, Home to Code, the page's own entry marked, the toggle at its foot.
    #[test]
    fn every_page_carries_one_menu_and_marks_where_it_is() {
        let _budget = TestBudget::new();
        let names = |page: &str| -> Vec<String> {
            let menu = &page[page.find(r#"<nav id="dash-menu""#).expect("the menu")..];
            let menu = &menu[..menu.find("</nav>").expect("its end")];
            text_of(
                &menu
                    .replace("<div class=\"nav-sub\">", "<span>")
                    .replace("</div>", "</span>"),
            )
            .trim()
            .split('\t')
            .map(String::from)
            .collect()
        };
        let task = with_menu("<body></body>", "/tasks/a-b", "");
        assert_eq!(
            names(&task),
            [
                "Home",
                "Tasks",
                "site_perso",
                "klar",
                "graal",
                "geodb-whatsnr",
                "Mail",
                "Tools",
                "Design",
                "Code"
            ]
        );
        assert!(
            task.contains(r#"<a href="/tasks" aria-current="page">Tasks</a>"#),
            "{task}"
        );
        assert_eq!(
            task.matches(r#"aria-current="page""#).count(),
            2,
            "Tasks and Human: {task}"
        );
        let graph = with_menu("<body></body>", "/graph", "");
        assert!(
            graph.contains(r#"<a href="/tasks" aria-current="page">Tasks</a>"#),
            "/graph is the board's view: {graph}"
        );
        let code = with_menu("<body></body>", "/deps", "");
        assert!(
            code.contains(r#"<a href="/deps" aria-current="page">Code</a>"#),
            "{code}"
        );
        let foot = &code[code.find(r#"class="nav-foot""#).expect("its foot")..];
        assert!(
            foot.starts_with(r#"class="nav-foot"><nav id="view-toggle""#),
            "{foot}"
        );
        assert!(
            foot.contains(r#"href="/deps?view=text" target="_top">Agent</a>"#),
            "{foot}"
        );
        assert_eq!(
            with_menu("<p>no body</p>", "/", "")
                .matches(r#"id="dash-menu""#)
                .count(),
            1
        );
    }

    /// The older app's pages carry the same menu (the orchestrator's answer, 2026-11-08: « Older pages too »).
    #[test]
    fn an_older_page_gets_the_menu_or_its_words_and_anything_else_goes_on_as_it_came() {
        let _budget = TestBudget::new();
        let page = "<!doctype html><html><BODY class=x><h1>Mail</h1><p>3 threads</p></BODY></html>";
        let head = |h: &str, body: &str| format!("{h}\r\nContent-Length: {}", body.len());
        let ok = "HTTP/1.1 200 OK\r\nset-cookie: k=v\r\nETag: \"a\"\r\ncontent-type: text/html; charset=utf-8\r\nConnection: keep-alive\r\ncontent-security-policy: base-uri 'self'; frame-ancestors 'self';";
        let dressed = older_page(&head(ok, page), page, "/mail/t/1", "", false).expect("dressed");
        let dressed = String::from_utf8(dressed).expect("utf-8");
        let (got, body) = dressed.split_once("\r\n\r\n").expect("a head");
        assert_eq!(
            got,
            format!(
                "HTTP/1.1 200 OK\r\nset-cookie: k=v\r\ncontent-security-policy: base-uri 'self'; frame-ancestors 'self';\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close",
                body.len()
            ),
            "its validator goes, the rest stays"
        );
        assert!(body.starts_with(r#"<!doctype html><html><BODY class=x><input type="checkbox" id="dash-nav-fold" hidden>"#), "{body}");
        assert!(
            body.contains(r#"<a href="/mail" aria-current="page">Mail</a>"#),
            "a thread is Mail's: {body}"
        );
        assert!(
            body.contains(r#"href="/mail/t/1?view=text" target="_top">Agent</a>"#),
            "{body}"
        );
        assert!(
            body.ends_with("<h1>Mail</h1><p>3 threads</p></BODY></html>"),
            "{body}"
        );
        assert_eq!(
            body.matches("#site-header,#site-nav,#site-nav-backdrop{display:none!important}")
                .count(),
            1,
            "its own bar hidden whole: {body}"
        );
        assert!(
            !with_menu(page, "/mail", "").contains("#site-header"),
            "our own pages have none to hide"
        );
        let words = String::from_utf8(
            older_page(&head(ok, page), page, "/mail", "", true).expect("its words"),
        )
        .expect("utf-8");
        assert!(
            words.contains("Content-Type: text/plain; charset=utf-8\r\n")
                && words.ends_with("\r\n\r\nMail\n3 threads\n"),
            "{words}"
        );
        let board = with_menu(page, "/projects/klar", "");
        assert!(
            board.contains(r#"<a href="/tasks" aria-current="page">Tasks</a>"#),
            "a project's board is under Tasks: {board}"
        );
        assert!(
            board.contains(r#"<a href="/tasks?project=klar" aria-current="page">klar</a>"#),
            "and its own link: {board}"
        );
        assert!(
            board.contains(r#"<a href="/tasks?project=graal">graal</a>"#),
            "{board}"
        );
        let ours = with_menu(page, "/tasks", "layout=list&project=graal");
        assert!(
            ours.contains(r#"<a href="/tasks?project=graal" aria-current="page">graal</a>"#),
            "Rust's board kept to it: {ours}"
        );
        assert_eq!(
            ours.matches(r#"aria-current="page""#).count(),
            3,
            "Tasks, graal and Human: {ours}"
        );
        assert_eq!(
            with_menu("<html><bodyx><body>a</body>", "/", "").find(r#"<input"#),
            Some(19),
            "a whole tag name only"
        );
        let html = "HTTP/1.1 200 OK\r\ncontent-type: text/html";
        for (h, body) in [
            (
                "HTTP/1.1 302 Found\r\nLocation: /x\r\ncontent-type: text/html",
                page,
            ),
            ("HTTP/1.1 200 OK\r\ncontent-type: application/json", "{}"),
            (
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-encoding: gzip",
                page,
            ),
            (
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ntransfer-encoding: chunked",
                page,
            ),
            (
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-security-policy: sandbox",
                page,
            ),
            (
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-security-policy: script-src 'none'",
                page,
            ),
            (html, "<p>no body tag</p>"),
        ] {
            assert_eq!(
                older_page(&head(h, body), body, "/mail", "", false),
                None,
                "{h}"
            );
        }
        assert_eq!(
            older_page(
                &format!("{html}\r\nContent-Length: 999"),
                page,
                "/mail",
                "",
                false
            ),
            None,
            "a short body"
        );
        assert_eq!(older_dressable(html), None, "no length");
        assert_eq!(older_dressable(&head(html, page)), Some(page.len()));
    }

    #[test]
    fn the_side_by_side_view_floats_its_toggle() {
        let _budget = TestBudget::new();
        assert!(
            !both("/d", r#"a="><script>"#).contains("<script>\""),
            "a query never ends the attribute"
        );
        let both = both("/design", "tab=data");
        for part in [
            r#"src="/design?tab=data&amp;view=html""#,
            r#"src="/design?tab=data&amp;view=text""#,
            r#"aria-current="page">Both</a>"#,
        ] {
            assert!(both.contains(part), "{part} in {both}");
        }
    }
}
