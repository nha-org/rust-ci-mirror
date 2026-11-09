//! The components every page draws with, each one of dioxuslabs.com/components ported to design.json's tokens (@nha,
//! 2026-11-09: « our design page should have https://dioxuslabs.com/components or equivalents either use or port what we
//! need », « We want all components to be dual-use »). Dual-use: each is a Dioxus element, so the window draws it and the
//! web's server render is its HTML, whose text form (crate::forms::text_of) is what an agent reads. /design?tab=components
//! shows each one's two forms side by side. `look` picks a component's look: its CSS class, never written by a page.
//! The gate (dash-gate's `page_local_markup`) refuses OWNED_TAGS and OWNED_CLASSES anywhere but here.
use dioxus::prelude::*;

/// The elements a component here draws: a page writes `Button {}`, never `button {}`.
pub const OWNED_TAGS: [&str; 2] = ["button", "input"];
/// The classes a component here draws, each a look a page passes as `look`.
pub const OWNED_CLASSES: [&str; 13] = [
    "card",
    "description",
    "item",
    "access",
    "badge",
    "chip",
    "count",
    "li-tag",
    "avatar",
    "li-who",
    "button",
    "input",
    "tabs",
];

/// A component's own class, then its look's, if any.
fn joined(base: &str, look: &str) -> String {
    match look {
        "" => base.to_string(),
        look => format!("{base} {look}"),
    }
}

/// A card: a link when it has `href`.
#[component]
pub fn Card(
    #[props(into)] id: String,
    #[props(default)] href: Option<String>,
    children: Element,
) -> Element {
    match href {
        Some(href) => rsx! { a { id, class: "card", href, {children} } },
        None => rsx! { div { id, class: "card", {children} } },
    }
}

/// One row of a list: /tools' accesses. `status` is `data-status`, which a look's colours read (components.css).
#[component]
pub fn Item(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default, into)] status: Option<String>,
    children: Element,
) -> Element {
    rsx! { div { id, class: joined("item", look), "data-status": status, {children} } }
}

#[component]
pub fn CardTitle(children: Element) -> Element {
    rsx! { b { class: "title", {children} } }
}

#[component]
pub fn CardDescription(children: Element) -> Element {
    rsx! { span { class: "description", {children} } }
}

/// A short label: a state, a count, a tag.
#[component]
pub fn Badge(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { span { class: joined("badge", look), {children} } }
}

/// Someone's initials in a circle.
#[component]
pub fn Avatar(#[props(into)] initials: String, #[props(default)] look: &'static str) -> Element {
    rsx! { span { class: joined("avatar", look), "{initials}" } }
}

/// A form's button: it submits the form around it.
#[component]
pub fn Button(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { button { class: joined("button", look), {children} } }
}

/// A form's field.
#[component]
pub fn Input(
    #[props(default = "text")] kind: &'static str,
    name: &'static str,
    #[props(into)] placeholder: String,
) -> Element {
    rsx! { input { class: "input", r#type: kind, name, placeholder } }
}

/// One view of several, each a link: the server render runs no handler, so a tab changes the URL.
#[component]
pub fn Tabs(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "tabs", "aria-label": label, {children} } }
}

#[component]
pub fn Tab(#[props(into)] href: String, current: bool, children: Element) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, "aria-current": on, {children} } }
}

/// /design?tab=components: (slug, name, where it is used, an example). Its page is dioxuslabs' component page of `slug`.
pub type Entry = (&'static str, &'static str, &'static str, fn() -> Element);

pub const CATALOG: [Entry; 7] = [
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
        "The search bar's button.",
        || rsx! { Button { "Search" } },
    ),
    ("input", "Input", "The search bar's field.", || {
        rsx! { Input { kind: "search", name: "q", placeholder: "Search tasks" } }
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
    fn a_button_with_no_look_is_the_button_class_alone() {
        let _budget = TestBudget::new();
        let (html, text) = example("button");
        assert_eq!(html, r#"<button class="button">Search</button>"#);
        assert_eq!(text, "Search\n");
    }

    /// A field with no value has no words: its text is nothing, its query is the URL's (`/search?q=`).
    #[test]
    fn an_input_is_its_kind_name_and_placeholder() {
        let _budget = TestBudget::new();
        let (html, text) = example("input");
        assert_eq!(
            html,
            r#"<input class="input" type="search" name="q" placeholder="Search tasks"/>"#
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
