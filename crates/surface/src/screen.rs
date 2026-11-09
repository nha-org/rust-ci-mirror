//! The one Dioxus component of a UI tree (task rust-dioxus-only-app; @nha, 2026-11-08: « DIOXUS now. And that serves the web
//! page too. »): dash serve renders it to HTML here, and dash ui's window draws the very same component with Dioxus Native.
//! It draws every element of the catalog, so every page's HTML is this component's (task rust-page-forms).
use super::{
    DARK, Element as Kind, TaskView, Tree, card_href, check, escaped, graph_svg, task_href,
};
use crate::components::{
    Access, Badge, Button, Card, CardDescription, CardTitle, Diagram, Form, Input, Item, LIVE_JS,
    Live, Skeleton, Tab, Tabs, Tool,
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
        Kind::Board => In::Board,
        Kind::List => In::List,
        _ => inside,
    };
    let board = inside == In::Board;
    let children = n.children.iter().map(|child| node(tree, child, inside));
    match &n.element {
        Kind::IssueGroup { status } if board => {
            crate::board::column(id, status, n.children.len(), rsx! { {children} })
        }
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
        } if board => crate::board::card(
            id,
            crate::board::Face {
                task,
                title,
                at: status,
                says,
                comments: *comments,
                said: comments_said(*comments),
                href: task_href(task, project.as_deref()),
                labels,
                assignee: assignee.as_deref(),
                movable: *movable,
            },
        ),
        Kind::Stack => rsx! { div { id, {children} } },
        Kind::Heading { text } => rsx! { h1 { id, "{text}" } },
        Kind::Text { text } => rsx! { p { id, "{text}" } },
        Kind::IssueGroup { status } => rsx! {
            section { id, class: "issue-group", "data-status": status.class(),
                h2 { {status_icon(status)} "{status.name()}" Badge { look: "count", "{n.children.len()}" } }
                {children}
            }
        },
        Kind::IssueRow {
            id: task,
            title,
            status,
            says,
            comments,
            project,
            ..
        } => {
            let href = task_href(task, project.as_deref());
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
        Kind::Board => crate::board::board(id, rsx! { {children} }),
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
        Kind::TaskViews { current, project } => {
            let links = TaskView::ALL.map(|v| {
                rsx! { Tab { href: v.href(project.as_deref()), current: v == *current, "{v.name()}" } }
            });
            rsx! { Tabs { id, label: "Views", {links.into_iter()} } }
        }
        Kind::Section { title } => rsx! {
            section { id, class: "issue-part",
                h2 { "{title}" }
                {children}
            }
        },
        Kind::SearchBar { placeholder } => rsx! {
            Form { id, role: "search", method: "get", action: "/search",
                Input { r#type: "search", name: "q", placeholder: "{placeholder}" }
                Button { "Search" }
            }
        },
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
        Kind::Mail { .. } | Kind::Compose { .. } => super::mail_parts(id, &n.element),
        Kind::Table { columns, rows } => rsx! {
            crate::components::Table { id, columns: columns.clone(), rows: rows.clone() }
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
            // the skeletons' one refill loop, once a page, outside the tree so a swapped part brings no copy of it
            let busy = (tree.elements.values()).any(|n| {
                matches!(
                    n.element,
                    Kind::Skeleton { .. }
                        | Kind::Tool { reading: 1.., .. }
                        | Kind::Card { reading: true, .. }
                ) || matches!(&n.element, Kind::Document(d) if d.busy())
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

/// The whole web page of a UI tree, its body the component's server render.
pub fn page(title: &str, tree: &str) -> Result<String, String> {
    let tree = checked(tree)?;
    let mut dom = VirtualDom::new_with_props(Screen, ScreenProps { tree, theme: "app" });
    dom.rebuild_in_place();
    let body = dioxus_ssr::render(&dom);
    let title = escaped(title);
    Ok(format!(
        r#"<!doctype html><html lang="en" class="app light"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><script>{DARK}</script></head><body>{body}</body></html>"#
    ))
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
            "board":{"type":"board","children":["todo","backlog"]},
            "todo":{"type":"issue-group","props":{"status":"todo"},"children":["r0"]},
            "backlog":{"type":"issue-group","props":{"status":"backlog"}},
            "r0":{"type":"issue-row","props":{"id":"a","title":"A","status":"todo","says":"next","comments":2,"labels":["site_perso"],"assignee":"Rust"}},
            "r1":{"type":"issue-row","props":{"id":"b","title":"B","status":"backlog","says":"","comments":0,"project":"klar"}}}}"#;
        let html = page("Tasks", board).expect("a page");
        for part in [
            r#"<a href="/tasks" aria-current="page">Board</a><a href="/tasks?layout=list">List</a><a href="/tasks?layout=graph">Graph</a>"#,
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

    /// Done-when 1 of rust-board-linear: the board's columns and cards are /design?tab=linear's, drawn by its one `face`: the
    /// column's status icon, name and count; each card its id, assignee, status icon, title, labels and line.
    #[test]
    fn the_board_s_cards_are_design_s_linear_cards() {
        let _budget = TestBudget::new();
        let board = r#"{"root":"board","elements":{
            "board":{"type":"board","children":["todo"]},
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
        let card = r#"<a id="r0" class="card li-card" data-task="a" href="/tasks/a"><div class="li-id">a</div><div class="li-assign" title="Rust"><span class="avatar li-who">R</span></div><div class="li-title"><svg width="16" height="16" viewBox="0 0 16 16" class="ic tx-warning">"#;
        assert!(html.contains(card), "{html}");
        for part in [
            r#"<span>A</span></div><div class="li-tags"><span class="badge li-tag">site_perso</span></div><div class="meta"><span class="says">running</span><span class="comments" data-comments="1"> · 1 comment</span></div></a>"#,
            r#"href="/projects/klar?issue=b"><div class="li-id">b</div><div class="li-assign" title="No assignee"><svg class="ic""#,
            r#"<span class="badge li-tag">klar</span>"#,
            // cards.css, /design's own: the board wears its rules
            ".li-card {",
            ".li-column-head {",
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        // /design?tab=linear's cards are the same face, its card's parts in the same order
        let design = crate::design::page("linear", None);
        let parts = r#"<div class="li-id">AAA-478</div><div class="li-assign" title="No assignee"><svg class="ic""#;
        assert!(design.contains(parts), "{design}");
        assert!(design.contains(r#"<div class="li-tags"><div class="li-urgent">!</div><span class="badge li-tag">dashboard</span></div>"#));
    }

    /// Done-when 1 of rust-nav-polish: Home's cards, the whole card its link; a link that is no page of ours nor http(s) is none.
    #[test]
    fn a_card_is_its_title_its_line_and_its_link() {
        let _budget = TestBudget::new();
        let cards = r#"{"root":"cards","elements":{
            "cards":{"type":"cards","children":["c0","c1"]},
            "c0":{"type":"card","props":{"title":"Ann <Acme>","line":"acme-intro","href":"/d/acme-intro"}},
            "c1":{"type":"card","props":{"title":"Run it","line":"","href":"javascript:alert(1)"}}}}"#;
        assert_eq!(
            crate::card_href(&Some("/\\evil.example".into())),
            None,
            "a backslash is no path of ours"
        );
        let html = page("Home", cards).expect("a page");
        let card = r#"<div class="cards" id="cards"><a id="c0" class="card" href="/d/acme-intro"><b class="title">Ann &#60;Acme&#62;</b><span class="description">acme-intro</span></a><div id="c1" class="card"><b class="title">Run it</b><span class="description"></span></div></div>"#;
        assert!(html.contains(card), "{html}");
        assert_eq!(
            text(cards).expect("a text"),
            "Ann <Acme>\tacme-intro\t/d/acme-intro\nRun it\n"
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
            r#"<a href="/tasks">Tasks</a><span class="meta">›</span><span>t&#60;1&#62;</span>"#,
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
        let brief = "Tasks\t› t<1>\nT\nw\nDone when\nnot judged: x\nLast run\nr\nProperties\nTodo\nnext\nPull requests\n#7\thttps://github.com/o/r/pull/7\n";
        assert!(
            said.contains(brief),
            "the brief's text, the same tree: {said}"
        );
        assert!(
            said.ends_with("\nfly <x>\texpired\tno reading\t401\t~/.fly\tcli\n"),
            "{said}"
        );
        let looped = r#"{"root":"a","elements":{"a":{"type":"stack","children":["a"]}}}"#;
        assert!(page("t", looped).is_err());
    }
}
