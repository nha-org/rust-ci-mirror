//! The one Dioxus component of a UI tree (task rust-dioxus-only-app; @nha, 2026-11-08: « DIOXUS now. And that serves the web
//! page too. »): dash serve renders it to HTML here, and dash ui's window draws the very same component with Dioxus Native.
//! Only the tasks list's elements are drawn so far; a tree holding any other is refused, so no page half-renders.
use super::{
    DARK, Element as Kind, IssueStatus, TaskView, Tree, card_href, card_text, escaped, render,
    task_href, views_text,
};
use dioxus::prelude::*;

/// Both stylesheets inside the component, so the window, which has no page around it, gets them too.
/// The root fills the window as the page: the window's html has no theme of its own, so the root carries the background
/// page.css gives html, and the body's padding, as rust/ui-spike's page does.
const CSS: &str = concat!(
    include_str!("theme.css"),
    include_str!("page.css"),
    "body { padding: 0; } div.app { box-sizing: border-box; min-height: 100vh; padding: var(--spacing-s4); background: var(--color-base-200); color: var(--color-base-content); }"
);

fn parsed(tree: &str) -> Result<Tree, String> {
    // The older renderer's walk is the check: a cycle, a missing id or a nesting past its limit is refused there first.
    render(tree)?;
    let tree: Tree = serde_json::from_str(tree).map_err(|e| e.to_string())?;
    let drawn = |e: &Kind| {
        matches!(
            e,
            Kind::Stack
                | Kind::Heading { .. }
                | Kind::Text { .. }
                | Kind::IssueGroup { .. }
                | Kind::IssueRow { .. }
                | Kind::Quote { .. }
                | Kind::Board
                | Kind::TaskViews { .. }
                | Kind::Section { .. }
                | Kind::Cards
                | Kind::Card { .. }
        )
    };
    match tree.elements.iter().find(|(_, n)| !drawn(&n.element)) {
        Some((id, _)) => Err(format!("{id}: not drawn by Dioxus yet")),
        None => Ok(tree),
    }
}

/// A tree checked for the component: what `Screen` is given.
pub fn checked(tree: &str) -> Result<String, String> {
    parsed(tree).map(|_| tree.to_string())
}

fn icon(status: &IssueStatus) -> Element {
    rsx! { span { class: "status-icon {status.class()}" } }
}

fn node(tree: &Tree, id: &str) -> Element {
    let Some(n) = tree.elements.get(id) else {
        return rsx! {};
    };
    let children = n.children.iter().map(|child| node(tree, child));
    match &n.element {
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
        } => {
            let href = task_href(task, project.as_deref());
            let said = match comments {
                0 => None,
                1 => Some("1 comment".to_string()),
                n => Some(format!("{n} comments")),
            };
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
        // checked() refused it
        _ => rsx! {},
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
        Ok(tree) => node(&tree, &tree.root),
        Err(e) => rsx! { p { "{e}" } },
    };
    rsx! {
        div { class: theme,
            style { {CSS} }
            {body}
        }
    }
}

/// A tree's text, one line an element in reading order: what a client that is no browser reads at the page's URL, so the
/// words are written once, in the tree (rust/CLAUDE.md rule 4). A row reads as its id, title and state, a tab between.
pub fn text(tree: &str) -> Result<String, String> {
    fn walk(tree: &Tree, id: &str, out: &mut String) {
        let Some(n) = tree.elements.get(id) else {
            return;
        };
        match &n.element {
            Kind::Heading { text } | Kind::Text { text } | Kind::Quote { text } => {
                *out += text;
                out.push('\n');
            }
            Kind::IssueGroup { status } => {
                *out += &format!("\n{} ({})\n", status.name(), n.children.len())
            }
            Kind::TaskViews { current, project } => {
                *out += &views_text(*current, project.as_deref());
                out.push('\n');
            }
            Kind::Section { title } => *out += &format!("\n{title}\n"),
            Kind::Card { title, line, href } => {
                *out += &card_text(title, line, card_href(href));
                out.push('\n');
            }
            Kind::IssueRow {
                id,
                title,
                says,
                comments,
                ..
            } => {
                *out += &format!("{id}\t{title}\t{says}");
                if *comments > 0 {
                    *out += &format!("\t{comments} comments");
                }
                out.push('\n');
            }
            _ => {}
        }
        for child in &n.children {
            walk(tree, child, out);
        }
    }
    let tree = parsed(tree)?;
    let mut out = String::new();
    walk(&tree, &tree.root, &mut out);
    Ok(out)
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
            "r0":{"type":"issue-row","props":{"id":"a","title":"A","status":"todo","says":"next","comments":0}}}}"#;
        let html = page("Tasks", board).expect("a page");
        for part in [
            r#"<a href="/tasks" aria-current="page">Board</a><a href="/tasks?layout=list">List</a><a href="/tasks?layout=graph">Graph</a>"#,
            r#"<div class="board" id="board"><section class="issue-group" id="todo""#,
            ".board {",
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        assert_eq!(
            text(board).expect("a text"),
            "Views: Board /tasks (shown), List /tasks?layout=list, Graph /tasks?layout=graph\n\nTodo (1)\na\tA\tnext\n\nBacklog (0)\n"
        );
    }

    /// Done-when 1 of rust-nav-polish: Home's cards, the whole card its link; a link that is no page of ours nor http(s) is none.
    #[test]
    fn a_card_is_its_title_its_line_and_its_link() {
        let _budget = TestBudget::new();
        let cards = r#"{"root":"cards","elements":{
            "cards":{"type":"cards","children":["c0","c1"]},
            "c0":{"type":"card","props":{"title":"Ann <Acme>","line":"acme-intro","href":"/d/acme-intro"}},
            "c1":{"type":"card","props":{"title":"Run it","line":"","href":"javascript:alert(1)"}}}}"#;
        assert_eq!(crate::card_href(&Some("/\\evil.example".into())), None, "a backslash is no path of ours");
        let html = page("Home", cards).expect("a page");
        let card = r#"<div class="cards" id="cards"><a class="card" id="c0" href="/d/acme-intro"><b class="title">Ann &#60;Acme&#62;</b><span class="line">acme-intro</span></a><div class="card" id="c1"><b class="title">Run it</b><span class="line"></span></div></div>"#;
        assert!(html.contains(card), "{html}");
        assert_eq!(text(cards).expect("a text"), "- Ann <Acme>: acme-intro /d/acme-intro\n- Run it\n");
        assert_eq!(crate::render(cards).map(|h| h.contains(r#"<a id="c0" class="card" href="/d/acme-intro">"#)), Ok(true));
    }

    #[test]
    fn a_tree_with_an_element_not_ported_is_refused() {
        let _budget = TestBudget::new();
        let tree =
            r#"{"root":"q","elements":{"q":{"type":"check","props":{"text":"x","met":null}}}}"#;
        assert_eq!(page("t", tree), Err("q: not drawn by Dioxus yet".into()));
        let looped = r#"{"root":"a","elements":{"a":{"type":"stack","children":["a"]}}}"#;
        assert!(page("t", looped).is_err());
    }
}
