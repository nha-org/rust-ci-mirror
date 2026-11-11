//! A page's three views at its one URL (rust/CLAUDE.md rule 4, task rust-design-page; @nha, 2026-11-09: « agent/human/code
//! where code is usually json »): `?view=html`, `?view=text`, `?view=data`, its tree as pretty JSON. The text is read off the
//! page's own HTML, never written twice (« make sure there is say html->text »). The menu links to none of them (task
//! rust-menu-no-view-toggle; @nha, 2026-11-10: « double menu, remove the bottom toggle »): each is its URL.

use crate::components::{
    Ask, BottomNav, BottomNavItem, BottomNavMore, Fold, Input, Tab, Toggle, ToggleOption, icon,
};
use dioxus::prelude::*;
use std::sync::LazyLock;

pub use crate::words::{every_word, text_of};

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

/// The menu, Linear's sidebar (task rust-nav-board; @nha, 2026-11-08: « http://localhost:7788/ needs a menu »): each entry's
/// name, its URL, and the paths it stands for. A mail thread and the projects' boards are the older app's, reached through the
/// front door; /mail is the mail plugin's. Home is the « dash » above it, no entry (task rust-home-prompt); Inbox, Plan and
/// Insights hold what Home showed and what it lacked.
use crate::design::families as i;

/// The menu's order by use (task rust-menu-order; @nha, 2026-11-09: « mail today inbox need to be higher. The ordering of the menu
/// and the sub menu needs a rethink. »): Inbox, Today and Mail first; a page that belongs to another sits under it, its paths the
/// parent's too, so both are marked (« actually comments is just a sub-agent menu imo », « watchers is a sub-tool menu
/// perhaps », « design and code are something like "self building" - there may be a better word. extensions etc will go there
/// later. settings too maybe »): that group's parent is Design, its own page, since no parent repeats a sub-entry's link (task
/// rust-menu-parent-own-page). Each: name, URL, icon, the paths it stands for, its sub-entries (name, URL).
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
    // the tool building itself: its design system, the parent's own page, and its code map under it; no sub-entry repeats its
    // parent's link (task rust-menu-parent-own-page; @nha, 2026-11-10: « does not lead anywhere »)
    (
        "Design",
        "/design",
        i::PEN,
        &["/design", "/deps"],
        &[("Code", "/deps")],
    ),
    // the backup and its reimport (task dash-settings-backup)
    ("Settings", "/settings", i::SLIDERS, &["/settings"], &[]),
];
/// How many of MENU's first entries a phone's BottomNav shows, More last for the rest (task rust-ui-bottom-nav; @nha,
/// 2026-11-10: « mobile - https://rust-ui.com/docs/components/bottom-nav »): four and More, the five slots a phone's bar holds.
const IN_BAR: usize = 4;

/// A sub-entry's text starts under its parent's label: NAV_CSS's `.nav-sub a{padding-left:32px}` is the entry's padding, its
/// icon and the gap after it (task rust-menu-sub-align; @nha, 2026-11-09: « let's start with alignement, since the logo added
/// now makes it slightle off »); `a_sub_entry_starts_under_its_parent_s_label` holds the three together.
/// His projects under Tasks, each Rust's board kept to it, `/tasks?project=<name>` (task rust-nav-polish, the orchestrator's
/// answer, 2026-11-08: « each sidebar project opens Rust's board filtered to it »). ponytail: Dashboard.TaskList.projects/0's
/// names copied, read from the older app once a project is added more than once a quarter.
const PROJECTS: [&str; 4] = ["site_perso", "klar", "graal", "geodb-whatsnr"];

/// An entry's sub-entries, name and URL: his projects' boards under Tasks, then its own. The sidebar and the menu's text both
/// read them here, so neither lists one the other lacks (/code-review of rust-menu-text).
fn subs_of(name: &str, subs: &[(&'static str, &'static str)]) -> Vec<(&'static str, String)> {
    let projects = PROJECTS.iter().filter(|_| name == "Tasks");
    let projects = projects.map(|&p| (p, format!("/tasks?project={p}")));
    projects
        .chain(subs.iter().map(|&(sub, at)| (sub, at.to_string())))
        .collect()
}

/// The menu as text, for an agent (task rust-menu-text; @nha, 2026-11-10: « can you see the menu, agent ? can you do dash
/// menu and see the menu ? everything should be dual-use »): the sidebar's links in its order, each name and URL a tab apart,
/// the « dash » home and the search field first, then MENU's entries, each one's sub-entries indented under it. `dash menu`
/// prints it, and every page's text form ends with it. Built once a process, as MENU_PARTS.
pub fn menu_text() -> &'static str {
    static TEXT: LazyLock<String> = LazyLock::new(|| {
        let mut text = String::from("Menu\ndash\t/\nSearch or ask\t/search?q=\n");
        for (name, url, _, _, subs) in MENU {
            text += &format!("{name}\t{url}\n");
            for (sub, at) in subs_of(name, subs) {
                text += &format!("  {sub}\t{at}\n");
            }
        }
        text
    });
    &TEXT
}

