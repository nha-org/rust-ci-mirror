use super::blocks::{DragBlocks, sample_blocks as blocks};
use super::button::Button;
use super::doc_tree::{DocTree, TreePage, sample_tree as tree};
use super::page_link::{PageLink, PageLinks};
use super::parts::{DOC, INBOX, PERSON, icon};
use super::splitter::{Split, Splitter};
use crate::component::{Component, Section};
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "doc_workspace",
        classes: &["workspace", "workspace-page", "island", "island-live", "island-picture"],
        tags: &[],
        name: "Doc workspace",
        used: "The menu, the doc tree and a document whose blocks drag, in one of two layouts, the columns sized by splitters. \
               Live on the web too: the page's liveview runs its first copy here (an island), the rest of /design stays a picture.",
        sections: &[Section {
            family: "layout",
            id: "doc-workspace",
            title: "Doc tree beside the menu, or in it",
            after: "shell",
            rows: &[
                ("A, his pick for /documents (@nha, 2026-11-12, in the task's dialog: « A: tree beside menu »): the tree its own column. Phone: the columns stack, so the tree pushes the document a screen down; it would \
                  move into the menu's drawer there, one more tap to a page.", |_| rsx! {
                    Island { name: WORKSPACE_ISLAND, DocWorkspace { id: "ds-workspace", layout: DocLayout::Beside } }
                }),
                ("B: the tree inside the menu. Phone: the menu's drawer already holds it, no tap more; on a laptop the menu grows \
                  as long as the tree.", |_| rsx! {
                    DocWorkspace { id: "ds-workspace-in", layout: DocLayout::InMenu }
                }),
            ],
        }],
        css: include_str!("workspace.css"),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// The island /design's liveview runs: the workspace, layout A first.
pub const WORKSPACE_ISLAND: &str = "doc-workspace";

/// What a page's liveview draws in the island it was opened for, by its name; none for a name it has none of. `docs` reads
/// /documents' list, which the design system never reads itself: called only for that island.
pub fn live_island(name: &str, docs: fn() -> Vec<Listed>) -> Option<Element> {
    match name {
        WORKSPACE_ISLAND => {
            Some(rsx! { DocWorkspace { id: "ds-live-workspace", layout: DocLayout::Beside } })
        }
        DOCUMENTS_ISLAND => Some(rsx! { DocumentsIndex { docs: docs() } }),
        _ => None,
    }
}

/// What a page's liveview runs: the compass's host, the parts' host, the island its page named, the reader of /documents' list,
/// and the parts its page placed.
pub type LiveRoot = (
    super::compass::CompassHost,
    super::place::PartHost,
    String,
    fn() -> Vec<Listed>,
    Vec<Option<super::part::Part>>,
);

/// A page's liveview root (dash's live.rs runs it): the compass, `host` its own, then the island its page named, if the design
/// system has it (task design-doc-tree-menu: no page is drawn outside crates/surface), then the parts its page placed, which
/// the client moves into their places (task rust-js-port-small).
pub fn live_root((host, parts_host, island, docs, parts): LiveRoot) -> Element {
    let compass = super::compass::compass_hosted(host);
    let island = live_island(&island, docs);
    let parts = super::part::live_parts(parts_host, parts, 0);
    rsx! { {compass} {island} {parts} }
}

/// What the window draws beside its page (dash's live.rs builds the hosts): the compass, then the parts the page placed, which
/// `parts_host` grafts into their places, drawn anew for each page given (`drawn`).
pub fn window_beside(
    host: super::compass::CompassHost,
    parts_host: super::place::PartHost,
    parts: Vec<Option<super::part::Part>>,
    drawn: u64,
) -> Element {
    let compass = super::compass::compass_hosted(host);
    let parts = super::part::live_parts(parts_host, parts, drawn);
    rsx! { {compass} {parts} }
}

/// A part of a page the web's liveview runs (rfc-rust-unified#ui-lib): `.island-live`, named by `data-live`, is where the
/// page's liveview client mounts and draws it live, the compass with it; until then, and in the window, which runs the
/// handlers natively, the picture is shown. A page's first island is its live one.
#[component]
pub fn Island(name: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "island",
            div { class: "island-live", "data-live": name }
            div { class: "island-picture", {children} }
        }
    }
}

