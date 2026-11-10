//! A page's three views at its one URL (rust/CLAUDE.md rule 4, task rust-design-page; @nha, 2026-11-09: « agent/human/code
//! where code is usually json »): `?view=html`, `?view=text`, `?view=data`, its tree as pretty JSON. The text is read off the
//! page's own HTML, never written twice (« make sure there is say html->text »), and every HTML view carries the toggle.

/// Kept out of the text: styling, scripts, drawings, what a page marks `aria-hidden` (a repeat, such as /design's dark cell)
/// and what it marks `hidden`, which no browser or window draws (the Notion tab's closed menus and its script's protos).
use crate::components::{Button, Fold, Input, Tab, Tabs, Textarea};
use dioxus::prelude::*;
use std::sync::LazyLock;

const SKIPPED: [&str; 5] = ["style", "script", "svg", "head", "template"];
/// Each starts a line of the text.
const BLOCKS: [&str; 23] = [
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

/// Text as HTML's text or an attribute's value: its quote or bracket never ends the attribute. Here, so this module and the
/// templates reading it lean on no crate root (dash-gate's no_crate_holds_a_module_cycle).
pub(crate) fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
use escaped as attr;

/// The three views as tabs, the one shown marked: Human the HTML, Agent the text (@nha, 2026-11-08: « the menu should have a
/// toggle human/agent »). `target=_top`: in the side-by-side view's frame it changes the whole tab.
fn toggle(path: &str, query: &str, current: &str) -> Element {
    let views = [("html", "Human"), ("text", "Agent"), ("data", "Data")].map(|(view, name)| {
        rsx! { Tab { href: href(path, query, view), current: view == current, target: "_top", "{name}" } }
    });
    rsx! { Tabs { id: "view-toggle", label: "View", {views.into_iter()} } }
}

/// The menu, Linear's sidebar (task rust-nav-board; @nha, 2026-11-08: « http://localhost:7788/ needs a menu »): each entry's
/// name, its URL, and the paths it stands for. A mail thread and the projects' boards are the older app's, reached through the
/// front door; /mail is the mail plugin's. Home is the « dash » above it, no entry (task rust-home-prompt); Inbox, Plan and
/// Insights hold what Home showed and what it lacked.
use crate::design::families as i;

/// The menu's order by use (task rust-menu-order; @nha, 2026-11-09: « mail today inbox need to be higher. The ordering of the menu
/// and the sub menu needs a rethink. »): Inbox, Today and Mail first; a page that belongs to another sits under it, its paths the
/// parent's too, so both are marked (« actually comments is just a sub-agent menu imo », « watchers is a sub-tool menu
/// perhaps », « design and code are something like "self building" - there may be a better word. extensions etc will go there
/// later. settings too maybe »). Each: name, URL, icon, the paths it stands for, its sub-entries (name, URL).
type Entry = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
);
const MENU: [Entry; 10] = [
    ("Inbox", "/inbox", i::INBOX, &["/inbox"], &[]),
    // the daily's lines, each opening its list (task rust-daily-today)
    ("Today", "/today", i::CALENDAR, &["/today"], &[]),
    ("Mail", "/mail", i::MAIL, &["/mail"], &[]),
    // his projects under it, from PROJECTS
    (
        "Tasks",
        "/tasks",
        i::CHECK,
        &["/tasks", "/graph", "/projects"],
        &[],
    ),
    ("Plan", "/plan", i::LIST, &["/plan"], &[]),
    ("Insights", "/insights", i::CHART, &["/insights"], &[]),
    // what runs and what it works on, then what waits for it (task rust-agents-comments-tabs)
    (
        "Agents",
        "/agents",
        i::AGENT,
        &["/agents", "/comments"],
        &[("Comments", "/comments")],
    ),
    (
        "Tools",
        "/tools",
        i::TOOL,
        &["/tools", "/watchers"],
        &[("Watchers", "/watchers")],
    ),
    // the tool building itself: its design system and its code map
    (
        "Workshop",
        "/design",
        i::PEN,
        &["/design", "/deps"],
        &[("Design", "/design"), ("Code", "/deps")],
    ),
    // the backup and its reimport (task dash-settings-backup)
    ("Settings", "/settings", i::SLIDERS, &["/settings"], &[]),
];
/// A sub-entry's text starts under its parent's label: NAV_CSS's `.nav-sub a{padding-left:32px}` is the entry's padding, its
/// icon and the gap after it (task rust-menu-sub-align; @nha, 2026-11-09: « let's start with alignement, since the logo added
/// now makes it slightle off »); `a_sub_entry_starts_under_its_parent_s_label` holds the three together.
/// His projects under Tasks, each Rust's board kept to it, `/tasks?project=<name>` (task rust-nav-polish, the orchestrator's
/// answer, 2026-11-08: « each sidebar project opens Rust's board filtered to it »). ponytail: Dashboard.TaskList.projects/0's
/// names copied, read from the older app once a project is added more than once a quarter.
const PROJECTS: [&str; 4] = ["site_perso", "klar", "graal", "geodb-whatsnr"];

