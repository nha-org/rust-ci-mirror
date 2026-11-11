//! The components every page draws with, each one of dioxuslabs.com/components ported to design.json's tokens (@nha,
//! 2026-11-09: « our design page should have https://dioxuslabs.com/components or equivalents either use or port what we
//! need », « We want all components to be dual-use »). Dual-use: each is a Dioxus element, so the window draws it and the
//! web's server render is its HTML, whose text form (crate::forms::text_of) is what an agent reads. /design?tab=components
//! (design/catalog.rs) shows each one's two forms side by side. `look` picks a component's look: its CSS class, never written by a page.
//! The gate (dash-gate's `page_local_markup`) refuses OWNED_TAGS and OWNED_CLASSES anywhere but here.
use dioxus::prelude::*;
use std::collections::BTreeMap;

/// The components' looks, which every page and /design load, and the menu scopes (crate::forms).
pub const CSS: &str = include_str!("components.css");

/// The elements a component here draws: a page writes `Button {}`, never `button {}`.
pub const OWNED_TAGS: [&str; 6] = ["button", "input", "textarea", "table", "form", "kbd"];
/// The classes a component here draws: its own (`timeline`, `when`), or a look a page passes as `look`.
pub const OWNED_CLASSES: [&str; 96] = [
    "chart",
    "dropzone",
    "dropzone-area",
    "dropzone-input",
    "dropzone-text",
    "dropzone-label",
    "dropzone-file",
    "copies",
    "prop-menu",
    "prop-menu-list",
    "prop-menu-item",
    "bar",
    "dialog",
    "dialog-title",
    "dialog-actions",
    "ask",
    "bottom-nav",
    "bottom-nav-grid",
    "bottom-nav-item",
    "bottom-nav-label",
    "alert",
    "alert-title",
    "aspect-ratio",
    "aspect-ratio-content",
    "image",
    "attachment",
    "attachment-group",
    "attachment-media",
    "attachment-content",
    "attachment-title",
    "attachment-description",
    "treemap",
    "treemap-box",
    "timeline",
    "strip",
    "when",
    "switch",
    "toolbar",
    "toolbar-button",
    "toolbar-group",
    "toolbar-separator",
    "toolbar-title",
    "switch-input",
    "switch-track",
    "switch-thumb",
    "hover-card",
    "hover-card-content",
    "kbd",
    "kbd-group",
    "pagination-ellipsis",
    "pagination",
    "pagination-content",
    "pagination-link",
    "dropdown-menu",
    "dropdown-menu-content",
    "dropdown-menu-item",
    "toggle",
    "board",
    "form",
    "li-column",
    "li-column-head",
    "fold",
    "table",
    "frame",
    "textarea",
    "card",
    "li-card",
    "issue-row",
    "thread-row",
    "mail-shell",
    "mail-side",
    "mail-account",
    "mail-compose",
    "mail-folders",
    "mail-folder",
    "mail-main",
    "mail-search",
    "mail-bar",
    "mail-head",
    "description",
    "item",
    "access",
    "badge",
    "chip",
    "count",
    "li-tag",
    "avatar",
    "li-who",
    "button",
    "input",
    "tabs",
    "tool",
    "skeleton",
    "logo",
    "live",
    "diagram",
];

/// A component's own class, then its look's, if any.
fn joined(base: &str, look: &str) -> String {
    match look {
        "" => base.to_string(),
        look => format!("{base} {look}"),
    }
}

/// A card: a link when it has `href`. `task`: the task it shows, `data-task`, the comment box's anchor (crate::forms).
/// `draggable`: a board's card his drag moves to another column (Board).
/// `reading`: its loading state, the same card under the same id, greyed with a grey line where its title will be: the page's
/// LIVE_JS swaps in the filled card in place (task rust-today-doc; @nha, 2026-11-09: « This is a card loading state. same
/// element. »). `children` are what is known already, its description.
#[component]
pub fn Card(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default)] href: Option<String>,
    #[props(default, into)] task: Option<String>,
    #[props(default)] draggable: bool,
    #[props(default)] reading: bool,
    children: Element,
) -> Element {
    let class = joined("card", look);
    let draggable = draggable.then_some("true");
    if reading {
        return rsx! { Skeleton { id, what: "", lines: 0, look: class, CardTitle {} {children} } };
    }
    match href {
        Some(href) => rsx! { a { id, class, "data-task": task, draggable, href, {children} } },
        None => rsx! { div { id, class, "data-task": task, draggable, {children} } },
    }
}

/// A board's script, once a page: a card dragged to another column posts `/tasks/move` (`id`, its task; `column`, the
/// column's `data-status`) and sits there at once. Once kept, the board view's diff patches both columns, their counts with
/// them (LIVE_JS, task rust-view-patches). A refusal puts the card back where it was and says why on it. The
/// window runs no script: its drag is design/linear.rs's mouse events. ponytail: HTML5 drag events, so a phone's touch does not
/// drag; pointer events if he asks for it there.
pub(crate) const BOARD_JS: &str = r#"(()=>{if(window.dashBoard)return;window.dashBoard=1;let held=null;const on=(t,f)=>document.addEventListener(t,f);const col=e=>e.target.closest&&e.target.closest(".board .li-column");on("dragstart",e=>{held=e.target.closest&&e.target.closest(".board [draggable=true][data-task]");if(held)e.dataTransfer.setData("text/plain",held.dataset.task)});on("dragend",()=>{held=null});on("dragover",e=>{if(held&&col(e))e.preventDefault()});on("drop",e=>{const to=held&&col(e);if(!to)return;e.preventDefault();const card=held,from=card.closest(".li-column"),next=card.nextSibling;held=null;if(from===to)return;to.querySelector(".li-column-head").after(card);fetch("/tasks/move",{method:"POST",body:new URLSearchParams({id:card.dataset.task,column:to.dataset.status})}).then(r=>r.text().then(t=>{if(!r.ok)throw t})).catch(t=>{from.insertBefore(card,next);const m=card.querySelector(".meta");if(m)m.textContent="Not moved: "+t})})})()"#;

/// The tasks board: its columns side by side, as Linear's (task rust-board-linear-order), and the script that moves a card.
/// `view`: the views' name its cards follow, its `data-view`: the page's LIVE_JS takes each diff of that view as a patch of
/// the cards and columns it changes, never the whole page (task rust-view-patches).
#[component]
pub fn Board(
    #[props(into)] id: String,
    #[props(default)] view: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        div { id, class: "board", "data-view": view, {children} }
        script { dangerous_inner_html: BOARD_JS }
    }
}

/// A board column's tasks it does not draw: how many, and the page that lists them all (a path of this app). A column of
/// 1205 cards drew 1.4 MB, its render 1.6 s under load (task rust-page-latency, trace 1794314321744293823).
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct More {
    pub count: usize,
    pub href: String,
}

/// A board's column: its status icon and name, its count, then its cards. `status`: its `data-status`, the column a card
/// dropped on it moves to. `more`: the cards it does not draw, counted in `count`, and the link to them all, a path of this
/// app (task rust-page-latency).
#[component]
pub fn Column(
    #[props(into)] id: String,
    status: &'static str,
    #[props(into)] name: String,
    count: usize,
    icon: Element,
    #[props(default)] more: Option<More>,
    children: Element,
) -> Element {
    rsx! {
        section { id, class: "li-column", "data-status": status,
            h2 { class: "li-column-head", {icon} "{name}" Badge { look: "meta count", "{count}" } }
            {children}
            MoreLink { more }
        }
    }
}

/// The one link to a group's tasks it does not draw, « N more », a board column's and a list group's alike; none when its
/// href is not a path of this app.
#[component]
pub fn MoreLink(more: Option<More>) -> Element {
    match more.filter(|m| local_path(&m.href)) {
        Some(more) => rsx! { a { class: "li-more", href: more.href, "{more.count} more" } },
        None => rsx! {},
    }
}

impl More {
    /// Its link, when it is a path of this app: what both forms draw.
    pub(crate) fn link(&self) -> Option<&str> {
        local_path(&self.href).then_some(self.href.as_str())
    }
}

/// A board card's last line: what its run says, then its comments when it has any.
#[component]
pub fn CardLine(#[props(into)] says: String, comments: usize, said: Option<String>) -> Element {
    rsx! {
        span { class: "says", "{says}" }
        if let Some(said) = said {
            span { class: "comments", "data-comments": "{comments}", " · {said}" }
        }
    }
}

/// One row of a list: /tools' accesses, /tasks' rows (a link when it has `href`). `status` is `data-status`, which a look's
/// colours read (components.css); `task` is `data-task`, as Card's.
#[component]
pub fn Item(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default, into)] status: Option<String>,
    #[props(default)] href: Option<String>,
    #[props(default, into)] task: Option<String>,
    children: Element,
) -> Element {
    let class = joined("item", look);
    match href {
        Some(href) => {
            rsx! { a { id, class, "data-status": status, "data-task": task, href, {children} } }
        }
        None => rsx! { div { id, class, "data-status": status, "data-task": task, {children} } },
    }
}

/// One conversation of /mail's list, as the older app's /mail drew it (Gmail's): the sender, the subject then its snippet,
/// the date; unread bold on the page's white, read on its grey. The whole row opens the conversation (task
/// rust-mail-design-parts; @nha, 2026-11-09: « they do not match the previous app /mail »). `href`: a path of this app or
/// an http(s) URL, else the row opens nothing. `count`: its mails, shown past one; `starred`: its star filled (task
/// rust-mail-parity).
#[component]
pub fn ThreadRow(
    #[props(into)] id: String,
    #[props(into)] from: String,
    #[props(into)] subject: String,
    #[props(into)] snippet: String,
    #[props(into)] at: String,
    unread: bool,
    #[props(default)] href: Option<String>,
    #[props(default)] count: u64,
    #[props(default)] starred: bool,
) -> Element {
    let class = if unread {
        "thread-row unread"
    } else {
        "thread-row"
    };
    let href = card_href(&href).map(str::to_string);
    let star = if starred { "star on" } else { "star" };
    let said = if starred { "Starred" } else { "Not starred" };
    // its own class names: /design's base.css styles a bare .subject and .snippet (/code-review)
    rsx! {
        a { id, class, href,
            span { class: star, role: "img", "aria-label": said, {icon(STAR)} }
            span { class: "from", "{from}"
                if count > 1 { span { class: "n", " {count}" } }
            }
            span { class: "words",
                span { class: "topic", "{subject}" }
                if !snippet.is_empty() {
                    span { class: "gist", " - {snippet}" }
                }
            }
            span { class: "at", "{at}" }
        }
    }
}

/// One mail of a conversation on /mail/t/<thread> (task rust-mail-thread): folded to its line unless `open`, then its words,
/// or its HTML framed with no script and nothing remote; `files`, each attachment's name, where it downloads and its MIME
/// type, drawn as `Attachment`s, a picture as its `Image` (task rust-ui-media). `address`: the sender's, beside the name, under
/// the initials' avatar, as the older app's head showed it (task rust-mail-parity).
#[component]
pub fn MailFold(
    #[props(into)] id: String,
    #[props(into)] from: String,
    #[props(into)] to: String,
    #[props(into)] at: String,
    #[props(into)] text: String,
    #[props(default)] html: Option<String>,
    open: bool,
    #[props(default)] files: Vec<(String, String, String)>,
    #[props(default, into)] address: String,
) -> Element {
    // a folded mail's line says how it starts, as Gmail's does
    let start: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(90)
        .collect();
    let line = if open || start.is_empty() {
        String::new()
    } else {
        format!(" · {start}")
    };
    let address = (!address.is_empty() && address != from).then(|| format!(" <{address}>"));
    let head = rsx! {
        Avatar { initials: initials_of(&from) }
        b { "{from}" }
        if let Some(address) = address { span { class: "address", "{address}" } }
        " · {at}{line}"
    };
    let any = !files.is_empty();
    let files = files
        .into_iter()
        .enumerate()
        .map(|(i, (name, href, kind))| {
            let picture = is_picture(&kind);
            rsx! { Attachment { id: "{id}-file-{i}", name, kind, href, picture } }
        });
    rsx! {
        Fold { id: id.clone(), open, summary: rsx! { {head} },
            p { "To {to}" }
            if let Some(html) = html {
                Frame { title: "The mail from {from}", html }
            } else {
                pre { "{text}" }
            }
            if any {
                AttachmentGroup { {files} }
            }
        }
    }
}

/// A card's link when it is one a page may open: a path of this app or an http(s) URL, never a script.
pub(crate) fn card_href(href: &Option<String>) -> Option<&str> {
    let href = href.as_deref()?;
    // a browser reads « /\\host » as « //host »: no backslash in a link of ours (/code-review of rust-nav-polish)
    if href.contains('\\') {
        return None;
    }
    (local_path(href) || href.starts_with("https://") || href.starts_with("http://"))
        .then_some(href)
}

/// A path of this app, never another host's (« //host », « /\\host »): a link or a form's target.
pub(crate) fn local_path(path: &str) -> bool {
    path.starts_with('/') && !path.starts_with("//") && !path.contains('\\')
}

/// One row of a Table: its cells in its columns' order, and where it opens, if anywhere.
#[derive(Clone, PartialEq, serde::Deserialize)]
pub struct TableRow {
    pub cells: Vec<String>,
    #[serde(default)]
    pub href: Option<String>,
}

/// How a table is shown, as its URL says (task rust-table-sortable; @nha, 2026-11-09: « the component should be a table and
/// that should be sortable etc. design system first. »): `?filter=<words>` its rows holding them, case aside, `&sort=<column>`,
/// `&dir=desc` reversed, `&page=<n>` from 1 (task design-agent-forms; @nha, 2026-11-09: « Is there pagination for agents ?
/// filters ? »). One order a page: ponytail, every table of a page reads the same keys; a key per table id once a page holds
/// two long ones.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Order {
    pub filter: Option<String>,
    pub sort: Option<String>,
    pub desc: bool,
    pub page: usize,
}

/// The keys a table's order takes in the URL: every other key of the query is kept in its links.
const ORDER_KEYS: [&str; 4] = ["filter", "sort", "dir", "page"];

/// The rows a page shows at most before Pagination takes the rest.
pub const PER_PAGE: usize = 50;

/// A query's pairs, each value decoded as a form sends it: `+` a space, `%XX` its byte; a bad escape stays as it is.
fn pairs(query: &str) -> impl Iterator<Item = (&str, String)> {
    query.split('&').filter(|p| !p.is_empty()).map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let bytes = value.as_bytes();
        let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
        while i < bytes.len() {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok());
            match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
                (b'%', Some(byte)) => {
                    out.push(byte);
                    i += 3;
                }
                (b'+', _) => {
                    out.push(b' ');
                    i += 1;
                }
                (b, _) => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        (key, String::from_utf8_lossy(&out).into_owned())
    })
}

/// A value as a query holds it: every byte but a letter, a digit, `-`, `_`, `.` and `~` escaped.
fn encoded(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect()
}

impl Order {
    /// The order a query names; a page that is not a number, or 0, is the first.
    pub fn of(query: &str) -> Order {
        let mut order = Order {
            page: 1,
            ..Order::default()
        };
        for (key, value) in pairs(query) {
            match key {
                "filter" if !value.trim().is_empty() => order.filter = Some(value.trim().into()),
                "sort" if !value.is_empty() => order.sort = Some(value),
                "dir" => order.desc = value == "desc",
                "page" => order.page = value.trim().parse().ok().filter(|&p| p > 0).unwrap_or(1),
                _ => {}
            }
        }
        order
    }