/// The page's comments in Rust's queue, each with its answer (task rust-compass-queue; @nha, 2026-11-09: « it is just a queue
/// of comments imo that the agent should work through »), the list folded. The agent's state is /agents' and /comments', never
/// a line here (task rust-agents-comments-tabs). As the older app's rail (task rust-comment-files-filter): Open and All, each
/// with its count, a link keeping the rest of the page's query (`comments=open`, else every comment shown, so an answer is never
/// out of sight), then the box that comments on the whole page, its images and PDFs attached.
fn notes(path: &str, query: &str, threads: &[crate::documents::Thread]) -> Element {
    // drawn by the server, open while the page holds any, each answer under its comment (task rust-comment-answer-on-page;
    // @nha, 2026-11-09: « answers are not visible in most pages. there is no UI element planned for comments. So answered just
    // means lost. »): the documents' thread component, as Notion's, never a list of its own; a post reloads the page
    let n = threads.len();
    let only_open = query.split('&').any(|p| p == "comments=open");
    let open = threads.iter().filter(|t| t.open).count();
    let kept: Vec<&str> = (query.split('&'))
        .filter(|p| !p.is_empty() && !p.starts_with("comments="))
        .collect();
    let to = |which: &str| {
        let mut q = kept.join("&");
        if !q.is_empty() {
            q.push('&');
        }
        format!("{path}?{q}comments={which}#dash-notes")
    };
    let shown: Vec<_> = (threads.iter())
        .filter(|t| t.open || !only_open)
        .cloned()
        .collect();
    let hidden = vec![
        ("page".to_string(), path.to_string()),
        ("quote".to_string(), String::new()),
    ];
    // open always: the box is where the first comment is written (/code-review)
    rsx! {
        div { id: "dash-notes", "data-page": "{path}",
            Fold { id: "dash-notes-fold", open: true, summary: rsx! { "Comments " span { id: "dash-notes-count", if n > 0 { "({n})" } } },
                Toggle { id: "dash-notes-filter", label: "Which comments",
                    ToggleOption { href: to("open"), current: only_open, icon: rsx! {}, "Open · {open}" }
                    ToggleOption { href: to("all"), current: !only_open, icon: rsx! {}, "All · {n}" }
                }
                {crate::documents::threads(&shown, false, true)}
                crate::documents::Composer { id: "dash-notes-box", action: "/compass", send: "Comment",
                    placeholder: "Your comment", hidden, attach: true }
            }
        }
    }
}

