//! /documents: every page of private/docs Rust's /v/ serves, in layout A, his pick on /design (task design-doc-tree-menu,
//! done-when 5): the doc tree beside the app's menu, a splitter, then the home document linking each page, which
//! /v/private/docs/<file> serves (docs.rs). Above them, its search over titles and text and New document, which starts one
//! from the rfc skill's template (task rust-doc-cowrite; @nha, 2026-11-12: « documents is basically the pendant of the elixir
//! app », « basic searh to start with but then we add flare features on top »). A page in a subfolder, or Markdown, stays
//! the older app's /v/ until docs.rs takes it.
use crate::pages::{Page, Reads};
use dash_surface::components::{DOC_TYPES, Listed, NEW_DOCUMENT};
use serde_json::json;
use std::path::Path;

/// The page a new document starts from: the rfc skill's template (.claude/skills/rfc, « Start a new page from
/// private/docs/rfc-template.html »).
const TEMPLATE: &str = "private/docs/rfc-template.html";

/// Each page of private/docs as /documents lists it, with its file's text: its title (its file's name when it has none), its
/// /v/ link, RFC or Page, and « in review » while its stages say so. RFCs first, then by title.
fn pages(repo: &Path) -> Vec<(Listed, String)> {
    let folder = repo.join("private/docs");
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return vec![];
    };
    let mut all: Vec<(Listed, String)> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".html") && !n.starts_with('.'))
        .map(|name| {
            // its title and stage as NeedsYou reads them (dash_views' rules): in review while its stages say so.
            // ponytail: the whole file read (4 MB for the 71 pages), and a stage the store sets over the page's not read;
            // the store's docs view once /documents is a dataflow view
            let body = std::fs::read_to_string(folder.join(&name)).unwrap_or_default();
            let facts = dash_views::rules::doc_facts(&name, &body);
            let review = dash_views::rules::in_review(None, facts.claimed);
            let listed = Listed {
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
            };
            (listed, body)
        })
        .collect();
    all.sort_by(|(a, _), (b, _)| (a.kind != "RFC", &a.title).cmp(&(b.kind != "RFC", &b.title)));
    all
}

/// Each page of private/docs as /documents lists it.
pub fn listed(repo: &Path) -> Vec<Listed> {
    pages(repo).into_iter().map(|(l, _)| l).collect()
}

/// A page's words, a block a line, as `dash doc extract` reads them (dash_ingest's HTML reader, rfc-rust-unified section 20i):
/// its markup, head, styles and scripts are never words.
fn plain(html: &str) -> String {
    let parsed = dash_ingest::html::read_html(html);
    let lines: Vec<&str> = parsed.elements.iter().map(|e| e.text.as_str()).collect();
    lines.join("\n")
}

/// The pages whose title or text holds every word of `q` (search.rs's words: folded, two letters at least), each with the
/// first line holding them all, else one holding any, among `pages` as read. `None`: too few letters to search.
/// ponytail: every page parsed at each search; the store's docs view, or Flare's index (SEARCH_NEXT), once a search waits on it.
fn found(pages: &[(Listed, String)], q: &str) -> Option<Vec<(Listed, String)>> {
    let words = crate::search::words(q)?;
    let hits = pages.iter().filter_map(|(d, body)| {
        let text = plain(body);
        crate::search::holds(&format!("{} {text}", d.title), &words).then(|| {
            let lines = || text.lines().map(str::trim).filter(|l| !l.is_empty());
            let line = (lines().find(|l| crate::search::holds(l, &words)))
                .or_else(|| {
                    lines().find(|l| {
                        words
                            .iter()
                            .any(|w| crate::search::holds(l, std::slice::from_ref(w)))
                    })
                })
                .map(|l| {
                    l.split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(200)
                        .collect()
                })
                .unwrap_or_default();
            (d.clone(), line)
        })
    });
    Some(hits.collect())
}

/// The page's tree: the search field and New document, a search's results when one is asked, then the tree and the home
/// document; each element the design system's.
pub fn tree(repo: &Path, q: &str) -> String {
    // the folder read once, for the list and the search alike (/code-review)
    let pages = pages(repo);
    let listed: Vec<&Listed> = pages.iter().map(|(l, _)| l).collect();
    let mut elements = json!({
        "bar": {"type": "bar", "children": ["search", "new"]},
        "search": {"type": "search-bar", "props": {"placeholder": "Search documents", "value": q, "action": "/documents"}},
        "new": {"type": "new-document"},
        "docs": {"type": "doc-index", "props": {"docs": listed}},
    });
    let mut children = vec!["bar", "docs"];
    if let Some(hits) = found(&pages, q) {
        let doc = dash_surface::documents::found(q.trim(), &hits);
        elements["found"] = json!({"type": "doc-view", "props": {"doc": doc}});
        children.insert(1, "found");
    }
    elements["page"] = json!({"type": "stack", "children": children});
    json!({"root": "page", "elements": elements}).to_string()
}