    /// The link to this order: the query's other keys kept, in their order, then the order's own, a default left out.
    pub fn href(&self, query: &str) -> String {
        let mut kept: Vec<String> = query
            .split('&')
            .filter(|p| !p.is_empty() && !ORDER_KEYS.contains(&p.split('=').next().unwrap_or(p)))
            .map(String::from)
            .collect();
        if let Some(filter) = &self.filter {
            kept.push(format!("filter={}", encoded(filter)));
        }
        if let Some(sort) = &self.sort {
            kept.push(format!("sort={}", encoded(sort)));
            if self.desc {
                kept.push("dir=desc".into());
            }
        }
        if self.page > 1 {
            kept.push(format!("page={}", self.page));
        }
        format!("?{}", kept.join("&"))
    }

    /// Where a click on `column`'s head leads: sorted by it, the same column again reversed, back to the first page, the
    /// filter kept (rust-ui's DataGrid sortable head cell, github.com/rust-ui/ui, MIT, notice kept in NOTICE-rust-ui).
    fn by(&self, column: &str) -> Order {
        let again = self.sort.as_deref() == Some(column);
        Order {
            filter: self.filter.clone(),
            sort: Some(column.into()),
            desc: again && !self.desc,
            page: 1,
        }
    }
}

/// A cell as a number, when it is written as one: digits first, so « inf » and « NaN » stay words.
fn number(cell: &str) -> Option<f64> {
    let cell = cell.trim();
    let digit = cell
        .trim_start_matches(['-', '+', '.'])
        .starts_with(|c: char| c.is_ascii_digit());
    cell.parse().ok().filter(|n: &f64| digit && n.is_finite())
}

/// Two cells compared as their column holds them (rust-ui's DataGrid compares each column by its own type): numbers by
/// value and before any words, words by their letters, case aside. A total order, so a column of « 2 », « 10 » and « 1a »
/// sorts instead of panicking (/code-review). ponytail: « 10 KB » is words; units parsed once a column needs them.
fn compared(a: &str, b: &str) -> std::cmp::Ordering {
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

/// A table as its order shows it: the shown page's rows, that page and the count of pages, the rows its filter keeps and
/// all of them.
pub struct Arranged {
    pub shown: Vec<TableRow>,
    pub page: usize,
    pub pages: usize,
    pub matched: usize,
    pub total: usize,
}

/// The rows a table shows for `order`: those holding its filter, sorted by its column, if it has one (stable, so ties keep
/// the page's order), then its page's slice; the page kept within the last.
pub fn arranged(columns: &[String], rows: &[TableRow], order: &Order, per_page: usize) -> Arranged {
    let total = rows.len();
    let words = order.filter.as_ref().map(|f| f.to_lowercase());
    // references sorted, the shown page's rows alone cloned (/code-review)
    let mut rows: Vec<&TableRow> = (rows.iter())
        .filter(|r| {
            words
                .as_ref()
                .is_none_or(|w| r.cells.iter().any(|c| c.to_lowercase().contains(w)))
        })
        .collect();
    if let Some(j) = order
        .sort
        .as_ref()
        .and_then(|s| columns.iter().position(|c| c == s))
    {
        // reversed by its comparison, never by the list, so ties keep the page's order both ways (/code-review)
        rows.sort_by(|a, b| match order.desc {
            false => compared(&a.cells[j], &b.cells[j]),
            true => compared(&b.cells[j], &a.cells[j]),
        });
    }
    let (matched, per_page) = (rows.len(), per_page.max(1));
    let pages = matched.div_ceil(per_page).max(1);
    let page = order.page.clamp(1, pages);
    let shown = rows
        .into_iter()
        .skip((page - 1) * per_page)
        .take(per_page)
        .cloned()
        .collect();
    Arranged {
        shown,
        page,
        pages,
        matched,
        total,
    }
}

impl Arranged {
    /// What both forms say first of a table (its caption, the text's first line): its count, its filter, its sort and its
    /// page among several (task design-agent-forms).
    pub fn line(&self, columns: &[String], order: &Order) -> String {
        let rows = |n: usize| match n {
            1 => "1 row".to_string(),
            n => format!("{n} rows"),
        };
        let mut parts = vec![match &order.filter {
            Some(words) => format!("{} of {} match « {words} »", self.matched, rows(self.total)),
            None => rows(self.total),
        }];
        if let Some(sort) = order.sort.as_ref().filter(|s| columns.contains(s)) {
            let way = if order.desc {
                "descending"
            } else {
                "ascending"
            };
            parts.push(format!("sorted by {sort}, {way}"));
        }
        if self.pages > 1 {
            parts.push(format!("page {} of {}", self.page, self.pages));
        }
        parts.join("; ") + "."
    }

    /// The next page's URL, past the line, so an agent reads a long table a page at a time: relative to the page read, as
    /// every link of it is. The HTML keeps it for its text alone (`.sr-only`): the pager shows him the way.
    pub fn next(&self, order: &Order, query: &str) -> Option<String> {
        (self.page < self.pages).then(|| {
            Order {
                page: self.page + 1,
                ..order.clone()
            }
            .href(query)
        })
    }
}

/// A queue or a list of records, a row each (task rust-comments-table; @nha, 2026-11-09: « there is nothing card-like to
/// comments. they are a queue. a table. »): `columns` its head. A row's first cell is its link, kept only where a card's
/// would be (`card_href`), which takes the whole row's tap (components.css); its text form a record per row, its fields a
/// tab apart. Each head is a link that sorts by its column, again reverses it, `aria-sort` on the one sorted; past
/// `per_page` rows, Pagination pages it and a filter field above keeps the rows holding its words; all in the URL (`query`,
/// the page's), and its caption says them with the total, the next page's URL for the text (`Arranged::line`, `next`).
/// With no `query` (a form's answer, whose links would GET its POST path), every row under plain heads (/code-review).
#[component]
pub fn Table(
    #[props(into)] id: String,
    columns: Vec<String>,
    rows: Vec<TableRow>,
    #[props(default, into)] query: Option<String>,
    #[props(default = PER_PAGE)] per_page: usize,
) -> Element {
    let order = Order::of(query.as_deref().unwrap_or_default());
    let per_page = query.as_ref().map_or(rows.len(), |_| per_page);
    let arranged = arranged(&columns, &rows, &order, per_page);
    let line = query.as_ref().map(|_| arranged.line(&columns, &order));
    let next = query.as_ref().and_then(|q| arranged.next(&order, q));
    let Arranged {
        ref shown,
        page,
        pages,
        ..
    } = arranged;
    // a GET form drops the URL's query: every other key rides along hidden, the page back to the first
    let kept: Vec<(String, String)> = query.as_deref().map_or(vec![], |q| {
        pairs(q)
            .filter(|(k, _)| !["filter", "page"].contains(k))
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    });
    let filtered = query.is_some() && (rows.len() > per_page || order.filter.is_some());
    let cell = |row: &TableRow, j: usize, text: &str| match (j, card_href(&row.href)) {
        (0, Some(href)) => rsx! { a { href: "{href}", "{text}" } },
        _ => rsx! { "{text}" },
    };
    let head = columns.iter().map(|c| {
        let sorted = (order.sort.as_ref() == Some(c)).then_some(if order.desc {
            "descending"
        } else {
            "ascending"
        });
        match &query {
            Some(query) => {
                let href = order.by(c).href(query);
                rsx! { th { scope: "col", "aria-sort": sorted, a { href, "{c}" } } }
            }
            None => rsx! { th { scope: "col", "{c}" } },
        }
    });
    rsx! {
        if filtered {
            form { class: "table-filter", method: "get", role: "search",
                for (k, v) in kept { input { r#type: "hidden", name: k, value: v } }
                input { class: "input", r#type: "search", name: "filter", value: order.filter.clone().unwrap_or_default(),
                    placeholder: "Filter rows", "aria-label": "Filter the table" }
            }
        }
        table { id: "{id}", class: "table",
            if let Some(line) = line {
                caption { "{line}"
                    if let Some(next) = next { span { class: "sr-only", " Next page: {next}" } }
                }
            }
            thead { tr { {head} } }
            tbody {
                for row in shown.iter() {
                    tr {
                        for (j, text) in row.cells.iter().enumerate() { td { {cell(row, j, text)} } }
                    }
                }
            }
        }
        if pages > 1 {
            Pagination { id: "{id}-pages", query: query.unwrap_or_default(), page, pages }
        }
    }
}

/// The pages of a long list: DioxusLabs/dioxus-components' Pagination (MIT OR Apache-2.0, notice kept in
/// NOTICE-dioxus-components), its markup kept: a navigation of links, the shown one `aria-current`, Previous and Next where
/// there is one, the first, the last and the shown one's neighbours, an ellipsis between. Each link keeps the rest of
/// `query`, so the sort holds across pages.
#[component]
pub fn Pagination(
    #[props(into)] id: String,
    #[props(into)] query: String,
    page: usize,
    pages: usize,
) -> Element {
    let order = Order::of(&query);
    let at = |p: usize| {
        Order {
            page: p,
            ..order.clone()
        }
        .href(&query)
    };
    let mut items = vec![];
    if page > 1 {
        let href = at(page - 1);
        items.push(rsx! { li { a { class: "pagination-link", href, "aria-label": "Go to previous page", "data-kind": "previous", "Previous" } } });
    }
    let mut last = 0;
    for p in (1..=pages).filter(|&p| p == 1 || p == pages || p.abs_diff(page) <= 1) {
        if p > last + 1 {
            items.push(
                rsx! { li { span { class: "pagination-ellipsis", "aria-hidden": "true", "…" } } },
            );
        }
        let (href, current) = (at(p), (p == page).then_some("page"));
        items.push(rsx! { li { a { class: "pagination-link", href, "aria-current": current, "data-active": p == page, "{p}" } } });
        last = p;
    }
    if page < pages {
        let href = at(page + 1);
        items.push(rsx! { li { a { class: "pagination-link", href, "aria-label": "Go to next page", "data-kind": "next", "Next" } } });
    }
    rsx! {
        nav { id: "{id}", class: "pagination", role: "navigation", "aria-label": "pagination",
            ul { class: "pagination-content", {items.into_iter()} }
        }
    }
}

#[component]
pub fn CardTitle(children: Element) -> Element {
    rsx! { b { class: "title", {children} } }
}

#[component]
pub fn CardDescription(children: Element) -> Element {
    rsx! { span { class: "description", {children} } }
}

/// A short label: a state, a count, a tag.
#[component]
pub fn Badge(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { span { class: joined("badge", look), {children} } }
}

/// Today's day number since 1970-01-01 (UTC), the clock's: what a page's `When` counts from, and /drafts' day.
/// ponytail: past 23:00 in Paris it is still yesterday, so an age is a day short for an hour or two; his zone here when it
/// matters (/code-review of rust-timeline-component).
pub fn today() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86_400) as i64)
}

/// How long ago a day was, or how far ahead, from `today`: « today », « yesterday », « 3 days ago », « in 2 weeks ».
/// `None` when `date` starts with no day (2026-11-02).
pub fn ago(date: &str, today: i64) -> Option<String> {
    let n = today - dash_core::days(date)?;
    let (count, unit) = match n.abs() {
        0 => return Some("today".into()),
        1 => return Some(if n > 0 { "yesterday" } else { "tomorrow" }.into()),
        d if d < 14 => (d, "days"),
        d if d < 60 => (d / 7, "weeks"),
        d if d < 730 => (d / 30, "months"),
        d => (d / 365, "years"),
    };
    Some(if n > 0 {
        format!("{count} {unit} ago")
    } else {
        format!("in {count} {unit}")
    })
}

/// A day as he reads it first, « 3 days ago », its full date a tap away; its text form carries both (task
/// rust-timeline-component; @nha, 2026-11-10: « I like momentago style dates (plus the actual date, maybe click to toggle and
/// default to momentago) »). A `details` toggles with no script, in the window and on the web, a key reaching it too.
/// `date` starts with its day, a time after it kept in the full form; one that does not is shown as given.
#[component]
pub fn When(#[props(into)] date: String, today: i64) -> Element {
    match ago(&date, today) {
        Some(ago) => rsx! {
            details { class: "when",
                summary { span { "{ago}" } time { datetime: date.get(..10), "{date}" } }
            }
        },
        None => rsx! { time { class: "when", "{date}" } },
    }
}

/// Events in time, oldest or newest first as the page gives them: each a `TimelineEvent`, its dot on the line that joins them.
/// `look` « strip »: the events side by side on one line, left to right, an automation's runs then its next (task
/// rust-automations-tab; @nha, 2026-11-10: « I suggest making a tab with a visual for both of us, always dual-use »).
#[component]
pub fn Timeline(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { ol { class: joined("timeline", look), {children} } }
}

/// One row of a `Timeline`: its date (`When`), then what happened. `mark`: its dot, done unless it failed or is still ahead.
#[component]
pub fn TimelineEvent(
    #[props(into)] date: String,
    today: i64,
    #[props(default)] mark: Mark,
    children: Element,
) -> Element {
    rsx! { li { class: mark.class(), When { date, today } div { {children} } } }
}

/// A `TimelineEvent`'s dot: done (filled), failed (the error colour) or next (hollow, still ahead).
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mark {
    #[default]
    Done,
    Failed,
    Next,
}

impl Mark {
    /// Its class on the event, none for done, so a plain timeline's HTML is unchanged.
    pub fn class(self) -> Option<&'static str> {
        match self {
            Mark::Done => None,
            Mark::Failed => Some("failed"),
            Mark::Next => Some("next"),
        }
    }

    /// Its word in the text form.
    pub fn word(self) -> &'static str {
        match self {
            Mark::Done => "ok",
            Mark::Failed => "failed",
            Mark::Next => "next",
        }
    }
}

/// One event of a `timeline` tree element: when, what, its dot, and what it opens: a page (`href`, a run's task) or more
/// words unfolded in place (`detail`, a run's error).
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Event {
    pub at: String,
    pub what: String,
    #[serde(default)]
    pub mark: Mark,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// What a `TimelineEvent` says and opens: its link, its words with the detail a tap away, or its words alone.
#[component]
pub fn EventSays(what: String, href: Option<String>, detail: Option<String>) -> Element {
    match (href, detail) {
        (Some(href), _) => rsx! { a { href, "{what}" } },
        (None, Some(detail)) => rsx! { details { summary { "{what}" } pre { "{detail}" } } },
        (None, None) => rsx! { span { "{what}" } },
    }
}

/// Someone's initials in a circle.
#[component]
pub fn Avatar(#[props(into)] initials: String, #[props(default)] look: &'static str) -> Element {
    rsx! { span { class: joined("avatar", look), "{initials}" } }
}

/// The tools' logos, Simple Icons' (CC0, simpleicons.org), by the word /tools names a tool by: its title and role stripped,
/// its colour the text's, so a dark theme draws it light.
const LOGOS: [(&str, &str); 26] = [
    ("vaultwarden", include_str!("logos/vaultwarden.svg")),
    ("anthropic", include_str!("logos/anthropic.svg")),
    ("aws", include_str!("logos/aws.svg")),
    ("buildkite", include_str!("logos/buildkite.svg")),
    ("circleci", include_str!("logos/circleci.svg")),
    ("cloudflare", include_str!("logos/cloudflare.svg")),
    ("datadog", include_str!("logos/datadog.svg")),
    ("docker", include_str!("logos/docker.svg")),
    ("fly", include_str!("logos/fly.svg")),
    ("github", include_str!("logos/github.svg")),
    ("gmail", include_str!("logos/gmail.svg")),
    ("hetzner", include_str!("logos/hetzner.svg")),
    ("huggingface", include_str!("logos/huggingface.svg")),
    ("kubectl", include_str!("logos/kubectl.svg")),
    ("linear", include_str!("logos/linear.svg")),
    ("linkedin", include_str!("logos/linkedin.svg")),
    ("malt", include_str!("logos/malt.svg")),
    ("npm", include_str!("logos/npm.svg")),
    ("openai", include_str!("logos/openai.svg")),
    ("paddle", include_str!("logos/paddle.svg")),
    ("scaleway", include_str!("logos/scaleway.svg")),
    ("stripe", include_str!("logos/stripe.svg")),
    ("tailnet", include_str!("logos/tailnet.svg")),
    ("terraform", include_str!("logos/terraform.svg")),
    ("warp", include_str!("logos/warp.svg")),
    ("whatsapp", include_str!("logos/whatsapp.svg")),
];

/// The tools Simple Icons lacks: each product's own favicon, 64 px PNG from its site (Google's favicon service, 2026-11-09),
/// a data URL built at compile time, so the page carries it. An image, never inlined markup: a fetched file runs no script on the page.
const PICTURES: [(&str, &str); 14] = [
    (
        "avrea",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/avrea.png.b64")
        ),
    ),
    (
        "blacksmith",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/blacksmith.png.b64")
        ),
    ),
    (
        "browserbase",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/browserbase.png.b64")
        ),
    ),
    (
        "bunnynet",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/bunnynet.png.b64")
        ),
    ),
    (
        "depot",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/depot.png.b64")
        ),
    ),
    (
        "fal",
        concat!("data:image/png;base64,", include_str!("logos/fal.png.b64")),
    ),
    (
        "freework",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/freework.png.b64")
        ),
    ),
    (
        "healthchecks",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/healthchecks.png.b64")
        ),
    ),
    (
        "openrouter",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/openrouter.png.b64")
        ),
    ),
    // his own nicolas-ha.com's
    (
        "site",
        concat!("data:image/png;base64,", include_str!("logos/site.png.b64")),
    ),
    (
        "skyvern",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/skyvern.png.b64")
        ),
    ),
    (
        "tinyfish",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/tinyfish.png.b64")
        ),
    ),
    (
        "typesafe",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/typesafe.png.b64")
        ),
    ),
    (
        "ubicloud",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/ubicloud.png.b64")
        ),
    ),
];

