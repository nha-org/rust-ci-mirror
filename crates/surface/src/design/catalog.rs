//! /design?tab=components: each component of crate::components drawn, its HTML and its text side by side, so a page's two
//! forms are seen where they are made (task rust-design-components). Here, not in components.rs: it reads crate::forms,
//! which draws the menu with the components.
use crate::components::*;
use dioxus::prelude::*;

/// /design?tab=components: (slug, name, where it is used, an example). Its page is dioxuslabs' component page of `slug`.
pub type Entry = (&'static str, &'static str, &'static str, fn() -> Element);

pub const CATALOG: [Entry; 19] = [
    (
        "card",
        "Card",
        "Only where a card was approved: the drafts' previews and the board's Linear cards (look li-card); a list of things is a List or a Table (@nha, 2026-11-09: « Cards are not the default UI element »).",
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
        "list",
        "List",
        "The default list: home's suggestions, /inbox, /today, /agents, /tools, a row each with its title, its line and its link; a tree's « list » draws its cards this way.",
        || {
            rsx! {
                div { id: "c-list", class: "list",
                    Item { id: "c-list-a", look: "issue-row", href: "/inbox".to_string(),
                        span { class: "title", "Inbox" }
                        span { class: "says", "2 wait on you" }
                    }
                    Item { id: "c-list-b", look: "issue-row",
                        span { class: "title", "Plan" }
                        span { class: "says", "now, next, later" }
                    }
                }
            }
        },
    ),
    (
        "table",
        "Table",
        "The comment queue at /comments, a row a comment, oldest first, each its link to where it was left.",
        || {
            let row = |cells: [&str; 3], href: Option<&str>| TableRow {
                cells: cells.map(String::from).to_vec(),
                href: href.map(String::from),
            };
            rsx! {
                Table { id: "c-table", columns: vec!["Comment".into(), "Page".into(), "Since".into()],
                    rows: vec![
                        row(["a table, not cards", "/comments", "2026-11-09 10:00"], Some("/comments#c-a")),
                        row(["the old app's", "/x", "2026-11-09 11:00"], None),
                    ],
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
                Access { id: "c-item", status: "works", name: "fly"
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
        "Any section still being read, filled in place when it lands: /agents' usage, the tasks pages' lists, /tools' cards, /today's lines (a card reading).",
        || {
            rsx! {
                Skeleton { id: "c-skeleton", what: "Usage" }
                Tool { id: "c-skeleton-tool", name: "fly", reading: 2 }
                Card { id: "c-skeleton-card", reading: true, CardDescription { "drafts approved, not yet sent" } }
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
                        Access { id: "c-tool-item", status: "works", name: "fly" }
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
    (
        "board",
        "Board, Column, CardLine",
        "The tasks board: its columns side by side, a card dragged to another column takes its status (/tasks).",
        || {
            rsx! {
                Board { id: "c-board",
                    Column { id: "c-column", status: "todo", name: "Todo", count: 1, icon: rsx! {},
                        Card { id: "c-board-card", look: "li-card", href: "/tasks/a".to_string(), task: "a", draggable: true,
                            CardLine { says: "next", comments: 1, said: Some("1 comment".to_string()) }
                        }
                    }
                }
            }
        },
    ),
    (
        "form",
        "Form",
        "Fields and one button that send: the search bar, a mail's reply, a draft's approval, /settings' backup read back (a file, an « upload » in a tree) and its confirmation (an « action »).",
        || {
            rsx! {
                Form { id: "c-form-search", role: "search", method: "get", action: "/search",
                    Input { r#type: "search", name: "q", placeholder: "Search tasks" }
                    Button { "Search" }
                }
                Form { id: "c-form-upload", action: "/settings/restore", file: true,
                    Input { r#type: "file", name: "file", accept: ".zip", required: true }
                    Button { "Read a backup" }
                }
            }
        },
    ),
    (
        "toggle",
        "Toggle",
        "One choice of several, the shown one pressed, each its icon beside its name: the tasks' Board, List and Graph views.",
        || {
            use crate::design::families::{BOARD, LIST, icon};
            rsx! {
                Toggle { id: "c-toggle", label: "Views",
                    ToggleOption { href: "/tasks", current: true, icon: icon(BOARD), "Board" }
                    ToggleOption { href: "/tasks?layout=list", current: false, icon: icon(LIST), "List" }
                }
            }
        },
    ),
    (
        "tabs",
        "Tabs",
        "The tasks board's views; a tree's « links »: /mail's accounts and pager, /settings' download.",
        || {
            rsx! {
                Tabs { id: "c-tabs", label: "Views",
                    Tab { href: "/tasks", current: true, "List" }
                    Tab { href: "/tasks?layout=board", current: false, "Board" }
                }
            }
        },
    ),
    (
        "dropdown_menu",
        "DropdownMenu",
        "The Notion top bar's « ... »: Export PDF, Export Markdown. Its trigger opens it, a click elsewhere closes it (Escape too on the web); closed, its items are no part of the text.",
        || {
            use crate::design::families::{DOTS, icon};
            rsx! {
                DropdownMenu { id: "c-menu", open: true, label: "Page menu", trigger: icon(DOTS),
                    DropdownMenuItem { id: "c-menu-pdf", "Export PDF" }
                    DropdownMenuItem { id: "c-menu-md", href: "data:text/markdown;charset=utf-8,%23%20Plan", download: "plan.md", "Export Markdown" }
                }
            }
        },
    ),
    (
        "diagram",
        "Diagram",
        "/deps' crate graph: its Mermaid source is the text, Mermaid with ELK's layout draws it, a box a layer.",
        || {
            rsx! {
                Diagram { id: "c-diagram", source: "flowchart LR\n  subgraph l0[\"Core\"]\n    c_core[\"core\"]\n  end\n  c_store[\"store\"] --> c_core\n" }
            }
        },
    ),
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

/// The Components tab: each component's example as a family's section, light and dark, its two forms folded under it.
#[component]
pub fn Components() -> Element {
    let rows = CATALOG.map(|(slug, name, used, example)| {
        super::families::specimen(slug, name, used, forms(example), Some(source(slug)))
    });
    rsx! {
        div { id: "ds-main", class: "ds-main",
            div { class: "ds-page",
                div { class: "ds-title", "Components" }
                div { class: "ds-what",
                    "Ported from dioxuslabs.com/components. Each is drawn light and dark; its HTML and the text an agent reads of it fold under it."
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
            r#"<div id="c-item" class="item access" data-status="works"><p class="head"><span class="badge chip">works</span><b class="name">fly</b></p><p class="usage"></p><p class="note"></p><p class="place"><span class="source"></span> </p></div><a id="c-row" class="item issue-row" data-task="a" href="/tasks/a"><b>a</b> Port the board</a>"#
        );
        assert_eq!(text, "works fly\na Port the board\n");
    }

    #[test]
    fn a_dropdown_menu_is_its_trigger_then_a_menu_of_its_items() {
        let _budget = TestBudget::new();
        let (html, text) = example("dropdown_menu");
        assert_eq!(
            html,
            r#"<div class="dropdown-menu" id="c-menu" data-state="open"><button class="button icon" type="button" aria-haspopup="menu" id="c-menu-trigger" aria-label="Page menu" aria-expanded="true" data-state="open"><svg class="ic" width="16" height="16" viewBox="0 0 24 24"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M5 12h.01M12 12h.01M19 12h.01"></path></svg></button><div class="dropdown-menu-content" role="menu" id="c-menu-content" aria-labelledby="c-menu-trigger" data-state="open"><button class="dropdown-menu-item" role="menuitem" type="button" id="c-menu-pdf">Export PDF</button><a class="dropdown-menu-item" role="menuitem" id="c-menu-md" href="data:text/markdown;charset=utf-8,%23%20Plan" download="plan.md">Export Markdown</a></div></div>"#
        );
        assert_eq!(text, "Export PDF\tExport Markdown\n");
        // closed, its items are hidden: no part of the text
        let closed = dioxus_ssr::render_element(rsx! {
            DropdownMenu { id: "m", open: false, label: "Menu", trigger: rsx! {}, DropdownMenuItem { id: "i", "Item" } }
        });
        assert!(closed.contains(r#"data-state="closed" hidden"#), "{closed}");
        assert_eq!(crate::forms::text_of(&closed).trim(), "");
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
                r#"<div id="c-skeleton" class="skeleton" aria-busy="true" role="status" aria-label="Usage: being read">{lines}<div id="c-skeleton-tool" class="tool skeleton" aria-busy="true" role="status" aria-label="fly: being read"><p class="head"><span class="avatar logo">{fly}</span><b class="title">fly</b></p>{lines}<div id="c-skeleton-card" class="card skeleton" aria-busy="true" role="status" aria-label="Being read"><b class="title"></b><span class="description">drafts approved, not yet sent</span></div>"#
            )
        );
        assert_eq!(text, "fly\ndrafts approved, not yet sent\n");
        assert_eq!(html.matches(crate::components::BUSY).count(), 3, "{html}");
    }

    /// The live part redraws itself from the page's URL in HTML, only while a busy card is in it, and stops.
    #[test]
    fn a_live_part_refreshes_itself_while_a_card_in_it_is_busy() {
        let _budget = TestBudget::new();
        let (html, text) = example("live");
        assert!(
            html.starts_with(r#"<div class="live" id="c-live"><div class="tool" id="c-tool">"#),
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
            "return o.remove()",
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
            html.starts_with(r#"<iframe referrerpolicy="no-referrer" sandbox="allow-popups allow-popups-to-escape-sandbox" class="frame themed" title="The mail as sent" srcdoc="&#60;meta http-equiv=&#34;Content-Security-Policy&#34; content=&#34;default-src &#39;none&#39;; img-src data:; style-src &#39;unsafe-inline&#39;; font-src data:&#34;&#62;"#),
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

    /// Task frame-dark-mode, its HTML side (the browser's is tests/notion_browser.rs): a mail that names no colour lets its
    /// document take the theme's scheme; one with a background, a text colour or a scheme of its own is left as sent.
    #[test]
    fn a_frame_follows_the_theme_unless_the_mail_has_colours() {
        let _budget = TestBudget::new();
        let scheme = r#"<meta name="color-scheme" content="light dark">"#;
        for plain in [
            "<p>Hello <b>Ada</b></p>",
            "<p>Your background in Rust: what color is the sky?</p>",
        ] {
            assert!(framed(plain).contains(scheme), "{plain}");
        }
        for own in [
            r##"<body bgcolor="#ffffff"><p>Hi</p>"##,
            r##"<table style="BACKGROUND:#fc0"><tr><td>Hi</td></tr></table>"##,
            r##"<p style="color:#333">Hi</p>"##,
            "<style>p { Color: red }</style><p>Hi</p>",
            r##"<body text="#000000" link="#0563c1"><p>Hi</p>"##,
            r#"<p>Hi</p><img src="data:image/png;base64,AA">"#,
        ] {
            assert!(!framed(own).contains(scheme), "{own}");
        }
    }

    /// A form sends to its action, by post unless asked get; a file field makes it multipart (task dash-settings-backup).
    #[test]
    fn a_form_is_its_fields_and_button_posted_unless_asked_get() {
        let _budget = TestBudget::new();
        let (html, text) = example("form");
        assert_eq!(
            html,
            r#"<form class="form" id="c-form-search" role="search" method="get" action="/search"><input class="input" name="q" placeholder="Search tasks" type="search"/><button class="button">Search</button></form><form class="form" id="c-form-upload" method="post" action="/settings/restore" enctype="multipart/form-data"><input class="input" accept=".zip" name="file" required=true type="file"/><button class="button">Read a backup</button></form>"#
        );
        assert_eq!(text, "Search\nRead a backup\n");
    }

    /// Task rust-tasks-views-toggle: a toggle is its options as links, the shown one marked, each its icon then its name.
    #[test]
    fn a_toggle_is_its_options_each_an_icon_and_a_name_the_shown_one_marked() {
        let _budget = TestBudget::new();
        let (html, text) = example("toggle");
        assert!(html.starts_with(r#"<nav class="toggle" id="c-toggle" aria-label="Views"><a href="/tasks" aria-current="page"><svg class="ic""#), "{html}");
        assert!(
            html.contains(r#"</svg>Board</a><a href="/tasks?layout=list"><svg"#),
            "{html}"
        );
        assert_eq!(text, "Board\tList\n");
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

    /// Done-when 1: /design?tab=components lists each, its library page linked, its HTML and its text; and each draws a class
    /// or an element the gate keeps to this file. Done-when 1 of rust-design-components-fit: each is a family's section, its
    /// name and Light and Dark over one row drawing it in both, its two forms in a closed fold, no raw pre (@nha, 2026-11-09:
    /// « these are all fairly ugly and don't fit other design tabs »).
    #[test]
    fn the_design_page_lists_every_component_with_both_forms() {
        let _budget = TestBudget::new();
        let page = crate::design::page("components", None);
        let text = crate::forms::text_of(&page);
        assert!(
            !page.contains(r#"<span class="ds-side">HTML</span>"#),
            "{page}"
        );
        // drawn light and dark, each id still once on the page (/code-review of rust-design-components-fit)
        let ids: Vec<&str> = page
            .split(r#" id=""#)
            .skip(1)
            .filter_map(|s| s.split('"').next())
            .collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "{ids:?}");
        for (slug, name, used, example) in CATALOG {
            let (html, words) = forms(example);
            let section = crate::design::families::specimen_html(
                slug,
                name,
                used,
                (&html, &words),
                Some(&source(slug)),
            );
            assert!(page.contains(&section), "{section} in {page}");
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