/// The page's comments in Rust's queue, each with its answer (task rust-compass-queue; @nha, 2026-11-09: « it is just a queue
/// of comments imo that the agent should work through »): read from `GET /compass?page=<path>`, the list folded. The agent's
/// state is /agents' and /comments', never a line here (task rust-agents-comments-tabs).
fn notes(path: &str) -> Element {
    rsx! {
        div { id: "dash-notes", "data-page": "{path}",
            Fold { id: "dash-notes-fold", open: false, summary: rsx! { "Comments " span { id: "dash-notes-count" } },
                ol { id: "dash-notes-list" }
            }
        }
    }
}

/// Drawn with textContent alone: a comment's words are never markup. A page opened on a comment's anchor opens the list.
const NOTES_JS: &str = "try{const n=document.getElementById('dash-notes'),\
el=(t,cls,text)=>{const e=document.createElement(t);if(cls)e.className=cls;e.textContent=text;return e};\
fetch('/compass?page='+encodeURIComponent(n.dataset.page)).then(r=>r.json()).then(d=>{\
document.getElementById('dash-notes-count').textContent=d.notes.length?'('+d.notes.length+')':'';\
const ol=document.getElementById('dash-notes-list');for(const c of d.notes){const li=document.createElement('li');li.id='c-'+c.id;\
if(c.quote)li.append(el('q','',c.quote));li.append(el('p','said',c.text),el('p','answer '+c.state,c.answer));ol.append(li)}\
if(d.notes.length&&location.hash.startsWith('#c-'))n.querySelector('details').open=true}).catch(()=>{})}catch(e){}";

/// The answer to the compass's post, for a browser with no script: where the comment landed, and a link back to it.
pub fn compass_sent(page: &str, id: &str) -> String {
    let back = attr(&format!("{page}#c-{id}"));
    format!(
        r#"<!doctype html><meta charset="utf-8"><title>Comment sent</title><p>Landed as {}. <a href="{back}">Back to the page</a></p>"#,
        attr(id)
    )
}