/// A tool's entry in one table of logos, the one lookup `has_logo` and `Logo` share. A name access.rs' provider() keeps
/// apart from a product's card wears that product's logo: DOCKERHUB_TOKEN's « dockerhub » is Docker's.
/// ponytail: the PNGs ride inline in every /tools answer, 27 KB a read and again each second the live part refetches; a
/// static URL of their own if that ever weighs.
fn found(table: &[(&str, &'static str)], name: &str) -> Option<&'static str> {
    let name = match name {
        "dockerhub" => "docker",
        word => word,
    };
    table.iter().find(|(word, _)| *word == name).map(|l| l.1)
}

/// Whether a tool has a logo here, Simple Icons' or its favicon.
pub fn has_logo(name: &str) -> bool {
    found(&LOGOS, name).or(found(&PICTURES, name)).is_some()
}

/// A tool's logo in an avatar's circle, or its first two letters where none is held: Avatar's image and its fallback.
#[component]
pub fn Logo(#[props(into)] name: String) -> Element {
    if let Some(svg) = found(&LOGOS, &name) {
        return rsx! { span { class: joined("avatar", "logo"), dangerous_inner_html: svg } };
    }
    if let Some(src) = found(&PICTURES, &name) {
        return rsx! { span { class: joined("avatar", "logo"), img { src, alt: "" } } };
    }
    let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
    rsx! { Avatar { initials, look: "logo" } }
}

/// A tool on /tools, a section of the list and no card (@nha, 2026-11-09: « commands tools, etc have no reason to use cards »): its
/// logo and name, then its accesses. While its provider answers, it is a Skeleton of `reading`
/// grey lines under its head (@nha, 2026-11-09: « cards greyed out show a loading state »).
#[component]
pub fn Tool(
    #[props(into)] id: String,
    #[props(into)] name: String,
    #[props(default)] reading: usize,
    children: Element,
) -> Element {
    let head = rsx! { p { class: "head", Logo { name: name.clone() } CardTitle { "{name}" } } };
    if reading == 0 {
        return rsx! { div { id, class: "tool", {head} {children} } };
    }
    rsx! { Skeleton { id, what: name, lines: reading, look: "tool", {head} } }
}

/// What a page holds while a `Skeleton` is on it: a reader with no script (the window) renders the page again until it is
/// gone, where a browser runs LIVE_JS.
pub const BUSY: &str = r#"aria-busy="true""#;

/// What is still being read: grey lines, greyed and `aria-busy`, that fill in place when the data lands, no reload: the
/// page's LIVE_JS swaps in the server's newer drawing of the same id (task rust-skeleton-live; @nha, 2026-11-09: « Reading every access:
/// the next reload shows it. does not make sense. », « cards greyed out show a loading state »). `what` names what is read,
/// for a screen reader; `look` is the component it stands in for (`tool`), `children` what is already known of it.
#[component]
pub fn Skeleton(
    #[props(into)] id: String,
    #[props(into)] what: String,
    #[props(default = 2)] lines: usize,
    #[props(default, into)] look: String,
    children: Element,
) -> Element {
    let class = match look.as_str() {
        "" => "skeleton".to_string(),
        look => format!("{look} skeleton"),
    };
    let label = match what.as_str() {
        "" => "Being read".to_string(),
        what => format!("{what}: being read"),
    };
    // all expressions, so the HTML keeps this order (a literal would be drawn first): id, class, then the busy state
    let (busy, role) = ("true", "status");
    rsx! {
        div { id, class, "aria-busy": busy, role, "aria-label": label,
            {children}
            for _ in 0..lines { span { class: "line" } }
        }
    }
}

/// The page's one patcher (`swap`): it fetches the page's own URL in HTML and puts in, for each id it is given, the newer
/// drawing of that element; an element the newer drawing no longer holds, the nearest element around it whose id it holds;
/// an element new to the page, the nearest element around it that the page holds. A section the newer drawing no longer holds
/// (only the page's root around it is shared) is removed, never the whole page swapped. Two callers:
/// - **a busy element:** once a second while one is `aria-busy`, its live part (`.live`), else its nearest id; it stops once
///   none is busy. A page that answers an error, or still busy after five minutes, is reloaded: never grey lines with nothing
///   saying so (/code-review of PR 1248 and of rust-skeleton-live; rust/CLAUDE.md 5).
/// - **a view's diff:** a page holding a `data-view` part listens to `?view=patches`, each event the ids a diff changes, and
///   once on (re)connecting the part whole, for a diff missed while away (task rust-view-patches; @nha, 2026-11-09: « both
///   should work together locally too, via the timely dataflow updates for views. »).
pub(crate) const LIVE_JS: &str = r#"(()=>{if(window.dashLive)return;window.dashLive=1;const up=e=>e.parentElement&&e.parentElement.closest("[id]");const put=(d,o,i)=>{if(o){if(!o.isConnected)return;let e=o;while(e&&!d.getElementById(e.id))e=up(e);if(!e)return;if(e!==o&&!up(e))return o.remove();return e.replaceWith(d.getElementById(e.id))}let n=d.getElementById(i);while(n&&!document.getElementById(n.id))n=up(n);if(n)document.getElementById(n.id).replaceWith(n)};const swap=ids=>{const u=new URL(location.href);u.searchParams.set("view","html");return fetch(u).then(r=>{if(!r.ok){location.reload();throw r.status}return r.text()}).then(t=>{const d=new DOMParser().parseFromString(t,"text/html"),os=ids.map(i=>document.getElementById(i));ids.forEach((i,k)=>put(d,os[k],i))})};let n=0;const tick=()=>{const busy=[...document.querySelectorAll("[aria-busy=true]")];if(!busy.length)return;if(++n>300)return location.reload();swap(busy.map(b=>b.closest(".live[id]")||b.closest("[id]")).filter(e=>e).map(e=>e.id)).catch(()=>{}).finally(()=>setTimeout(tick,1000))};setTimeout(tick,1000);const v=document.querySelector("[data-view][id]");if(v&&window.EventSource){const u=new URL(location.href);u.searchParams.set("view","patches");const s=new EventSource(u);s.onopen=()=>swap([v.id]).catch(()=>{});s.onmessage=e=>swap(JSON.parse(e.data)).catch(()=>{})}})()"#;

/// The id of a terminal's screen inside the `Terminal` drawn under `id`: the one part its patch names.
pub fn terminal_screen(id: &str) -> String {
    format!("{id}-screen")
}

/// A terminal's screen as its program left it, then one line he types into it, posted to `action` (task rust-terminal-page,
/// rfc-rust-unified 20f). The screen alone follows the terminal's view (`data-view`): the page's LIVE_JS swaps it at each
/// change, and the field below keeps what he is typing. The page scrolls, never the screen: a swapped box would start at its
/// top again, hiding the prompt (/code-review).
#[component]
pub fn Terminal(
    #[props(into)] id: String,
    #[props(into)] text: String,
    #[props(into)] action: String,
) -> Element {
    let screen = terminal_screen(&id);
    rsx! {
        div { id, class: "terminal",
            pre { id: screen, class: "terminal-screen", "data-view": "terminal", "{text}" }
            Form { action,
                Input { r#type: "text", name: "text", placeholder: "A line to type; Enter sends it", autocomplete: "off" }
                Button { "Send" }
            }
        }
    }
}

/// Where a page stands when it cannot show its thing, its title then what to do about it (task rust-terminal-no-hc: /terminal
/// with no hc). A status, not an error: the page answers 200 and says so in the same place on the web, in the window and in
/// the text.
#[component]
pub fn Alert(
    #[props(into)] id: String,
    #[props(into)] title: String,
    children: Element,
) -> Element {
    rsx! {
        div { id, class: "alert", role: "status",
            b { class: "alert-title", "{title}" }
            p { {children} }
        }
    }
}

/// A part of a page refilled whole while a skeleton in it is busy: /tools' themes, whose cards appear as providers answer.
#[component]
pub fn Live(#[props(into)] id: String, children: Element) -> Element {
    rsx! { div { id, class: "live", {children} } }
}

/// A button: it submits the form around it unless given `r#type: "button"`. Any button attribute passes through.
#[component]
pub fn Button(
    #[props(default)] look: &'static str,
    #[props(extends = GlobalAttributes, extends = button)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! { button { class: joined("button", look), ..attributes, {children} } }
}

/// A link drawn as a button: an action that opens a page, a draft's Change and its Cancel (task rust-draft-state-edit).
#[component]
pub fn ButtonLink(
    #[props(default)] look: &'static str,
    #[props(into)] href: String,
    children: Element,
) -> Element {
    rsx! { a { class: joined("button", look), href, {children} } }
}

/// Fields and one button that send to `action`: the search bar (`get`), a reply, an upload (`file`: its picker, sent as
/// multipart), a step he confirms. Its own tag, so no page writes one by hand (task dash-settings-backup's forms were).
#[component]
pub fn Form(
    #[props(into)] id: Option<String>,
    #[props(into)] action: String,
    #[props(default = "post")] method: &'static str,
    #[props(default)] file: bool,
    role: Option<&'static str>,
    children: Element,
) -> Element {
    let enctype = file.then_some("multipart/form-data");
    rsx! { form { id, class: "form", role, method, action, enctype, {children} } }
}

/// The search field: it GETs /search, `value` the words shown back in it, `hidden` the fields sent with them (/inbox's
/// `in=inbox`). One component for a page's field and the menu's (task rust-search-port).
#[component]
pub fn SearchBar(
    #[props(into)] id: Option<String>,
    #[props(into)] placeholder: String,
    #[props(default, into)] value: String,
    #[props(default)] hidden: Vec<(String, String)>,
) -> Element {
    let hidden = hidden.iter().map(|(name, value)| {
        rsx! { Input { r#type: "hidden", name: "{name}", value: "{value}" } }
    });
    let value = (!value.is_empty()).then_some(value);
    rsx! {
        Form { id, role: "search", method: "get", action: "/search",
            {hidden}
            Input { r#type: "search", name: "q", placeholder: "{placeholder}", value }
            Button { "Search" }
        }
    }
}

/// The menu's « Search or ask » field, beside the compass on every page (task rust-notion-one-menu; @nha, 2026-11-10: « the
/// "ask" field is a good idea though it needs to live somewhere else »): it GETs /search on Enter, ⌘ K shown in it and
/// focusing it (forms.rs's NAV_JS). Its name is a word of the text, so an agent reads what the field is; the keys sit outside
/// its label, so a screen reader names it « Search or ask » alone (/code-review).
#[component]
pub fn Ask(#[props(into)] id: Option<String>) -> Element {
    rsx! {
        Form { id, role: "search", method: "get", action: "/search",
            div { class: "ask",
                label {
                    span { class: "sr-only", "Search or ask" }
                    Input { r#type: "search", name: "q", placeholder: "Search or ask" }
                }
                KbdGroup { Kbd { "⌘" } Kbd { "K" } }
            }
        }
    }
}

/// One access on /tools: its state's pill, then its name with room between them (task design-access-pill; @nha,
/// 2026-11-09: « "works" pill eats the space next to fly and overlaps »), then what it used, why and where from. The design
/// page draws this same component, never a copy of its markup.
#[component]
pub fn Access(
    #[props(into)] id: String,
    #[props(into)] status: String,
    #[props(into)] name: String,
    #[props(default, into)] usage: String,
    #[props(default, into)] note: String,
    #[props(default, into)] place: String,
    #[props(default, into)] source: String,
) -> Element {
    rsx! {
        Item { id, look: "access", status: status.clone(),
            p { class: "head", Badge { look: "chip", "{status}" } b { class: "name", "{name}" } }
            p { class: "usage", "{usage}" }
            p { class: "note", "{note}" }
            p { class: "place", span { class: "source", "{source}" } " {place}" }
        }
    }
}

/// A field: any input attribute passes through (`r#type`, `name`, `placeholder`, `hidden`).
#[component]
pub fn Input(
    #[props(extends = GlobalAttributes, extends = input)] attributes: Vec<Attribute>,
) -> Element {
    rsx! { input { class: "input", ..attributes } }
}

/// A file to send, dropped or chosen: rust-ui's Dropzone (notice in NOTICE-rust-ui) on dioxus-components' Input inside its
/// Label (notice in NOTICE-dioxus-components), which has no drop zone (task settings-upload-styled; @nha, 2026-11-09:
/// « unstyled chose file, no design system, does not follow button rules where the button is inside the alement (input or
/// other) »). Kept from rust-ui: a dashed area, an upload icon and its words, a tint while a file is dragged over it, the
/// chosen file named with its size before anything is sent. What sends it (`children`, /settings' submit) sits inside the
/// zone, never beside it. The browser's own input covers the area, transparent, so a click or a drop on it works with no
/// script; DROPZONE_JS, which it carries, names the file and takes a drop anywhere in the zone. ponytail: no list with
/// a remove per file nor previews (rust-ui's DropzoneFileList); add when a page sends several files to review first.
#[component]
pub fn FileUpload(
    #[props(into)] id: Option<String>,
    #[props(into)] name: String,
    #[props(into)] accept: String,
    #[props(into)] label: String,
    #[props(default)] multiple: bool,
    #[props(default)] required: bool,
    children: Element,
) -> Element {
    let (multiple, required) = (multiple.then_some(true), required.then_some(true));
    rsx! {
        div { id, class: "dropzone",
            label { class: "dropzone-area",
                input { class: "dropzone-input", r#type: "file", name, accept, multiple, required }
                {icon(UPLOAD)}
                span { class: "dropzone-text",
                    span { class: "dropzone-label", "{label}" }
                    span { class: "dropzone-file", "aria-live": "polite", "No file chosen" }
                }
            }
            {children}
            script { dangerous_inner_html: DROPZONE_JS }
        }
    }
}

/// FileUpload's words for a file of `accept`, its types joined by « or »: /settings' restore and its /design example say the
/// same (task settings-upload-styled).
pub fn drop_words(accept: &str) -> String {
    let types: Vec<&str> = accept.split(',').map(str::trim).collect();
    format!("Drop a {} file here, or choose one", types.join(" or "))
}

/// FileUpload's script, bound once on the document, so a zone a script clones (the compass's box) works too: every zone
/// carries it, as the menu does, and a page without either gets the browser's own picker under the words (/code-review).
/// A zone is tinted while files, not text or a link, are over it: a count of enters and leaves, as rust-ui's, so its own
/// parts do not flicker it. A drop anywhere in it takes the files its `accept` lists, added to those chosen when it takes
/// several, else the first alone; none taken, it says so and keeps what it had. Each chosen file is named with its size,
/// as rust-ui's `size_display`, its unit the one its rounding stays under 1024 in.
pub const DROPZONE_JS: &str = r#"(()=>{if(window.dashDropzone)return;window.dashDropzone=1;
const zone=e=>{const z=e.target.closest&&e.target.closest('.dropzone');return z&&z.querySelector('.dropzone-input')?z:null};
const files=e=>[...(e.dataTransfer&&e.dataTransfer.types||[])].includes('Files');
const size=b=>{if(b<1024)return b+' B';let i=-1;do{b/=1024;i++}while(+b.toFixed(2)>=1024&&i<2);return b.toFixed(2)+' '+['KB','MB','GB'][i]};
const takes=(i,f)=>{const a=i.accept.split(',').map(t=>t.trim().toLowerCase()).filter(Boolean),n=f.name.toLowerCase(),k=(f.type||'').toLowerCase();
return !a.length||a.some(t=>t[0]==='.'?n.endsWith(t):t.endsWith('/*')?k.startsWith(t.slice(0,-1)):k===t)};
const say=(z,t)=>{z.querySelector('.dropzone-file').textContent=t};
const show=z=>{const i=z.querySelector('.dropzone-input');say(z,i.files.length?[...i.files].map(f=>f.name+', '+size(f.size)).join('; '):'No file chosen')};
const calm=z=>{z.dataset.over=0;z.removeAttribute('data-dragging')};
document.addEventListener('change',e=>{const z=zone(e);if(z&&e.target.matches('.dropzone-input'))show(z)});
document.addEventListener('dragenter',e=>{const z=zone(e);if(z&&files(e)){z.dataset.over=(+z.dataset.over||0)+1;z.setAttribute('data-dragging','')}});
document.addEventListener('dragleave',e=>{const z=zone(e);if(z&&files(e)&&(z.dataset.over=(+z.dataset.over||1)-1)<1)calm(z)});
document.addEventListener('dragover',e=>{if(zone(e)&&files(e))e.preventDefault()});
document.addEventListener('drop',e=>{const z=zone(e);if(!z||!files(e))return;e.preventDefault();calm(z);const i=z.querySelector('.dropzone-input'),t=new DataTransfer(),
dropped=[...e.dataTransfer.files],ok=dropped.filter(f=>takes(i,f));if(!ok.length){say(z,dropped.map(f=>f.name).join(', ')+': not taken, '+i.accept+' only');return}
(i.multiple?[...i.files,...ok]:ok.slice(0,1)).forEach(f=>t.items.add(f));i.files=t.files;i.dispatchEvent(new Event('change',{bubbles:true}))})})()"#;

/// Lucide's upload, FileUpload's mark.
const UPLOAD: &str = "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12";

/// The comment compass, as the older app's top bar carried it (`DashboardWeb.CommentThread.comment_page_button/1`; @nha,
/// 2026-11-09: « the menu is getting nice. It needs the compass still. »): a tap, then a tap on any element of the page, comments
/// on it. Its drawing is `DashboardWeb.Library.compass/1`'s; `data-pick-button` lets an older page's pick mark it pressed and
/// take a second tap as a cancel, as it did its own button. A Button of the design system (rust/CLAUDE.md rule 6). `id`: the
/// menu's, which forms.rs's COMPASS_JS binds; none on /design's Page comments, which draws it too (task
/// rust-design-page-comments-compose). Its name is also its text form's (`.text-only`): an icon says nothing to an agent.
pub fn compass(id: Option<&'static str>) -> Element {
    rsx! {
        Button { look: "icon", r#type: "button", id, "data-pick-button": "", aria_pressed: "false",
            title: "Comment on an element: tap it next", aria_label: "Comment on an element",
            span { class: "text-only", "Comment on an element" }
            svg { view_box: "0 0 24 24", width: "16", height: "16", fill: "none", stroke: "currentColor", stroke_width: "2",
                stroke_linecap: "round", stroke_linejoin: "round", "aria-hidden": "true",
                path { d: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z" }
                path { d: "M16 8l-2 6-6 2 2-6z" }
            }
        }
    }
}

/// The compass's pick bar, the older app's (comment_thread.ex's `data-pickbar`): shown while the compass picks, it says what
/// the next tap does, and takes the whole page instead of an element, or stops the pick (task rust-comment-files-filter).
/// forms.rs's COMPASS_JS binds its buttons and NAV_CSS shows it while the compass is pressed.
pub fn pick_bar() -> Element {
    rsx! {
        div { class: "pick-bar", role: "toolbar", aria_label: "Comment on",
            span { "Tap the element to comment on" }
            Button { look: "quiet", r#type: "button", "data-whole-page": "", "Whole page" }
            Button { look: "quiet", r#type: "button", "data-pick-cancel": "", "Cancel" }
        }
    }
}

/// A field of several lines: a draft's « Request corrections », its text when he changes it (`text`, the words it opens on).
#[component]
pub fn Textarea(
    #[props(default, into)] text: String,
    #[props(extends = GlobalAttributes, extends = textarea)] attributes: Vec<Attribute>,
) -> Element {
    // a parser drops one newline right after <textarea>: a text opening on a blank line keeps it (/code-review)
    let lead = if text.starts_with('\n') { "\n" } else { "" };
    rsx! { textarea { class: "textarea", ..attributes, "{lead}{text}" } }
}

/// One choice of several, each a link, the shown one pressed: /design's « One of several » toggle group as a component
/// (task rust-tasks-views-toggle; @nha, 2026-11-09: « list/graph => use the design system toggle group, or better the logos etc
/// from the original design »). Its options are `ToggleOption`s.
#[component]
pub fn Toggle(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "toggle", "aria-label": label, {children} } }
}

/// One option of a `Toggle`: its icon beside its name, `aria-current` on the one shown.
#[component]
pub fn ToggleOption(
    #[props(into)] href: String,
    current: bool,
    icon: Element,
    children: Element,
) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, "aria-current": on, {icon} {children} } }
}

/// A page's top line, Linear's view head (board_live.ex's `view_head`): its parts in a row, the first at the left, the rest
/// at the right end (the board's title, then its « + New issue »).
#[component]
pub fn Bar(#[props(into)] id: String, children: Element) -> Element {
    rsx! { div { id, class: "bar", {children} } }
}

/// A plus in Lucide's strokes, New issue's mark: here, and re-exported by design::families, so no module loop.
pub(crate) const PLUS: &str = "M12 5v14M5 12h14";

/// Where a `NewIssue` form posts: `dash task file` files its words on the board's project (dash main.rs's `new_issue`).
pub const NEW_ISSUE: &str = "/tasks/new";

/// A modal over the page: dioxus-components' Dialog (notice in NOTICE-dioxus-components) on the browser's own `<dialog>`,
/// opened and closed by a button's `commandfor` and `command`, so no script runs (task rust-board-buttons-new-issue).
/// `closedby="closerequest"`: Esc closes it, a stray click outside never does, so a tap beside a long description keeps it
/// (/code-review of PR 1395).
#[component]
pub fn Dialog(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { dialog { id, class: "dialog", "aria-label": label, "closedby": "closerequest", {children} } }
}

/// Linear's « + New issue », top right of the board (board_live.ex's `new-issue-button`; @nha, 2026-11-10: « there should be
/// a "new issue" button »): a pill that opens a `Dialog` of its form, a title and a description, posted to `NEW_ISSUE`.
/// `project`: the board's, sent with them; none files on site_perso's. Its text form says where it posts and what.
#[component]
pub fn NewIssue(#[props(into)] id: String, project: Option<String>) -> Element {
    let dialog = format!("{id}-dialog");
    rsx! {
        button { id, class: "button pill", r#type: "button", "commandfor": "{dialog}", "command": "show-modal",
            {icon(PLUS)} "New issue"
        }
        Dialog { id: "{dialog}", label: "New issue",
            Form { id: "{dialog}-form", action: NEW_ISSUE,
                b { class: "dialog-title", "New issue" }
                Input { name: "title", placeholder: "Issue title", required: true, "autofocus": "autofocus", aria_label: "Title" }
                Textarea { name: "description", placeholder: "Add description...", rows: 4, aria_label: "Description" }
                if let Some(project) = project {
                    Input { r#type: "hidden", name: "project", value: "{project}" }
                }
                div { class: "dialog-actions",
                    button { class: "button quiet", r#type: "button", "commandfor": "{dialog}", "command": "close", "Cancel" }
                    Button { "Create issue" }
                }
            }
        }
    }
}

/// Copies `text` on a tap, Linear's round copy buttons over an issue (board_live.ex's `copy_button`: Copy link, Copy ID,
/// Copy branch name; task rust-board-buttons-new-issue). `origin`: `text` is a path, copied after the page's origin, so the
/// link works from any address the page was opened at. Its label is its title and its text form. It sits in a `Copies`,
/// whose script writes the clipboard.
#[component]
pub fn CopyButton(
    #[props(into)] id: String,
    label: &'static str,
    #[props(into)] text: String,
    #[props(default)] origin: bool,
    icon: Element,
) -> Element {
    rsx! {
        button { id, class: "button icon round", r#type: "button", title: label, "aria-label": label, "data-copy": "{text}",
            "data-origin": origin.then_some("true"),
            span { class: "text-only", "{label}: {text}" }
            {icon}
        }
    }
}

/// Copies a message's whole text on a tap, for him to paste where it goes (the older draft page's « Copy the text »; task
/// rust-draft-state-edit): its words then say « Copied », or « Copy failed, the text is selected » with the element `select`
/// names selected, so he copies it by hand. It sits in a `Copies`, whose script writes the clipboard.
#[component]
pub fn CopyText(
    #[props(into)] id: String,
    #[props(into)] text: String,
    #[props(into)] select: String,
) -> Element {
    rsx! {
        button { id, class: "button", r#type: "button", "data-copy": "{text}", "data-label": "Copy the text",
            "data-select": "{select}", "Copy the text"
        }
    }
}

/// CopyButtons side by side, their script once: it writes the clipboard on a tap and says « Copied » in the title a moment.
#[component]
pub fn Copies(children: Element) -> Element {
    rsx! {
        div { class: "copies", {children} }
        script { dangerous_inner_html: COPY_JS }
    }
}

/// Copies' script, once a page however many groups draw it.
/// The clipboard API needs a secure page, and his phone opens plain http on the tailnet address: there a selected text's copy
/// command does it (/code-review of PR 1395). Either way the title says whether it copied; a CopyText says it in its words,
/// and on a failure selects its text.
const COPY_JS: &str = r#"(()=>{if(window.dashCopy)return;window.dashCopy=1;const put=t=>{if(navigator.clipboard&&isSecureContext)return navigator.clipboard.writeText(t);const a=document.createElement("textarea");a.value=t;document.body.append(a);a.select();const ok=document.execCommand("copy");a.remove();return ok?Promise.resolve():Promise.reject()};document.addEventListener("click",e=>{const b=e.target.closest&&e.target.closest("[data-copy]");if(!b)return;const was=b.dataset.was??(b.dataset.was=b.title),label=b.dataset.label;const say=w=>{b.title=w;if(label)b.textContent=w;clearTimeout(b.dashSaid);b.dashSaid=setTimeout(()=>{b.title=was;if(label)b.textContent=label},2500)};const pick=()=>{const t=b.dataset.select&&document.getElementById(b.dataset.select);if(!t)return;const r=document.createRange();r.selectNodeContents(t);const s=getSelection();s.removeAllRanges();s.addRange(r)};put((b.dataset.origin?location.origin:"")+b.dataset.copy).then(()=>say("Copied"),()=>{pick();say(label?"Copy failed, the text is selected":"Not copied")})})})()"#;

/// One of an issue's properties as Linear's sidebar sets it (board_live.ex's `property_menu`; task
/// rust-board-buttons-new-issue): its value, a tap opens its choices, each a button posting `field` and its value to `action`.
/// A `<details>`, so it opens with no script; the choice shown is checked. Its field filters the choices as he types, as
/// Linear's menus do (task rust-issue-sidebar-menus), the page drawing `menu_filter` once; without it every choice stays. `many`: each
/// choice turns one value on or off (labels), the others kept, posting `on` 1 or 0 beside it. `initials`: each choice opens on its name's avatar (assignee).
/// `numbered`: each choice ends on its place from 0, and that digit picks it while the menu is open, as the older app's
/// assignee and « Remind me » menus (its `data-n`).
#[component]
pub fn PropertyMenu(
    #[props(into)] id: String,
    #[props(into)] action: String,
    field: &'static str,
    label: &'static str,
    current: Element,
    /// Each choice: its value, its name, whether it is the one shown.
    choices: Vec<(String, String, bool)>,
    #[props(default)] many: bool,
    #[props(default)] initials: bool,
    #[props(default)] numbered: bool,
) -> Element {
    let role = if many {
        "menuitemcheckbox"
    } else {
        "menuitemradio"
    };
    rsx! {
        details { id, class: "prop-menu",
            summary { title: label, {current} }
            div { class: "prop-menu-list", role: "menu", "aria-label": label,
                // for agents alone: where a pick posts, each choice's value and the one set
                span { class: "text-only", "{label}: POST {action} {field}=" if many { " with on=1 to turn one on, on=0 off" } }
                Input { r#type: "search", class: "prop-menu-filter", placeholder: label, "aria-label": label,
                    autocomplete: "off" }
                for (n, (value, name, on)) in choices.into_iter().enumerate() {
                    Form { action: action.clone(),
                        Input { r#type: "hidden", name: field, value: "{value}" }
                        // the state he wants, never « flip what is stored »: a list read since may hold it already
                        // (/code-review of PR 1399)
                        if many {
                            Input { r#type: "hidden", name: "on", value: if on { "0" } else { "1" } }
                        }
                        button { class: "prop-menu-item", role, "aria-checked": if on { "true" } else { "false" },
                            span { class: "text-only", "{value}:" }
                            if initials {
                                span { "aria-hidden": "true", Avatar { initials: initials_of(&name), look: "li-who" } }
                            }
                            // its own box, as the older app's `flex-1 truncate` span: beside the avatar and the digit it
                            // is a word of its own, never « CClaude0 » (dash parity read it glued)
                            span { "{name}" }
                            if on {
                                span { class: "text-only", "(set)" }
                            }
                            if numbered {
                                span { class: "prop-menu-n", "aria-hidden": "true", "data-n": "{n}", "{n}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// PropertyMenu's filter script, drawn once by the page that holds the menus (/code-review of PR 1399: once a menu sent it
/// eight times on an issue page).
pub fn menu_filter() -> Element {
    rsx! { script { dangerous_inner_html: FILTER_JS } }
}

/// A name's avatar letters, as the older app's: each word's first, two at most (« Nicolas Ha » NH, « Rust » R).
pub fn initials_of(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// PropertyMenu's filter: a choice whose words lack his text is hidden, its digit no word of it. A digit picks the numbered
/// choice of the menu he is in (the one open, when focus is nowhere), never one the filter hid, never while he types in a
/// field, an editor or a select, and once per press (/code-review of rust-issue-sidebar-menus). One pair of listeners a page.
const FILTER_JS: &str = r#"(()=>{if(window.dashMenuFilter)return;window.dashMenuFilter=1;const words=b=>[...b.childNodes].filter(c=>!(c.classList&&c.classList.contains("prop-menu-n"))).map(c=>c.textContent).join(" ").toLowerCase();document.addEventListener("input",e=>{const f=e.target;if(!f.matches||!f.matches(".prop-menu-filter"))return;const q=f.value.trim().toLowerCase();f.closest(".prop-menu-list").querySelectorAll(".prop-menu-item").forEach(b=>{b.closest("form").hidden=!!q&&!words(b).includes(q)})});document.addEventListener("keydown",e=>{const t=e.target;if(e.repeat||e.isComposing||e.metaKey||e.ctrlKey||e.altKey||!/^[0-9]$/.test(e.key)||!t.closest||t.isContentEditable||t.matches("input,textarea,select"))return;const open=document.querySelectorAll("details.prop-menu[open]");const m=t.closest("details.prop-menu[open]")||(t===document.body&&open.length===1?open[0]:null);const n=m&&m.querySelector('.prop-menu-n[data-n="'+e.key+'"]');const f=n&&n.closest("form");if(!f||f.hidden)return;e.preventDefault();f.requestSubmit()})})()"#;

/// One view of several, each a link: the server render runs no handler, so a tab changes the URL.
#[component]
pub fn Tabs(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "tabs", "aria-label": label, {children} } }
}

#[component]
pub fn Tab(#[props(into)] href: String, current: bool, children: Element) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, "aria-current": on, {children} } }
}

/// A phone's menu at the foot of the screen: rust-ui's BottomNav (notice in NOTICE-rust-ui), which dioxus-components lacks,
/// on this system's tokens and in dioxus-components' way, its parts plain markup and `aria-current` the state (task
/// rust-ui-bottom-nav; @nha, 2026-11-10: « mobile - https://rust-ui.com/docs/components/bottom-nav »). Kept from rust-ui:
/// one grid row of equal columns, each an icon over its label, the current one in the primary tone, a press that shrinks it,
/// no tap flash or callout, and room for the phone's home bar. Where it sits is its page's: it draws in place.
#[component]
pub fn BottomNav(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "bottom-nav", "aria-label": label, div { class: "bottom-nav-grid", {children} } } }
}

/// One page of a `BottomNav`: its icon over its name, `aria-current` on the shown one.
#[component]
pub fn BottomNavItem(
    #[props(into)] href: String,
    current: bool,
    icon: Element,
    children: Element,
) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, class: "bottom-nav-item", "aria-current": on, {icon} span { class: "bottom-nav-label", {children} } } }
}

/// A `BottomNav`'s last slot, what it leaves out: the label of the checkbox `r#for` that shows it, so it needs no script;
/// marked current (`aria-current="true"`, a button is no page) when the page is one of those, `controls` the part it shows.
/// A key reaches it, and its `aria-expanded` follows, through its page's script, as the checkbox is hidden.
#[component]
pub fn BottomNavMore(
    #[props(into)] id: String,
    #[props(into)] r#for: String,
    #[props(into)] controls: String,
    current: bool,
    icon: Element,
    children: Element,
) -> Element {
    let on = current.then_some("true");
    rsx! {
        label { id, r#for, class: "bottom-nav-item", role: "button", tabindex: "0", "aria-controls": controls, "aria-current": on,
            {icon} span { class: "bottom-nav-label", {children} }
        }
    }
}

/// A part folded to its one line, opened by a tap: a conversation's older mails (task rust-mail-thread).
#[component]
pub fn Fold(#[props(into)] id: String, open: bool, summary: Element, children: Element) -> Element {
    // a present `open`, even « false », opens it: only an open fold carries one
    let open = open.then_some(true);
    rsx! { details { id, class: "fold", open, summary { {summary} } {children} } }
}

/// A menu its trigger opens: DioxusLabs/dioxus-components' DropdownMenu (MIT OR Apache-2.0, notice kept in
/// NOTICE-dioxus-components), rebuilt on its primitive's markup since dioxus-primitives needs Dioxus 0.7: the trigger a button
/// with `aria-haspopup` and `aria-expanded`, the content labelled by it, `data-state` open or closed on all three. Its items
/// are actions, so a `menu` of `menuitem`s where upstream has a listbox of options (/code-review). `open` draws it open;
/// `toggle` is the window's handler (an Option: a default handler needs a runtime, which a catalog example's props lack), and
/// the web's page script flips the same attributes (the Notion top bar's « ... », task notion-page-menu-export).
#[component]
pub fn DropdownMenu(
    #[props(into)] id: String,
    open: bool,
    label: &'static str,
    trigger: Element,
    toggle: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    let state = if open { "open" } else { "closed" };
    rsx! {
        div { id: "{id}", class: "dropdown-menu", "data-state": state,
            button { id: "{id}-trigger", class: "button icon", r#type: "button", "aria-label": label,
                "aria-haspopup": "menu", "aria-expanded": if open { "true" } else { "false" }, "data-state": state,
                onclick: move |e| {
                    if let Some(toggle) = toggle {
                        toggle.call(e);
                    }
                },
                {trigger}
            }
            div { id: "{id}-content", class: "dropdown-menu-content", role: "menu", "aria-labelledby": "{id}-trigger",
                "data-state": state, hidden: !open,
                {children}
            }
        }
    }
}

/// One item of a `DropdownMenu`: a link, `download` the file name it saves as; with no `href`, a button whose action its page's
/// handler runs, so a key reaches it too (/code-review).
#[component]
pub fn DropdownMenuItem(
    #[props(into)] id: String,
    #[props(into)] href: Option<String>,
    download: Option<&'static str>,
    children: Element,
) -> Element {
    match href {
        Some(href) => {
            rsx! { a { id, class: "dropdown-menu-item", role: "menuitem", href, download, {children} } }
        }
        None => {
            rsx! { button { id, class: "dropdown-menu-item", role: "menuitem", r#type: "button", {children} } }
        }
    }
}

/// An on or off choice: dioxus-components' Switch (notice in NOTICE-dioxus-components), its track and thumb kept, its button
/// and hidden input made one checkbox with `role="switch"`, so a form sends it and a tap flips it with no script, on the web
/// and in the window. `children` are its words, the whole of it one label. ponytail: its text is its words alone, not its
/// state; a state word when a page's agent needs it.
#[component]
pub fn Switch(
    #[props(into)] id: String,
    name: &'static str,
    checked: bool,
    #[props(default)] disabled: bool,
    children: Element,
) -> Element {
    // a present `checked` or `disabled`, even « false », sets it: only a set one carries one
    let (checked, disabled) = (checked.then_some(true), disabled.then_some(true));
    rsx! {
        label { class: "switch",
            input { id, class: "switch-input", r#type: "checkbox", role: "switch", name, checked, disabled }
            span { class: "switch-track", "aria-hidden": "true", span { class: "switch-thumb" } }
            {children}
        }
    }
}

/// Whether a toolbar's buttons run down (its separators across) or along.
#[derive(Clone, Copy)]
struct Vertical(bool);

/// Buttons acting on one thing: dioxus-components' Toolbar (notice in NOTICE-dioxus-components), its `role="toolbar"`,
/// orientation and separators kept. A press on it never reaches what is under it nor moves the focus, so the selection it
/// acts on stays (the Notion tab's bar over a pick). ponytail: no arrow-key focus moves, Tab reaches each button; the
/// primitive's roving focus when a page needs it.
#[component]
pub fn Toolbar(
    #[props(into)] id: String,
    label: &'static str,
    #[props(default)] look: &'static str,
    #[props(default)] vertical: bool,
    children: Element,
) -> Element {
    use_context_provider(|| Vertical(vertical));
    let orientation = if vertical { "vertical" } else { "horizontal" };
    rsx! {
        div { id, class: joined("toolbar", look), role: "toolbar", "aria-label": label,
            "aria-orientation": orientation, "data-orientation": orientation,
            // nor takes the focus from it (/code-review): the web's page script does the same
            onmousedown: move |e| {
                e.prevent_default();
                e.stop_propagation();
            },
            {children}
        }
    }
}

/// One button of a `Toolbar`; `onclick` its action (an Option, as DropdownMenu's `toggle`, so a catalog example needs no
/// runtime). `title` names it on a hover and to a screen reader when its face is a sign; `mark`, its `data-format`, the
/// formatting a Composer's bar puts on the selection.
#[component]
pub fn ToolbarButton(
    #[props(into)] id: Option<String>,
    #[props(default)] look: &'static str,
    #[props(default)] title: Option<&'static str>,
    #[props(default)] mark: Option<&'static str>,
    onclick: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    rsx! {
        button { id, class: joined("toolbar-button", look), r#type: "button", title, "aria-label": title, "data-format": mark,
            onclick: move |e| {
                if let Some(onclick) = onclick {
                    onclick.call(e);
                }
            },
            {children}
        }
    }
}

/// A line between a `Toolbar`'s parts, across its run: a vertical toolbar's runs across.
#[component]
pub fn ToolbarSeparator() -> Element {
    let vertical = try_use_context::<Vertical>().is_some_and(|v| v.0);
    let orientation = if vertical { "horizontal" } else { "vertical" };
    rsx! { div { class: "toolbar-separator", role: "separator", "aria-orientation": orientation } }
}

/// Buttons of a `Toolbar` kept together, `title` its words over them.
#[component]
pub fn ToolbarGroup(
    #[props(default)] look: &'static str,
    title: Option<&'static str>,
    children: Element,
) -> Element {
    rsx! {
        div { class: joined("toolbar-group", look), role: "group",
            if let Some(title) = title {
                span { class: "toolbar-title", "{title}" }
            }
            {children}
        }
    }
}

/// More about a thing, shown while the pointer rests on it or a key focuses it: dioxus-components' HoverCard (notice in
/// NOTICE-dioxus-components), its content drawn under its trigger after a short wait, by CSS alone, no script. Upstream's
/// `tooltip` role is left out: shown, the content is part of what holds it (a board card's link), which names it.
/// `children` are the trigger, `content` the card. Its content is `hidden` but on a hover, so the text, as a closed
/// DropdownMenu's, holds the trigger's words alone (a board card's face; its task's page holds the rest). `open` draws it
/// open, for the catalog.
#[component]
pub fn HoverCard(#[props(default)] open: bool, content: Element, children: Element) -> Element {
    let state = if open { "open" } else { "closed" };
    rsx! {
        div { class: "hover-card", "data-state": state,
            {children}
            div { class: "hover-card-content", "data-state": state, hidden: !open, {content} }
        }
    }
}

/// A key to press: rust-ui's Kbd (notice in NOTICE-rust-ui) on this system's tokens, a small keycap no pointer selects.
#[component]
pub fn Kbd(children: Element) -> Element {
    rsx! { kbd { class: "kbd", {children} } }
}

/// Keys pressed together, as rust-ui's KbdGroup: `Kbd`s side by side.
#[component]
pub fn KbdGroup(children: Element) -> Element {
    rsx! { kbd { class: "kbd-group", {children} } }
}

/// One link of a `links` element.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct Link {
    pub name: String,
    pub href: String,
    #[serde(default)]
    pub current: bool,
}

/// A document, as a Lucide-style stroke (crate::design::families holds the others).
pub(crate) const DOC: &str = "M7 3h7l4 4v14H7zM14 3v4h4";

/// A stroke `d` in `currentColor`, the one way an SVG follows the theme (a CSS stroke does not reach it in Blitz): every
/// icon of the design system.
pub(crate) fn icon(d: &'static str) -> Element {
    rsx! {
        svg { class: "ic", width: "16", height: "16", view_box: "0 0 24 24",
            path { d, fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round" }
        }
    }
}

/// A box held to `ratio`, its width over its height: dioxus-components' AspectRatio (notice in NOTICE-dioxus-components), its
/// two boxes kept, the outer one's height its padding as upstream's, so what it holds takes its room before it loads. A ratio
/// that is no positive number is a square.
#[component]
pub fn AspectRatio(#[props(default = 1.0)] ratio: f64, children: Element) -> Element {
    let ratio = if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    };
    let pad = 100.0 / ratio;
    rsx! {
        div { class: "aspect-ratio", style: "padding-bottom: {pad}%",
            div { class: "aspect-ratio-content", {children} }
        }
    }
}

/// A picture held to its `ratio` while it loads, cropped to fill it: rust-ui's Image (notice in NOTICE-rust-ui), its `alt`,
/// lazy load and async decode kept, on `AspectRatio`. Its text form is nothing: what holds it names it.
#[component]
pub fn Image(
    #[props(into)] src: String,
    #[props(into)] alt: String,
    #[props(default = 1.0)] ratio: f64,
) -> Element {
    rsx! {
        AspectRatio { ratio,
            img { class: "image", src, alt, loading: "lazy", decoding: "async" }
        }
    }
}

/// Whether a file of MIME `kind` draws as a picture: a raster image every browser decodes, read case-blind and without its
/// parameters. An SVG, a TIFF or a HEIC stays a file (/code-review: the mail server's bytes come as octet-stream, which no
/// browser draws as SVG, and few decode the others).
pub fn is_picture(kind: &str) -> bool {
    let kind = kind
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/avif",
    ]
    .contains(&kind.as_str())
}

/// A file to open or download: rust-ui's Attachment (notice in NOTICE-rust-ui) on this system's tokens, built on
/// `AspectRatio`. Its media first, a document's icon in a square or, for a picture (`is_picture`), the picture itself as an
/// `Image`, the tile then upright as rust-ui's vertical one; then its name on one line and its kind, the name's extension
/// or else the MIME type. The whole tile is its link, as rust-ui's trigger; a link no page may open (`card_href`) leaves it
/// a tile. A picture is fetched only from this app (`local_path`) or drawn from its own bytes (a `data:image/` one, the
/// catalog's), never from another host, which would see it read (/code-review). ponytail: no size, no upload states nor
/// remove action, and a picture loads whole, no thumbnail; rust-ui's states when a page uploads, a thumbnail when one
/// weighs.
#[component]
pub fn Attachment(
    #[props(into)] id: String,
    #[props(into)] name: String,
    #[props(into)] kind: String,
    #[props(into)] href: String,
    #[props(default)] picture: bool,
) -> Element {
    let link = card_href(&Some(href.clone())).map(str::to_string);
    let bytes = href.starts_with("data:image/") && !href.starts_with("data:image/svg");
    let src = (picture && (local_path(&href) || bytes)).then_some(href);
    // an extension is letters and digits alone: « Minutes 10.30 am » has none (/code-review)
    let what = match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            ext.to_ascii_uppercase()
        }
        _ => kind,
    };
    let orientation = if src.is_some() {
        "vertical"
    } else {
        "horizontal"
    };
    let media = match src {
        // its name follows in words: the picture says it once more to no one (/code-review)
        Some(src) => rsx! { Image { src, alt: "", ratio: 4.0 / 3.0 } },
        None => {
            rsx! { AspectRatio { {icon(DOC)} } }
        }
    };
    let parts = rsx! {
        div { class: "attachment-media", {media} }
        span { class: "attachment-content",
            b { class: "attachment-title", "{name}" }
            span { class: "attachment-description", "{what}" }
        }
    };
    match link {
        Some(href) => rsx! {
            a { id, class: "attachment", "data-orientation": orientation, href, title: "{name}", {parts} }
        },
        None => rsx! { div { id, class: "attachment", "data-orientation": orientation, {parts} } },
    }
}

/// Attachments side by side, as rust-ui's AttachmentGroup: one row that scrolls along, each tile snapping to its start.
#[component]
pub fn AttachmentGroup(children: Element) -> Element {
    rsx! { div { class: "attachment-group", {children} } }
}

/// What a mail's HTML may do in its frame: nothing loads but its own data: images and inline styles, and no script runs
/// (the frame's sandbox holds no allow-scripts, and the policy says none). A `meta` of its own that sets a policy or a refresh
/// is made inert, so the mail cannot loosen this one or reload the frame from its server.
const FRAMED: &str = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:"><base target="_blank">"#;

/// A mail that follows the theme lets its document take either scheme: the theme ground through its frame, its text light on dark.
const FOLLOWS_THEME: &str = r#"<meta name="color-scheme" content="light dark">"#;

/// Whether a mail follows the theme: its markup, its tags and style blocks but never its words, names no colour (a style's,
/// or the old `bgcolor`, `text=`, `link=` attributes) and holds no image, all of which were picked against white
/// (/code-review: « Your background in Rust » is words, not a colour; a transparent logo vanishes on dark).
pub fn follows_theme(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    !lower.split('<').skip(1).any(|piece| {
        let tag = piece.split_once('>').map_or(piece, |(tag, _)| tag);
        let markup = if tag.starts_with("style") { piece } else { tag };
        ["color", "background", "text=", "link=", "img", "svg"]
            .iter()
            .any(|m| markup.contains(m))
    })
}

/// A mail's HTML as its frame's document: the policy first, then the mail, its own http-equiv metas made inert. A mail that
/// follows the theme (`follows_theme`) may take either scheme; any other is drawn as sent, light unless it names a
/// color-scheme of its own, which stays in force.
pub fn framed(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::from(FRAMED);
    if follows_theme(html) {
        out += FOLLOWS_THEME;
    }
    let mut at = 0;
    while let Some(found) = lower[at..].find("http-equiv") {
        out += &html[at..at + found];
        out += "data-off";
        at += found + "http-equiv".len();
    }
    out + &html[at..]
}

/// Someone else's HTML, framed with scripts off and nothing remote loaded (`framed`): a mail's body.
#[component]
pub fn Frame(#[props(into)] html: String, #[props(into)] title: String) -> Element {
    // links open outside the frame, in a tab of their own, still with no script of the frame's
    rsx! {
        iframe { class: if follows_theme(&html) { "frame themed" } else { "frame" }, title, referrerpolicy: "no-referrer", "sandbox": "allow-popups allow-popups-to-escape-sandbox", srcdoc: framed(&html) }
    }
}

/// A diagram from its text form, Mermaid (rust/CLAUDE.md rule 4: « diagrams etc. need to have a text form. say mermaid for
/// example. »), drawn to SVG here on the server by mermaid-rs-renderer: no script and no CDN on the page (@nha, 2026-11-09:
/// « Mermaid JS should never load from a CDN. The tool is local first. »). The fold under it holds the source an agent
/// reads; a source the renderer refuses says why, its source still folded under it.
#[component]
pub fn Diagram(#[props(into)] id: String, #[props(into)] source: String) -> Element {
    let drawn = match diagram_svg(&source) {
        Ok(svg) => {
            rsx! { div { id: "{id}", class: "diagram", role: "img", "aria-label": "Diagram", dangerous_inner_html: svg } }
        }
        Err(e) => {
            rsx! { p { id: "{id}", class: "diagram", "This diagram could not be drawn: {e}" } }
        }
    };
    rsx! {
        {drawn}
        Fold { id: "{id}-source", open: false, summary: rsx! { "Mermaid source" }, pre { "{source}" } }
    }
}

/// One look for every diagram: the neutral theme at 11 px (the code map's boxes are the Treemap component). Each source is
/// drawn once per process: the crate graph's routing takes 1.4 s in a release build.
pub fn diagram_svg(source: &str) -> Result<String, String> {
    static DRAWN: std::sync::Mutex<BTreeMap<String, Result<String, String>>> =
        std::sync::Mutex::new(BTreeMap::new());
    // ponytail: grows by one entry per distinct source; a page's diagrams are build constants today
    let mut drawn = DRAWN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(done) = drawn.get(source) {
        return done.clone();
    }
    let done = draw(source);
    drawn.insert(source.to_string(), done.clone());
    done
}

fn draw(source: &str) -> Result<String, String> {
    let mut theme = mermaid_rs_renderer::Theme::neutral();
    theme.font_size = 11.0;
    let layout = mermaid_rs_renderer::LayoutConfig::default();
    let options = mermaid_rs_renderer::RenderOptions { theme, layout };
    mermaid_rs_renderer::render_with_options(source, options).map_err(|e| e.to_string())
}

/// One box of a Treemap: its name, its size (its area), what it uses, each a box's path (its names from the top joined by
/// `/`), and the boxes nested in it. A box with children is as big as they are, its own `size` unread.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Nest {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub uses: Vec<String>,
    #[serde(default)]
    pub children: Vec<Nest>,
}

impl Nest {
    pub fn area(&self) -> u64 {
        match self.children.is_empty() {
            true => self.size,
            false => self.children.iter().map(Nest::area).sum(),
        }
    }
}

/// x, y, width, height on the treemap's canvas.
type Rect = (f64, f64, f64, f64);
const CANVAS: (f64, f64) = (1200.0, 700.0);

/// Squarified treemap (Bruls, Huizing and van Wijk, 2000): `sizes`, largest first, laid in `r` a row at a time along its
/// shorter side, a row growing while its worst aspect ratio improves.
fn squarify(sizes: &[f64], r: Rect) -> Vec<Rect> {
    let (mut x, mut y, mut w, mut h) = r;
    let scale = w * h / sizes.iter().sum::<f64>();
    let worst = |row: &[f64], side: f64| {
        let s = row.iter().sum::<f64>() * scale;
        let (big, small) = (row[0] * scale, row[row.len() - 1] * scale);
        (side * side * big / (s * s)).max(s * s / (side * side * small))
    };
    let (mut out, mut i) = (Vec::new(), 0);
    while i < sizes.len() {
        let side = w.min(h);
        let mut j = i + 1;
        while j < sizes.len() && worst(&sizes[i..=j], side) <= worst(&sizes[i..j], side) {
            j += 1;
        }
        let thick = sizes[i..j].iter().sum::<f64>() * scale / side;
        let mut along = 0.0;
        for v in &sizes[i..j] {
            let len = v * scale / thick;
            out.push(match w >= h {
                true => (x, y + along, thick, len),
                false => (x + along, y, len, thick),
            });
            along += len;
        }
        if w >= h {
            (x, w) = (x + thick, w - thick);
        } else {
            (y, h) = (y + thick, h - thick);
        }
        i = j;
    }
    out
}

/// A box as drawn: its path, name, size, depth, place and uses.
struct Placed<'a> {
    path: String,
    nest: &'a Nest,
    depth: usize,
    rect: Rect,
}

/// Each box with lines, parents before their children; a box too small to hold its children draws none of them. A parent's
/// title line and frame take room from its children, so its weight adds its frame's area, laid out again until that holds: a
/// file's area is its lines' share wherever it sits (/code-review: a small module's title line took 45% of its files' room).
fn placed(boxes: &[Nest]) -> Vec<Placed<'_>> {
    fn weight(b: &Nest, path: &str, per_line: f64, frames: &BTreeMap<String, f64>) -> f64 {
        if b.children.is_empty() {
            return b.size as f64 * per_line;
        }
        let inner = b
            .children
            .iter()
            .map(|c| weight(c, &format!("{path}/{}", c.name), per_line, frames));
        inner.sum::<f64>() + frames.get(path).copied().unwrap_or(0.0)
    }
    type Frames = BTreeMap<String, f64>;
    fn lay<'a>(
        boxes: &'a [Nest],
        r: Rect,
        at: &str,
        depth: usize,
        w: (f64, &Frames),
        out: &mut Vec<Placed<'a>>,
    ) {
        let path = |b: &Nest| {
            if at.is_empty() {
                b.name.clone()
            } else {
                format!("{at}/{}", b.name)
            }
        };
        let mut sized: Vec<(f64, &Nest)> = (boxes.iter().filter(|b| b.area() > 0))
            .map(|b| (weight(b, &path(b), w.0, w.1), b))
            .collect();
        sized.sort_by(|a, b| b.0.total_cmp(&a.0));
        let weights: Vec<f64> = sized.iter().map(|(a, _)| *a).collect();
        if weights.is_empty() {
            return;
        }
        for ((_, b), rect) in sized.into_iter().zip(squarify(&weights, r)) {
            out.push(Placed {
                path: path(b),
                nest: b,
                depth,
                rect,
            });
            // a name's line on top, 2 px of frame around
            let (x, y, wide, high) = rect;
            if !b.children.is_empty() && wide > 12.0 && high > 30.0 {
                lay(
                    &b.children,
                    (x + 2.0, y + 16.0, wide - 4.0, high - 18.0),
                    &path(b),
                    depth + 1,
                    w,
                    out,
                );
            }
        }
    }
    let (canvas, lines) = (
        CANVAS.0 * CANVAS.1,
        boxes.iter().map(Nest::area).sum::<u64>() as f64,
    );
    let mut frames = Frames::new();
    let mut out = Vec::new();
    // ponytail: a fixed 8 rounds; a frame's area moves less each round, under 1% of a box by the fourth on /deps
    for _ in 0..8 {
        out.clear();
        let per_line = (canvas - frames.values().sum::<f64>()).max(0.0) / lines.max(1.0);
        lay(
            boxes,
            (0.0, 0.0, CANVAS.0, CANVAS.1),
            "",
            0,
            (per_line, &frames),
            &mut out,
        );
        let drawn: BTreeMap<&str, Rect> = out.iter().map(|p| (p.path.as_str(), p.rect)).collect();
        let parent = |p: &Placed| {
            !p.nest.children.is_empty()
                && drawn
                    .keys()
                    .any(|k| k.strip_prefix(&p.path).is_some_and(|r| r.starts_with('/')))
        };
        frames = (out.iter().filter(|p| parent(p)))
            .map(|p| {
                (
                    p.path.clone(),
                    p.rect.2 * p.rect.3 - (p.rect.2 - 4.0) * (p.rect.3 - 18.0),
                )
            })
            .collect();
    }
    out
}

/// Thousands with commas: 14800 is 14,800.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What a hover shows: for each drawn box, a rule that outlines the boxes it uses (solid) and those using it (dashed), only
/// while it is under the pointer. CSS alone, no script: `scope` holds only [A-Za-z0-9_-], each box's key its index.
fn hover_css(scope: &str, drawn: &[Placed]) -> String {
    let at: BTreeMap<&str, usize> = drawn
        .iter()
        .enumerate()
        .map(|(k, p)| (p.path.as_str(), k))
        .collect();
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); drawn.len()];
    let uses: Vec<Vec<usize>> = (drawn.iter().enumerate())
        .map(|(k, p)| {
            let mut to: Vec<usize> = p
                .nest
                .uses
                .iter()
                .filter_map(|u| at.get(u.as_str()).copied())
                .filter(|t| *t != k)
                .collect();
            to.sort_unstable();
            to.dedup();
            to.iter().for_each(|t| users[*t].push(k));
            to
        })
        .collect();
    let is = |ks: &[usize]| {
        ks.iter()
            .map(|k| format!("[data-k=\"{k}\"]"))
            .collect::<Vec<_>>()
            .join(",")
    };
    // a box both used and using is double, never one rule overriding the other (/code-review)
    let mut css = String::new();
    for k in 0..drawn.len() {
        let on = format!("[data-map=\"{scope}\"]:has([data-k=\"{k}\"]:hover)");
        let both: Vec<usize> = uses[k]
            .iter()
            .filter(|t| users[k].contains(t))
            .copied()
            .collect();
        let only = |ks: &[usize]| {
            ks.iter()
                .filter(|t| !both.contains(t))
                .copied()
                .collect::<Vec<_>>()
        };
        for (ks, look) in [
            (only(&uses[k]), "solid var(--color-primary)"),
            (only(&users[k]), "dashed var(--color-accent)"),
            (both.clone(), "double var(--color-primary)"),
        ] {
            if !ks.is_empty() {
                css += &format!(
                    "{on} :is({}){{outline:3px {look};outline-offset:-3px}}\n",
                    is(&ks)
                );
            }
        }
    }
    css
}

/// Nested boxes whose area is their size, as windirstat draws a disk (task rust-deps-files-hover; @nha, 2026-11-09: « Something
/// similar to the matroshka / windirstat but where we can see nested files. Hoovering on a file / box shows direct links to
/// others. »): laid out here on the server, a box a positioned div, hovering one outlines what it uses and what uses it, by
/// CSS alone. Its text form (`treemap_text`) is the same tree indented, held in the HTML too (`.sr-only`, the drawn boxes
/// `aria-hidden`), so the page's text and the tree's say the same (task design-agent-forms).
#[component]
pub fn Treemap(#[props(into)] id: String, boxes: Vec<Nest>) -> Element {
    let drawn = placed(&boxes);
    let scope: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    let css = hover_css(&scope, &drawn);
    let pct = |v: f64, of: f64| format!("{:.3}%", v * 100.0 / of);
    let text = treemap_text(&boxes).join("\n");
    rsx! {
        p { class: "treemap-key", "Area is lines of code. Hover a box: a solid outline on what it uses, dashed on what uses it, double on both." }
        // its rules inside the element its id names, so a live patch by id swaps them with the boxes (/code-review)
        div { id, class: "treemap", "data-map": "{scope}", "aria-hidden": "true",
            style { dangerous_inner_html: css }
            for (k, p) in drawn.iter().enumerate() {
                div {
                    class: "treemap-box",
                    "data-k": "{k}",
                    "data-depth": "{p.depth.min(3)}",
                    "data-path": "{p.path}",
                    title: "{p.path}: {grouped(p.nest.area())} lines",
                    style: "left:{pct(p.rect.0, CANVAS.0)};top:{pct(p.rect.1, CANVAS.1)};width:{pct(p.rect.2, CANVAS.0)};height:{pct(p.rect.3, CANVAS.1)}",
                    "{p.nest.name}"
                }
            }
        }
        pre { class: "sr-only", "{text}" }
    }
}

/// A Treemap's text form: a box with an area a line, indented under its parent, its lines and what it uses, each named from
/// its nearest shared ancestor; a box using what its parent uses says it once, on the parent.
pub fn treemap_text(boxes: &[Nest]) -> Vec<String> {
    fn walk(boxes: &[Nest], above: &mut Vec<String>, theirs: &[String], out: &mut Vec<String>) {
        for b in boxes.iter().filter(|b| b.area() > 0) {
            let rel = |u: &String| {
                let near = above
                    .iter()
                    .rev()
                    .find_map(|a| u.strip_prefix(&format!("{a}/")));
                near.unwrap_or(u).to_string()
            };
            let uses = match b.uses.is_empty() || b.uses == theirs {
                true => String::new(),
                false => format!(
                    "; uses {}",
                    b.uses.iter().map(rel).collect::<Vec<_>>().join(", ")
                ),
            };
            let (n, s) = (b.area(), if b.area() == 1 { "" } else { "s" });
            let depth = "  ".repeat(above.len());
            out.push(format!("{depth}{}: {} line{s}{uses}", b.name, grouped(n)));
            let path = above
                .last()
                .map_or(b.name.clone(), |a| format!("{a}/{}", b.name));
            above.push(path);
            walk(&b.children, above, &b.uses, out);
            above.pop();
        }
    }
    let mut out = Vec::new();
    walk(boxes, &mut Vec::new(), &[], &mut out);
    out
}

/// One named run of a chart's values, a value a label (rust-ui's `series_names` and `json_values`, notice in NOTICE-rust-ui).
#[derive(Clone, PartialEq, Debug)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f64>,
}

/// How a line or an area runs from one value to the next: rust-ui's ChartCurve.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ChartCurve {
    #[default]
    Smooth,
    Straight,
    Stepline,
}

#[derive(Clone, Copy, PartialEq)]
enum Chart {
    Area,
    Bar,
    Line,
}

/// Series filled down to zero, a gradient unless `gradient` is false: rust-ui's AreaChart (task rust-charts-rust-ui; @nha,
/// 2026-11-10: « https://rust-ui.com/charts/area-chart let's replace our charts with these in our design system »). rust-ui
/// hands its values to ApexCharts in the browser; here the server draws the SVG, so the page runs no script and loads nothing
/// remote, and the text is the values as a table. Its ideas kept: values under labels, curve, gradient fill, a y axis shown
/// unless asked not to, a grid only when asked, negatives in the muted colour; the first series the primary colour.
#[component]
pub fn AreaChart(
    #[props(into)] id: String,
    /// The labels' column name in the table: « Week », « Month ».
    #[props(into)]
    axis: String,
    labels: Vec<String>,
    series: Vec<ChartSeries>,
    #[props(default)] curve: ChartCurve,
    #[props(default = true)] gradient: bool,
    #[props(default = true)] show_yaxis: bool,
    #[props(default)] show_grid: bool,
) -> Element {
    let look = Look {
        kind: Chart::Area,
        curve,
        gradient,
        show_yaxis,
        show_grid,
        spark: false,
    };
    chart(&id, &axis, &labels, &series, look)
}

/// A bar a value from zero, a negative one in the muted colour, its top rounded: rust-ui's BarChart. Several series stand
/// side by side in their label's slot.
#[component]
pub fn BarChart(
    #[props(into)] id: String,
    #[props(into)] axis: String,
    labels: Vec<String>,
    series: Vec<ChartSeries>,
    #[props(default = true)] show_yaxis: bool,
    #[props(default)] show_grid: bool,
) -> Element {
    let look = Look {
        kind: Chart::Bar,
        curve: ChartCurve::Straight,
        gradient: false,
        show_yaxis,
        show_grid,
        spark: false,
    };
    chart(&id, &axis, &labels, &series, look)
}

/// A line through each value, a dot on each: rust-ui's LineChart. Look `spark`: a sparkline, the line alone at a word's
/// height, no axis, grid or dot.
#[component]
pub fn LineChart(
    #[props(into)] id: String,
    #[props(into)] axis: String,
    labels: Vec<String>,
    series: Vec<ChartSeries>,
    #[props(default)] curve: ChartCurve,
    #[props(default = true)] show_yaxis: bool,
    #[props(default)] show_grid: bool,
    #[props(default)] look: &'static str,
) -> Element {
    let spark = look == "spark";
    let look = Look {
        kind: Chart::Line,
        curve,
        gradient: false,
        show_yaxis: show_yaxis && !spark,
        show_grid: show_grid && !spark,
        spark,
    };
    chart(&id, &axis, &labels, &series, look)
}

#[derive(Clone, Copy)]
struct Look {
    kind: Chart,
    curve: ChartCurve,
    gradient: bool,
    show_yaxis: bool,
    show_grid: bool,
    spark: bool,
}

/// Labels from words: a chart's `labels`.
pub fn chart_labels(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// A value as its table cell and axis label: grouped by thousands, up to two decimals, nothing for no number (NaN).
fn chart_number(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let cents = (v.abs() * 100.0).round();
    if cents >= u64::MAX as f64 {
        return format!("{v}");
    }
    let cents = cents as u64;
    let sign = if v < 0.0 && cents > 0 { "-" } else { "" };
    let decimals = match cents % 100 {
        0 => String::new(),
        c => format!(".{c:02}").trim_end_matches('0').to_string(),
    };
    format!("{sign}{}{decimals}", grouped(cents / 100))
}

/// The y axis's ticks: a round step (1, 2 or 5 a power of ten) giving at most five gaps from zero, or from the lowest value
/// to the highest for a sparkline, whose trend is the point (/code-review).
fn chart_ticks(values: impl Iterator<Item = f64>, spark: bool) -> Vec<f64> {
    let finite: Vec<f64> = values.filter(|v| v.is_finite()).collect();
    let from_zero = if spark { f64::NAN } else { 0.0 };
    let lo = finite.iter().copied().fold(from_zero, f64::min);
    let hi = finite.iter().copied().fold(from_zero, f64::max);
    if lo.is_nan() || hi <= lo {
        // no value, or all equal: one step up
        let lo = if lo.is_nan() { 0.0 } else { lo };
        return vec![lo, lo + 1.0];
    }
    if spark {
        return vec![lo, hi];
    }
    let mag = 10f64.powf(((hi - lo) / 5.0).log10().floor());
    let gaps = |s: f64| ((lo / s).floor() as i64, (hi / s).ceil() as i64);
    let step = [1.0, 2.0, 5.0, 10.0, 20.0]
        .into_iter()
        .map(|m| m * mag)
        .find(|&s| gaps(s).1 - gaps(s).0 <= 5)
        .unwrap_or(20.0 * mag);
    let (from, to) = gaps(step);
    (from..=to).map(|k| k as f64 * step).collect()
}

/// A path through the points: straight, in steps, or smooth (Catmull-Rom's curve as Béziers, near ApexCharts' « smooth »).
fn chart_path(points: &[(f64, f64)], curve: ChartCurve) -> String {
    let mut d = String::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        if i == 0 {
            d += &format!("M{x:.1},{y:.1}");
            continue;
        }
        let (px, py) = points[i - 1];
        d += &match curve {
            ChartCurve::Straight => format!(" L{x:.1},{y:.1}"),
            ChartCurve::Stepline => format!(" H{x:.1} V{y:.1}"),
            ChartCurve::Smooth => {
                let (bx, by) = points[i.saturating_sub(2)];
                let (nx, ny) = points[(i + 1).min(points.len() - 1)];
                // each handle held between its segment's ends, so the curve never overshoots a value or the zero line
                let hold = |c: f64| c.clamp(py.min(y), py.max(y));
                let (c1x, c1y) = (px + (x - bx) / 6.0, hold(py + (y - by) / 6.0));
                let (c2x, c2y) = (x - (nx - px) / 6.0, hold(y - (ny - py) / 6.0));
                format!(" C{c1x:.1},{c1y:.1} {c2x:.1},{c2y:.1} {x:.1},{y:.1}")
            }
        };
    }
    d
}

/// A bar from `base` to `y`, its far end rounded as rust-ui's (`borderRadius: 4`, at the end).
fn chart_bar(x: f64, w: f64, base: f64, y: f64) -> String {
    let h = (base - y).abs();
    let r = 4f64.min(w / 2.0).min(h);
    let dir = if y <= base { 1.0 } else { -1.0 };
    let (yr, right) = (y + dir * r, x + w);
    format!(
        "M{x:.1},{base:.1} V{yr:.1} Q{x:.1},{y:.1} {:.1},{y:.1} H{:.1} Q{right:.1},{y:.1} {right:.1},{yr:.1} V{base:.1} Z",
        x + r,
        right - r
    )
}

/// Where each series' key sits: (series, x, y), a new line when the next would pass `end` (/code-review: long names ran
/// past the chart).
fn chart_key(series: &[ChartSeries], left: f64, end: f64) -> Vec<(usize, f64, f64)> {
    let (mut x, mut y) = (left, 6.0);
    let mut out = vec![];
    for (k, s) in series.iter().enumerate() {
        // ponytail: a name's width guessed at 10 units a letter, a text measure if names run long
        let width = 36.0 + 10.0 * s.name.chars().count() as f64;
        if x > left && x + width > end {
            (x, y) = (left, y + 22.0);
        }
        out.push((k, x, y));
        x += width;
    }
    out
}

// Every mark's colour is an attribute in `currentColor` too: Blitz draws an inline svg as one image that no CSS reaches
// (icon()), so there each mark takes the chart's colour; a browser's CSS gives each series its own.
fn chart(id: &str, axis: &str, labels: &[String], series: &[ChartSeries], look: Look) -> Element {
    let (w, h) = if look.spark {
        (160.0, 44.0)
    } else {
        (600.0, 220.0)
    };
    // every value drawn and listed, a label short of one left blank (/code-review: a value past the labels was dropped)
    let n = (series.iter().map(|s| s.values.len()))
        .chain([labels.len()])
        .max()
        .unwrap_or(0);
    let label = |i: usize| labels.get(i).map_or("", String::as_str);
    let left = match (look.spark, look.show_yaxis) {
        (true, _) => 3.0,
        (false, true) => 60.0,
        (false, false) => 16.0,
    };
    let right = if look.spark { 3.0 } else { 16.0 };
    // more than one series: a key over the plot, rust-ui's legend, each name beside its colour
    let key = match series.len() > 1 && !look.spark {
        true => chart_key(series, left, w - right),
        false => vec![],
    };
    let (top, bottom) = match (look.spark, key.last()) {
        (true, _) => (3.0, 3.0),
        (false, Some((_, _, y))) => (y + 28.0, 30.0),
        (false, None) => (12.0, 30.0),
    };
    let (pw, ph) = (w - left - right, (h - top - bottom).max(1.0));
    let ticks = chart_ticks(
        series.iter().flat_map(|s| s.values.iter().copied()),
        look.spark,
    );
    let (lo, hi) = (ticks[0], ticks[ticks.len() - 1]);
    let sy = |v: f64| top + ph * (hi - v.clamp(lo, hi)) / (hi - lo);
    let bar = look.kind == Chart::Bar;
    let sx = |i: usize| match (bar, n) {
        (true, _) => left + pw * (i as f64 + 0.5) / n as f64,
        (false, 0 | 1) => left + pw / 2.0,
        (false, _) => left + pw * i as f64 / (n - 1) as f64,
    };
    let base = sy(lo.max(0.0).min(hi));
    // a label every so often past twelve, as rust-ui's tick amount
    let every = n.div_ceil(12).max(1);
    let ruled: Vec<(f64, String)> = ticks.iter().map(|&t| (sy(t), chart_number(t))).collect();
    let shown: Vec<(f64, &str, &str)> = (0..n)
        .step_by(every)
        .map(|i| {
            let anchor = match (bar, i) {
                (true, _) => "middle",
                (false, 0) if n > 1 => "start",
                (false, _) if i + 1 == n && n > 1 => "end",
                _ => "middle",
            };
            (sx(i), label(i), anchor)
        })
        .collect();
    let slot = if n == 0 { pw } else { pw / n as f64 };
    let bar_w = slot * 0.6 / series.len().max(1) as f64;
    let class = if look.spark { "chart spark" } else { "chart" };
    let view = format!("0 0 {w} {h}");
    rsx! {
        div { id, class,
            svg { class: "chart-svg", view_box: view, "aria-hidden": "true",
                if look.show_grid {
                    for (y, _) in ruled.iter() { line { class: "chart-grid", stroke: "currentColor", stroke_opacity: "0.2", x1: "{left}", x2: "{w - right}", y1: "{y:.1}", y2: "{y:.1}" } }
                }
                if look.show_yaxis {
                    for (y, t) in ruled.iter() { text { class: "chart-tick", fill: "currentColor", x: "{left - 8.0}", y: "{y + 5.0:.1}", text_anchor: "end", "{t}" } }
                }
                for (k, x, y) in key {
                    g { class: "chart-s{k.min(2)}",
                        rect { fill: "currentColor", x: "{x:.1}", y: "{y}", width: "12", height: "12", rx: "2" }
                        text { class: "chart-key", fill: "currentColor", x: "{x + 18.0:.1}", y: "{y + 11.0}", "{series[k].name}" }
                    }
                }
                if !look.spark {
                    line { class: "chart-base", stroke: "currentColor", stroke_opacity: "0.2", x1: "{left}", x2: "{w - right}", y1: "{base:.1}", y2: "{base:.1}" }
                    for (x, label, anchor) in shown { text { class: "chart-tick", fill: "currentColor", x: "{x:.1}", y: "{h - 8.0}", text_anchor: anchor, "{label}" } }
                }
                for (k, s) in series.iter().enumerate() {
                    g { class: "chart-s{k.min(2)}",
                        if bar {
                            for (i, v) in s.values.iter().enumerate().filter(|(_, v)| v.is_finite()) {
                                path {
                                    class: if *v < 0.0 { "chart-bar neg" } else { "chart-bar" },
                                    fill: "currentColor",
                                    d: chart_bar(sx(i) - slot * 0.3 + bar_w * k as f64, bar_w, base, sy(*v)),
                                    title { "{label(i)}: {chart_number(*v)}" }
                                }
                            }
                        } else {
                            {chart_run(id, k, s, labels, &sx, &sy, base, look)}
                        }
                    }
                }
            }
            table { class: "sr-only",
                thead { tr { th { "{axis}" } for s in series { th { "{s.name}" } } } }
                tbody {
                    for i in 0..n {
                        tr {
                            th { "{label(i)}" }
                            for s in series { td { {s.values.get(i).map_or(String::new(), |v| chart_number(*v))} } }
                        }
                    }
                }
            }
        }
    }
}

/// One series of a line or an area chart: its fill under it, then its line, then its dots. A value that is no number (NaN)
/// is left out, the line joining its neighbours.
#[allow(clippy::too_many_arguments)]
fn chart_run(
    id: &str,
    k: usize,
    s: &ChartSeries,
    labels: &[String],
    sx: &dyn Fn(usize) -> f64,
    sy: &dyn Fn(f64) -> f64,
    base: f64,
    look: Look,
) -> Element {
    let drawn: Vec<(usize, f64, f64)> = (s.values.iter().enumerate())
        .filter(|(_, v)| v.is_finite())
        .map(|(i, v)| (i, sx(i), sy(*v)))
        .collect();
    let points: Vec<(f64, f64)> = drawn.iter().map(|&(_, x, y)| (x, y)).collect();
    let line = chart_path(&points, look.curve);
    let fill = match (points.first(), points.last()) {
        (Some((x0, _)), Some((x1, _))) if look.kind == Chart::Area => {
            Some(format!("{line} L{x1:.1},{base:.1} L{x0:.1},{base:.1} Z"))
        }
        _ => None,
    };
    let fade = format!("{id}-fade{k}");
    let label = |i: usize| labels.get(i).map_or("", String::as_str);
    let dots = look.kind == Chart::Line && !look.spark;
    rsx! {
        if let Some(d) = fill {
            if look.gradient {
                linearGradient { id: "{fade}", x1: "0", x2: "0", y1: "0", y2: "1",
                    stop { offset: "0", "stop-color": "currentColor", "stop-opacity": "0.8" }
                    stop { offset: "0.9", "stop-color": "currentColor", "stop-opacity": "0.05" }
                }
                path { class: "chart-area", d, fill: "url(#{fade})" }
            } else {
                path { class: "chart-area", d, fill: "currentColor", fill_opacity: "0.4" }
            }
        }
        path { class: "chart-line", d: line, fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linejoin: "round", stroke_linecap: "round" }
        if dots {
            for (i, x, y) in drawn {
                circle { class: "chart-dot", cx: "{x:.1}", cy: "{y:.1}", r: "3.5", fill: "none", stroke: "currentColor", stroke_width: "2",
                    title { "{label(i)}: {chart_number(s.values[i])}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// rust-charts-rust-ui: the y axis holds zero and every value on round steps, at most five gaps even across zero, a
    /// sparkline its own range; a smooth curve never leaves the band between its two ends; numbers grouped, decimals too, and a
    /// NaN drawn nowhere and listed blank; a value past the labels still drawn and listed; a long key wraps (/code-review).
    #[test]
    fn a_charts_ticks_hold_zero_and_its_curve_never_overshoots() {
        let _budget = TestBudget::new();
        let ticks = |v: &[f64], spark| chart_ticks(v.iter().copied(), spark);
        assert_eq!(ticks(&[6.0, 11.0], false), [0.0, 5.0, 10.0, 15.0]);
        assert_eq!(
            ticks(&[2500.0, -230.0], false),
            [-1000.0, 0.0, 1000.0, 2000.0, 3000.0]
        );
        assert_eq!(ticks(&[-1.0, 9.0], false), [-5.0, 0.0, 5.0, 10.0]);
        assert_eq!(ticks(&[], false), [0.0, 1.0]);
        assert_eq!(ticks(&[f64::NAN], false), [0.0, 1.0]);
        assert_eq!(ticks(&[1000.0, 1010.0, 1005.0], true), [1000.0, 1010.0]);
        assert_eq!(ticks(&[], true), [0.0, 1.0]);
        let fine: Vec<String> = ticks(&[0.25, 0.5], false)
            .into_iter()
            .map(chart_number)
            .collect();
        assert_eq!(fine, ["0", "0.1", "0.2", "0.3", "0.4", "0.5"]);
        let d = chart_path(
            &[(0.0, 10.0), (10.0, 0.0), (20.0, 10.0), (30.0, 10.0)],
            ChartCurve::Smooth,
        );
        let ys: Vec<f64> = (d.split([' ', 'C', 'M']))
            .filter_map(|p| p.split_once(','))
            .map(|(_, y)| y.parse().unwrap())
            .collect();
        assert!(ys.iter().all(|y| (0.0..=10.0).contains(y)), "{d}");
        assert_eq!(
            chart_path(&[(0.0, 4.0), (5.0, 2.0)], ChartCurve::Stepline),
            "M0.0,4.0 H5.0 V2.0"
        );
        let empty = dioxus_ssr::render_element(
            rsx! { LineChart { id: "e", axis: "Day", labels: vec![], series: vec![] } },
        );
        assert_eq!(crate::words::text_of(&empty), "Day\n");
        let numbers: Vec<String> = [1234.5, -12000.0, -0.001, 0.996, f64::NAN]
            .into_iter()
            .map(chart_number)
            .collect();
        assert_eq!(numbers, ["1,234.5", "-12,000", "0", "1", ""]);
        let series = vec![ChartSeries {
            name: "Rate".into(),
            values: vec![1.0, f64::NAN, 3.0],
        }];
        let html = dioxus_ssr::render_element(
            rsx! { LineChart { id: "n", axis: "Day", labels: chart_labels(&["a", "b"]), series } },
        );
        assert!(!html.contains("NaN"), "{html}");
        assert_eq!(html.matches("<circle").count(), 2, "{html}");
        assert_eq!(crate::words::text_of(&html), "Day\tRate\na\t1\nb\n3\n");
        let long = |name: &str| ChartSeries {
            name: name.into(),
            values: vec![],
        };
        let names = [
            "Applications sent",
            "Replies received",
            "Calls booked",
            "Offers signed",
        ]
        .map(long);
        let key = chart_key(&names, 60.0, 584.0);
        assert!(
            key.iter().all(
                |&(k, x, _)| x + 36.0 + 10.0 * names[k].name.len() as f64 <= 584.0 || x == 60.0
            ),
            "{key:?}"
        );
        assert!(key.last().unwrap().2 > 6.0, "wrapped: {key:?}");
    }

    /// Done-when 1 of rust-deps-files-hover: every file's area is its share of the lines, inside a parent or not, and a child
    /// sits inside its parent; a box of no lines is not drawn.
    #[test]
    fn a_treemap_box_has_its_share_of_the_area_inside_its_parent() {
        let _budget = TestBudget::new();
        let leaf = |name: &str, size| Nest {
            name: name.into(),
            size,
            uses: vec![],
            children: vec![],
        };
        let boxes = vec![
            Nest {
                name: "a".into(),
                size: 0,
                uses: vec![],
                children: vec![leaf("x", 300), leaf("y", 100), leaf("t", 0)],
            },
            leaf("b", 600),
            leaf("c", 100),
        ];
        let drawn = placed(&boxes);
        let paths: Vec<&str> = drawn.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["b", "a", "a/x", "a/y", "c"]);
        // every leaf the same area per line, the files in « a » too, despite its title line
        let per_line: Vec<f64> = [("b", 600.0), ("a/x", 300.0), ("a/y", 100.0), ("c", 100.0)]
            .iter()
            .map(|(path, lines)| {
                let r = drawn.iter().find(|p| p.path == *path).expect(path).rect;
                r.2 * r.3 / lines
            })
            .collect();
        assert!(
            per_line
                .iter()
                .all(|v| (v / per_line[0] - 1.0).abs() < 0.01),
            "{per_line:?}"
        );
        let (a, x) = (drawn[1].rect, drawn[2].rect);
        assert!(
            x.0 >= a.0
                && x.1 >= a.1
                && x.0 + x.2 <= a.0 + a.2 + 1e-9
                && x.1 + x.3 <= a.1 + a.3 + 1e-9,
            "{a:?} {x:?}"
        );
    }

    /// Done-when 2 and 3 of rust-timeline-component: a day reads « 3 days ago », its full date on a tap (the details opens); the
    /// text form carries both, so an agent reads the exact day.
    #[test]
    fn a_date_reads_its_age_first_and_carries_its_full_date() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-10").expect("a day");
        let html = dioxus_ssr::render_element(rsx! { When { date: "2026-11-07", today } });
        assert_eq!(
            html,
            r#"<details class="when"><summary><span>3 days ago</span><time datetime="2026-11-07">2026-11-07</time></summary></details>"#
        );
        // the text form carries both, the age then the full date, whichever the screen shows
        assert_eq!(crate::forms::text_of(&html), "3 days ago\t2026-11-07\n");
        // the toggle is the details' `open`: closed shows the age alone, open the full date alone (the browser test clicks it)
        assert!(
            CSS.contains(".when:not([open]) time, .when[open] span { display: none; }"),
            "{CSS}"
        );
        let opened = html.replacen(
            r#"<details class="when">"#,
            r#"<details class="when" open="">"#,
            1,
        );
        assert_eq!(
            crate::forms::text_of(&opened),
            "3 days ago\t2026-11-07\n",
            "open, its text still carries both"
        );
        let odd = dioxus_ssr::render_element(rsx! { When { date: "soon", today } });
        assert_eq!(odd, r#"<time class="when">soon</time>"#);
        let said: Vec<_> = [
            "2026-11-10",
            "2026-11-09",
            "2026-11-11",
            "2026-10-31",
            "2026-10-11",
            "2026-01-10",
            "2023-11-10",
            "2026-11-24 10:00 UTC",
            "11/10",
        ]
        .iter()
        .map(|d| ago(d, today))
        .collect();
        assert_eq!(
            said,
            [
                Some("today"),
                Some("yesterday"),
                Some("tomorrow"),
                Some("10 days ago"),
                Some("4 weeks ago"),
                Some("10 months ago"),
                Some("3 years ago"),
                Some("in 2 weeks"),
                None
            ]
            .map(|s| s.map(String::from))
        );
    }

    /// Done-when 1 and 3 of rust-timeline-component: a row a event, its date then its words; the dot and the line are its style.
    #[test]
    fn a_timeline_is_a_row_an_event_its_date_then_what_happened() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-10").expect("a day");
        let html = dioxus_ssr::render_element(rsx! {
            Timeline {
                TimelineEvent { date: "2026-11-07", today, "Her reply" }
                TimelineEvent { date: "2026-11-02 09:00 UTC", today, "First message" }
            }
        });
        assert_eq!(
            html,
            r#"<ol class="timeline"><li><details class="when"><summary><span>3 days ago</span><time datetime="2026-11-07">2026-11-07</time></summary></details><div>Her reply</div></li><li><details class="when"><summary><span>8 days ago</span><time datetime="2026-11-02">2026-11-02 09:00 UTC</time></summary></details><div>First message</div></li></ol>"#
        );
        assert_eq!(
            crate::forms::text_of(&html),
            "3 days ago\t2026-11-07\nHer reply\n8 days ago\t2026-11-02 09:00 UTC\nFirst message\n"
        );
    }

    /// The crate graph's diagram: each box's name drawn, a label is text and never markup, and a source the renderer refuses
    /// is an error, never a page without its diagram unsaid.
    #[test]
    fn a_diagram_is_svg_drawn_here_its_labels_text_and_a_bad_source_an_error() {
        let _budget = TestBudget::new();
        let svg = diagram_svg("flowchart LR\n  a[\"surface\"] --> b[\"<script>x</script>\"]\n")
            .expect("drawn");
        assert!(
            svg.starts_with("<svg") && svg.contains(">surface<"),
            "{svg}"
        );
        assert!(!svg.contains("<script"), "{svg}");
        assert!(diagram_svg("not a diagram").is_err());
    }
}

// /mail's frame, as the older app's /mail on :7799 drew it (Gmail's; task rust-mail-parity; @nha, 2026-11-10: « the mail app
// needs to be _mostly_ the same and we're maybe 20% there »): the side column (the account switcher, Compose, the folders and
// labels with their counts), the search over the list, the list's bar (refresh, the range, the pager), a conversation's head.
// Each shown on /design?tab=mail, so the list, a thread and Compose draw with them alone (rust/CLAUDE.md rule 6).
pub(crate) const INBOX: &str = "M4 13l3-8h10l3 8v6H4zM4 13h5l1 2h4l1-2h5";
pub(crate) const MAIL: &str = "M4 6h16v12H4zM4 7l8 6 8-6";
pub(crate) const PEN: &str = "M4 20l4-1 11-11-3-3L5 16z";
pub(crate) const PERSON: &str = "M12 4a4 4 0 100 8 4 4 0 000-8zM5 20a7 7 0 0114 0";
pub(crate) const SEARCH: &str = "M11 4a7 7 0 100 14 7 7 0 000-14zM16 16l5 5";
pub(crate) const SEND: &str = "M12 19V6M6 12l6-6 6 6";

pub(crate) const STAR: &str =
    "M12 3l2.8 5.7 6.2.9-4.5 4.4 1 6.2-5.5-2.9-5.5 2.9 1-6.2L3 9.6l6.2-.9z";
const DRAFT: &str = "M7 3h7l4 4v14H7zM14 3v4h4";
const SPAM: &str = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 8v5M12 16h.01";
const TRASH: &str = "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3";
const LABEL: &str = "M3 12V4h8l10 10-8 8z M7.5 7.5h.01";
const REFRESH: &str = "M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7";
const CHEVRON: &str = "M6 9l6 6 6-6";
const BACK: &str = "M19 12H5M11 6l-6 6 6 6";
const PREVIOUS: &str = "M15 6l-6 6 6 6";
const NEXT: &str = "M9 6l6 6-6 6";

/// One folder or label of the side column: its name, where it lists, its count (none at 0), its kind, the one shown marked.
/// `kind`: a folder's JMAP role (inbox, sent, drafts, junk, trash), « starred », « all », or « label » for one of his own.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, Debug)]
pub struct Folder {
    pub name: String,
    pub href: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub current: bool,
}

fn folder_icon(kind: &str) -> &'static str {
    match kind {
        "inbox" => INBOX,
        "starred" => STAR,
        "sent" => SEND,
        "drafts" => DRAFT,
        "junk" => SPAM,
        "trash" => TRASH,
        "all" => MAIL,
        _ => LABEL,
    }
}

/// /mail's frame: the side column, then the search and `children`, the list or a conversation. `accounts`: the switch's
/// addresses, one marked shown; `compose`: where Compose opens; `hidden`: the search's fields kept (the account, the folder).
/// On a phone the side column folds above the list, its folders one scrolling row.
#[component]
pub fn MailShell(
    #[props(into)] id: String,
    #[props(into)] account: String,
    accounts: Vec<Link>,
    #[props(into)] compose: String,
    folders: Vec<Folder>,
    #[props(into)] search: String,
    #[props(default)] hidden: Vec<(String, String)>,
    children: Element,
) -> Element {
    let switch = accounts.into_iter().enumerate().map(|(i, a)| {
        let href = card_href(&Some(a.href)).map(str::to_string);
        let on = a.current.then_some("true");
        rsx! { DropdownMenuItem { id: "{id}-account-{i}", href, span { "aria-checked": on, "{a.name}" } } }
    });
    let folders = folders.into_iter().map(|f| {
        let href = card_href(&Some(f.href)).unwrap_or("#").to_string();
        let on = f.current.then_some("page");
        let class = if f.count > 0 {
            "mail-folder busy"
        } else {
            "mail-folder"
        };
        rsx! {
            a { class, href, "aria-current": on,
                {icon(folder_icon(&f.kind))}
                span { class: "name", "{f.name}" }
                if f.count > 0 { span { class: "count", "{f.count}" } }
            }
        }
    });
    let hidden = hidden.into_iter().map(|(name, value)| {
        rsx! { Input { r#type: "hidden", name: "{name}", value: "{value}" } }
    });
    let compose = card_href(&Some(compose))
        .unwrap_or("/mail/compose")
        .to_string();
    rsx! {
        div { id: "{id}", class: "mail-shell",
            aside { class: "mail-side",
                details { class: "mail-account dropdown-menu",
                    summary { {icon(PERSON)} span { class: "name", "{account}" } {icon(CHEVRON)} }
                    div { class: "dropdown-menu-content", role: "menu", {switch} }
                }
                a { class: "button mail-compose", href: compose, {icon(PEN)} "Compose" }
                nav { class: "mail-folders", "aria-label": "Folders", {folders} }
            }
            div { class: "mail-main",
                form { class: "form mail-search", role: "search", method: "get", action: "/mail",
                    {hidden}
                    label { {icon(SEARCH)} span { class: "sr-only", "Search mail" }
                        Input { r#type: "search", name: "q", placeholder: "Search mail", value: search }
                    }
                }
                {children}
            }
        }
    }
}

/// An arrow of a bar: a link while it leads somewhere, else greyed.
fn arrow(d: &'static str, label: &'static str, href: Option<String>) -> Element {
    match href
        .as_ref()
        .and_then(|h| card_href(&Some(h.clone())).map(str::to_string))
    {
        Some(href) => {
            rsx! { a { class: "button icon", href, "aria-label": label, title: label, {icon(d)} } }
        }
        None => {
            rsx! { span { class: "button icon", "aria-disabled": "true", "aria-label": label, {icon(d)} } }
        }
    }
}

/// The list's bar, Gmail's: Refresh at the left, the range (« 1–50 of 382 ») and the pager's arrows at the right.
#[component]
pub fn MailBar(
    #[props(into)] id: String,
    #[props(into)] range: String,
    #[props(into)] refresh: String,
    #[props(default)] newer: Option<String>,
    #[props(default)] older: Option<String>,
) -> Element {
    rsx! {
        div { id: "{id}", class: "mail-bar", role: "toolbar", "aria-label": "List",
            {arrow(REFRESH, "Refresh", Some(refresh))}
            span { class: "range", "{range}" }
            {arrow(PREVIOUS, "Newer", newer)}
            {arrow(NEXT, "Older", older)}
        }
    }
}

/// A conversation's head: back to its list, its subject, its folder as a chip, how many mails it holds.
#[component]
pub fn MailHead(
    #[props(into)] id: String,
    #[props(into)] back: String,
    #[props(into)] subject: String,
    #[props(into)] folder: String,
    #[props(into)] count: String,
) -> Element {
    let back = if local_path(&back) {
        back
    } else {
        "/mail".into()
    };
    rsx! {
        div { id: "{id}", class: "mail-head",
            div { class: "mail-bar", role: "toolbar", "aria-label": "Conversation",
                a { class: "button icon", href: back, "aria-label": "Back to the list", title: "Back to the list", {icon(BACK)} }
            }
            h1 { "{subject}" " "
                if !folder.is_empty() { Badge { "{folder}" } }
                span { class: "count", " {count}" }
            }
        }
    }
}