/// The sidebar: the menu with the compass in its head, his projects under Tasks, the page's comments. Collapsed on a wide
/// screen and a drawer on a phone by one checkbox, so it needs no script; the script only
/// remembers a wide screen's fold.
fn sidebar(path: &str, query: &str, threads: &[crate::documents::Thread]) -> String {
    // a page under an entry is the entry's: /tasks/<id> and /projects/<name> mark Tasks, a mail thread Mail
    let under = |p: &str| {
        path == p
            || path
                .strip_prefix(p)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    // a sub-entry with a query is marked on its path with that pair: a project's board, or the older app's /projects/<name>
    let current = |at: &str| match at.split_once('?') {
        Some((p, pair)) => {
            (path == p && query.split('&').any(|q| q == pair))
                || (pair.strip_prefix("project="))
                    .is_some_and(|name| under(&format!("/projects/{name}")))
        }
        None => under(at),
    };
    let menu = MENU.iter().map(|&(name, url, d, paths, subs)| {
        rsx! {
            Tab { href: url, current: paths.iter().any(|p| under(p)), {icon(d)} "{name}" }
            if !subs_of(name, subs).is_empty() {
                div { class: "nav-sub",
                    for (sub, at) in subs_of(name, subs) {
                        Tab { href: at.clone(), current: current(&at), "{sub}" }
                    }
                }
            }
        }
    });
    let aside = dioxus_ssr::render_element(rsx! {
        label { r#for: "dash-nav-fold", id: "dash-nav-open", title: "Menu", "\u{2630}" }
        aside { id: "dash-nav", aria_label: "Menu",
            // home and the fold on its first line, the Search or ask field beside the compass on its second (task
            // rust-notion-one-menu). Every page's search, as the older app's menu carried it: /search (task
            // rust-search-port), whose own field holds the words there: one field a page (/code-review)
            div { class: "nav-head",
                a { href: "/", id: "dash-home", aria_current: (path == "/").then_some("page"), "dash" }
                label { r#for: "dash-nav-fold", title: "Fold the menu", "\u{ab}" }
                if path != "/search" {
                    Ask { id: "dash-search" }
                }
                {crate::components::compass(Some("dash-compass"))}
            }
            nav { id: "dash-menu", aria_label: "Pages", {menu}
                if let Some(href) = reference_href(path, query) {
                    Tab { href, current: false, {icon(i::LINK)} "Elixir reference" }
                }
            }
            {notes(path, query, threads)}
        }
        // shown while the compass picks (NAV_CSS), over the page, the drawer it was tapped in closed or not
        div { id: "dash-pickbar", {crate::components::pick_bar()} }
        // a phone's menu (NAV_CSS): MENU's first entries, More opening the sidebar, which then shows the rest
        div { id: "dash-tabbar",
            BottomNav { id: "dash-bottom", label: "Main pages",
                for &(name, url, d, paths, _) in &MENU[..IN_BAR] {
                    BottomNavItem { href: url, current: paths.iter().any(|p| under(p)), icon: icon(d), "{name}" }
                }
                BottomNavMore { id: "dash-more", r#for: "dash-nav-fold", controls: "dash-nav",
                    current: MENU[IN_BAR..].iter().any(|e| e.3.iter().any(|p| under(p))),
                    icon: icon(i::DOTS), "More"
                }
            }
        }
    });
    format!(
        "{}{aside}{}<style>{NAV_CSS}{}{}</style><script>{NAV_JS}</script><script>{COMPASS_JS}</script><script>{}</script><script>{}</script>",
        MENU_PARTS.0,
        MENU_PARTS.1,
        *IN_BAR_HIDDEN,
        *MENU_LOOKS,
        crate::documents::EDITOR_JS,
        crate::components::DROPZONE_JS,
    )
}

/// The frozen Elixir app's port, set by `dash serve` while the older app exists (task rust-elixir-reference; @nha, 2026-11-10:
/// « I would like http://localhost:64151/mail to be on some port I can always compare like http://localhost:7799/mail »):
/// every page's menu links the same path there. Unset (the window, a test server), no link.
static REFERENCE: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

/// The menu links each page to its Elixir reference on `port` from now on.
pub fn link_reference(port: u16) {
    let _ = REFERENCE.set(port);
}

/// The page's path and query on the Elixir reference, when one is set. On localhost: the reference answers this Mac alone.
fn reference_href(path: &str, query: &str) -> Option<String> {
    let port = REFERENCE.get()?;
    let query = match query {
        "" => String::new(),
        q => format!("?{q}"),
    };
    Some(format!("http://localhost:{port}{path}{query}"))
}

/// On a phone the sidebar More opens holds what the BottomNav does not: a bar entry is hidden there, unless it heads
/// sub-entries (Tasks its projects), which keep their parent above them. Read off MENU and IN_BAR, so a reorder keeps both.
static IN_BAR_HIDDEN: LazyLock<String> = LazyLock::new(|| {
    let shown_below = MENU[..IN_BAR].iter().enumerate();
    let hidden: Vec<String> = shown_below
        .filter(|(_, e)| subs_of(e.0, e.4).is_empty())
        .map(|(n, _)| format!("#dash-menu>a:nth-of-type({})", n + 1))
        .collect();
    if hidden.is_empty() {
        return String::new();
    }
    format!(
        "@media (max-width:700px){{{}{{display:none}}}}",
        hidden.join(",")
    )
});

/// The menu's fold and comment box, drawn once a process: their markup never changes (/code-review).
static MENU_PARTS: LazyLock<(String, String)> = LazyLock::new(|| {
    let drawn = |part: fn() -> Element| dioxus_ssr::render_element(part());
    (drawn(fold), drawn(compose))
});

/// The looks of the components the menu draws (each registered with `menu`), scoped to the menu and its comment box: an older page loads no
/// components.css, and its daisyUI classes of the same names must stay its own (/code-review). Read off components.css, so
/// a look is written once.
static MENU_LOOKS: LazyLock<String> = LazyLock::new(|| {
    let scoped = |rule: &str| {
        let (selectors, body) = rule.split_once('{')?;
        let each = selectors.split(',').map(str::trim);
        // the phone's BottomNav sits apart from the column, in its own holder
        let holders: &[&str] = match rule.starts_with(".bottom-nav") {
            true => &["#dash-tabbar"],
            false => &["#dash-nav", "#dash-compose", "#dash-pickbar"],
        };
        let all: Vec<String> = each
            .flat_map(|s| holders.iter().map(move |at| format!("{at} {s}")))
            .collect();
        Some(format!("{}{{{body}", all.join(",")))
    };
    // the components' and the comment threads' (task rust-comment-answer-on-page), the rail drawing them
    (crate::component::css()
        .lines()
        .chain(crate::documents::CSS.lines()))
    .filter(|l| {
        // the comment threads' classes and the menu's own: no component registers them
        let own = [
            ".doc-threads",
            ".doc-thread",
            ".doc-quote",
            ".doc-said",
            ".doc-who",
            ".doc-words",
            ".doc-text",
            ".doc-meta",
            ".doc-compose",
            ".text-only",
            // the documents editor in the comment box (task compose-uses-doc-editor)
            ".editor",
            ".editor-tools",
            ".editor-field",
            ".editor-controls",
            ".dv",
            ".sr-only",
            // the compass's pick bar (task rust-comment-files-filter)
            ".pick-bar",
        ];
        // each component the menu or its box draws registers so (crate::component's `menu`): no list of them here
        let drawn = crate::component::registered()
            .iter()
            .filter(|c| c.menu)
            .flat_map(|c| c.classes.iter().map(|c| format!(".{c}")));
        // a class whole: `.folder` is no `.fold`
        own.iter().map(|c| c.to_string()).chain(drawn).any(|c| {
            l.strip_prefix(c.as_str())
                .is_some_and(|rest| !rest.starts_with(|ch: char| ch.is_alphanumeric() || ch == '-'))
        })
    })
    .filter_map(scoped)
    .collect()
});

/// The menu's fold: one checkbox, folded when wide and open when narrow (NAV_CSS), never shown itself.
fn fold() -> Element {
    rsx! { Input { r#type: "checkbox", id: "dash-nav-fold", hidden: true } }
}

/// The compass's comment box, which COMPASS_JS clones into its form: the quote, the documents editor, Cancel, each a component
/// (the compass shipped an unstyled input, rust/CLAUDE.md rule 6). A template draws nothing and holds no words of the page.
fn compose() -> Element {
    rsx! { template { id: "dash-compose-form", {crate::documents::comment_box("")} } }
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
const stop=()=>{b.setAttribute('aria-pressed','false');document.documentElement.style.cursor='';document.removeEventListener('click',take,true);unmark()};
const page=document.body,root=document.documentElement;
let hov=null;const mark=e=>{const el=e.target;if(el.closest('#dash-compass,#dash-compose,#dash-pickbar'))return;if(hov)hov.removeAttribute('data-dash-pick');hov=el;el.setAttribute('data-dash-pick','')};
const unmark=()=>{document.removeEventListener('mouseover',mark,true);if(hov)hov.removeAttribute('data-dash-pick');hov=null};
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
box.querySelector('blockquote').textContent=whole?'The whole page':quote;document.body.append(box);const t=box.querySelector('.editor-field .dv'),said=box.querySelector('[role=status]');t.focus();
box.querySelector('[value=cancel]').onclick=close;
box.onsubmit=e=>{e.preventDefault();const w=t.innerText.trim();if(!w)return;said.textContent='Sending';
fetch('/compass',{method:'POST',body:new URLSearchParams({page:location.pathname,quote,path,text:w})})
.then(async r=>{if(r.ok){said.textContent='Sent';setTimeout(()=>location.reload(),600)}
else said.textContent='Not sent: '+await r.text()})
.catch(e=>{said.textContent='Not sent: '+e})}};
const take=e=>{const el=e.target;if(el.closest('#dash-pickbar'))return;
if(older&&!el.closest('#dash-nav,#dash-tabbar')){stop();return}
e.preventDefault();e.stopPropagation();stop();
if(el.closest('#dash-compass,#dash-compose'))return;compose(el)};
b.addEventListener('click',()=>{const thread=document.getElementById('thread');older=!!thread;
if(thread)thread.dispatchEvent(new CustomEvent('comments:pick',{bubbles:true}));
b.setAttribute('aria-pressed','true');root.style.cursor='crosshair';document.addEventListener('mouseover',mark,true);
setTimeout(()=>document.addEventListener('click',take,true));});
const bar=document.getElementById('dash-pickbar');if(bar){bar.querySelector('[data-whole-page]').onclick=()=>{stop();compose(page)};
bar.querySelector('[data-pick-cancel]').onclick=stop}
document.addEventListener('keydown',e=>{if(e.key!=='Escape')return;stop();if(box&&!box.querySelector('.editor-field .dv').innerText.trim())close()});})()"#;

/// `#dash-nav-fold` is hidden by rule as well as by its attribute: an older page's daisyUI `.input` sets a display, which wins
/// over `[hidden]`. Every id and rule starts `dash-`: the older app's pages wear the same sidebar, and its own layout has an `id="menu"`.
/// Wide: a fixed column, folded by the checkbox. Narrow (a phone): the BottomNav at the screen's foot, the page padded clear
/// of it, its More the checkbox's label that opens the column over the page; the top left button is the wide fold's alone.
/// The column sits over an older page's own drawer (z-index 50 there), and ends above the bar, every slot in reach. The bar's
/// room is `--dash-bar`, 0 when wide: a page sized to the viewport subtracts it (design/base.css's `.app`).
const NAV_CSS: &str = "html{padding-left:220px}html:has(#dash-nav-fold:checked){padding-left:0}\
#dash-nav{position:fixed;top:0;bottom:0;left:0;width:220px;box-sizing:border-box;z-index:60;display:flex;flex-direction:column;gap:8px;padding:12px 10px;background:var(--color-base-100,#fafafa);color:var(--color-base-content,#222);border-right:1px solid #8883;font:13px Inter,system-ui,sans-serif}\
#dash-nav-fold:checked~#dash-nav{display:none}\
#dash-nav .nav-head{display:grid;grid-template-columns:minmax(0,1fr) auto;align-items:center;gap:6px;padding:2px 8px}\
#dash-home{justify-self:start}#dash-nav .nav-head>label{justify-self:end}\
#dash-home{color:inherit;text-decoration:none;font-weight:700}#dash-home:hover{text-decoration:underline}\
#dash-nav label,#dash-nav-open{cursor:pointer;user-select:none}\
#dash-nav-fold{display:none!important}\
#dash-compass{grid-column:2}\
#dash-search{margin:0;min-width:0}\
[data-dash-pick]{outline:2px dashed var(--color-primary,#2563eb);outline-offset:2px}\
#dash-compose{position:fixed;right:16px;bottom:16px;z-index:62;width:min(420px,calc(100vw - 32px));border-radius:var(--radius-box,8px);color:var(--color-base-content,#222);box-shadow:0 8px 24px #0004;font:13px Inter,system-ui,sans-serif}\
#dash-pickbar{display:none;position:fixed;left:50%;bottom:16px;transform:translateX(-50%);z-index:63;max-width:calc(100vw - 32px);color:var(--color-base-content,#222);font:13px Inter,system-ui,sans-serif}\
html:not(:has(#thread)):has(#dash-compass[aria-pressed=true]) #dash-pickbar{display:block}\
#dash-served{padding:16px;text-align:center;font:12px system-ui,sans-serif;opacity:.6}\
#dash-nav-open{display:none;position:fixed;top:12px;left:12px;z-index:61;padding:2px 9px;border-radius:6px;background:var(--color-base-100,#fafafa);border:1px solid #8883;font:16px system-ui,sans-serif}\
#dash-nav-fold:checked~#dash-nav-open{display:block}#dash-tabbar{display:none;font:13px Inter,system-ui,sans-serif}\
html{--dash-bar:0px}\
#dash-menu{display:flex;flex-direction:column;gap:1px;flex:1;overflow-y:auto}\
#dash-menu a{display:flex;align-items:center;gap:8px;color:inherit;text-decoration:none;padding:5px 8px;border-radius:6px}\
#dash-menu a:hover{background:#8882}#dash-menu a[aria-current]{background:#8883;font-weight:600}\
#dash-menu .nav-sub{display:flex;flex-direction:column}#dash-menu .nav-sub a{padding-left:32px;opacity:.8}\
#dash-notes{max-height:45vh;overflow-y:auto;padding:0 8px}#dash-notes summary{cursor:pointer;font-weight:600}\
#dash-notes ol{margin:4px 0;padding-left:16px}#dash-notes li{margin-bottom:6px}#dash-notes p{margin:2px 0}\
#dash-notes q{display:block;opacity:.7;font-style:italic}#dash-notes .answer{opacity:.8}#dash-notes .answer.waiting{font-style:italic}\
@media (max-width:700px){html{--dash-bar:calc(var(--bottom-nav-height,56px) + 1px + env(safe-area-inset-bottom,0px))}\
html,html:has(#dash-nav-fold:checked){padding-left:0;padding-bottom:var(--dash-bar)}\
#dash-nav{display:none;bottom:var(--dash-bar);box-shadow:0 0 24px #0005}#dash-nav-open,#dash-nav-fold:checked~#dash-nav-open{display:none}\
#dash-nav-fold:checked~#dash-nav{display:flex}\
#dash-tabbar{display:block;position:fixed;left:0;right:0;bottom:0;z-index:59}\
#dash-compose,#dash-pickbar{bottom:calc(var(--dash-bar) + 16px)}}";
/// A wide screen's fold is kept across pages; a phone's drawer opens closed on each. The checkbox means folded when wide and
/// open when narrow, so crossing the width sets it afresh: a wide fold never becomes an open drawer (/code-review).
/// ⌘ K (Ctrl K off a Mac: there Ctrl K stays a text field's kill to the line's end) focuses the menu's Search or ask, the
/// menu opened if folded (for this page only: the kept fold is his click's), else the page's first search field, /search's
/// own (task rust-notion-one-menu).
const NAV_JS: &str = "try{const f=document.getElementById('dash-nav-fold'),w=matchMedia('(min-width:701px)');\
const kept=()=>{try{return w.matches&&localStorage.getItem('dash-nav-folded')==='1'}catch(e){return false}};\
f.checked=kept();w.addEventListener('change',()=>{f.checked=kept()});\
document.addEventListener('keydown',e=>{if(!(/Mac|iP/.test(navigator.platform)?e.metaKey:e.ctrlKey)||e.altKey||e.shiftKey||e.key.toLowerCase()!=='k')return;\
const q=document.querySelector('#dash-search input')||document.querySelector('form[role=search] input[name=q]');if(!q)return;e.preventDefault();\
if(q.closest('#dash-nav'))f.checked=!w.matches;q.focus();q.select()});\
f.addEventListener('change',()=>{try{if(w.matches)localStorage.setItem('dash-nav-folded',f.checked?'1':'0')}catch(e){}});\
const m=document.getElementById('dash-more');if(m){const shown=()=>m.setAttribute('aria-expanded',!w.matches&&f.checked);\
shown();f.addEventListener('change',shown);w.addEventListener('change',shown);\
m.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();if(!e.repeat)f.click()}})}}catch(e){}";

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
    with_menu_and(html, path, query, &[])
}

/// `with_menu`, the page's comments in its rail, each with its answer (task rust-comment-answer-on-page).
pub fn with_menu_and(
    html: &str,
    path: &str,
    query: &str,
    threads: &[crate::documents::Thread],
) -> String {
    dressed(html, sidebar(path, query, threads), RUST)
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
        (
            "text/plain; charset=utf-8",
            text_of(body) + "\n" + menu_text(),
        )
    } else {
        body_open(body)?;
        (
            "text/html; charset=utf-8",
            dressed(body, sidebar(path, query, &[]) + OLDER_CSS, OLDER),
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

    /// Done-when 3 of rust-elixir-reference: with the reference set, as `dash serve --older` sets it, every page's menu links
    /// its own path and query on :7799; unset, none does.
    #[test]
    fn every_page_menu_links_its_elixir_reference_once_set() {
        let _budget = TestBudget::new();
        let page = "<html><body></body></html>";
        assert!(!with_menu(page, "/mail", "").contains("Elixir reference"));
        link_reference(7799);
        for (path, query, href) in [
            ("/mail", "", "http://localhost:7799/mail"),
            (
                "/tasks",
                "project=klar",
                "http://localhost:7799/tasks?project=klar",
            ),
            ("/", "", "http://localhost:7799/"),
        ] {
            let html = with_menu(page, path, query);
            let menu = &html[html.find(r#"id="dash-menu""#).expect("the menu")..];
            let menu = &menu[..menu.find("</nav>").expect("its end")];
            assert!(
                menu.contains(&format!(r#"href="{href}""#)),
                "{path}: {menu}"
            );
            assert!(menu.contains("Elixir reference"), "{path}");
        }
    }

    /// Task rust-menu-text: every MENU entry, its URL and each sub-entry, his projects included, stand in the menu's text and
    /// in the sidebar's HTML; an older page's text ends with that menu too.
    #[test]
    fn every_menu_entry_and_sub_entry_is_in_the_text_and_the_html() {
        let _budget = TestBudget::new();
        let text = menu_text();
        let html = with_menu("<html><body></body></html>", "/", "");
        assert!(
            text.starts_with("Menu\ndash\t/\nSearch or ask\t/search?q=\n"),
            "{text}"
        );
        assert!(
            html.contains(r#"<a href="/" id="dash-home""#) && html.contains(r#"action="/search""#),
            "{html}"
        );
        for (name, url, _, _, subs) in MENU {
            let line = format!("\n{name}\t{url}\n");
            let at_line = text
                .find(&line)
                .unwrap_or_else(|| panic!("{name} in {text}"));
            let under = &text[at_line + line.len()..];
            assert!(
                html.contains(&format!(r#"href="{url}""#)),
                "{name} in {html}"
            );
            for (sub, at) in subs_of(name, subs) {
                let mut own = under.split('\n').take_while(|l| l.starts_with("  "));
                assert!(
                    own.any(|l| l == format!("  {sub}\t{at}")),
                    "{sub} under {name}: {text}"
                );
                assert!(
                    html.contains(&format!(r#"href="{at}">{sub}</a>"#)),
                    "{sub} in {html}"
                );
            }
        }
        assert_eq!(
            text.lines().count(),
            3 + MENU.len() + PROJECTS.len() + MENU.iter().map(|e| e.4.len()).sum::<usize>(),
            "{text}"
        );
    }

    /// Task compass-hover-outline (@nha, 2026-11-09: « the compass itself seems like it does not hoover and highlight zones
    /// like in the previous version. That was very good. »): while picking, the element under the pointer is marked and a dashed
    /// outline drawn; the mark goes on pick, cancel or Escape, all of which stop.
    #[test]
    fn while_picking_the_hovered_element_is_outlined_and_the_mark_clears_on_stop() {
        let _budget = TestBudget::new();
        for part in [
            "document.addEventListener('mouseover',mark,true)",
            "el.setAttribute('data-dash-pick','')",
            "document.removeEventListener('mouseover',mark,true);if(hov)hov.removeAttribute('data-dash-pick')",
            "document.removeEventListener('click',take,true);unmark()",
        ] {
            assert!(COMPASS_JS.contains(part), "{part}");
        }
        assert!(
            NAV_CSS.contains("[data-dash-pick]{outline:2px dashed"),
            "{NAV_CSS}"
        );
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
    /// Tools, Code under Design, none of them top level; a sub-page marks its entry and its parent.
    #[test]
    fn the_menu_opens_on_inbox_today_mail_and_sub_pages_sit_under_their_parent() {
        let _budget = TestBudget::new();
        let names: Vec<&str> = MENU.iter().map(|e| e.0).collect();
        assert_eq!(names[..3], ["Inbox", "Today", "Mail"]);
        for sub in ["Comments", "Watchers", "Code"] {
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

    /// Task rust-menu-parent-own-page (@nha, 2026-11-10: « does not lead anywhere »): a parent that opens one of its own
    /// sub-entries' pages goes nowhere from that page, so every parent's URL is its own.
    #[test]
    fn no_menu_parent_opens_one_of_its_sub_entries_pages() {
        let _budget = TestBudget::new();
        for (name, url, _, _, subs) in MENU {
            for (sub, at) in subs {
                assert_ne!(url, *at, "{name} opens its sub-entry {sub}'s page");
            }
        }
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
            r#"<template id="dash-compose-form"><div class="doc-compose"><blockquote></blockquote><input class="input" name="text" type="hidden" value=""/><div class="editor">"#,
            r#"<div class="dv notion" contenteditable="plaintext-only" role="textbox" aria-multiline="true" title="Your comment"></div>"#,
            r#"<button class="button editor-send" title="Send (⌘Enter)">Send</button></div></div></div><div><button class="button quiet" type="button" value="cancel">Cancel</button> <span role="status"></span></div></div></template>"#,
            "getElementById('dash-compose-form').content.cloneNode(true)",
        ] {
            assert!(page.contains(part), "{part} in {page}");
        }
        assert!(!text_of(&page).contains("Send"), "{}", text_of(&page));
        // an older page loads no components.css: the menu carries its controls' looks, scoped, and no other
        for look in [
            "#dash-nav .button,#dash-compose .button,#dash-pickbar .button{",
            "#dash-nav .button.icon:hover,#dash-compose .button.icon:hover,",
            "#dash-nav .textarea,#dash-compose .textarea,#dash-pickbar .textarea{",
            "#dash-nav .editor-field,#dash-compose .editor-field,#dash-pickbar .editor-field{",
            "#dash-nav .toolbar,#dash-compose .toolbar,#dash-pickbar .toolbar{",
            "#dash-nav .doc-compose,#dash-compose .doc-compose,#dash-pickbar .doc-compose{",
            // the rail's filter and its box's picker, the pick bar (task rust-comment-files-filter)
            "#dash-nav .toggle,",
            "#dash-nav .dropzone,#dash-compose .dropzone,#dash-pickbar .dropzone{",
            "#dash-nav .dropzone-input,#dash-compose .dropzone-input,#dash-pickbar .dropzone-input{",
            "#dash-pickbar .pick-bar{",
            "#dash-nav .fold > summary,",
        ] {
            assert!(page.contains(look), "{look} in {page}");
        }
        assert!(
            !page.contains("#dash-nav .card") && !page.contains("#dash-nav .tabs"),
            "{page}"
        );
    }
    use dash_meter::memory::TestBudget;

    /// The page up to the phone's BottomNav: the column, whose marks these tests count; the bar marks its own.
    /// The menu's column, its comments' rail left out: the rail's filter marks its own choice.
    fn column(page: &str) -> &str {
        &page[..page.find(r#"id="dash-notes""#).expect("the rail")]
    }

    /// Done-when 2 of rust-ui-bottom-nav: the phone's BottomNav holds MENU's first IN_BAR entries, each its icon and name, then
    /// More, the fold's label; the page's entry is marked there too, More when the page is one of the rest; on a phone the
    /// column More opens hides the bar's entries, keeping one that heads sub-entries, and the top left button is gone.
    #[test]
    fn a_phone_s_bottom_nav_holds_the_first_entries_and_more_the_rest() {
        let _budget = TestBudget::new();
        let bar = |path: &str| {
            let page = with_menu("<body></body>", path, "");
            let at = page.find(r#"<div id="dash-tabbar">"#).expect("the bar");
            page[at..at + page[at..].find("</nav>").expect("its end")].to_string()
        };
        let inbox = bar("/inbox");
        for (n, (name, url, d, _, _)) in MENU.iter().enumerate() {
            let shown = inbox.contains(&format!(r#"href="{url}""#));
            assert_eq!(shown, n < IN_BAR, "{name}: {inbox}");
            if shown {
                assert!(
                    inbox.contains(&format!(r#"d="{d}""#))
                        && inbox.contains(&format!(">{name}</span>")),
                    "{name}: {inbox}"
                );
            }
        }
        assert!(
            inbox.contains(r#"<a class="bottom-nav-item" href="/inbox" aria-current="page">"#),
            "{inbox}"
        );
        assert!(inbox.contains(r#"<label class="bottom-nav-item" role="button" tabindex="0" id="dash-more" for="dash-nav-fold" aria-controls="dash-nav"><svg"#), "{inbox}");
        assert_eq!(text_of(&inbox), "Inbox\tToday\tMail\tTasks\tMore\n");
        assert!(
            inbox.contains(r#"aria-label="Main pages""#),
            "a landmark of its own: {inbox}"
        );
        assert_eq!(bar("/tasks/a-b").matches("aria-current").count(), 1);
        assert!(bar("/tasks/a-b").contains(r#"href="/tasks" aria-current="page""#));
        for rest in ["/design", "/deps", "/settings"] {
            assert!(
                bar(rest).contains(r#"id="dash-more" for="dash-nav-fold" aria-controls="dash-nav" aria-current="true""#),
                "{rest}: {}",
                bar(rest)
            );
            assert_eq!(bar(rest).matches("aria-current").count(), 1, "{rest}");
        }
        assert_eq!(
            *IN_BAR_HIDDEN,
            "@media (max-width:700px){#dash-menu>a:nth-of-type(1),#dash-menu>a:nth-of-type(2),#dash-menu>a:nth-of-type(3){display:none}}"
        );
        let page = with_menu("<body></body>", "/", "");
        assert!(
            page.contains(IN_BAR_HIDDEN.as_str())
                && page.contains("#dash-tabbar .bottom-nav-item{"),
            "{page}"
        );
        for rule in [
            "#dash-nav-open,#dash-nav-fold:checked~#dash-nav-open{display:none}",
            "#dash-tabbar{display:block;position:fixed;left:0;right:0;bottom:0;z-index:59}",
        ] {
            let phone = &NAV_CSS[NAV_CSS.find("@media (max-width:700px)").expect("a phone's")..];
            assert!(phone.contains(rule), "{rule}");
        }
    }

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
        // the menu carries no view links (task rust-menu-no-view-toggle)
        assert!(!page.contains("view="), "{page}");
        assert!(
            page.starts_with(r#"<html><body><input class="input" hidden=true id="dash-nav-fold" type="checkbox"/>"#),
            "first in the body: {page}"
        );
        // done-when 3 of rust-board-linear: the footer names who served it, last in the body
        assert!(
            page.ends_with(r#"<p>x</p><footer id="dash-served">Served by Rust (dash serve)</footer></body></html>"#),
            "{page}"
        );
        // done-when 2 of rust-board-linear: the compass in the sidebar's head, and its pick that posts a comment; done-when 2
        // of rust-search-port and of rust-notion-one-menu: beside it the one Search or ask field, the design system's Ask,
        // targeting /search
        assert!(
            page.contains(
                r#"<div class="nav-head"><a href="/" id="dash-home">dash</a><label for="dash-nav-fold" title="Fold the menu">«</label><form class="form" id="dash-search" role="search" method="get" action="/search"><div class="ask"><label><span class="sr-only">Search or ask</span><input class="input" name="q" placeholder="Search or ask" type="search"/></label><kbd class="kbd-group"><kbd class="kbd">⌘</kbd><kbd class="kbd">K</kbd></kbd></div></form><button class="button icon" aria-label="Comment on an element" aria-pressed="false" data-pick-button="" id="dash-compass""#
            ),
            "{page}"
        );
        assert_eq!(page.matches(r#"role="search""#).count(), 1, "one field");
        // ⌘ K focuses it, opening a folded menu first
        for part in [
            "/Mac|iP/.test(navigator.platform)?e.metaKey:e.ctrlKey",
            "document.querySelector('#dash-search input')||document.querySelector('form[role=search] input[name=q]')",
            "f.checked=!w.matches;q.focus()",
        ] {
            assert!(NAV_JS.contains(part), "{part}");
        }
        // its looks reach an older page, which loads no components.css
        for rule in ["#dash-nav .ask,", "#dash-nav .kbd,", "#dash-nav .sr-only,"] {
            assert!(MENU_LOOKS.contains(rule), "{rule} in {}", *MENU_LOOKS);
        }
        // /search holds its own field with the words: one field a page
        let search = with_menu("<html><body></body></html>", "/search", "q=x");
        assert!(!search.contains(r#"id="dash-search""#), "{search}");
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

    /// Done-when 1 of rust-nav-board: one menu, Inbox to Code (Home the « dash » above it since rust-home-prompt, Agents and Comments since rust-agents-comments-tabs), the page's own entry marked, no toggle at its foot.
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
            !agents.contains("dash-notes-agent"),
            "no status line: {agents}"
        );
        assert!(
            task.contains(r#"<a href="/tasks" aria-current="page"><svg class="ic""#)
                && task.contains("Tasks</a>"),
            "{task}"
        );
        assert_eq!(
            column(&task).matches(r#"aria-current="page""#).count(),
            1,
            "Tasks alone: {task}"
        );
        let graph = with_menu("<body></body>", "/graph", "");
        assert!(
            graph.contains(r#"<a href="/tasks" aria-current="page"><svg class="ic""#)
                && graph.contains("Tasks</a>"),
            "/graph is the board's view: {graph}"
        );
        // on /design the parent alone is marked, and no sub-entry links back to it (task rust-menu-parent-own-page)
        let design = with_menu("<body></body>", "/design", "");
        assert_eq!(
            column(&design).matches(r#"aria-current="page""#).count(),
            1,
            "{design}"
        );
        assert_eq!(design.matches(r#"href="/design""#).count(), 1, "{design}");
        let code = with_menu("<body></body>", "/deps", "");
        // Code sits under Design (task rust-menu-parent-own-page): both marked
        assert!(
            code.contains(r#"<a href="/design" aria-current="page"><svg class="ic""#)
                && code.contains(r#"<a href="/deps" aria-current="page">Code</a>"#),
            "{code}"
        );
        // No second row of tabs at the foot (task rust-menu-no-view-toggle; @nha, 2026-11-10: « double menu, remove the bottom toggle »)
        assert!(
            !code.contains("view-toggle") && !code.contains("nav-foot"),
            "{code}"
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
                    r#"<div id="dash-notes" data-page="{path}"><details class="fold" id="dash-notes-fold" open=true>"#
                )),
                "{aside}"
            );
            assert!(html.contains(COMPASS_JS) && html.contains(r#"id="dash-notes""#));
            assert!(
                html.contains(&format!(
                    r#"<footer id="dash-served">Served by {served}</footer>"#
                )),
                "{html}"
            );
        }
        for part in [
            "fetch('/compass',{method:'POST'",
            // the editor's words, read off its field (task compose-uses-doc-editor)
            "{page:location.pathname,quote,path,text:w}",
        ] {
            assert!(COMPASS_JS.contains(part), "{part}");
        }
        assert!(
            !COMPASS_JS.contains("fetch('/comment'"),
            "a Rust page's comment goes to Rust's queue"
        );
        // task rust-comment-answer-on-page: the rail is drawn by the server, each comment a thread at its anchor with its
        // answer, open while the page holds any; a comment's words are never markup
        use crate::documents::{Said, Thread};
        let said = |who: &str, words: &str| Said {
            who: who.into(),
            when: String::new(),
            words: words.into(),
        };
        let thread = Thread {
            id: "c-a".into(),
            quote: "the map".into(),
            said: vec![
                said("Nicolas", "why <b>here</b>?"),
                said("Comment agent", "Because the crate holds it."),
            ],
            open: false,
            files: vec![],
        };
        let page = with_menu_and("<body></body>", "/deps", "", &[thread]);
        let rail = &page[page.find(r#"id="dash-notes""#).expect("the rail")..];
        assert!(
            rail.contains(r#"<details class="fold" id="dash-notes-fold" open=true>"#),
            "{rail}"
        );
        assert!(
            rail.contains(r#"class="doc-thread" id="c-a""#)
                && rail.contains("Because the crate holds it."),
            "{rail}"
        );
        assert!(
            rail.contains("why &#60;b&#62;here&#60;/b&#62;?") && !rail.contains("<b>here"),
            "{rail}"
        );
        assert!(
            !page.contains("fetch('/compass?page='"),
            "no script fills it"
        );
    }

    /// Task rust-comment-files-filter, done-when 2: the rail filters Open and All, each with its count, every comment shown
    /// unless Open is asked; a comment names Whole page or its passage; its box attaches images and PDFs; the compass's pick
    /// bar takes the whole page or stops (the older app's rail, comment_thread.ex).
    #[test]
    fn the_rail_filters_open_and_all_and_each_comment_names_what_it_is_on() {
        let _budget = TestBudget::new();
        use crate::documents::{Said, Thread};
        let thread = |id: &str, quote: &str, open: bool| Thread {
            id: format!("c-{id}"),
            quote: quote.into(),
            said: vec![Said {
                who: "Nicolas".into(),
                when: String::new(),
                words: format!("said {id}"),
            }],
            open,
            files: vec![],
        };
        let threads = [thread("a", "", true), thread("b", "the code map", false)];
        let rail = |query: &str| {
            let page = with_menu_and("<body></body>", "/deps", query, &threads);
            let at = page.find(r#"id="dash-notes""#).expect("the rail");
            page[at..].to_string()
        };
        let all = rail("tab=x");
        // each a link keeping the page's own query, All shown when none is asked
        for (part, is) in [
            (
                r#"href="/deps?tab=x&#38;comments=open#dash-notes">Open · 1</a>"#,
                true,
            ),
            (
                r#"href="/deps?tab=x&#38;comments=all#dash-notes" aria-current="page">All · 2</a>"#,
                true,
            ),
            (r#"<div class="doc-quote">Whole page</div>"#, true),
            (r#"<div class="doc-quote">the code map</div>"#, true),
            ("said b", true),
        ] {
            assert_eq!(all.contains(part), is, "{part}: {all}");
        }
        let open = rail("comments=open&tab=x");
        assert!(
            open.contains(r#"href="/deps?tab=x&#38;comments=open#dash-notes" aria-current="page">Open · 1</a>"#),
            "{open}"
        );
        assert!(
            open.contains("said a") && !open.contains("said b"),
            "{open}"
        );
        // the box: a multipart form to the queue, on the whole page, the design system's drop zone for images and PDFs
        for part in [
            r#"action="/compass" enctype="multipart/form-data""#,
            r#"name="page" type="hidden" value="/deps""#,
            r#"<div class="dropzone"><label class="dropzone-area"><input class="dropzone-input" type="file" name="files" accept=".png,.jpg,.jpeg,.gif,.webp,.pdf" multiple=true/>"#,
            r#"<span class="dropzone-label">Attach images or PDFs: drop them here, or choose</span>"#,
            r#"title="Comment (⌘Enter)">Comment</button>"#,
        ] {
            assert!(all.contains(part), "{part}: {all}");
        }
        // the pick bar, outside the rail, shown while the compass picks; its buttons bound by the compass's script
        let page = with_menu_and("<body></body>", "/deps", "", &threads);
        let bar = &page[page.find(r#"id="dash-pickbar""#).expect("the pick bar")..];
        assert!(
            bar.contains("Tap the element to comment on")
                && bar.contains(r#"data-whole-page="" type="button">Whole page</button>"#)
                && bar.contains(r#"data-pick-cancel="" type="button">Cancel</button>"#),
            "{bar}"
        );
        assert!(NAV_CSS.contains("#dash-compass[aria-pressed=true]) #dash-pickbar{display:block}"));
        assert!(COMPASS_JS.contains("[data-whole-page]').onclick=()=>{stop();compose(page)}"));
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
        assert!(!body.contains("view-toggle"), "{body}");
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
                && words.ends_with(&format!("\r\n\r\nMail\n3 threads\n\n{}", menu_text())),
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
            column(&ours).matches(r#"aria-current="page""#).count(),
            2,
            "Tasks and graal: {ours}"
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