/// The island /documents' liveview runs: its tree and links, sized by its splitter.
pub const DOCUMENTS_ISLAND: &str = "documents";

/// A document /documents lists: its title, its page, its type and its stage.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Listed {
    pub title: String,
    pub href: String,
    pub kind: String,
    #[serde(default)]
    pub stage: String,
}

/// /documents in layout A, his pick on /design (task design-doc-tree-menu, done-when 5): beside the app's menu, the doc tree,
/// a fold a type, a splitter, then the documents as a page of links. `docs` come in the order shown.
#[component]
pub fn DocumentsIndex(docs: Vec<Listed>) -> Element {
    let mut kinds: Vec<TreePage> = vec![];
    for d in &docs {
        let page = TreePage::new(&d.title, &d.href, vec![]);
        match kinds.iter_mut().find(|k| k.title == d.kind) {
            Some(k) => k.pages.push(page),
            None => kinds.push(TreePage::new(
                &d.kind,
                &format!("#kind-{}", d.kind),
                vec![page],
            )),
        }
    }
    let shown: Vec<String> = kinds.iter().map(|k| k.title.clone()).collect();
    rsx! {
        Split { id: "documents-split", label: "Tree and documents", widths: vec![260.0],
            DocTree { id: "documents-tree", label: "Documents", pages: kinds, current: "" }
            Splitter { at: 0, label: "Tree width" }
            // a run of links a type, its id the anchor its fold in the tree links to
            div { class: "workspace-page",
                for kind in shown {
                    PageLinks { id: "kind-{kind}",
                        for d in docs.iter().filter(|d| d.kind == kind) {
                            PageLink { href: d.href.clone(), title: d.title.clone(), kind: d.kind.clone(), stage: d.stage.clone() }
                        }
                    }
                }
            }
        }
    }
}

/// Where the doc tree goes: its own column beside the menu, or inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DocLayout {
    Beside,
    InMenu,
}

/// The app's menu as /design draws it, the tree under it in layout B.
fn menu(tree_in: bool) -> Element {
    rsx! {
        div { class: "ds-menu",
            div { class: "nav-item", {icon(INBOX)} " Inbox" }
            div { class: "nav-item on", {icon(DOC)} " Documents" }
            div { class: "nav-item", {icon(PERSON)} " Job search" }
            if tree_in {
                DocTree { id: "ds-menu-tree", label: "Documents", pages: tree(), current: "/tasks/rust-fonts" }
            }
        }
    }
}

