//! The one Dioxus component of a UI tree (task rust-dioxus-only-app; @nha, 2026-11-08: « DIOXUS now. And that serves the web
//! page too. »): dash serve renders it to HTML here, and dash ui's window draws the very same component with Dioxus Native.
//! It draws every element of the catalog, so every page's HTML is this component's (task rust-page-forms).
use super::{
    DARK, Element as Kind, TaskView, Tree, card_href, check, escaped, graph_svg, task_href,
};
use crate::components::{
    Access, Badge, Bar, Button, Card, CardDescription, CardTitle, Diagram, EventSays, Form, Input,
    Item, LIVE_JS, Live, MailFold, MoreLink, NewIssue, SearchBar, Skeleton, Tab, Tabs, ThreadRow,
    Timeline, TimelineEvent, Toggle, ToggleOption, Tool, Treemap,
};
use crate::documents::status_icon;
use dioxus::prelude::*;

/// Both stylesheets inside the component, so the window, which has no page around it, gets them too.
/// The root fills the window as the page: the window's html has no theme of its own, so the root carries the background
/// page.css gives html, and the body's padding, as rust/ui-spike's page does.
const CSS: &str = concat!(
    include_str!("theme.css"),
    include_str!("components.css"),
    include_str!("design/cards.css"),
    include_str!("documents.css"),
    include_str!("page.css"),
    "body { padding: 0; } div.app { box-sizing: border-box; min-height: 100vh; padding: var(--spacing-s4); background: var(--color-base-200); color: var(--color-base-content); }"
);

/// A cycle, a missing id, a nesting past its limit or a link that is not GitHub's is refused here first, so `node` draws a
/// tree known whole.
fn parsed(tree: &str) -> Result<Tree, String> {
    check(tree)
}

/// A tree checked for the component: what `Screen` is given.
pub fn checked(tree: &str) -> Result<String, String> {
    parsed(tree).map(|_| tree.to_string())
}

/// A row's or a card's count of his comments, in words: none when there are none.
fn comments_said(comments: usize) -> Option<String> {
    match comments {
        0 => None,
        1 => Some("1 comment".to_string()),
        n => Some(format!("{n} comments")),
    }
}

/// Where an element sits: inside the board, where a status is a column and a task a card, /design?tab=linear's (task
/// rust-board-linear). A task's page is its task brief's template (crate::documents), no context of its own.
#[derive(Clone, Copy, PartialEq)]
enum In {
    Page,
    Board,
    List,
}