/// The sidebar: the menu with the compass in its head, his projects under Tasks, the page's comments, the view toggle at its
/// foot. Collapsed on a wide screen and a drawer on a phone by one checkbox, so it needs no script; the script only
/// remembers a wide screen's fold.
fn sidebar(path: &str, query: &str) -> String {
    // a page under an entry is the entry's: /tasks/<id> and /projects/<name> mark Tasks, a mail thread Mail
    let under = |p: &str| {
        path == p
            || path
                .strip_prefix(p)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    let shown = |p: &str| {
        (path == "/tasks" && query.split('&').any(|q| q == format!("project={p}")))
            || under(&format!("/projects/{p}"))
    };
    let menu = MENU.iter().map(|&(name, url, d, paths, subs)| {
        rsx! {
            Tab { href: url, current: paths.iter().any(|p| under(p)), {crate::design::families::icon(d)} "{name}" }
            if name == "Tasks" {
                div { class: "nav-sub",
                    for p in PROJECTS {
                        Tab { href: "/tasks?project={p}", current: shown(p), "{p}" }
                    }
                }
            }
            if !subs.is_empty() {
                div { class: "nav-sub",
                    for (sub, at) in subs.iter() {
                        Tab { href: *at, current: under(at), "{sub}" }
                    }
                }
            }
        }
    });
    let aside = dioxus_ssr::render_element(rsx! {
        label { r#for: "dash-nav-fold", id: "dash-nav-open", title: "Menu", "\u{2630}" }
        aside { id: "dash-nav", aria_label: "Menu",
            div { class: "nav-head",
                a { href: "/", id: "dash-home", aria_current: (path == "/").then_some("page"), "dash" }
                {compass()}
                label { r#for: "dash-nav-fold", title: "Fold the menu", "\u{ab}" }
            }
            nav { id: "dash-menu", aria_label: "Pages", {menu} }
            {notes(path)}
            div { class: "nav-foot", {toggle(path, query, "html")} }
        }
    });
    format!(
        "{}{aside}{}<style>{NAV_CSS}{}</style><script>{NOTES_JS}</script><script>{NAV_JS}</script><script>{COMPASS_JS}</script>",
        MENU_PARTS.0, MENU_PARTS.1, *MENU_LOOKS,
    )
}

/// The menu's fold and comment box, drawn once a process: their markup never changes (/code-review).
static MENU_PARTS: LazyLock<(String, String)> = LazyLock::new(|| {
    let drawn = |part: fn() -> Element| dioxus_ssr::render_element(part());
    (drawn(fold), drawn(compose))
});

/// The components' button, textarea, tabs and fold looks, scoped to the menu, its comment box and the side-by-side view's
/// toggle: an older page loads no
/// components.css, and its daisyUI classes of the same names must stay its own (/code-review). Read off components.css, so
/// a look is written once.
static MENU_LOOKS: LazyLock<String> = LazyLock::new(|| {
    let scoped = |rule: &str| {
        let (selectors, body) = rule.split_once('{')?;
        let each = selectors.split(',').map(str::trim);
        let all: Vec<String> = each
            .flat_map(|s| ["#dash-nav", "#dash-compose"].map(|at| format!("{at} {s}")))
            .collect();
        Some(format!("{}{{{body}", all.join(",")))
    };
    crate::components::CSS
        .lines()
        .filter(|l| {
            // a class whole: `.folder` is no `.fold`
            [".button", ".textarea", ".tabs", ".fold"].iter().any(|c| {
                l.strip_prefix(c).is_some_and(|rest| {
                    !rest.starts_with(|ch: char| ch.is_alphanumeric() || ch == '-')
                })
            })
        })
        .filter_map(scoped)
        .collect()
});

/// The menu's fold: one checkbox, folded when wide and open when narrow (NAV_CSS), never shown itself.
fn fold() -> Element {
    rsx! { Input { r#type: "checkbox", id: "dash-nav-fold", hidden: true } }
}

/// The comment compass, as the older app's top bar carried it (`DashboardWeb.CommentThread.comment_page_button/1`; @nha,
/// 2026-11-09: « the menu is getting nice. It needs the compass still. »): a tap, then a tap on any element of the page, comments
/// on it. Its drawing is `DashboardWeb.Library.compass/1`'s; `data-pick-button` lets an older page's pick mark it pressed and
/// take a second tap as a cancel, as it did its own button. A Button of the design system (rust/CLAUDE.md rule 6).
fn compass() -> Element {
    rsx! {
        Button { look: "icon", r#type: "button", id: "dash-compass", "data-pick-button": "", aria_pressed: "false",
            title: "Comment on an element: tap it next", aria_label: "Comment on an element",
            svg { view_box: "0 0 24 24", width: "16", height: "16", fill: "none", stroke: "currentColor", stroke_width: "2",
                stroke_linecap: "round", stroke_linejoin: "round", "aria-hidden": "true",
                path { d: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z" }
                path { d: "M16 8l-2 6-6 2 2-6z" }
            }
        }
    }
}

/// The compass's comment box, which COMPASS_JS clones into its form: the quote, the words, Send and Cancel, each a component
/// (the compass shipped an unstyled input, rust/CLAUDE.md rule 6). A template draws nothing and holds no words of the page.
fn compose() -> Element {
    rsx! {
        template { id: "dash-compose-form",
            blockquote {}
            Textarea { name: "text", rows: "3", placeholder: "Your comment", required: true }
            div {
                Button { r#type: "submit", "Send" }
                " "
                Button { look: "quiet", r#type: "button", value: "cancel", "Cancel" }
                " "
                span { role: "status" }
            }
        }
    }
}

/// An older page holds its comment thread (`#thread`): the compass starts its own pick, the one it always had. A page of ours has
/// none: the next tap picks an element, its words go in a box at the foot, and Send posts them, with the page's path, to Rust's
/// comment queue (`POST /compass`, task rust-compass-queue), where the queue's agent answers it; the page then reloads, so the
/// sidebar's comments list it. Its `path` (where on the page) is anchored on a card's task, never its place (`row-3` is another
/// task tomorrow), else on an id, else on body. Any element takes it, the sidebar and its items included (task
/// rust-compass-any-element; @nha, 2026-11-09: « the comments should work on any element. It doesn't work on anything on the
/// menu »): only a tap on the box or the compass again stops the pick without a comment. An older page's pick never reaches
/// this sidebar (its shell is `#site-sidebar`), so a tap there is ours too and lands in Rust's queue; the rest of an older
/// page, the phone's menu button included, stays its own pick's. Escape stops it, and closes the box only while it holds
/// no words. The quote is the element's words (a list's chosen option, a text box's typed value), else what names it
/// (aria-label, title, alt, placeholder, a field's value), else those of an ancestor up to its own control (a link, button
/// or label) or, outside one, its parent, never a whole menu's name; else its tag (/code-review).
const COMPASS_JS: &str = r#"(()=>{const b=document.getElementById('dash-compass');if(!b)return;let box=null,older=false;
const stop=()=>{b.setAttribute('aria-pressed','false');document.documentElement.style.cursor='';document.removeEventListener('click',take,true)};
const page=document.body,root=document.documentElement;
const where=el=>{const p=[];for(let e=el;e&&e!==page&&e!==root;e=e.parentElement){
if(e.dataset.task){p.unshift('[data-task="'+CSS.escape(e.dataset.task)+'"]');return p.join(' > ')}
if(e.id){p.unshift('#'+CSS.escape(e.id));return p.join(' > ')}
p.unshift(e.localName+':nth-child('+([...e.parentElement.children].indexOf(e)+1)+')')}return['body',...p].join(' > ')};
const close=()=>{box&&box.remove();box=null};
const text=n=>(n.localName==='select'?n.selectedOptions[0]?.text:n.localName==='textarea'?n.value:n.innerText||n.textContent||'').trim().replace(/\s+/g,' ').slice(0,500);
const named=n=>n.getAttribute('aria-label')||n.getAttribute('title')||n.getAttribute('alt')||n.getAttribute('placeholder')||(/^(text|search|email|url|tel|number)$/.test(n.type)&&n.value)||'';
const quoted=el=>{const top=el.closest('a,button,label,[role=button]')||el.parentElement||el;
for(let n=el;n&&n!==top.parentElement;n=n.parentElement){const t=text(n)||named(n);if(t)return t}return el.localName};
const compose=el=>{close();const whole=el===page||el===root;
const words=quoted(el);
const quote=whole?'':words,path=whole?'':where(el);
box=document.createElement('form');box.id='dash-compose';box.append(document.getElementById('dash-compose-form').content.cloneNode(true));
box.querySelector('blockquote').textContent=whole?'The whole page':quote;document.body.append(box);const t=box.querySelector('textarea'),said=box.querySelector('[role=status]');t.focus();
box.querySelector('[value=cancel]').onclick=close;
t.addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.shiftKey&&!e.isComposing){e.preventDefault();box.requestSubmit()}});
box.onsubmit=e=>{e.preventDefault();if(!t.value.trim())return;said.textContent='Sending';
fetch('/compass',{method:'POST',body:new URLSearchParams({page:location.pathname,quote,path,text:t.value})})
.then(async r=>{if(r.ok){said.textContent='Sent';setTimeout(()=>location.reload(),600)}
else said.textContent='Not sent: '+await r.text()})
.catch(e=>{said.textContent='Not sent: '+e})}};
const take=e=>{const el=e.target;
if(older&&!el.closest('#dash-nav')){stop();return}
e.preventDefault();e.stopPropagation();stop();
if(el.closest('#dash-compass,#dash-compose'))return;compose(el)};
b.addEventListener('click',()=>{const thread=document.getElementById('thread');older=!!thread;
if(thread)thread.dispatchEvent(new CustomEvent('comments:pick',{bubbles:true}));
b.setAttribute('aria-pressed','true');root.style.cursor='crosshair';
setTimeout(()=>document.addEventListener('click',take,true));});
document.addEventListener('keydown',e=>{if(e.key!=='Escape')return;stop();if(box&&!box.querySelector('textarea').value.trim())close()});})()"#;

/// `#dash-nav-fold` is hidden by rule as well as by its attribute: an older page's daisyUI `.input` sets a display, which wins
/// over `[hidden]`. Every id and rule starts `dash-`: the older app's pages wear the same sidebar, and its own layout has an `id="menu"`.
/// Wide: a fixed column, folded by the checkbox. Narrow (a phone): off screen until the checkbox opens it over the page. The
/// button that opens it sits bottom left, clear of a page's own header and its menu button, and the column over an older
/// page's own drawer (z-index 50 there).
const NAV_CSS: &str = "html{padding-left:220px}html:has(#dash-nav-fold:checked){padding-left:0}\
#dash-nav{position:fixed;top:0;bottom:0;left:0;width:220px;box-sizing:border-box;z-index:60;display:flex;flex-direction:column;gap:8px;padding:12px 10px;background:var(--color-base-100,#fafafa);color:var(--color-base-content,#222);border-right:1px solid #8883;font:13px Inter,system-ui,sans-serif}\
#dash-nav-fold:checked~#dash-nav{display:none}\
#dash-nav .nav-head{display:flex;justify-content:space-between;align-items:center;padding:2px 8px}\
#dash-home{color:inherit;text-decoration:none;font-weight:700}#dash-home:hover{text-decoration:underline}\
#dash-nav label,#dash-nav-open{cursor:pointer;user-select:none}\
#dash-nav-fold{display:none!important}\
#dash-compass{margin-left:auto;margin-right:6px}\
#dash-compose{position:fixed;right:16px;bottom:16px;z-index:62;width:min(420px,calc(100vw - 32px));box-sizing:border-box;display:flex;flex-direction:column;gap:6px;padding:10px;border-radius:8px;background:var(--color-base-100,#fff);color:var(--color-base-content,#222);border:1px solid #8885;box-shadow:0 8px 24px #0004;font:13px Inter,system-ui,sans-serif}\
#dash-compose blockquote{margin:0;padding-left:8px;border-left:3px solid #8886;max-height:5em;overflow:hidden;opacity:.8}\
#dash-served{padding:16px;text-align:center;font:12px system-ui,sans-serif;opacity:.6}\
#dash-nav-open{display:none;position:fixed;top:12px;left:12px;z-index:61;padding:2px 9px;border-radius:6px;background:var(--color-base-100,#fafafa);border:1px solid #8883;font:16px system-ui,sans-serif}\
#dash-nav-fold:checked~#dash-nav-open{display:block}\
#dash-menu{display:flex;flex-direction:column;gap:1px;flex:1;overflow-y:auto}\
#dash-menu a{display:flex;align-items:center;gap:8px;color:inherit;text-decoration:none;padding:5px 8px;border-radius:6px}\
#dash-menu a:hover{background:#8882}#dash-menu a[aria-current]{background:#8883;font-weight:600}\
#dash-menu .nav-sub{display:flex;flex-direction:column}#dash-menu .nav-sub a{padding-left:32px;opacity:.8}\
#dash-notes{max-height:45vh;overflow-y:auto;padding:0 8px}#dash-notes summary{cursor:pointer;font-weight:600}\
#dash-notes ol{margin:4px 0;padding-left:16px}#dash-notes li{margin-bottom:6px}#dash-notes p{margin:2px 0}\
#dash-notes q{display:block;opacity:.7;font-style:italic}#dash-notes .answer{opacity:.8}#dash-notes .answer.waiting{font-style:italic}\
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
    dressed(html, sidebar(path, query), RUST)
}

/// Who served a page, in its footer (task rust-board-linear; @nha, 2026-11-09: « /mail => very nice. hard to say which part is
/// elixir and which is rust though. »).
pub const RUST: &str = "Rust (dash serve)";
pub const OLDER: &str = "the older app (Elixir)";

/// An older page's own navigation goes whole, the sidebar the one menu (task rust-nav-polish: « An older-app page behind
/// the front door shows the sidebar only: its own top nav hidden »; the PM check, 2026-11-08: a second bar of older-app
/// controls is no « one menu »): the plain layout's header, the bell and the comment button with it, and the full
/// layout's side column, its grid then one column. The page's comment rail at its right edge stays: commenting needs no bar.
const OLDER_CSS: &str = "<style>#site-header,#site-nav,#site-nav-backdrop{display:none!important}\
.linear{grid-template-columns:minmax(0,1fr)!important}</style>";

/// The page with `bar` first in its body and a footer naming who served it last, just before its `</body>`.
fn dressed(html: &str, bar: String, served: &str) -> String {
    let footer = format!(r#"<footer id="dash-served">Served by {served}</footer>"#);
    let html = match html.to_ascii_lowercase().rfind("</body") {
        Some(at) => format!("{}{footer}{}", &html[..at], &html[at..]),
        None => format!("{html}{footer}"),
    };
    match body_open(&html) {
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
            dressed(body, sidebar(path, query) + OLDER_CSS, OLDER),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Task rust-menu-icons (@nha, 2026-11-09: « logo for each tab please. like the elixir / linear had »): every menu entry
    /// draws its icon from the design system's set, then its name; the text form keeps the names alone.
    #[test]
    fn each_menu_entry_draws_its_icon_then_its_name() {
        let _budget = dash_meter::memory::TestBudget::new();
        let page = with_menu(
            "<!doctype html><html><head></head><body><p>x</p></body></html>",
            "/inbox",
            "",
        );
        let menu = &page[page.find(r#"id="dash-menu""#).expect("the menu")..];
        let menu = &menu[..menu.find("</nav>").expect("its end")];
        for (name, url, d, _, _) in MENU {
            let entry = &menu[menu.find(&format!(r#"href="{url}""#)).expect(name)..];
            let entry = &entry[..entry.find("</a>").expect(name)];
            assert!(
                entry.contains(&format!(r#"d="{d}""#)) && entry.ends_with(name),
                "{name}: {entry}"
            );
        }
        assert!(text_of(&page).contains("Inbox"), "{}", text_of(&page));
    }

    /// Task menu-fold-open-top (@nha, 2026-11-09: « when collapsing the menu is hidden at the bottom. »): a folded menu reopens
    /// from the top left, where it was.
    #[test]
    fn a_folded_menu_reopens_from_the_top_left() {
        let _budget = TestBudget::new();
        // its own rule, the one that places it, not a selector list it shares
        let rule = NAV_CSS
            .split("#dash-nav-open{display:none;")
            .nth(1)
            .and_then(|r| r.split('}').next())
            .expect("its rule");
        assert!(
            rule.contains("top:12px") && rule.contains("left:12px"),
            "{rule}"
        );
        assert!(!rule.contains("bottom:"), "{rule}");
    }

    /// Task rust-menu-sub-align: a sub-entry's text starts where its parent's label does, the entry's padding, its icon's width
    /// and the gap after it; the icon is the families' 16 px one.
    #[test]
    fn a_sub_entry_starts_under_its_parent_s_label() {
        let _budget = TestBudget::new();
        const MENU_PAD_PX: usize = 8;
        const MENU_ICON_PX: usize = 16;
        const MENU_GAP_PX: usize = 8;
        let indent = MENU_PAD_PX + MENU_ICON_PX + MENU_GAP_PX;
        assert!(
            NAV_CSS.contains(&format!(".nav-sub a{{padding-left:{indent}px")),
            "{NAV_CSS}"
        );
        assert!(NAV_CSS.contains(&format!(
            "gap:{MENU_GAP_PX}px;color:inherit;text-decoration:none;padding:5px {MENU_PAD_PX}px"
        )));
        let icon = dioxus_ssr::render_element(crate::design::families::icon(
            crate::design::families::INBOX,
        ));
        assert!(
            icon.contains(&format!(r#"width="{MENU_ICON_PX}""#)),
            "{icon}"
        );
    }

    /// Task rust-menu-order and rust-watchers-under-tools: Inbox, Today and Mail first; Comments under Agents, Watchers under
    /// Tools, Design and Code under Workshop, none of them top level; a sub-page marks its entry and its parent.
    #[test]
    fn the_menu_opens_on_inbox_today_mail_and_sub_pages_sit_under_their_parent() {
        let _budget = TestBudget::new();
        let names: Vec<&str> = MENU.iter().map(|e| e.0).collect();
        assert_eq!(names[..3], ["Inbox", "Today", "Mail"]);
        for sub in ["Comments", "Watchers", "Design", "Code"] {
            assert!(!names.contains(&sub), "{sub} is no top entry");
        }
        let page = with_menu(
            "<!doctype html><html><head></head><body></body></html>",
            "/watchers",
            "",
        );
        let menu = &page[page.find(r#"id="dash-menu""#).expect("the menu")..];
        let tools = menu
            .find(r#"href="/tools" aria-current="page""#)
            .expect(menu);
        let sub = menu.find(r#"<div class="nav-sub"><a href="/watchers" aria-current="page">Watchers</a></div>"#).expect(menu);
        assert!(tools < sub, "{menu}");
        let page = with_menu(
            "<!doctype html><html><head></head><body></body></html>",
            "/comments",
            "",
        );
        assert!(
            page.contains(r#"href="/agents" aria-current="page""#)
                && page.contains(r#"href="/comments" aria-current="page">Comments"#),
            "{page}"
        );
    }

    /// rust/CLAUDE.md rule 6 (the compass shipped an unstyled input): the menu's fold, compass and comment box are the
    /// components', and the box's template adds no word to the page's text.
    #[test]
    fn the_menu_s_controls_are_the_components() {
        let _budget = dash_meter::memory::TestBudget::new();
        let page = with_menu("<html><body><p>x</p></body></html>", "/", "");
        for part in [
            r#"<input class="input" hidden=true id="dash-nav-fold" type="checkbox"/>"#,
            r#"<button class="button icon" aria-label="Comment on an element" aria-pressed="false" data-pick-button="" id="dash-compass" title="Comment on an element: tap it next" type="button">"#,
            r#"<template id="dash-compose-form"><blockquote></blockquote><textarea class="textarea" name="text" placeholder="Your comment" required=true rows="3"></textarea><div><button class="button" type="submit">Send</button> <button class="button quiet" type="button" value="cancel">Cancel</button> <span role="status"></span></div></template>"#,
            "getElementById('dash-compose-form').content.cloneNode(true)",
        ] {
            assert!(page.contains(part), "{part} in {page}");
        }
        assert!(!text_of(&page).contains("Send"), "{}", text_of(&page));
        // an older page loads no components.css: the menu carries its controls' looks, scoped, and no other
        for look in [
            "#dash-nav .button,#dash-compose .button{",
            "#dash-nav .button.icon:hover,#dash-compose .button.icon:hover,",
            "#dash-nav .textarea,#dash-compose .textarea{",
            "#dash-nav .tabs a[aria-current],",
            "#dash-nav .fold > summary,",
        ] {
            assert!(page.contains(look), "{look} in {page}");
        }
        assert!(!page.contains("#dash-nav .card"), "{page}");
    }
    use dash_meter::memory::TestBudget;

    #[test]
    fn the_text_is_the_page_s_words_one_line_a_block() {
        let _budget = TestBudget::new();
        let html = r#"<!doctype html><html><head><title>t</title><style>p > a { x }</style></head><body><h1>Rust &#38; tasks</h1><!--x--><div class="a">one <b>two</b><span>three</span></div><div aria-hidden="true">one <i>again</i></div><div hidden=true>closed <b>menu</b></div><p hidden>shut</p><p class="card hidden" title='is hidden'>shown</p><p>after<br>break &lt;ok&gt; &bogus;</p><svg><text>drawn</text></svg><pre>  kept
  as is</pre></body></html>"#;
        assert_eq!(
            text_of(html),
            "Rust & tasks\none two three\nshown\nafter\nbreak <ok> &bogus;\n  kept\n  as is\n"
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
            href("/design", "tab=data&view=text", "data"),
            "/design?tab=data&view=data"
        );
        assert_eq!(href("/", "", "text"), "/?view=text");
        let page = with_menu("<html><body><p>x</p></body></html>", "/design", "tab=data");
        assert!(
            page.contains(r#"<a href="/design?tab=data&#38;view=text" target="_top">Agent</a>"#),
            "{page}"
        );
        assert!(page.contains(r#"<a href="/design?tab=data&#38;view=html" aria-current="page" target="_top">Human</a>"#), "{page}");
        assert!(
            page.starts_with(r#"<html><body><input class="input" hidden=true id="dash-nav-fold" type="checkbox"/>"#),
            "first in the body: {page}"
        );
        // done-when 3 of rust-board-linear: the footer names who served it, last in the body
        assert!(
            page.ends_with(r#"<p>x</p><footer id="dash-served">Served by Rust (dash serve)</footer></body></html>"#),
            "{page}"
        );
        // done-when 2 of rust-board-linear: the compass in the sidebar's head, and its pick that posts a comment
        assert!(
            page.contains(
                r#"<div class="nav-head"><a href="/" id="dash-home">dash</a><button class="button icon" aria-label="Comment on an element" aria-pressed="false" data-pick-button="" id="dash-compass""#
            ),
            "{page}"
        );
        for part in [
            "dispatchEvent(new CustomEvent('comments:pick'",
            // since rust-compass-queue, to Rust's comment queue
            "fetch('/compass',{method:'POST'",
            // anchored on a card's task, never its place on the board
            "if(e.dataset.task){p.unshift('[data-task=\"'",
        ] {
            assert!(COMPASS_JS.contains(part), "{part}");
        }
        assert!(page.contains(COMPASS_JS), "the page runs it");
        assert!(
            with_menu("<p>no body</p>", "/", "").ends_with(
                r#"<p>no body</p><footer id="dash-served">Served by Rust (dash serve)</footer>"#
            ),
            "a page with no body tag gets it last"
        );
    }

    /// Done-when 1 of rust-nav-board: one menu, Inbox to Code (Home the « dash » above it since rust-home-prompt, Agents and Comments since rust-agents-comments-tabs), the page's own entry marked, the toggle at its foot.
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
                "Inbox",
                "Today",
                "Mail",
                "Tasks",
                "site_perso",
                "klar",
                "graal",
                "geodb-whatsnr",
                "Plan",
                "Insights",
                "Agents",
                "Comments",
                "Tools",
                "Watchers",
                "Workshop",
                "Design",
                "Code",
                "Settings"
            ]
        );
        // done-when 4 of rust-home-prompt: the « dash » is the home link, marked on Home, which has no entry of its own
        assert!(
            task.contains(r#"<a href="/" id="dash-home">dash</a>"#),
            "{task}"
        );
        let home = with_menu("<body></body>", "/", "");
        assert!(
            home.contains(r#"<a href="/" id="dash-home" aria-current="page">dash</a>"#),
            "{home}"
        );
        let menu = &home[home.find(r#"id="dash-menu""#).expect("the menu")..];
        assert!(
            !menu[..menu.find("</nav>").expect("its end")].contains("aria-current"),
            "{home}"
        );
        // done-when 2 of rust-agents-comments-tabs: the sidebar links both pages and prints no agent's state
        let agents = with_menu("<body></body>", "/agents", "");
        assert!(
            agents.contains(r#"<a href="/agents" aria-current="page"><svg class="ic""#)
                && agents.contains("Agents</a>"),
            "{agents}"
        );
        assert!(
            !agents.contains("dash-notes-agent") && !NOTES_JS.contains("d.agent"),
            "no status line: {agents}"
        );
        assert!(
            task.contains(r#"<a href="/tasks" aria-current="page"><svg class="ic""#)
                && task.contains("Tasks</a>"),
            "{task}"
        );
        assert_eq!(
            task.matches(r#"aria-current="page""#).count(),
            2,
            "Tasks and Human: {task}"
        );
        let graph = with_menu("<body></body>", "/graph", "");
        assert!(
            graph.contains(r#"<a href="/tasks" aria-current="page"><svg class="ic""#)
                && graph.contains("Tasks</a>"),
            "/graph is the board's view: {graph}"
        );
        let code = with_menu("<body></body>", "/deps", "");
        // Code sits under Workshop (task rust-menu-order): both marked
        assert!(
            code.contains(r#"<a href="/design" aria-current="page"><svg class="ic""#)
                && code.contains(r#"<a href="/deps" aria-current="page">Code</a>"#),
            "{code}"
        );
        let foot = &code[code.find(r#"class="nav-foot""#).expect("its foot")..];
        assert!(
            foot.starts_with(r#"class="nav-foot"><nav class="tabs" id="view-toggle""#),
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

    /// Done-when 1, 2 and 3 of rust-compass-queue: every page's sidebar holds the compass, a Rust page's posting to Rust's queue
    /// with its path; the sidebar lists the page's comments with their answers and the queue's agent; the footer names what
    /// served the page.
    #[test]
    fn every_sidebar_has_the_compass_its_comments_and_a_footer_naming_what_served_it() {
        let _budget = TestBudget::new();
        let page = "<!doctype html><html><body><p>x</p></body></html>";
        let ok = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\nContent-Length: {}",
            page.len()
        );
        let older = older_page(&ok, page, "/mail/t/\"1", "", false).expect("dressed");
        let older = String::from_utf8(older).expect("utf-8");
        let ours = with_menu(page, "/tasks/a", "");
        for (html, path, served) in [(&ours, "/tasks/a", RUST), (&older, "/mail/t/&#34;1", OLDER)] {
            let aside =
                &html[html.find("<aside").expect("aside")..html.find("</aside>").expect("its end")];
            assert!(
                aside.contains(r#"id="dash-compass""#),
                "the compass sits in the sidebar"
            );
            assert!(
                aside.contains(&format!(
                    r#"<div id="dash-notes" data-page="{path}"><details class="fold" id="dash-notes-fold">"#
                )),
                "{aside}"
            );
            assert!(html.contains(COMPASS_JS) && html.contains(NOTES_JS));
            assert!(
                html.contains(&format!(
                    r#"<footer id="dash-served">Served by {served}</footer>"#
                )),
                "{html}"
            );
        }
        for part in [
            "fetch('/compass',{method:'POST'",
            "{page:location.pathname,quote,path,text:t.value}",
        ] {
            assert!(COMPASS_JS.contains(part), "{part}");
        }
        assert!(
            !COMPASS_JS.contains("fetch('/comment'"),
            "a Rust page's comment goes to Rust's queue"
        );
        assert!(
            NOTES_JS.contains("fetch('/compass?page='"),
            "it lists the page's comments"
        );
        assert!(
            !NOTES_JS.contains("innerHTML"),
            "a comment's words are never markup"
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
        assert!(body.starts_with(r#"<!doctype html><html><BODY class=x><input class="input" hidden=true id="dash-nav-fold" type="checkbox"/>"#), "{body}");
        assert!(
            body.contains(r#"<a href="/mail" aria-current="page"><svg class="ic""#)
                && body.contains("Mail</a>"),
            "a thread is Mail's: {body}"
        );
        assert!(
            body.contains(r#"href="/mail/t/1?view=text" target="_top">Agent</a>"#),
            "{body}"
        );
        assert!(
            body.ends_with(r#"<h1>Mail</h1><p>3 threads</p><footer id="dash-served">Served by the older app (Elixir)</footer></BODY></html>"#),
            "{body}"
        );
        // its compass too, which starts the older thread's own pick there
        assert!(
            body.contains(r#"id="dash-compass""#) && body.contains(COMPASS_JS),
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
            board.contains(r#"<a href="/tasks" aria-current="page"><svg class="ic""#)
                && board.contains("Tasks</a>"),
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
}