/// `POST /documents/new`: the request line names it.
pub fn new_posted(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words.next().map(|t| t.split('?').next()) == Some(Some(NEW_DOCUMENT))
}

/// A title as a file's name: its words folded and joined by hyphens, any other character dropped; letters of any script kept
/// (/code-review: « Œuvre » lost its Œ). A leading « rfc » is dropped: the type gives the prefix, never the title.
fn slug(title: &str) -> String {
    let folded = crate::search::fold(title);
    let words: Vec<String> = folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    let skip = usize::from(words.first().is_some_and(|w| w == "rfc"));
    words[skip..].join("-")
}

/// A new document of `kind` titled `title`: the template read in through `dash doc import`, its title set, then kept and its
/// HTML file written by `dash doc save` (docs_log::save), so it opens in the Notion view at once. Its repo path. A name taken is
/// refused, the file left as it is.
pub fn create(dir: &Path, repo: &Path, kind: &str, title: &str) -> Result<String, String> {
    // one create at a time: two posts of one title (a double submit) would both pass the check below (/code-review)
    static CREATING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = CREATING.lock().unwrap_or_else(|e| e.into_inner());
    let title = title.trim();
    let prefix = match DOC_TYPES.iter().find(|t| t.0 == kind) {
        Some(("rfc", _)) => "rfc-",
        Some(_) => "",
        None => {
            return Err(format!(
                "no document type {kind}: one of {}",
                DOC_TYPES.map(|t| t.0).join(", ")
            ));
        }
    };
    let name = slug(title);
    if name.is_empty() {
        return Err("a document needs a title with a letter or a digit".into());
    }
    let path = format!("private/docs/{prefix}{name}.html");
    if path == TEMPLATE
        || repo.join(&path).exists()
        || crate::docs_log::stored(dir, &path)?.is_some()
    {
        return Err(format!("{path} exists: open it, or pick another title"));
    }
    let mut doc = dash_ingest::doc::import(&repo.join(TEMPLATE))?;
    for b in doc["content"].as_array_mut().into_iter().flatten() {
        match b["type"].as_str() {
            Some("meta") => b["attrs"]["title"] = title.into(),
            Some("title") => b["content"] = json!([{"type": "text", "text": title}]),
            _ => {}
        }
    }
    crate::docs_log::save(dir, repo, &path, &doc)?;
    Ok(path)
}

/// New document's post (`type`, `title`): the document made and a 303 to its page, or why not.
pub fn post(dir: &Path, repo: &Path, form: &str) -> crate::pages::Response {
    let field = |name: &str| {
        (form.split('&'))
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| crate::pages::form_decoded(k) == name)
            .map(|(_, v)| crate::pages::form_decoded(v))
            .unwrap_or_default()
    };
    match create(dir, repo, &field("type"), &field("title")) {
        Ok(path) => (
            "303 See Other",
            crate::pages::TEXT,
            dash_comments::threads::shown_on(&path),
        ),
        Err(why) => (
            "400 Bad Request",
            crate::pages::TEXT,
            format!("Not created: {why}\n\nBack to the documents: /documents\n"),
        ),
    }
}

