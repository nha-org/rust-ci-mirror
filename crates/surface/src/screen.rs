//! The one Dioxus component of a UI tree (task rust-dioxus-only-app; @nha, 2026-11-08: « DIOXUS now. And that serves the web
//! page too. »): dash serve renders it to HTML here, and dash ui's window draws the very same component with Dioxus Native.
//! It draws every element of the catalog, so every page's HTML is this component's (task rust-page-forms).
use super::{
    DARK, Element as Kind, IssueStatus, TaskView, Tree, card_href, escaped, graph_svg, pr_number,
    render, task_href, verdict,
};
use crate::design::{families::status as linear_status, linear::face};
use dioxus::prelude::*;

/// Both stylesheets inside the component, so the window, which has no page around it, gets them too.
/// The root fills the window as the page: the window's html has no theme of its own, so the root carries the background
/// page.css gives html, and the body's padding, as rust/ui-spike's page does.
const CSS: &str = concat!(
    include_str!("theme.css"),
    include_str!("design/cards.css"),
    include_str!("page.css"),
    "body { padding: 0; } div.app { box-sizing: border-box; min-height: 100vh; padding: var(--spacing-s4); background: var(--color-base-200); color: var(--color-base-content); }"
);

fn parsed(tree: &str) -> Result<Tree, String> {
    // The older renderer's walk is the check: a cycle, a missing id, a nesting past its limit or a link that is not GitHub's
    // is refused there first, so `node` draws a tree known whole.
    render(tree)?;
    serde_json::from_str(tree).map_err(|e| e.to_string())
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

fn icon(status: &IssueStatus) -> Element {
    rsx! { span { class: "status-icon {status.class()}" } }
}

/// An element and its children. `board`: inside the board, where a status is a column and a task a card, /design?tab=linear's
/// (task rust-board-linear).
fn node(tree: &Tree, id: &str, board: bool) -> Element {
    let Some(n) = tree.elements.get(id) else {
        return rsx! {};
    };
    let board = board || matches!(n.element, Kind::Board);
    let children = n.children.iter().map(|child| node(tree, child, board));
    match &n.element {
        Kind::IssueGroup { status } if board => rsx! {
            section { id, class: "li-column", "data-status": status.class(),
                h2 { class: "li-column-head", {linear_status(status.kind())} "{status.name()}" span { class: "meta count", "{n.children.len()}" } }
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
            labels,
            assignee,
        } if board => {
            let href = task_href(task, project.as_deref());
            let said = comments_said(*comments);
            let line = rsx! {
                span { class: "says", "{says}" }
                if let Some(said) = said {
                    span { class: "comments", "data-comments": "{comments}", " · {said}" }
                }
            };
            rsx! {
                a { id, class: "li-card", "data-task": "{task}", href,
                    {face(task, title, status.kind(), labels, assignee.as_deref(), false, line)}
                }
            }
        }
        Kind::Stack => rsx! { div { id, {children} } },
        Kind::Heading { text } => rsx! { h1 { id, "{text}" } },
        Kind::Text { text } => rsx! { p { id, "{text}" } },
        Kind::IssueGroup { status } => rsx! {
            section { id, class: "issue-group", "data-status": status.class(),
                h2 { {icon(status)} "{status.name()}" span { class: "count", "{n.children.len()}" } }
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
                a { id, class: "issue-row", "data-task": "{task}", href,
                    {icon(status)}
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
        Kind::Board => rsx! { div { id, class: "board", {children} } },
        Kind::Cards => rsx! { div { id, class: "cards", {children} } },
        Kind::Card { title, line, href } => match card_href(href) {
            Some(href) => rsx! {
                a { id, class: "card", href,
                    b { class: "title", "{title}" }
                    span { class: "line", "{line}" }
                }
            },
            None => rsx! {
                div { id, class: "card",
                    b { class: "title", "{title}" }
                    span { class: "line", "{line}" }
                }
            },
        },
        Kind::TaskViews { current, project } => {
            let links = TaskView::ALL.map(|v| {
                let on = (v == *current).then_some("page");
                rsx! { a { href: v.href(project.as_deref()), "aria-current": on, "{v.name()}" } }
            });
            rsx! { nav { id, class: "task-views", "aria-label": "Views", {links.into_iter()} } }
        }
        Kind::Section { title } => rsx! {
            section { id, class: "issue-part",
                h2 { "{title}" }
                {children}
            }
        },
        Kind::SearchBar { placeholder } => rsx! {
            form { id, role: "search", action: "/search",
                input { r#type: "search", name: "q", placeholder: "{placeholder}" }
                button { "Search" }
            }
        },
        // ponytail: the graph's SVG is the string renderer's, its words escaped there, inner HTML that only a web page shows;
        // the window draws no graph today (its tree is /tasks?view=tree's list). An rsx port when the window shows one.
        // `parsed` refused a graph `graph_svg` refuses, so its error is never met here.
        Kind::TaskGraph { tasks } => {
            let svg = graph_svg(&escaped(id), tasks).unwrap_or_default();
            rsx! { div { dangerous_inner_html: svg } }
        }
        Kind::IssueBar { id: task } => rsx! {
            nav { id, class: "issue-bar", a { href: "/tasks", "Tasks" } " > {task}" }
        },
        Kind::IssueState { status, says } => rsx! {
            p { id, class: "issue-state", "data-status": status.class(), {icon(status)} b { "{status.name()}" } " {says}" }
        },
        Kind::Check { text, met } => {
            let (class, word) = verdict(*met);
            rsx! { p { id, class: "check {class}", span { class: "word", "{word}" } " {text}" } }
        }
        Kind::PullRequest { url } => {
            let number = pr_number(id, url).unwrap_or_default();
            rsx! { p { id, a { class: "pr", href: "{url}", "#{number}" } } }
        }
        Kind::Row { cells } => rsx! {
            p { id, class: "row", for cell in cells { span { class: "cell", "{cell}" } } }
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
        Ok(tree) => node(&tree, &tree.root, false),
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
            r#"href="/tasks/a%20b""#,
            // dioxus-ssr's escapes are numeric
            r#"<span class="title">&#60;Tom &#38; Jerry&#62;</span>"#,
            r#"data-comments="2">2 comments</span>"#,
            r#"<span class="count">1</span>"#,
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
            html.contains(r#"In Progress<span class="meta count">2</span></h2>"#),
            "{html}"
        );
        let card = r#"<a class="li-card" id="r0" data-task="a" href="/tasks/a"><div class="li-id">a</div><div class="li-assign" title="Rust"><span class="li-who">R</span></div><div class="li-title"><svg width="16" height="16" viewBox="0 0 16 16" class="ic tx-warning">"#;
        assert!(html.contains(card), "{html}");
        for part in [
            r#"<span>A</span></div><div class="li-tags"><span class="li-tag">site_perso</span></div><div class="meta"><span class="says">running</span><span class="comments" data-comments="1"> · 1 comment</span></div></a>"#,
            r#"href="/projects/klar?issue=b"><div class="li-id">b</div><div class="li-assign" title="No assignee"><svg class="ic""#,
            r#"<span class="li-tag">klar</span>"#,
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
        assert!(design.contains(r#"<div class="li-tags"><div class="li-urgent">!</div><span class="li-tag">dashboard</span></div>"#));
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
        let card = r#"<div class="cards" id="cards"><a class="card" id="c0" href="/d/acme-intro"><b class="title">Ann &#60;Acme&#62;</b><span class="line">acme-intro</span></a><div class="card" id="c1"><b class="title">Run it</b><span class="line"></span></div></div>"#;
        assert!(html.contains(card), "{html}");
        assert_eq!(
            text(cards).expect("a text"),
            "Ann <Acme>\tacme-intro\t/d/acme-intro\nRun it\n"
        );
        assert_eq!(
            crate::render(cards)
                .map(|h| h.contains(r#"<a id="c0" class="card" href="/d/acme-intro">"#)),
            Ok(true)
        );
    }

    #[test]
    /// Task rust-page-forms: the component draws every element of the catalog, so no page needs the string renderer; a tree
    /// the walk refuses is refused here too.
    fn every_element_is_drawn_and_a_broken_tree_refused() {
        let _budget = TestBudget::new();
        let tree = r#"{"root":"p","elements":{
            "p":{"type":"stack","children":["s","g","b","st","c","pr","r"]},
            "s":{"type":"search-bar","props":{"placeholder":"Find"}},
            "g":{"type":"task-graph","props":{"tasks":[{"id":"a","state":"next","says":"go","waits":[]}]}},
            "b":{"type":"issue-bar","props":{"id":"t<1>"}},
            "st":{"type":"issue-state","props":{"status":"todo","says":"next"}},
            "c":{"type":"check","props":{"text":"x","met":null}},
            "pr":{"type":"pull-request","props":{"url":"https://github.com/o/r/pull/7"}},
            "r":{"type":"row","props":{"cells":["gh","ok"]}}}}"#;
        let html = page("t", tree).expect("a page");
        for part in [
            r#"role="search""#,
            r#"data-task="a""#,
            "Tasks</a> &#62; t&#60;1&#62;",
            r#"data-status="todo""#,
            r#"class="check unjudged"><span class="word">not judged</span> x"#,
            r#"class="pr" href="https://github.com/o/r/pull/7">#7"#,
            r#"<span class="cell">gh</span><span class="cell">ok</span>"#,
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        let looped = r#"{"root":"a","elements":{"a":{"type":"stack","children":["a"]}}}"#;
        assert!(page("t", looped).is_err());
    }
}
