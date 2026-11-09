//! /design?tab=components: each component of crate::components drawn, its HTML and its text side by side, so a page's two
//! forms are seen where they are made (task rust-design-components). Here, not in components.rs: it reads crate::forms,
//! which draws the menu with the components.
use crate::components::*;
use dioxus::prelude::*;

/// /design?tab=components: (slug, name, where it is used, an example). Its page is dioxuslabs' component page of `slug`.
pub type Entry = (&'static str, &'static str, &'static str, fn() -> Element);

pub const CATALOG: [Entry; 8] = [
    ("card", "Card", "The tasks board's home cards.", || {
        rsx! {
            Card { id: "c-card", href: "/tasks?project=site_perso".to_string(),
                CardTitle { "site_perso" }
                CardDescription { "3 open, 1 in review" }
            }
        }
    }),
    (
        "item",
        "Item",
        "The tools page's accesses, a row each.",
        || {
            rsx! {
                Item { id: "c-item", look: "access", status: "works",
                    p { Badge { look: "chip", "works" } b { "fly" } }
                }
            }
        },
    ),
    (
        "badge",
        "Badge",
        "An access's state, a column's count, a card's labels.",
        || {
            rsx! { Badge { look: "chip", "works" } Badge { look: "count", "4" } Badge { look: "li-tag", "rust" } }
        },
    ),
    (
        "avatar",
        "Avatar",
        "A board card's assignee.",
        || rsx! { Avatar { initials: "C", look: "li-who" } },
    ),
    (
        "button",
        "Button",
        "The search bar's button, the menu's compass (icon), the comment box's Send and Cancel.",
        || rsx! { Button { "Search" } Button { look: "quiet", r#type: "button", "Cancel" } Button { look: "icon", r#type: "button", aria_label: "Menu", "☰" } },
    ),
    (
        "input",
        "Input",
        "The search bar's field, the menu's fold.",
        || {
            rsx! { Input { r#type: "search", name: "q", placeholder: "Search tasks" } }
        },
    ),
    ("textarea", "Textarea", "The compass's comment box.", || {
        rsx! { Textarea { name: "text", rows: "3", placeholder: "Your comment", required: true } }
    }),
    ("tabs", "Tabs", "The tasks board's views.", || {
        rsx! {
            Tabs { id: "c-tabs", label: "Views",
                Tab { href: "/tasks", current: true, "List" }
                Tab { href: "/tasks?layout=board", current: false, "Board" }
            }
        }
    }),
];

/// The library's page of a component.
pub fn source(slug: &str) -> String {
    format!("https://dioxuslabs.github.io/dioxus-components/component/?name={slug}")
}

/// An example's two forms: its HTML, the server render, and the text an agent reads of it.
pub fn forms(example: fn() -> Element) -> (String, String) {
    let html = dioxus_ssr::render_element(example());
    let text = crate::forms::text_of(&html);
    (html, text)
}

/// The Components tab: each component's example drawn, its HTML and its text side by side, in the bar's theme.
#[component]
pub fn Components(dark: bool) -> Element {
    let cell = if dark {
        "app dark ds-cell"
    } else {
        "app light ds-cell"
    };
    let rows = CATALOG.map(|(slug, name, used, example)| {
        let (html, text) = forms(example);
        rsx! {
            section { id: "s-{slug}", key: "{slug}", class: "ds-section",
                div { class: "ds-head",
                    a { class: "ds-name", href: source(slug), "{name}" }
                    span { class: "ds-side", "HTML" }
                    span { class: "ds-side", "Text" }
                }
                div { class: "ds-row",
                    div { class: "ds-label", "{used}" }
                    div { class: cell,
                        {example()}
                        pre { class: "ds-code", "{html}" }
                    }
                    div { class: cell, pre { class: "ds-code", "{text}" } }
                }
            }
        }
    });
    rsx! {
        div { id: "ds-main", class: "ds-main",
            div { class: "ds-page",
                div { class: "ds-title", "Components" }
                div { class: "ds-what",
                    "Ported from dioxuslabs.com/components. Each is drawn with its HTML, then the text an agent reads of it."
                }
                {rows.into_iter()}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{OWNED_CLASSES, OWNED_TAGS};
    use dash_meter::memory::TestBudget;

    fn example(slug: &str) -> (String, String) {
        let entry = CATALOG.iter().find(|e| e.0 == slug).expect(slug);
        forms(entry.3)
    }

    #[test]
    fn a_card_is_a_link_its_title_then_its_description() {
        let _budget = TestBudget::new();
        let (html, text) = example("card");
        assert_eq!(
            html,
            r#"<a class="card" id="c-card" href="/tasks?project=site_perso"><b class="title">site_perso</b><span class="description">3 open, 1 in review</span></a>"#
        );
        assert_eq!(text, "site_perso 3 open, 1 in review\n");
    }

    #[test]
    fn an_item_is_a_row_its_status_for_its_look() {
        let _budget = TestBudget::new();
        let (html, text) = example("item");
        assert_eq!(
            html,
            r#"<div id="c-item" class="item access" data-status="works"><p><span class="badge chip">works</span><b>fly</b></p></div>"#
        );
        assert_eq!(text, "works fly\n");
    }

    #[test]
    fn a_badge_is_its_look_and_its_words() {
        let _budget = TestBudget::new();
        let (html, text) = example("badge");
        assert_eq!(
            html,
            r#"<span class="badge chip">works</span><span class="badge count">4</span><span class="badge li-tag">rust</span>"#
        );
        assert_eq!(text, "works 4 rust\n");
    }

    #[test]
    fn an_avatar_is_the_initials() {
        let _budget = TestBudget::new();
        let (html, text) = example("avatar");
        assert_eq!(html, r#"<span class="avatar li-who">C</span>"#);
        assert_eq!(text, "C\n");
    }

    #[test]
    fn a_button_with_no_look_is_the_button_class_alone_its_attributes_passed() {
        let _budget = TestBudget::new();
        let (html, text) = example("button");
        assert_eq!(
            html,
            r#"<button class="button">Search</button><button class="button quiet" type="button">Cancel</button><button class="button icon" aria-label="Menu" type="button">☰</button>"#
        );
        assert_eq!(text, "Search\tCancel\t☰\n");
    }

    /// A field with no value has no words: its text is nothing, its query is the URL's (`/search?q=`).
    #[test]
    fn an_input_is_its_kind_name_and_placeholder() {
        let _budget = TestBudget::new();
        let (html, text) = example("input");
        assert_eq!(
            html,
            r#"<input class="input" name="q" placeholder="Search tasks" type="search"/>"#
        );
        assert_eq!(text, "");
    }

    /// Its words are its value: empty, so its text is nothing.
    #[test]
    fn a_textarea_is_its_name_rows_and_placeholder() {
        let _budget = TestBudget::new();
        let (html, text) = example("textarea");
        assert_eq!(
            html,
            r#"<textarea class="textarea" name="text" placeholder="Your comment" required=true rows="3"></textarea>"#
        );
        assert_eq!(text, "");
    }

    #[test]
    fn tabs_are_links_the_current_one_marked() {
        let _budget = TestBudget::new();
        let (html, text) = example("tabs");
        assert_eq!(
            html,
            r#"<nav class="tabs" id="c-tabs" aria-label="Views"><a href="/tasks" aria-current="page">List</a><a href="/tasks?layout=board">Board</a></nav>"#
        );
        assert_eq!(text, "List\tBoard\n");
    }

    /// Done-when 1: /design?tab=components lists each, its library page linked, its HTML and its text side by side; and each
    /// draws a class or an element the gate keeps to this file.
    #[test]
    fn the_design_page_lists_every_component_with_both_forms() {
        let _budget = TestBudget::new();
        let page = crate::design::page("components", None);
        let text = crate::forms::text_of(&page);
        for (slug, name, _, example) in CATALOG {
            let (html, words) = forms(example);
            let link = format!(
                r#"href="{}">{name}</a>"#,
                source(slug).replace('&', "&#38;")
            );
            assert!(page.contains(&link), "{link} in {page}");
            assert!(page.contains(&html), "{slug}'s HTML drawn: {page}");
            let shown = html
                .replace('&', "&#38;")
                .replace('<', "&#60;")
                .replace('>', "&#62;")
                .replace('"', "&#34;");
            assert!(page.contains(&shown), "{slug}'s HTML shown: {page}");
            for line in words.lines() {
                assert!(text.contains(line), "{slug}'s text {line:?} in {text}");
            }
            let owned = OWNED_TAGS
                .iter()
                .any(|t| html.starts_with(&format!("<{t} ")))
                || OWNED_CLASSES
                    .iter()
                    .any(|c| html.contains(&format!(r#"class="{c}"#)));
            assert!(owned, "{slug} draws nothing OWNED names: {html}");
        }
    }
}