/// The menu, the doc tree and a document whose blocks drag, in `layout`, the columns sized by splitters (task
/// design-doc-tree-menu; @nha, 2026-11-12: « This is heavy on the design-system-first imo. »). Its toggle shows the other
/// layout in place, so both are tried on one page.
#[component]
pub fn DocWorkspace(#[props(into)] id: String, layout: DocLayout) -> Element {
    let mut layout = use_signal(|| layout);
    let pressed = |l: DocLayout| if layout() == l { "true" } else { "false" };
    let document = rsx! { div { class: "workspace-page", DragBlocks { id: "{id}-blocks", blocks: blocks() } } };
    rsx! {
        div { id: "{id}", class: "workspace",
            div { role: "group", "aria-label": "Doc tree layout",
                Button { look: "quiet", r#type: "button", "aria-pressed": pressed(DocLayout::Beside),
                    onclick: move |_| layout.set(DocLayout::Beside), "A: tree beside the menu" }
                Button { look: "quiet", r#type: "button", "aria-pressed": pressed(DocLayout::InMenu),
                    onclick: move |_| layout.set(DocLayout::InMenu), "B: tree in the menu" }
            }
            match layout() {
                DocLayout::Beside => rsx! {
                    Split { id: "{id}-split", label: "Menu, tree and document", widths: vec![170.0, 220.0],
                        {menu(false)}
                        Splitter { at: 0, label: "Menu width" }
                        DocTree { id: "{id}-tree", label: "Documents", pages: tree(), current: "/tasks/rust-fonts" }
                        Splitter { at: 1, label: "Tree width" }
                        {document}
                    }
                },
                DocLayout::InMenu => rsx! {
                    Split { id: "{id}-split", label: "Menu and document", widths: vec![240.0],
                        {menu(true)}
                        Splitter { at: 0, label: "Menu width" }
                        {document}
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use dash_meter::dash_test;

    /// Done-when 5 of design-doc-tree-menu, the page: /documents' `doc-index` element is layout A, his pick: in its island,
    /// so liveview runs its splitter on the web, the doc tree a fold a type, a splitter, then a link to each page with its type
    /// and stage. No script of ours.
    #[dash_test]
    fn documents_index_is_layout_a_live_in_its_island() {
        let tree = serde_json::json!({"root": "docs", "elements": {"docs": {"type": "doc-index", "props": {"docs": [
            {"title": "RFC: unified", "href": "/v/private/docs/rfc-b.html", "kind": "RFC", "stage": "in review"},
            {"title": "Notes", "href": "/v/private/docs/notes.html", "kind": "Page"},
        ]}}}});
        let html = crate::screen::page("Documents", &tree.to_string()).expect("drawn");
        let html = crate::markup(&html);
        for part in [
            r#"<div class="island-live" data-live="documents"></div><div class="island-picture"><div class="split""#,
            r#"style="--split:260px 6px minmax(0, 1fr)""#,
            r#"role="separator""#,
            r#"<nav class="doc-tree" id="documents-tree" aria-label="Documents">"#,
            r##"<summary><a href="#kind-RFC""##,
            r##"<summary><a href="#kind-Page""##,
            // each fold's link opens its type's run of links
            r#"<div class="page-links" id="kind-RFC">"#,
            r#"<div class="page-links" id="kind-Page">"#,
            r#"<a href="/v/private/docs/rfc-b.html"><svg class="ic""#,
            r#"<span class="title">RFC: unified</span><span class="kind">RFC</span><span class="badge">in review</span>"#,
        ] {
            assert!(html.contains(part), "{part}: {html}");
        }
        let element = &html[html.find(r#"<div id="docs">"#).expect("the element")..];
        assert!(!element.contains("<script"), "{element}");
    }

    /// Done-when 4 of design-doc-tree-menu, the page: /design's Layout tab draws the two layouts with the app's menu, light and
    /// dark, A's columns sized by two splitters and B's by one, each layout's phone cost named in its label; A's first copy is
    /// the island the page's liveview runs, its picture shown until then. No script of ours.
    #[dash_test]
    fn design_shows_both_layouts_with_their_splitters_and_phone_cost() {
        let page = crate::markup(&crate::design::page("layout", None, "tab=layout"));
        let at = page.find(r#"id="s-doc-workspace""#).expect("its section");
        let section = &page[at..at + page[at..].find("</section>").expect("its end")];
        assert!(section.contains(r#"<div class="island-live" data-live="doc-workspace"></div><div class="island-picture">"#), "{section}");
        let a = r#"style="--split:170px 6px 220px 6px minmax(0, 1fr)""#;
        let b = r#"style="--split:240px 6px minmax(0, 1fr)""#;
        assert!(section.contains(a) && section.contains(b), "{section}");
        assert_eq!(
            section.matches(r#"role="separator""#).count(),
            2 * (2 + 1),
            "A's two and B's one, light and dark"
        );
        assert!(section.contains("app light ds-cell") && section.contains("app dark ds-cell"));
        let words = crate::forms::text_of(section);
        for cost in [
            "Phone: the columns stack",
            "Phone: the menu's drawer already holds it",
        ] {
            assert!(words.contains(cost), "{cost}: {words}");
        }
        // done-when 5: his pick, not a stand-in's, is said where the layouts are compared
        assert!(
            words.contains("A, his pick for /documents (@nha, 2026-11-12, in the task's dialog: « A: tree beside menu »)"),
            "{words}"
        );
        assert!(!section.contains("<script"), "{section}");
    }
}
