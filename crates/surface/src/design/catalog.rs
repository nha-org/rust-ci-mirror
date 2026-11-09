//! /design?tab=components: each component of crate::components drawn, its HTML and its text side by side, so a page's two
//! forms are seen where they are made (task rust-design-components). Here, not in components.rs: it reads crate::forms,
//! which draws the menu with the components.
use crate::components::*;
use dioxus::prelude::*;

/// /design?tab=components: (slug, name, where it is used, an example). Its page is dioxuslabs' component page of `slug`.
pub type Entry = (&'static str, &'static str, &'static str, fn() -> Element);

pub const CATALOG: [Entry; 12] = [
    (
        "card",
        "Card",
        "The home cards; the tasks board's Linear cards, look li-card, each its task's link.",
        || {
            rsx! {
                Card { id: "c-card", href: "/tasks?project=site_perso".to_string(),
                    CardTitle { "site_perso" }
                    CardDescription { "3 open, 1 in review" }
                }
            }
        },
    ),
    (
        "item",
        "Item",
        "The tools page's accesses, a row each; the tasks list's rows, look issue-row, each its task's link.",
        || {
            rsx! {
                Item { id: "c-item", look: "access", status: "works",
                    p { Badge { look: "chip", "works" } b { "fly" } }
                }
                Item { id: "c-row", look: "issue-row", href: "/tasks/a".to_string(), task: "a", b { "a" } " Port the board" }
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
        "A board card's assignee; a tool's logo (Simple Icons', else its favicon), its initials where none is held.",
        || {
            rsx! {
                Avatar { initials: "C", look: "li-who" }
                Logo { name: "fly" }
                Logo { name: "tinyfish" }
                Logo { name: "vapid" }
            }
        },
    ),
    (
        "skeleton",
        "Skeleton",
        "Any section still being read, filled in place when it lands: /agents' usage, the tasks pages' lists, /tools' cards.",
        || {
            rsx! {
                Skeleton { id: "c-skeleton", what: "Usage" }
                Tool { id: "c-skeleton-tool", name: "fly", reading: 2 }
            }
        },
    ),
    (
        "live",
        "Live",
        "A part a skeleton in it refills whole: the tools page's themes, whose cards appear as each provider answers.",
        || {
            rsx! {
                Live { id: "c-live",
                    Tool { id: "c-tool", name: "fly",
                        Item { id: "c-tool-item", look: "access", status: "works", p { Badge { look: "chip", "works" } b { "fly" } } }
                    }
                }
            }
        },
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
    (
        "collapsible",
        "Fold",
        "A conversation's mails on /mail/t/<thread>: the older ones folded to a line, the latest open.",
        || {
            rsx! {
                Fold { id: "c-fold", open: false, summary: rsx! { "Ada · 2026-11-08 · Thanks for the call" },
                    pre { "Thanks for the call, see you Monday." }
                }
            }
        },
    ),
    (
        "frame",
        "Frame",
        "A mail's HTML on /mail/t/<thread>: no script runs, nothing remote loads.",
        || rsx! { Frame { title: "The mail as sent", html: "<p>Hello <b>Ada</b></p>" } },
    ),
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
            r#"<a id="c-card" class="card" href="/tasks?project=site_perso"><b class="title">site_perso</b><span class="description">3 open, 1 in review</span></a>"#
        );
        assert_eq!(text, "site_perso 3 open, 1 in review\n");
    }

    #[test]
    fn an_item_is_a_row_its_status_for_its_look_a_link_with_its_task() {
        let _budget = TestBudget::new();
        let (html, text) = example("item");
        assert_eq!(
            html,
            r#"<div id="c-item" class="item access" data-status="works"><p><span class="badge chip">works</span><b>fly</b></p></div><a id="c-row" class="item issue-row" data-task="a" href="/tasks/a"><b>a</b> Port the board</a>"#
        );
        assert_eq!(text, "works fly\na Port the board\n");
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
        let fly = include_str!("../logos/fly.svg");
        let tinyfish = include_str!("../logos/tinyfish.png.b64");
        // Simple Icons' inline, a favicon an image (a fetched file runs no script), initials where none is held
        assert_eq!(
            html,
            format!(
                r#"<span class="avatar li-who">C</span><span class="avatar logo">{fly}</span><span class="avatar logo"><img alt="" src="data:image/png;base64,{tinyfish}"/></span><span class="avatar logo">VA</span>"#
            )
        );
        // a logo has no words: its tool's name is beside it
        assert!(
            fly.starts_with(r#"<svg aria-hidden="true" fill="currentColor" "#)
                && !fly.contains("<title>")
        );
        assert_eq!(text, "C VA\n");
    }

    /// Done-when 1 of rust-skeleton-live and 3 (rust-tools-themes' card on it): a section still read is greyed, busy, grey
    /// lines and no words of waiting (the page draws its refill once, screen::Screen); a tool's card is the same skeleton, its
    /// name and logo kept, a grey line for each access to come.
    #[test]
    fn a_skeleton_is_busy_grey_lines_and_a_reading_tool_is_one() {
        let _budget = TestBudget::new();
        let (html, text) = example("skeleton");
        let fly = include_str!("../logos/fly.svg");
        let lines = r#"<span class="line"></span><span class="line"></span></div>"#;
        assert_eq!(
            html,
            format!(
                r#"<div id="c-skeleton" class="skeleton" aria-busy="true" role="status" aria-label="Usage: being read">{lines}<div id="c-skeleton-tool" class="card tool skeleton" aria-busy="true" role="status" aria-label="fly: being read"><p class="head"><span class="avatar logo">{fly}</span><b class="title">fly</b></p>{lines}"#
            )
        );
        assert_eq!(text, "fly\n");
        assert_eq!(html.matches(crate::components::BUSY).count(), 2, "{html}");
    }

    /// The live part redraws itself from the page's URL in HTML, only while a busy card is in it, and stops.
    #[test]
    fn a_live_part_refreshes_itself_while_a_card_in_it_is_busy() {
        let _budget = TestBudget::new();
        let (html, text) = example("live");
        assert!(
            html.starts_with(
                r#"<div class="live" id="c-live"><div class="card tool" id="c-tool">"#
            ),
            "{html}"
        );
        // its refill is the skeletons' one loop: a filled part carries no script
        assert!(!html.contains("<script>"), "{html}");
        for part in [
            "if(window.dashLive)return;window.dashLive=1",
            r#"querySelectorAll("[aria-busy=true]")"#,
            r#"b.closest(".live[id]")||b.closest("[id]")"#,
            "while(e&&!d.getElementById(e.id))",
            r#"searchParams.set("view","html")"#,
            "e.replaceWith(d.getElementById(e.id))",
            "own.remove()",
            "if(!r.ok){location.reload()",
            "if(++n>300)return location.reload()",
        ] {
            assert!(LIVE_JS.contains(part), "{part}");
        }
        assert_eq!(text, "fly\nworks fly\n");
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

    /// A fold is its line, its words under it, and open only when asked: a present `open`, even « false », opens it.
    #[test]
    fn a_fold_is_closed_unless_asked_open() {
        let _budget = TestBudget::new();
        let (html, text) = example("collapsible");
        assert_eq!(
            html,
            r#"<details class="fold" id="c-fold"><summary>Ada · 2026-11-08 · Thanks for the call</summary><pre>Thanks for the call, see you Monday.</pre></details>"#
        );
        assert!(
            text.contains("Thanks for the call, see you Monday."),
            "{text}"
        );
        let open = dioxus_ssr::render_element(
            rsx! { Fold { id: "f", open: true, summary: rsx! { "a" }, "b" } },
        );
        assert!(open.contains(" open=true"), "{open}");
    }

    /// Done-when 2 of rust-mail-thread, its HTML side (the browser's is tests/notion_browser.rs): no script, the frame's
    /// sandbox holding no allow-scripts and its policy allowing none; nothing remote, its policy loading data: alone.
    #[test]
    fn a_frame_runs_no_script_and_loads_nothing_remote() {
        let _budget = TestBudget::new();
        let (html, text) = example("frame");
        assert!(
            html.starts_with(r#"<iframe class="frame" referrerpolicy="no-referrer" sandbox="allow-popups allow-popups-to-escape-sandbox" title="The mail as sent" srcdoc="&#60;meta http-equiv=&#34;Content-Security-Policy&#34; content=&#34;default-src &#39;none&#39;; img-src data:; style-src &#39;unsafe-inline&#39;; font-src data:&#34;&#62;"#),
            "{html}"
        );
        assert!(
            !html.contains("allow-scripts") && !html.contains("allow-same-origin"),
            "{html}"
        );
        assert_eq!(
            text, "",
            "a frame's words are the mail's, which the page's text gives"
        );
        let mail = r#"<META HTTP-EQUIV="refresh" content="0;url=https://x.example/r"><meta http-equiv="Content-Security-Policy" content="img-src *"><img src="https://x.example/p.gif">"#;
        let framed = framed(mail);
        assert_eq!(
            framed.matches("http-equiv").count(),
            1,
            "only ours: {framed}"
        );
        assert!(framed.ends_with(r#"<META data-off="refresh" content="0;url=https://x.example/r"><meta data-off="Content-Security-Policy" content="img-src *"><img src="https://x.example/p.gif">"#), "{framed}");
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
