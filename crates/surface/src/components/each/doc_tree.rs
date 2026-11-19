use super::parts::{DOC, icon};
use crate::component::{Component, Section};
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "doc_tree",
        classes: &["doc-tree"],
        tags: &[],
        name: "Doc tree",
        used: "The documents beside the menu, as Notion's sidebar: each page a line, a page holding others opens to them, the page \
               shown marked and its parents open. A fold is a <details>, so it opens with no script, in the window and the browser.",
        example: Some(|_| rsx! { DocTree { id: "c-doc-tree", label: "Documents", pages: sample_tree(), current: "/tasks/rust-fonts" } }),
        sections: &[Section {
            family: "layout",
            id: "doc-tree",
            title: "Doc tree",
            after: "doc-workspace",
            rows: &[
                ("Nested, the current page's parents open", |_| rsx! {
                    DocTree { id: "ds-doc-tree", label: "Documents", pages: sample_tree(), current: "/tasks/rust-fonts" }
                }),
                ("No page of it shown: all folded", |_| rsx! {
                    DocTree { id: "ds-doc-tree-folded", label: "Documents", pages: sample_tree(), current: "" }
                }),
            ],
        }],
        css: include_str!("doc_tree.css"),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A page in the tree: its title, its link, the pages under it.
#[derive(Clone, Debug, PartialEq)]
pub struct TreePage {
    pub title: String,
    pub href: String,
    pub pages: Vec<TreePage>,
}

impl TreePage {
    pub fn new(title: &str, href: &str, pages: Vec<TreePage>) -> Self {
        let (title, href) = (title.to_string(), href.to_string());
        Self { title, href, pages }
    }
}

/// /design's tree: an RFC holding its parts, one of them holding a task, then two pages alone.
pub fn sample_tree() -> Vec<TreePage> {
    let page = TreePage::new;
    vec![
        page(
            "Rendering plan",
            "/design?tab=documents",
            vec![
                page(
                    "One tree, two renderers",
                    "/design?tab=documents#s-rfc",
                    vec![],
                ),
                page(
                    "Fonts",
                    "/design?tab=documents#s-fonts",
                    vec![page(
                        "Fonts: one face a window",
                        "/tasks/rust-fonts",
                        vec![],
                    )],
                ),
            ],
        ),
        page(
            "Job search",
            "/crm",
            vec![page("Ada Lovelace", "/crm/ada", vec![])],
        ),
        page("Daily notes", "/today", vec![]),
    ]
}

fn holds(page: &TreePage, current: &str) -> bool {
    page.href == current || page.pages.iter().any(|p| holds(p, current))
}

fn node(page: &TreePage, current: &str) -> Element {
    let here = (page.href == current).then_some("page");
    let link = rsx! { a { href: "{page.href}", "aria-current": here, {icon(DOC)} span { "{page.title}" } } };
    if page.pages.is_empty() {
        return rsx! { li { {link} } };
    }
    // a present `open`, even « false », opens it: only a fold holding the current page carries one
    let open = holds(page, current).then_some(true);
    rsx! {
        li {
            details { open,
                summary { {link} }
                ul { for p in page.pages.iter() { {node(p, current)} } }
            }
        }
    }
}

/// The documents' tree beside the menu, Notion's sidebar (task design-doc-tree-menu; @nha, 2026-11-12: « notion also has a
/// tree-like menu »): a page a line, a page holding others a fold of them, `current`'s link marked and its parents open.
#[component]
pub fn DocTree(
    #[props(into)] id: String,
    label: &'static str,
    pages: Vec<TreePage>,
    #[props(into)] current: String,
) -> Element {
    rsx! {
        nav { id, class: "doc-tree", "aria-label": label,
            ul { for p in pages.iter() { {node(p, &current)} } }
        }
    }
}

#[cfg(test)]
mod tests {
    use dash_meter::dash_test;

    /// Done-when 1 of design-doc-tree-menu: /design's Layout tab draws the doc tree, light and dark, its HTML and its text: a
    /// page holding others is a fold, nested a level down, the current page's link marked and its parents' folds open, the
    /// others closed; with no current page, every fold closed. The Components tab holds it too, its two forms under it.
    #[dash_test]
    fn design_shows_the_doc_tree_nested_with_its_current_page() {
        let page = crate::markup(&crate::design::page("layout", None, "tab=layout"));
        let at = page.find(r#"id="s-doc-tree""#).expect("its section");
        let section = &page[at..at + page[at..].find("</section>").expect("its end")];
        let tree = r#"<nav class="doc-tree" id="ds-doc-tree" aria-label="Documents"><ul><li><details open=true><summary><a href="/design?tab=documents">"#;
        assert!(
            section.contains(tree),
            "a fold, open as it holds the current page: {section}"
        );
        let current = r#"<li><a href="/tasks/rust-fonts" aria-current="page">"#;
        assert!(
            section.contains(&format!(r#"<ul>{current}"#)),
            "nested, marked: {section}"
        );
        assert!(
            section.contains(r#"<li><details><summary><a href="/crm">"#),
            "the others closed: {section}"
        );
        assert!(
            section.contains("app light ds-cell") && section.contains("app dark ds-cell"),
            "{section}"
        );
        let folded = &section[section.find("ds-doc-tree-folded").expect("folded")..];
        assert!(
            !folded[..folded.find("</nav>").expect("its end")].contains("open"),
            "{folded}"
        );
        let words = crate::forms::text_of(section);
        assert!(
            words.contains("Rendering plan\nOne tree, two renderers\nFonts\nFonts: one face a window\nJob search\nAda Lovelace\nDaily notes"),
            "its text, a page a line: {words}"
        );
        let components = crate::markup(&crate::design::page("components", None, "tab=components"));
        assert!(
            components.contains(r#"id="s-doc_tree""#),
            "the Components tab's entry"
        );
    }
}