/// An element and its children, drawn as `inside` asks.
fn node(tree: &Tree, id: &str, inside: In) -> Element {
    let Some(n) = tree.elements.get(id) else {
        return rsx! {};
    };
    let inside = match n.element {
        Kind::Board { .. } => In::Board,
        Kind::List => In::List,
        _ => inside,
    };
    let board = inside == In::Board;
    let children = n.children.iter().map(|child| node(tree, child, inside));
    match &n.element {
        Kind::IssueGroup {
            status,
            title,
            more,
        } if board => crate::board::column(
            id,
            status,
            title.as_deref(),
            n.children.len() + more.as_ref().map_or(0, |m| m.count),
            more.clone(),
            rsx! { {children} },
        ),
        Kind::IssueRow {
            id: task,
            title,
            status,
            says,
            comments,
            project,
            labels,
            assignee,
            movable,
            href,
            why,
        } if board => crate::board::card(
            id,
            crate::board::Face {
                task,
                title,
                at: status,
                says,
                comments: *comments,
                said: comments_said(*comments),
                href: href
                    .clone()
                    .unwrap_or_else(|| task_href(task, project.as_deref())),
                labels,
                assignee: assignee.as_deref(),
                movable: *movable,
                why,
            },
        ),
        Kind::Stack => rsx! { div { id, {children} } },
        Kind::Heading { text } => rsx! { h1 { id, "{text}" } },
        Kind::Text { text } => rsx! { p { id, "{text}" } },
        Kind::IssueGroup {
            status,
            title,
            more,
        } => {
            let count = n.children.len() + more.as_ref().map_or(0, |m| m.count);
            rsx! {
                section { id, class: "issue-group", "data-status": status.class(),
                    h2 { {status_icon(status)} {title.as_deref().unwrap_or(status.name())} Badge { look: "count", "{count}" } }
                    {children}
                    MoreLink { more: more.clone() }
                }
            }
        }
        Kind::IssueRow {
            id: task,
            title,
            status,
            says,
            comments,
            project,
            href,
            ..
        } => {
            let href = href
                .clone()
                .unwrap_or_else(|| task_href(task, project.as_deref()));
            let said = comments_said(*comments);
            rsx! {
                Item { id, look: "issue-row", href, task: task.clone(),
                    {status_icon(status)}
                    span { class: "key", "{task}" }
                    span { class: "title", "{title}" }
                    span { class: "says", "{says}" }
                    if let Some(said) = said {
                        span { class: "comments", "data-comments": "{comments}", "{said}" }
                    }
                }
            }
        }
        Kind::Quote { text } => rsx! { pre { id, class: "quote", "{text}" } },
        Kind::Diagram { source } => rsx! { Diagram { id, source: source.clone() } },
        Kind::Treemap { boxes } => rsx! { Treemap { id, boxes: boxes.clone() } },
        Kind::Timeline { events, strip } => {
            let today = crate::components::today();
            let each = events.iter().map(|e| {
                let href = card_href(&e.href).map(str::to_string);
                rsx! {
                    TimelineEvent { date: e.at.clone(), today, mark: e.mark,
                        EventSays { what: e.what.clone(), href, detail: e.detail.clone() }
                    }
                }
            });
            let look: &'static str = if *strip { "strip" } else { "" };
            rsx! { div { id, Timeline { look, {each} } } }
        }
        Kind::Board { view } => crate::board::board(id, view.clone(), rsx! { {children} }),
        Kind::Cards => rsx! { div { id, class: "cards", {children} } },
        Kind::List => rsx! { div { id, class: "list", {children} } },
        Kind::Live => rsx! { Live { id, {children} } },
        Kind::Skeleton { what } => rsx! { Skeleton { id, what: what.clone() } },
        Kind::Tool { name, reading } => rsx! { Tool { id, name, reading: *reading, {children} } },
        // in a list a card is a row: the same title, line and link, drawn by Item; still reading, its row's grey lines
        Kind::Card {
            line,
            reading: true,
            ..
        } if inside == In::List => rsx! {
            Skeleton { id, what: "", lines: 1, look: "issue-row", span { class: "says", "{line}" } }
        },
        Kind::Card {
            title, line, href, ..
        } if inside == In::List => rsx! {
            Item { id, look: "issue-row", href: card_href(href).map(str::to_string),
                span { class: "title", "{title}" }
                span { class: "says", "{line}" }
            }
        },
        Kind::Card {
            line,
            reading: true,
            ..
        } => rsx! { Card { id, reading: true, CardDescription { "{line}" } } },
        Kind::Card {
            title, line, href, ..
        } => rsx! {
            Card { id, href: card_href(href).map(str::to_string),
                CardTitle { "{title}" }
                CardDescription { "{line}" }
            }
        },
        // a form posts only to a path of this app
        Kind::Upload {
            action,
            label,
            accept,
        } if crate::components::local_path(action) => rsx! {
            Form { id, action: action.clone(), file: true,
                Input { r#type: "file", name: "file", accept: "{accept}", required: true }
                Button { r#type: "submit", "{label}" }
            }
        },
        Kind::Action { action, label } if crate::components::local_path(action) => rsx! {
            Form { id, action: action.clone(),
                Button { r#type: "submit", "{label}" }
            }
        },
        Kind::Upload { .. } | Kind::Action { .. } => {
            rsx! { p { id, "This form posts outside the app: not drawn." } }
        }
        Kind::Links { links } => {
            let each = links.iter().map(|l| {
                let href = card_href(&Some(l.href.clone())).unwrap_or("#").to_string();
                rsx! { Tab { href, current: l.current, "{l.name}" } }
            });
            rsx! { Tabs { id, label: "Links", {each} } }
        }
        Kind::Bar => rsx! { Bar { id, {children} } },
        Kind::NewIssue { project } => rsx! { NewIssue { id, project: project.clone() } },
        Kind::TaskViews { current, project } => {
            let links = TaskView::ALL.map(|v| {
                let icon = crate::design::families::icon(v.icon());
                rsx! { ToggleOption { href: v.href(project.as_deref()), current: v == *current, icon, "{v.name()}" } }
            });
            rsx! { Toggle { id, label: "Views", {links.into_iter()} } }
        }
        // for agents alone: the text form has it, his page nothing of it
        Kind::Section { agents: true, .. } => rsx! {},
        Kind::Section { title, .. } => rsx! {
            section { id, class: "issue-part",
                h2 { "{title}" }
                {children}
            }
        },
        Kind::SearchBar {
            placeholder,
            value,
            hidden,
        } => rsx! { SearchBar { id, placeholder, value, hidden: hidden.clone() } },
        // ponytail: the graph's SVG is graph_svg's string, its words escaped there, inner HTML; the window draws it as part of
        // the page's HTML (rust-window-in-process). An rsx port when a window draws this component itself.
        // `parsed` refused a graph `graph_svg` refuses, so its error is never met here.
        Kind::TaskGraph { tasks } => {
            let svg = graph_svg(&escaped(id), tasks).unwrap_or_default();
            rsx! { div { dangerous_inner_html: svg } }
        }
        Kind::Access {
            name,
            status,
            usage,
            note,
            place,
            source,
        } => rsx! {
            Access { id, status: status.to_string(), name: name.clone(), usage: usage.clone(), note: note.clone(),
                place: place.clone(), source: source.clone() }
        },
        Kind::Document(doc) => crate::documents::draw(id, doc),
        Kind::DocView { doc, view, anchors } => rsx! {
            div { id, crate::documents::DocView { doc: doc.clone(), view: *view, anchors: anchors.clone() } }
        },
        Kind::Mail {
            from,
            to,
            at,
            text,
            html,
            open,
            files,
            address,
        } => rsx! {
            MailFold { id, from, to, at, text, html: html.clone(), open: *open, files: files.clone(), address }
        },
        Kind::Compose {
            action,
            send,
            placeholder,
            hidden,
            fields,
            format,
        } => rsx! {
            crate::documents::Composer { id, action, send, placeholder, hidden: hidden.clone(), fields: fields.clone(), format: *format }
        },
        Kind::MailShell {
            account,
            accounts,
            compose,
            folders,
            search,
            hidden,
        } => rsx! {
            crate::components::MailShell { id, account, accounts: accounts.clone(), compose, folders: folders.clone(), search,
                hidden: hidden.clone(), {children} }
        },
        Kind::MailBar {
            range,
            refresh,
            newer,
            older,
        } => rsx! {
            crate::components::MailBar { id, range, refresh, newer: newer.clone(), older: older.clone() }
        },
        Kind::MailHead {
            back,
            subject,
            folder,
            count,
        } => rsx! {
            crate::components::MailHead { id, back, subject, folder, count }
        },
        Kind::Alert { title, text } => rsx! { crate::components::Alert { id, title, "{text}" } },
        Kind::Terminal { text, action } => rsx! {
            crate::components::Terminal { id, text, action }
        },
        Kind::ThreadRow {
            from,
            subject,
            snippet,
            at,
            unread,
            href,
            count,
            starred,
        } => rsx! {
            ThreadRow { id, from, subject, snippet, at, unread: *unread, href: href.clone(), count: *count, starred: *starred }
        },
        Kind::Table {
            columns,
            rows,
            query,
        } => rsx! {
            crate::components::Table { id, columns: columns.clone(), rows: rows.clone(), query: query.clone() }
        },
    }
}

/// A UI tree, as `checked` returned it. `theme`: the root's classes, `app light` in the window; on the web `app`, its colours
/// the page's root's, which a browser in dark mode switches.
#[derive(Props, Clone, PartialEq)]
pub struct ScreenProps {
    pub tree: String,
    pub theme: &'static str,
}

#[allow(non_snake_case)]
pub fn Screen(ScreenProps { tree, theme }: ScreenProps) -> Element {
    let body = match serde_json::from_str::<Tree>(&tree) {
        Ok(tree) => {
            // the page's one patcher, once a page, outside the tree so a swapped part brings no copy of it: for its skeletons,
            // and for a board that follows a view (task rust-view-patches)
            let busy = (tree.elements.values()).any(|n| {
                matches!(
                    n.element,
                    Kind::Skeleton { .. }
                        | Kind::Tool { reading: 1.., .. }
                        | Kind::Card { reading: true, .. }
                        | Kind::Board { view: Some(_) }
                        | Kind::Terminal { .. }
                ) || matches!(&n.element, Kind::Document(d) if d.busy())
                    || matches!(&n.element, Kind::DocView { doc, .. } if crate::documents::reading(doc))
            });
            let refill = busy.then_some(LIVE_JS);
            rsx! {
                {node(&tree, &tree.root, In::Page)}
                if let Some(js) = refill { script { dangerous_inner_html: js } }
            }
        }
        Err(e) => rsx! { p { "{e}" } },
    };
    rsx! {
        div { class: theme,
            style { {CSS} }
            {body}
        }
    }
}

/// A page render's span, as the binary makes it: the closure finishes the span, `None` while no tracer is set or the
/// switch is off.
type RenderSpan = fn(&str, &str) -> Option<Box<dyn FnOnce()>>;

/// The page render's span, set once by the binary: the surface crate names no tracer (the gate keeps `statsd` the binary's
/// alone).
static RENDER: std::sync::OnceLock<RenderSpan> = std::sync::OnceLock::new();

/// What a page render's span is, set by the binary before it serves. The surface crate holds no tracer of its own.
pub fn set_render_span(render: RenderSpan) {
    let _ = RENDER.set(render);
}

/// The whole web page of a UI tree, its body the component's server render. Under a request's root span, the render is a
/// child of it (task rust-dd-spans): a page's time in Datadog is the render's own.
pub fn page(title: &str, tree: &str) -> Result<String, String> {
    let span = RENDER
        .get()
        .and_then(|render| render("surface.page", title));
    let drawn = || -> Result<String, String> {
        let tree = checked(tree)?;
        let mut dom = VirtualDom::new_with_props(Screen, ScreenProps { tree, theme: "app" });
        dom.rebuild_in_place();
        let body = dioxus_ssr::render(&dom);
        let title = escaped(title);
        Ok(format!(
            r#"<!doctype html><html lang="en" class="app light"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><script>{DARK}</script></head><body>{body}</body></html>"#
        ))
    };
    let answer = drawn();
    if let Some(span) = span {
        span();
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text;
    use dash_meter::memory::TestBudget;

    const LIST: &str = r#"{"root":"page","elements":{
        "page":{"type":"stack","children":["title","todo"]},
        "title":{"type":"heading","props":{"text":"Rust tasks"}},
        "todo":{"type":"issue-group","props":{"status":"todo"},"children":["r0"]},
        "r0":{"type":"issue-row","props":{"id":"a b","title":"<Tom & Jerry>","status":"todo","says":"next","comments":2}}}}"#;

    #[test]
    fn the_web_page_is_the_component_rendered() {
        let _budget = TestBudget::new();
        let html = page("Rust tasks", LIST).expect("a page");
        assert!(html.starts_with("<!doctype html>"), "{html}");
        for part in [
            r#"<h1 id="title">Rust tasks</h1>"#,
            r#"class="issue-group" id="todo" data-status="todo""#,
            r#"<a id="r0" class="item issue-row" data-task="a b" href="/tasks/a%20b">"#,
            // dioxus-ssr's escapes are numeric
            r#"<span class="title">&#60;Tom &#38; Jerry&#62;</span>"#,
            r#"data-comments="2">2 comments</span>"#,
            r#"<span class="badge count">1</span>"#,
            ".issue-row {",
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
    }

    /// rust/CLAUDE.md rule 4: the text is the same tree's, never a second writer.
    #[test]
    fn the_text_view_is_the_tree_read_in_order() {
        let _budget = TestBudget::new();
        assert_eq!(
            text(LIST).expect("a text"),
            "Rust tasks\n\nTodo (1)\na b\t<Tom & Jerry>\tnext\t2 comments\n"
        );
    }

    /// Done-when 2 of rust-nav-board: the board's columns and its views, in the HTML and the text of one tree.
    #[test]
    fn the_board_draws_its_columns_and_names_its_other_views() {
        let _budget = TestBudget::new();
        let board = r#"{"root":"page","elements":{
            "page":{"type":"stack","children":["views","board"]},
            "views":{"type":"task-views","props":{"current":"board"}},
            "board":{"type":"board","props":{},"children":["todo","backlog"]},
            "todo":{"type":"issue-group","props":{"status":"todo"},"children":["r0"]},
            "backlog":{"type":"issue-group","props":{"status":"backlog"}},
            "r0":{"type":"issue-row","props":{"id":"a","title":"A","status":"todo","says":"next","comments":2,"labels":["site_perso"],"assignee":"Rust"}},
            "r1":{"type":"issue-row","props":{"id":"b","title":"B","status":"backlog","says":"","comments":0,"project":"klar"}}}}"#;
        let html = page("Tasks", board).expect("a page");
        for part in [
            // the views' toggle, each option its icon then its name (task rust-tasks-views-toggle)
            r#"<nav class="toggle" id="views" aria-label="Views"><a href="/tasks" aria-current="page"><svg"#,
            r#"</svg>Board</a><a href="/tasks?layout=list"><svg"#,
            r#"</svg>Graph</a></nav>"#,
            r#"<div class="board" id="board"><section class="li-column" id="todo""#,
            ".board {",
            // Backlog's icon is dashed, Todo's whole
            r#"stroke-dasharray="2 2""#,
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        assert_eq!(
            text(board).expect("a text"),
            "Views: Board /tasks (shown), List /tasks?layout=list, Graph /tasks?layout=graph\n\nTodo (1)\na\tA\tnext\t2 comments\t#site_perso\t@Rust\n\nBacklog (0)\n"
        );
    }

    /// Done-when 2 of rust-board-buttons-new-issue: the board's head is its title, then « + New issue » at its right end, a
    /// pill that opens a dialog with no script (`commandfor`), whose form posts a title and a description to /tasks/new with
    /// the board's project; the text form names the same post. The views keep Elixir's pill group (done-when 1).
    #[test]
    fn the_board_head_opens_a_new_issue_form_that_posts_its_words() {
        let _budget = TestBudget::new();
        let board = r#"{"root":"page","elements":{
            "page":{"type":"stack","children":["head","views"]},
            "head":{"type":"bar","children":["title","new"]},
            "title":{"type":"heading","props":{"text":"Tasks: klar"}},
            "new":{"type":"new-issue","props":{"project":"klar"}},
            "views":{"type":"task-views","props":{"current":"board","project":"klar"}}}}"#;
        let html = page("Tasks", board).expect("a page");
        for part in [
            // the order of a tag's attributes is Dioxus's, so each is asserted alone
            r#"<div class="bar" id="head"><h1 id="title">Tasks: klar</h1><button class="button pill""#,
            r#"commandfor="new-dialog""#,
            r#"command="show-modal""#,
            r#"</svg>New issue</button><dialog class="dialog""#,
            r#"closedby="closerequest""#,
            r#"action="/tasks/new""#,
            r#"name="title""#,
            "required=true",
            r#"autofocus="autofocus""#,
            r#"name="description""#,
            r#"value="klar""#,
            r#"command="close""#,
            r#">Cancel</button><button class="button">Create issue</button></div></form></dialog></div>"#,
            // a bar's first part at the left pushes the rest to its right end; the toggle is Elixir's pill group, its current one tinted
            ".bar > :first-child { margin: 0 auto 0 0; }",
            ".toggle { display: inline-flex; max-width: 100%; margin: var(--spacing-s2) 0; border-radius: 999px;",
            ".toggle a[aria-current] { background: color-mix(in oklab, var(--color-primary) 15%, transparent);",
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        assert_eq!(
            text(board).expect("a text"),
            "Tasks: klar\nNew issue: POST /tasks/new title, description, project=klar\nViews: Board /tasks?project=klar (shown), List /tasks?layout=list&project=klar, Graph /tasks?layout=graph\n"
        );
    }

    /// Task rust-page-latency: a column past its first cards counts them all and links the rest after its cards, in the HTML
    /// and the text; a link to another host is not drawn.
    #[test]
    fn a_column_counts_the_cards_it_does_not_draw_and_links_them() {
        let _budget = TestBudget::new();
        let board = |href: &str| {
            format!(
                r#"{{"root":"board","elements":{{
            "board":{{"type":"board","props":{{}},"children":["todo"]}},
            "todo":{{"type":"issue-group","props":{{"status":"todo","more":{{"count":3,"href":"{href}"}}}},"children":["r0"]}},
            "r0":{{"type":"issue-row","props":{{"id":"a","title":"A","status":"todo","says":"","comments":0}}}}}}}}"#
            )
        };
        let html = page("Tasks", &board("/tasks?layout=list#todo")).expect("a page");
        assert!(
            html.contains(r#"Todo<span class="badge meta count">4</span></h2>"#),
            "{html}"
        );
        assert!(
            html.contains(
                r#"</a><a class="li-more" href="/tasks?layout=list#todo">3 more</a></section>"#
            ),
            "{html}"
        );
        assert_eq!(
            text(&board("/tasks?layout=list#todo")).expect("a text"),
            "\nTodo (4)\na\tA\t\n3 more: /tasks?layout=list#todo\n"
        );
        // another host's link is drawn in neither form (/code-review)
        let elsewhere = page("Tasks", &board("//evil.example/x")).expect("a page");
        assert!(!elsewhere.contains(r#"class="li-more""#), "{elsewhere}");
        let said = text(&board("//evil.example/x")).expect("a text");
        assert_eq!(said, "\nTodo (4)\na\tA\t\n", "{said}");
        // a list's group counts and links the same way (/code-review)
        let list =
            board("/tasks?layout=list#todo").replace(r#""root":"board""#, r#""root":"todo""#);
        let html = page("Tasks", &list).expect("a page");
        assert!(
            html.contains(r#"<span class="badge count">4</span>"#)
                && html.contains(
                    r#"<a class="li-more" href="/tasks?layout=list#todo">3 more</a></section>"#
                ),
            "{html}"
        );
    }

    /// Done-when 1 of rust-board-linear: the board's columns and cards are /design?tab=linear's, drawn by its one `face`: the
    /// column's status icon, name and count; each card its id, assignee, status icon, title, labels and line.
    #[test]
    fn the_board_s_cards_are_design_s_linear_cards() {
        let _budget = TestBudget::new();
        let board = r#"{"root":"board","elements":{
            "board":{"type":"board","props":{},"children":["todo"]},
            "todo":{"type":"issue-group","props":{"status":"in-progress"},"children":["r0","r1"]},
            "r0":{"type":"issue-row","props":{"id":"a","title":"A","status":"in-progress","says":"running","comments":1,"labels":["site_perso"],"assignee":"Rust"}},
            "r1":{"type":"issue-row","props":{"id":"b","title":"B","status":"in-progress","says":"","comments":0,"project":"klar","labels":["klar"]}}}}"#;
        let html = page("Tasks", board).expect("a page");
        let head = r#"<h2 class="li-column-head"><svg width="16" height="16" viewBox="0 0 16 16" class="ic tx-warning">"#;
        assert!(html.contains(head), "the column's status icon: {html}");
        assert!(
            html.contains(r#"In Progress<span class="badge meta count">2</span></h2>"#),
            "{html}"
        );
        let card = r#"<a id="r0" class="card li-card" data-task="a" href="/tasks/a"><div class="hover-card" data-state="closed"><div class="li-id">a</div><div class="li-assign" title="Rust"><span class="avatar li-who">R</span></div><div class="li-title"><svg width="16" height="16" viewBox="0 0 16 16" class="ic tx-warning">"#;
        assert!(html.contains(card), "{html}");
        for part in [
            r#"<span>A</span></div><div class="li-tags"><span class="badge li-tag">site_perso</span></div><div class="meta"><span class="says">running</span><span class="comments" data-comments="1"> · 1 comment</span></div><div class="hover-card-content" data-state="closed" hidden=true><b class="title">A</b></div></div></a>"#,
            r#"href="/projects/klar?issue=b"><div class="hover-card" data-state="closed"><div class="li-id">b</div><div class="li-assign" title="No assignee"><svg class="ic""#,
            r#"<span class="badge li-tag">klar</span>"#,
            // cards.css, /design's own: the board wears its rules
            ".li-card {",
            ".li-column-head {",
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        // /design?tab=linear's cards are the same face, its card's parts in the same order
        let design = crate::design::page("linear", None, "");
        let parts = r#"<div class="li-id">AAA-478</div><div class="li-assign" title="No assignee"><svg class="ic""#;
        assert!(design.contains(parts), "{design}");
        assert!(design.contains(r#"<div class="li-tags"><div class="li-urgent">!</div><span class="badge li-tag">dashboard</span></div>"#));
    }

    /// Done-when 2 of rust-ui-controls: a board card's face is a HoverCard's trigger, its content the task's whole title and
    /// its why, hidden until a hover, so the text keeps the card's words alone; a task with no why shows its title.
    #[test]
    fn a_board_card_shows_its_task_in_a_hover_card() {
        let _budget = TestBudget::new();
        let board = r#"{"root":"board","elements":{
            "board":{"type":"board","props":{},"children":["todo"]},
            "todo":{"type":"issue-group","props":{"status":"todo"},"children":["r0","r1"]},
            "r0":{"type":"issue-row","props":{"id":"a","title":"Port the board","status":"todo","says":"","comments":0,"why":"It drifts from the design system."}},
            "r1":{"type":"issue-row","props":{"id":"b","title":"B","status":"todo","says":"","comments":0}}}}"#;
        let html = page("Tasks", board).expect("a page");
        assert!(
            html.contains(r#"<a id="r0" class="card li-card" data-task="a" href="/tasks/a"><div class="hover-card" data-state="closed"><div class="li-id">a</div>"#),
            "{html}"
        );
        let content = |words: &str| {
            format!(
                r#"<div class="hover-card-content" data-state="closed" hidden=true><b class="title">{words}</div></div></a>"#
            )
        };
        assert!(
            html.contains(&content(r#"Port the board</b><span class="description">It drifts from the design system.</span>"#)),
            "{html}"
        );
        assert!(html.contains(&content("B</b>")), "{html}");
        assert!(
            !crate::forms::text_of(&html).contains("It drifts"),
            "{html}"
        );
    }

    /// Done-when 1 of rust-nav-polish: Home's cards, the whole card its link; a link that is no page of ours nor http(s) is none.
    #[test]
    fn a_card_is_its_title_its_line_and_its_link() {
        let _budget = TestBudget::new();
        let cards = r#"{"root":"cards","elements":{
            "cards":{"type":"cards","children":["c0","c1"]},
            "c0":{"type":"card","props":{"title":"Ann <Acme>","line":"acme-intro","href":"/drafts/acme-intro"}},
            "c1":{"type":"card","props":{"title":"Run it","line":"","href":"javascript:alert(1)"}}}}"#;
        assert_eq!(
            crate::card_href(&Some("/\\evil.example".into())),
            None,
            "a backslash is no path of ours"
        );
        let html = page("Home", cards).expect("a page");
        let card = r#"<div class="cards" id="cards"><a id="c0" class="card" href="/drafts/acme-intro"><b class="title">Ann &#60;Acme&#62;</b><span class="description">acme-intro</span></a><div id="c1" class="card"><b class="title">Run it</b><span class="description"></span></div></div>"#;
        assert!(html.contains(card), "{html}");
        assert_eq!(
            text(cards).expect("a text"),
            "Ann <Acme>\tacme-intro\t/drafts/acme-intro\nRun it\n"
        );
    }

    /// The tree as the server hands it for `query` (crate::queried).
    fn ordered(tree: &str, query: &str) -> String {
        match crate::queried(
            crate::Drawing::Tree {
                title: "t".into(),
                tree: tree.into(),
            },
            query,
        ) {
            crate::Drawing::Tree { tree, .. } => tree,
            crate::Drawing::Page(_) => unreachable!("a tree stays a tree"),
        }
    }

    /// Done-when 1 and 2 of rust-table-sortable: a head is a link that sorts by its column, numbers by value, the same head
    /// again reverses it, the page's other keys kept; the sort is in the URL, and the text form orders its rows and says it.
    #[test]
    fn a_head_click_sorts_by_its_column_again_reverses_it_in_the_url_and_the_text() {
        let _budget = TestBudget::new();
        let tree = r#"{"root":"t","elements":{"t":{"type":"table","props":{"columns":["Store","Rows"],"rows":[
            {"cells":["mail","10"]},{"cells":["Tasks","9"]},{"cells":["comments","100"]}]}}}}"#;
        let up = ordered(tree, "tab=x&sort=Rows");
        let html = page("Settings", &up).expect("a page");
        let head = r#"<thead><tr><th scope="col"><a href="?tab=x&#38;sort=Store">Store</a></th><th scope="col" aria-sort="ascending"><a href="?tab=x&#38;sort=Rows&#38;dir=desc">Rows</a></th></tr></thead>"#;
        assert!(html.contains(head), "{html}");
        assert!(
            html.contains("<caption>3 rows; sorted by Rows, ascending.</caption>"),
            "{html}"
        );
        let text = |tree: &str| crate::text(tree).expect("a text");
        assert_eq!(
            text(&up),
            "3 rows; sorted by Rows, ascending.\nStore\tRows\tLink\nTasks\t9\t\nmail\t10\t\ncomments\t100\t\n"
        );
        // the same head again: reversed, and its link back to ascending
        let down = ordered(tree, "tab=x&sort=Rows&dir=desc");
        let html = page("Settings", &down).expect("a page");
        assert!(html.contains(r#"<th scope="col" aria-sort="descending"><a href="?tab=x&#38;sort=Rows">Rows</a></th>"#), "{html}");
        assert_eq!(
            text(&down),
            "3 rows; sorted by Rows, descending.\nStore\tRows\tLink\ncomments\t100\t\nmail\t10\t\nTasks\t9\t\n"
        );
        // words by their letters, case aside; a column the table lacks orders nothing and says nothing
        assert!(
            text(&ordered(tree, "sort=Store"))
                .starts_with("3 rows; sorted by Store, ascending.\nStore\tRows\tLink\ncomments\t100\t\nmail\t10\t\nTasks")
        );
        assert_eq!(text(&ordered(tree, "sort=Nope")), text(&ordered(tree, "")));
        // a name with a space or an ampersand goes through the URL and back
        let spaced = tree.replace("\"Rows\"", "\"In the backup & now\"");
        let html = page("Settings", &ordered(&spaced, "")).expect("a page");
        assert!(
            html.contains(r#"href="?sort=In%20the%20backup%20%26%20now""#),
            "{html}"
        );
        assert!(
            text(&ordered(&spaced, "sort=In+the+backup+%26+now&dir=desc"))
                .starts_with("3 rows; sorted by In the backup & now, descending.\n")
        );
    }

    /// Done-when 3 of rust-table-sortable: past 50 rows dioxus-components' Pagination pages a table, the page in the URL with
    /// the sort kept, the shown one marked, and the text form shows that page's rows and says which.
    #[test]
    fn pagination_pages_a_long_table_the_page_in_the_url_and_the_text() {
        let _budget = TestBudget::new();
        let long = |count: usize| {
            let rows: Vec<_> = (1..=count)
                .map(|n| serde_json::json!({"cells": [n.to_string()]}))
                .collect();
            serde_json::json!({"root": "t", "elements": {"t": {"type": "table", "props": {"columns": ["N"], "rows": rows}}}})
                .to_string()
        };
        let tree = long(120);
        let second = ordered(&tree, "sort=N&dir=desc&page=2");
        let text = crate::text(&second).expect("a text");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1 + 50 + 1, "{text}");
        assert_eq!((lines[2], lines[51]), ("70\t", "21\t"), "{text}");
        assert_eq!(
            lines[0],
            "120 rows; sorted by N, descending; page 2 of 3. Next page: ?sort=N&dir=desc&page=3"
        );
        let html = page("Long", &second).expect("a page");
        let pager = r#"<nav class="pagination" role="navigation" aria-label="pagination" id="t-pages"><ul class="pagination-content"><li><a class="pagination-link" aria-label="Go to previous page" data-kind="previous" href="?sort=N&#38;dir=desc">Previous</a></li><li><a class="pagination-link" href="?sort=N&#38;dir=desc" data-active=false>1</a></li><li><a class="pagination-link" href="?sort=N&#38;dir=desc&#38;page=2" aria-current="page" data-active=true>2</a></li><li><a class="pagination-link" href="?sort=N&#38;dir=desc&#38;page=3" data-active=false>3</a></li><li><a class="pagination-link" aria-label="Go to next page" data-kind="next" href="?sort=N&#38;dir=desc&#38;page=3">Next</a></li></ul></nav>"#;
        assert!(html.contains(pager), "{html}");
        // a sort from any page starts again at the first
        assert!(html.contains(r#"<a href="?sort=N">N</a>"#), "{html}");
        // past the last, the last; unsorted, the text says the page alone; one page, no pager
        let last = crate::text(&ordered(&tree, "page=9")).expect("a text");
        assert!(
            last.starts_with("120 rows; page 3 of 3.\nN\tLink\n101\t\n")
                && last.ends_with("120\t\n"),
            "{last}"
        );
        assert!(
            !page("Long", &ordered(&tree, ""))
                .expect("a page")
                .contains("…"),
            "a window of three pages has no gap"
        );
        let html = page("Long", &ordered(&long(500), "page=5")).expect("a page");
        let shown: Vec<&str> = html
            .split(r#"data-active="#)
            .skip(1)
            .map(|s| &s[s.find('>').expect("a link") + 1..s.find("</a>").expect("its end")])
            .collect();
        assert_eq!(shown, ["1", "4", "5", "6", "10"], "{html}");
        assert_eq!(
            html.matches(r#"class="pagination-ellipsis""#).count(),
            2,
            "{html}"
        );
        let html = page("Short", &ordered(&long(50), "page=2")).expect("a page");
        assert!(
            !html.contains(r#"<nav class="pagination""#)
                && html.contains("<caption>50 rows.</caption>"),
            "{html}"
        );
    }

    /// /code-review of rust-table-sortable: a column of numbers and words sorts, numbers first, never panicking on a cycle;
    /// a tie keeps the page's order both ways; a table handed no query (a form's answer, a CLI's text) shows every row
    /// under plain heads, since its links would GET a POST path.
    #[test]
    fn a_mixed_column_sorts_ties_keep_their_order_and_an_unqueried_table_shows_every_row() {
        let _budget = TestBudget::new();
        let column = |cells: &[&str]| {
            let rows: Vec<_> = cells
                .iter()
                .map(|c| serde_json::json!({"cells": [c, cells.len().to_string()]}))
                .collect();
            serde_json::json!({"root": "t", "elements": {"t": {"type": "table", "props": {"columns": ["V", "Tie"], "rows": rows}}}})
                .to_string()
        };
        let firsts = |tree: &str| -> Vec<String> {
            let text = crate::text(tree).expect("a text");
            // past its line, when it has one, and its head
            text.lines()
                .skip_while(|l| !l.starts_with("V\t"))
                .skip(1)
                .filter_map(|l| l.split_once('\t'))
                .map(|(v, _)| v.to_string())
                .collect()
        };
        let mixed = column(&["1a", "10", "inf", "2", "NaN", "-3"]);
        assert_eq!(
            firsts(&ordered(&mixed, "sort=V")),
            ["-3", "2", "10", "1a", "inf", "NaN"]
        );
        assert_eq!(
            firsts(&ordered(&mixed, "sort=V&dir=desc")),
            ["NaN", "inf", "1a", "10", "2", "-3"]
        );
        let tied = column(&["b", "a", "c"]);
        assert_eq!(firsts(&ordered(&tied, "sort=Tie")), ["b", "a", "c"]);
        assert_eq!(
            firsts(&ordered(&tied, "sort=Tie&dir=desc")),
            ["b", "a", "c"]
        );
        let cells: Vec<String> = (1..=60).map(|n| n.to_string()).collect();
        let long = column(&cells.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(
            firsts(&long).len(),
            60,
            "unqueried: every row, no page line"
        );
        let html = page("Restore", &long).expect("a page");
        assert!(
            html.contains(r#"<th scope="col">V</th>"#)
                && !html.contains(r#"<nav class="pagination""#),
            "{html}"
        );
    }

    /// Task rust-comments-table: a table is its head then a row each, in both forms; a row's first cell its link where a card's
    /// would be one, every text record one shape; a row of the wrong width, or a child, refused.
    #[test]
    fn a_table_is_its_head_then_a_row_each_its_first_cell_the_link() {
        let _budget = TestBudget::new();
        let table = r#"{"root":"q","elements":{"q":{"type":"table","props":{"columns":["Comment","Page"],"rows":[
            {"cells":["Ann <Acme>","/deps"],"href":"/deps#c-a"},
            {"cells":["Run it","/x"],"href":"javascript:alert(1)"}]}}}}"#;
        let html = page("Comments", table).expect("a page");
        let drawn = r#"<table class="table" id="q"><thead><tr><th scope="col">Comment</th><th scope="col">Page</th></tr></thead><tbody><tr><td><a href="/deps#c-a">Ann &#60;Acme&#62;</a></td><td>/deps</td></tr><tr><td>Run it</td><td>/x</td></tr></tbody></table>"#;
        assert!(html.contains(drawn), "{html}");
        assert_eq!(
            text(table).expect("a text"),
            "Comment\tPage\tLink\nAnn <Acme>\t/deps\t/deps#c-a\nRun it\t/x\t\n"
        );
        assert_eq!(page("Comments", table).map(|h| h.contains(drawn)), Ok(true));
        let wide = table.replace(r#"["Run it","/x"]"#, r#"["Run it"]"#);
        assert_eq!(
            page("Comments", &wide),
            Err("q: a row of 1 cells under 2 columns".into())
        );
        let parent = r#"{"root":"q","elements":{"q":{"type":"table","props":{"columns":[],"rows":[]},"children":["t"]},
            "t":{"type":"text","props":{"text":"x"}}}}"#;
        assert_eq!(
            page("Comments", parent),
            Err("q: a table holds no children".into())
        );
    }

    #[test]
    /// Task rust-page-forms: the component draws every element of the catalog, so no page needs the string renderer; a tree
    /// the walk refuses is refused here too.
    fn every_element_is_drawn_and_a_broken_tree_refused() {
        let _budget = TestBudget::new();
        let tree = r#"{"root":"p","elements":{
            "p":{"type":"stack","children":["s","g","d","ac"]},
            "s":{"type":"search-bar","props":{"placeholder":"Find"}},
            "g":{"type":"task-graph","props":{"tasks":[{"id":"a","state":"next","says":"go","waits":[]}]}},
            "d":{"type":"document","props":{"kind":"task-brief","id":"t<1>","title":"T","status":"todo","says":"next","why":"w","checks":[{"text":"x","met":null}],"no_checks":"","run":"r","pr":"https://github.com/o/r/pull/7"}},
            "ac":{"type":"access","props":{"name":"fly <x>","status":"expired","usage":"no reading","note":"401","place":"~/.fly","source":"cli"}}}}"#;
        let html = page("t", tree).expect("a page");
        for part in [
            r#"role="search""#,
            r#"data-task="a""#,
            // a document, by its type's template (task rust-doc-types)
            r#"<article id="d" class="doc doc-task-brief">"#,
            r#"<a href="/tasks">Tasks</a><span class="meta">›</span><span class="li-crumb-title">"#,
            r#"<span class="meta li-crumb-key">t&#60;1&#62;</span>"#,
            r#"<div data-status="todo"><div class="li-issue">"#,
            r#"<div class="li-check" title="not judged">"#,
            r#"<a class="li-tag li-pr" href="https://github.com/o/r/pull/7">"#,
            // an access: its status a chip before its name (task rust-tools-human)
            r#"<div id="ac" class="item access" data-status="expired"><p class="head"><span class="badge chip">expired</span><b class="name">fly &#60;x&#62;</b></p><p class="usage">no reading</p><p class="note">401</p><p class="place"><span class="source">cli</span> ~/.fly</p></div>"#,
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        // and one table line in the text, its fields a tab apart
        let said = text(tree).expect("a text");
        let brief = "Tasks\t› T t<1>\nCopy link: /tasks/t%3C1%3E\tCopy ID: t<1>\tCopy branch name: task/t<1>\nT\nw\nDone when\nnot judged: x\nLast run\nr\nActivity\nUndo\tRedo\nPreview\nDiscard\tComment\nProperties\nTodo\nnext\nNo priority\nSet priority: POST /tasks/t%3C1%3E/set priority=\n1: Urgent\n2: High\n3: Medium\n4: Low\nnone: No priority (set)\nNo due date\nSet due date: POST /tasks/t%3C1%3E/set due=\nnone: No due date (set)\nPull requests\n#7\thttps://github.com/o/r/pull/7\n";
        assert!(
            said.contains(brief),
            "the brief's text, the same tree: {said:?}"
        );
        assert!(
            said.ends_with("\nfly <x>\texpired\tno reading\t401\t~/.fly\tcli\n"),
            "{said}"
        );
        let looped = r#"{"root":"a","elements":{"a":{"type":"stack","children":["a"]}}}"#;
        assert!(page("t", looped).is_err());
    }

    /// Done-when 2 of rust-terminal-no-hc, its markup: an alert is the design system's Alert, its title then its words, a
    /// status; its text form the title, then the words.
    #[test]
    fn an_alert_draws_its_title_then_its_words() {
        let _budget = TestBudget::new();
        let tree = serde_json::json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["note"]},
            "note": {"type": "alert", "props": {"title": "No hc to run", "text": "Install <hc> at ~/.local/bin/hc."}}}})
        .to_string();
        let html = page("t7", &tree).expect("a page");
        let alert = r#"<div class="alert" role="status" id="note"><b class="alert-title">No hc to run</b><p>Install &#60;hc&#62; at ~/.local/bin/hc.</p></div>"#;
        assert!(html.contains(alert), "{html}");
        assert_eq!(
            text(&tree).expect("a text"),
            "No hc to run\nInstall <hc> at ~/.local/bin/hc.\n"
        );
        let held = serde_json::json!({"root": "note", "elements": {
            "note": {"type": "alert", "children": ["leaf"], "props": {"title": "t", "text": "x"}},
            "leaf": {"type": "text", "props": {"text": "x"}}}})
        .to_string();
        let refused = page("t", &held).expect_err("an alert holds no children");
        assert!(refused.contains("an alert holds no children"), "{refused}");
    }

    /// Done-when 1 of rust-terminal-page: a terminal's screen is drawn with the design system's Terminal, its lines as read and
    /// escaped, the screen alone following the view; its text form holds the very same lines, then where a line is posted.
    #[test]
    fn a_terminal_draws_its_screen_and_its_text_holds_the_same_lines() {
        let _budget = TestBudget::new();
        let screen = "$ echo <hi> & bye\n<hi> & bye\n  indented  \n$ ";
        let tree = serde_json::json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["term"]},
            "term": {"type": "terminal", "props": {"text": screen, "action": "/terminal/t7/send"}}}})
        .to_string();
        let html = page("t7", &tree).expect("a page");
        for part in [
            r#"<div class="terminal" id="term"><pre class="terminal-screen" data-view="terminal" id="term-screen">$ echo &#60;hi&#62; &#38; bye"#,
            r#"<form class="form" method="post" action="/terminal/t7/send"><input class="input" autocomplete="off" name="text""#,
            "window.dashLive",
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        let said = text(&tree).expect("a text");
        assert_eq!(
            said,
            format!(
                "{}\n\nType a line: POST /terminal/t7/send, the line as text\n",
                screen.trim_end_matches('\n')
            ),
        );
        let lines: Vec<&str> = said.lines().take(screen.lines().count()).collect();
        assert_eq!(lines, screen.lines().collect::<Vec<_>>(), "the same lines");
        let outside = tree.replace("/terminal/t7/send", "//elsewhere/send");
        assert!(
            page("t", &outside).is_err(),
            "a line posts to this app only"
        );
    }
}