inventory::submit! {
    Page { path: "/documents", step: "route_documents", reads: Reads::Nothing, redraw: true, serve: |a| {
        let tree = tree(&crate::pages::repo(), &a.param("q").unwrap_or_default());
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
        let tree = tree(&dir.0, "");
        let text = dash_surface::text(&tree).expect("its text");
        assert_eq!(
            text.lines().filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            [
                // task rust-doc-cowrite's New document, where it posts and what
                "New document: POST /documents/new type=rfc|page, title",
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

    /// A scratch repo whose private/docs holds `pages`, and the data folder the server reads.
    fn scratch(
        name: &str,
        pages: &[(&str, &str)],
    ) -> (dash_meter::TestDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = dash_meter::TestDir::new(name);
        let (repo, data) = (dir.0.join("repo"), dir.0.join("data"));
        std::fs::create_dir_all(repo.join("private/docs")).expect("repo");
        std::fs::create_dir_all(&data).expect("data");
        for (name, html) in pages {
            std::fs::write(repo.join("private/docs").join(name), html).expect("a page");
        }
        crate::pages::serve_from(&data);
        (dir, repo, data)
    }

    /// Task rust-doc-cowrite, done-when 2: New document picks a type, RFC first, and a title; its post writes the rfc skill's
    /// template (the test RFC stands in for it) as an HTML page of private/docs titled so, kept by the store, and 303s to its
    /// page, which draws it through the Notion view as the RFC type. A title taken, or a type not offered, writes nothing.
    #[dash_test]
    fn new_document_starts_an_rfc_from_the_template_in_the_notion_view() {
        let template = include_str!("../tests/fixtures/rfc-t.html");
        let (_dir, repo, data) = scratch("doc-new", &[("rfc-template.html", template)]);
        assert_eq!(DOC_TYPES[0], ("rfc", "RFC"), "RFC first");
        assert!(new_posted("POST /documents/new HTTP/1.1"));
        assert!(!new_posted("GET /documents/new HTTP/1.1"));
        assert!(!new_posted("POST /documents HTTP/1.1"));
        let path = "private/docs/rfc-co-write-an-ete.html";
        let made = post(&data, &repo, "type=rfc&title=Co-write+an+%C3%A9t%C3%A9");
        assert_eq!(
            made,
            ("303 See Other", crate::pages::TEXT, format!("/v/{path}"))
        );
        let file = std::fs::read_to_string(repo.join(path)).expect("its HTML file");
        // saved as HTML: `dash doc import` reads the file back to the document, its title the one given
        let back = dash_ingest::doc::import(&repo.join(path)).expect("an HTML page");
        let meta = (back["content"].as_array().into_iter().flatten())
            .find(|b| b["type"] == "meta")
            .expect("its meta");
        assert_eq!(meta["attrs"]["title"], "Co-write an été");
        assert!(file.contains("Co-write an été"), "{file}");
        assert!(
            crate::docs_log::stored(&data, path)
                .expect("reads")
                .is_some(),
            "the store keeps it"
        );
        let drawing = crate::docs::page(&data, &repo, &format!("/v/{path}"), "")
            .expect("reads")
            .expect("a page");
        let dash_surface::Drawing::Tree { title, tree } = &drawing else {
            panic!("a tree")
        };
        assert_eq!(title, "Co-write an été");
        let tree: serde_json::Value = serde_json::from_str(tree).expect("json");
        assert_eq!(
            tree["elements"]["doc"]["props"]["kind"], "rfc",
            "the RFC type, its Notion view"
        );
        let text = crate::pages::drawn(false, false, Some(Ok(drawing))).2;
        assert!(
            text.contains("Co-write an été\n1. Intent\nWriter notes"),
            "the template's sections: {text}"
        );
        assert_eq!(listed(&repo)[0].title, "Co-write an été", "RFCs first");

        let again = post(&data, &repo, "type=rfc&title=Co-write+an+%C3%A9t%C3%A9");
        assert_eq!(again.0, "400 Bad Request");
        assert!(again.2.contains("exists"), "{}", again.2);
        let page = post(&data, &repo, "type=page&title=Plain+notes");
        assert_eq!(
            page.2, "/v/private/docs/plain-notes.html",
            "a page's file has no rfc- prefix"
        );
        // the type gives the prefix, never the title; a letter of any script is kept (/code-review)
        let page = post(&data, &repo, "type=page&title=RFC+review+notes");
        assert_eq!(page.2, "/v/private/docs/review-notes.html");
        let greek = post(
            &data,
            &repo,
            "type=rfc&title=RFC%3A+%CE%9B%CF%8C%CE%B3%CE%BF%CF%82",
        );
        assert_eq!(
            (greek.0, greek.2.as_str()),
            (
                "303 See Other",
                "/v/private/docs/rfc-%CE%BB%CF%8C%CE%B3%CE%BF%CF%82.html"
            )
        );
        // the template is never a new document's file
        for refused in [
            "type=skill&title=X",
            "type=rfc&title=%E2%80%94",
            "type=rfc&title=Template",
        ] {
            assert_eq!(
                post(&data, &repo, refused).0,
                "400 Bad Request",
                "{refused}"
            );
        }
        assert_eq!(listed(&repo).len(), 5, "nothing more written");
    }

    /// Task rust-doc-cowrite, done-when 4: /documents' search finds each page holding every word in its title or its text,
    /// accents folded, its markup and styles never read as words, each hit a link with its line holding them; its results end
    /// on the layer search grows by, Flare's features.
    #[dash_test]
    fn search_finds_titles_and_text_and_names_flare_next() {
        let rfc = include_str!("../tests/fixtures/search/rfc-ledger.html");
        let notes = include_str!("../tests/fixtures/search/notes.html");
        let (_dir, repo, _data) = scratch(
            "doc-search",
            &[("rfc-ledger.html", rfc), ("notes.html", notes)],
        );
        let text = |q: &str| dash_surface::text(&tree(&repo, q)).expect("its text");
        let both = text("ledger");
        assert!(
            both.contains(
                "Documents holding « ledger »\nRFC: ledger RFC\nLedger notes Page\nSearch finds"
            ),
            "a title's word, with no line of its text: {both}"
        );
        let words = text("ECRITURE book");
        assert!(
            words.contains("RFC: ledger RFC\nThe book keeps every écriture once.\n"),
            "the text's words, accents folded, its line shown: {words}"
        );
        assert!(
            !words.contains("Ledger notes Page"),
            "every word, not any: {words}"
        );
        let none = text("zebra");
        assert!(
            none.contains("No document holds every word."),
            "a style is no word: {none}"
        );
        for t in [&both, &words, &none] {
            assert!(
                t.contains("Next layer: Flare's features"),
                "Flare named next: {t}"
            );
        }
        assert!(
            !text("").contains("Documents holding"),
            "no search, no results"
        );
    }
}
