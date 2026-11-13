//! /documents: every page of private/docs Rust's /v/ serves, in layout A, his pick on /design (task design-doc-tree-menu,
//! done-when 5): the doc tree beside the app's menu, a splitter, then a link to each page, which /v/private/docs/<file>
//! serves (docs.rs). No viewer of its own. A page in a subfolder, or Markdown, stays the older app's /v/ until docs.rs takes it.
use crate::pages::{Page, Reads};
use dash_surface::components::Listed;
use serde_json::json;
use std::path::Path;

/// Each page of private/docs as /documents lists it: its title (its file's name when it has none), its /v/ link, RFC or Page,
/// and « in review » while its stages say so. RFCs first, then by title.
pub fn listed(repo: &Path) -> Vec<Listed> {
    let folder = repo.join("private/docs");
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return vec![];
    };
    let mut all: Vec<Listed> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".html") && !n.starts_with('.'))
        .map(|name| {
            // its title and stage as NeedsYou reads them (dash_views' rules): in review while its stages say so.
            // ponytail: the whole file read (4 MB for the 71 pages), and a stage the store sets over the page's not read;
            // the store's docs view once /documents is a dataflow view
            let body = std::fs::read_to_string(folder.join(&name)).unwrap_or_default();
            let facts = dash_views::rules::doc_facts(&name, &body);
            let review = dash_views::rules::in_review(None, facts.claimed);
            Listed {
                title: facts.title,
                // each segment encoded, as a comment's link to the page
                href: dash_comments::threads::shown_on(&format!("private/docs/{name}")),
                kind: if name.starts_with("rfc-") {
                    "RFC"
                } else {
                    "Page"
                }
                .into(),
                stage: if review { "in review" } else { "" }.into(),
            }
        })
        .collect();
    all.sort_by(|a, b| (a.kind != "RFC", &a.title).cmp(&(b.kind != "RFC", &b.title)));
    all
}

/// The page's tree: one element, the design system draws it.
pub fn tree(repo: &Path) -> String {
    let docs = json!({"type": "doc-index", "props": {"docs": listed(repo)}});
    json!({"root": "docs", "elements": {"docs": docs}}).to_string()
}

inventory::submit! {
    Page { path: "/documents", step: "route_documents", reads: Reads::Nothing, redraw: true, serve: |a| {
        let tree = tree(&crate::pages::repo());
        a.form(Some(Ok(dash_surface::Drawing::Tree { title: "Documents".into(), tree })))
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 5 of design-doc-tree-menu, the data: each page of private/docs (here the fixtures' folder) listed once, RFCs
    /// first, its title (its file's name when it has none), its type, « in review » while its stages say so, its /v/ link
    /// encoded; a file not a page left out. The page's text form is a record a page. Its markup is the design system's test
    /// (`documents_index_is_layout_a_live_in_its_island`).
    #[dash_test]
    fn documents_lists_each_page_with_its_type_stage_and_link() {
        let repo =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/documents");
        // the fixtures are the folder itself: a repo whose private/docs they are
        let dir = dash_meter::TestDir::new("doc-index");
        std::fs::create_dir_all(dir.0.join("private")).expect("repo");
        std::os::unix::fs::symlink(&repo, dir.0.join("private/docs")).expect("docs");
        let tree = tree(&dir.0);
        let text = dash_surface::text(&tree).expect("its text");
        assert_eq!(
            text.lines().filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            [
                "RFC: unified\tRFC\tin review\t/v/private/docs/rfc-b.html",
                // its meta claims review, its stages say Accepted: no longer in review
                "Notes\tPage\t\t/v/private/docs/notes.html",
                // a name a link would cut, encoded
                "Plan v2\tPage\t\t/v/private/docs/plan%20v2%23draft.html",
                "untitled.html\tPage\t\t/v/private/docs/untitled.html",
            ],
            "{tree}"
        );
    }
}
