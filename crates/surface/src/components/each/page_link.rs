use super::badge::Badge;
use super::parts::{DOC, LIST, MAIL, PERSON, icon, local_path};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "page_link",
        classes: &["page-link", "page-links"],
        tags: &[],
        name: "Link to a page",
        used: "A block of a document that opens another page of ours, as Notion's: its type's icon, its title, its type and \
               its stage. A page of mostly links (an index, a project's documents) reads as a column of them, one a line.",
        example: Some(|_| rsx! {
            PageLinks { id: "c-page-links",
                PageLink { href: "/design?tab=documents", title: "One tree, two renderers", kind: "RFC", stage: "draft" }
                PageLink { href: "/tasks/rust-fonts", title: "Fonts: one face a window", kind: "Task", stage: "open" }
                PageLink { href: "/drafts/intro", title: "Intro to the platform team", kind: "Draft", stage: "approved" }
                PageLink { href: "/crm/ada", title: "Ada Lovelace", kind: "Contact", stage: "replied" }
            }
        }),
        css: include_str!("page_link.css"),
        ..Component::BARE
    }
}

/// A type's icon: a task's list, a draft's mail, a person's head, else a page.
fn mark(kind: &str) -> &'static str {
    match kind {
        "Task" => LIST,
        "Draft" | "Mail" => MAIL,
        "Contact" | "Person" => PERSON,
        _ => DOC,
    }
}

/// A link to a page of ours, a document's block (task design-doc-tree-menu; @nha, 2026-11-12: « how links to internal pages
/// render »): its type's icon, its title, its type and its stage as a badge. A target not of this app draws its title alone.
#[component]
pub fn PageLink(
    #[props(into)] href: String,
    #[props(into)] title: String,
    #[props(into)] kind: String,
    #[props(into)] stage: String,
) -> Element {
    // a page of ours only: anything else, another site included, draws its title alone, never dressed as one of ours
    if !local_path(&href) {
        return rsx! { div { class: "page-link", span { class: "title", "{title}" } } };
    }
    rsx! {
        // a block of its own, so its text form is a line of its own
        div { class: "page-link",
            a { href,
                {icon(mark(&kind))}
                span { class: "title", "{title}" }
                span { class: "kind", "{kind}" }
                if !stage.is_empty() { Badge { "{stage}" } }
            }
        }
    }
}

/// A run of links to pages, one a line: a page of mostly links.
#[component]
pub fn PageLinks(#[props(into)] id: String, children: Element) -> Element {
    rsx! { div { id, class: "page-links", {children} } }
}

#[cfg(test)]
mod tests {
    use dash_meter::dash_test;
    use serde_json::json;

    /// Done-when 2 of design-doc-tree-menu: a link-to-page block is a block of the one document model, drawn by DocView as
    /// the design system's PageLink: its type's icon, its title, its type, its stage; a page of mostly links reads, in its text
    /// form, a link a line.
    #[dash_test]
    fn a_document_draws_its_page_blocks_as_links_one_a_line() {
        let page = |title: &str, href: &str, kind: &str, stage: &str| json!({"type": "page", "attrs": {"href": href, "kind": kind, "stage": stage}, "content": [{"type": "text", "text": title}]});
        let doc = json!({"type": "doc", "content": [
            {"type": "title", "content": [{"type": "text", "text": "Rust port"}]},
            page("One tree, two renderers", "/design?tab=documents", "RFC", "draft"),
            page("Fonts: one face a window", "/tasks/rust-fonts", "Task", "open"),
            page("Ada Lovelace", "/crm/ada", "Contact", "replied"),
            page("Elsewhere", "https://evil.example", "Task", "open"),
        ]});
        let (html, text) = crate::documents::view_forms(doc, crate::documents::View::Notion);
        let link = r#"<div class="page-link"><a href="/tasks/rust-fonts"><svg class="ic""#;
        assert!(html.contains(link), "{html}");
        assert!(html.contains(r#"<span class="title">Fonts: one face a window</span><span class="kind">Task</span><span class="badge">open</span></a></div>"#), "{html}");
        // each link leads with its type's icon: a task's list, a contact's head, an RFC's page
        for (href, mark) in [
            ("/tasks/rust-fonts", super::LIST),
            ("/crm/ada", super::PERSON),
            ("/design?tab=documents", super::DOC),
        ] {
            let icon = crate::markup(&dioxus_ssr::render_element(super::icon(mark)));
            let leads = format!(r#"<a href="{href}">{icon}<span class="title">"#);
            assert!(html.contains(&leads), "{leads} in {html}");
        }
        assert_eq!(
            text,
            "Rust port\nOne tree, two renderers RFC draft\nFonts: one face a window Task open\nAda Lovelace Contact replied\nElsewhere\n"
        );
        // another site's link is no page of ours: its title alone, no link, icon, type or stage
        assert!(
            html.contains(r#"<div class="page-link"><span class="title">Elsewhere</span></div>"#),
            "{html}"
        );
        assert!(!html.contains("evil.example"), "{html}");
    }
}
