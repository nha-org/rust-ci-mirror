//! /design?tab=components: each component of crate::components drawn, its HTML and its text side by side, so a page's two
//! forms are seen where they are made (task rust-design-components). Here, not in components.rs: it reads crate::forms,
//! which draws the menu with the components.
use crate::components::*;
use dioxus::prelude::*;

/// /design?tab=components: (slug, name, where it is used, an example). Its page is dioxuslabs' component page of `slug`.
pub type Entry = (
    &'static str,
    &'static str,
    &'static str,
    fn(&str) -> Element,
);

pub const CATALOG: [Entry; 32] = [
    (
        "card",
        "Card",
        "Only where a card was approved: the drafts' previews and the board's Linear cards (look li-card); a list of things is a List or a Table (@nha, 2026-11-09: « Cards are not the default UI element »).",
        |_| {
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
        |_| {
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
        "The comment queue at /comments, /settings' stores: a head click sorts by its column, again reverses it; past its rows a page, Pagination pages it and a field filters it, all in the URL; its caption, the text's first line too, says the total, and the text the next page's URL.",
        |query| {
            let row = |cells: [&str; 3], href: Option<&str>| TableRow {
                cells: cells.map(String::from).to_vec(),
                href: href.map(String::from),
            };
            rsx! {
                Table { id: "c-table", columns: vec!["Comment".into(), "Page".into(), "Since".into()],
                    rows: vec![
                        row(["a table, not cards", "/comments", "2026-11-09 10:00"], Some("/comments#c-a")),
                        row(["the old app's", "/x", "2026-11-09 11:00"], None),
                        row(["sortable, design system first", "/settings", "2026-11-09 09:30"], Some("/settings")),
                        row(["Agent view reads it", "/design", "2026-11-10 08:15"], Some("/design")),
                        row(["a pager for long queues", "/comments", "2026-11-10 12:40"], None),
                    ],
                    query: query.to_string(), per_page: 3,
                }
            }
        },
    ),
    (
        "item",
        "Item",
        "The tools page's accesses, a row each; the tasks list's rows, look issue-row, each its task's link.",
        |_| {
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
        |_| {
            rsx! { Badge { look: "chip", "works" } Badge { look: "count", "4" } Badge { look: "li-tag", "rust" } }
        },
    ),
    (
        "avatar",
        "Avatar",
        "A board card's assignee; a tool's logo (Simple Icons', else its favicon), its initials where none is held.",
        |_| {
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
        |_| {
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
        |_| {
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
        |_| rsx! { Button { "Search" } Button { look: "quiet", r#type: "button", "Cancel" } Button { look: "icon", r#type: "button", aria_label: "Menu", "☰" } },
    ),
    (
        "input",
        "Input",
        "The search bar's field, the menu's fold.",
        |_| {
            rsx! { Input { r#type: "search", name: "q", placeholder: "Search tasks" } }
        },
    ),
    (
        "textarea",
        "Textarea",
        "A draft's Request corrections; every compose box is the documents editor (Composer).",
        |_| {
            rsx! { Textarea { name: "text", rows: "3", placeholder: "Your comment", required: true } }
        },
    ),
    (
        "collapsible",
        "Fold",
        "A conversation's mails on /mail/t/<thread>: the older ones folded to a line, the latest open.",
        |_| {
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
        |_| rsx! { Frame { title: "The mail as sent", html: "<p>Hello <b>Ada</b></p>" } },
    ),
    (
        "board",
        "Board, Column, CardLine",
        "The tasks board: its columns side by side, a card dragged to another column takes its status (/tasks); a column past its first cards links to them all.",
        |_| {
            let more = Some(crate::components::More {
                count: 41,
                href: "/tasks?layout=list#todo".into(),
            });
            rsx! {
                Board { id: "c-board",
                    Column { id: "c-column", status: "todo", name: "Todo", count: 42, icon: rsx! {}, more,
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
        |_| {
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
        |_| {
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
        "dialog",
        "Dialog",
        "The board's « + New issue »: a pill that opens the browser's own modal, no script, its form a title and a description posted to /tasks/new.",
        |_| rsx! { NewIssue { id: "c-new-issue", project: None } },
    ),
    (
        "copy",
        "CopyButton",
        "An issue page's round copy buttons, as Linear's: Copy link (after the page's origin), Copy ID, Copy branch name.",
        |_| {
            use crate::design::families::{LINK, icon};
            rsx! { Copies { CopyButton { id: "c-copy", label: "Copy link", text: "/tasks/a-task", origin: true, icon: icon(LINK) } } }
        },
    ),
    (
        "select",
        "PropertyMenu",
        "An issue's property he sets from its page, Linear's sidebar menus: status, priority, due date; each pick posts its field to /tasks/<id>/set, with no script.",
        |_| {
            rsx! {
                PropertyMenu { id: "c-prop", action: "/tasks/a-task/set", field: "priority", label: "Set priority",
                    current: rsx! { span { "High" } },
                    choices: vec![("1".into(), "Urgent".into(), false), ("2".into(), "High".into(), true)] }
            }
        },
    ),
    (
        "tabs",
        "Tabs",
        "The tasks board's views; a tree's « links »: /mail's accounts and pager, /settings' download.",
        |_| {
            rsx! {
                Tabs { id: "c-tabs", label: "Views",
                    Tab { href: "/tasks", current: true, "List" }
                    Tab { href: "/tasks?layout=board", current: false, "Board" }
                }
            }
        },
    ),
    (
        "bottom_nav",
        "BottomNav",
        "Every page's menu on a phone, under 700 px wide: its first entries at the screen's foot, the rest behind More, which opens the sidebar. Built on rust-ui's BottomNav, which dioxus-components lacks.",
        |_| {
            use crate::design::families::{CALENDAR, DOTS, INBOX, MAIL, icon};
            rsx! {
                BottomNav { id: "c-bottom-nav", label: "Pages",
                    BottomNavItem { href: "/inbox", current: true, icon: icon(INBOX), "Inbox" }
                    BottomNavItem { href: "/today", current: false, icon: icon(CALENDAR), "Today" }
                    BottomNavItem { href: "/mail", current: false, icon: icon(MAIL), "Mail" }
                    BottomNavMore { id: "c-bottom-nav-more", r#for: "c-bottom-nav-fold", controls: "c-bottom-nav-rest", current: false, icon: icon(DOTS), "More" }
                }
            }
        },
    ),
    (
        "dropdown_menu",
        "DropdownMenu",
        "The Notion top bar's « ... »: Export PDF, Export Markdown. Its trigger opens it, a click elsewhere closes it (Escape too on the web); closed, its items are no part of the text.",
        |_| {
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
        "switch",
        "Switch",
        "An on or off choice in a form: a tap flips it, the form sends it, no script. No page holds one yet.",
        |_| {
            rsx! {
                Switch { id: "c-switch-on", name: "notify", checked: true, "Notify me" }
                Switch { id: "c-switch-off", name: "digest", checked: false, "Daily digest" }
            }
        },
    ),
    (
        "toolbar",
        "Toolbar",
        "The Notion tab's bar over a pick: its marks, Comment and the skills, a press on it keeping the pick.",
        |_| {
            rsx! {
                Toolbar { id: "c-toolbar", label: "Selection",
                    ToolbarGroup {
                        ToolbarButton { "B" }
                        ToolbarButton { "I" }
                    }
                    ToolbarSeparator {}
                    ToolbarButton { id: "c-toolbar-comment", "Comment" }
                }
            }
        },
    ),
    (
        "hover_card",
        "HoverCard",
        "A tasks board card: resting on it shows its task's title and why under it.",
        |_| {
            rsx! {
                HoverCard { open: true,
                    content: rsx! { CardTitle { "Port the board" } CardDescription { "The old board drifts from the design system." } },
                    Badge { look: "li-tag", "AAA-12" }
                }
            }
        },
    ),
    (
        "kbd",
        "Kbd",
        "A key to press: ⌘ K in the menu's Search or ask field (Ask). Built on rust-ui's Kbd, which dioxus-components lacks.",
        |_| rsx! { KbdGroup { Kbd { "⌘" } Kbd { "K" } } Kbd { "Esc" } },
    ),
    (
        "ask",
        "Ask",
        "The menu's Search or ask field, beside the compass on every page: ⌘ K focuses it, Enter searches /search. Built on Input and Kbd.",
        |_| rsx! { Ask { id: "c-ask" } },
    ),
    (
        "aspect_ratio",
        "AspectRatio",
        "Holds a picture's room while it loads: under each Image and an Attachment's media square.",
        |_| {
            rsx! {
                AspectRatio { ratio: 16.0 / 9.0, Badge { look: "chip", "16:9" } }
            }
        },
    ),
    (
        "image",
        "Image",
        "A picture a mail carries on /mail/t/<thread>, inside its Attachment, cropped to its ratio. Built on rust-ui's Image, which dioxus-components lacks.",
        |_| {
            rsx! {
                Image { src: PIXEL, alt: "A green square", ratio: 4.0 / 3.0 }
            }
        },
    ),
    (
        "attachment",
        "Attachment",
        "A mail's files on /mail/t/<thread>, side by side: a document its icon, its name and kind, a picture shown upright. Built on rust-ui's Attachment, which dioxus-components lacks.",
        |_| {
            rsx! {
                AttachmentGroup {
                    Attachment { id: "c-attachment-pdf", name: "slots.pdf", kind: "application/pdf", href: "/mail/blob/b1?name=slots.pdf" }
                    Attachment { id: "c-attachment-png", name: "room.png", kind: "image/png", href: PIXEL, picture: true }
                    Attachment { id: "c-attachment-none", name: "notes", kind: "text/plain", href: "javascript:alert(1)" }
                }
            }
        },
    ),
    (
        "diagram",
        "Diagram",
        "/deps' crate graph: its Mermaid source is the text, Mermaid with ELK's layout draws it, a box a layer.",
        |_| {
            rsx! {
                Diagram { id: "c-diagram", source: "flowchart LR\n  subgraph l0[\"Core\"]\n    c_core[\"core\"]\n  end\n  c_store[\"store\"] --> c_core\n" }
            }
        },
    ),
    (
        "treemap",
        "Treemap",
        "/deps' code map: layers, crates, modules and files nested, a box's area its lines of code; hovering a box outlines what it uses and what uses it.",
        |_| {
            let leaf = |name: &str, size: u64, uses: &[&str]| Nest {
                name: name.into(),
                size,
                uses: uses.iter().map(|u| u.to_string()).collect(),
                children: vec![],
            };
            let boxes = vec![
                Nest {
                    name: "Rings".into(),
                    size: 0,
                    uses: vec!["Core".into()],
                    children: vec![
                        leaf("store", 900, &["Core/core"]),
                        leaf("surface", 1400, &["Core/core"]),
                    ],
                },
                Nest {
                    name: "Core".into(),
                    size: 0,
                    uses: vec![],
                    children: vec![leaf("core", 700, &[])],
                },
            ];
            rsx! { Treemap { id: "c-treemap", boxes } }
        },
    ),
];

/// A one-pixel green PNG: the catalog's picture, drawn with nothing fetched.
const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/// The library's page of a component: rust-ui's for those dioxus-components lacks (Kbd, Image, Attachment, BottomNav).
pub fn source(slug: &str) -> String {
    match slug {
        "kbd" | "image" | "attachment" | "bottom_nav" => {
            format!(
                "https://rust-ui.com/docs/components/{}",
                slug.replace('_', "-")
            )
        }
        // ours, on dioxus-components' Input
        "ask" => "https://dioxuslabs.github.io/dioxus-components/component/?name=input".into(),
        _ => format!("https://dioxuslabs.github.io/dioxus-components/component/?name={slug}"),
    }
}

/// An example's two forms: its HTML, the server render, and the text an agent reads of it.
pub fn forms(example: fn(&str) -> Element, query: &str) -> (String, String) {
    let html = dioxus_ssr::render_element(example(query));
    let text = crate::forms::text_of(&html);
    (html, text)
}

/// The Components tab: each component's example as a family's section, light and dark, its two forms folded under it.
#[component]
pub fn Components(query: String) -> Element {
    let rows = CATALOG.map(|(slug, name, used, example)| {
        super::families::specimen(slug, name, used, forms(example, &query), Some(source(slug)))
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
        forms(entry.3, "")
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

    /// Done-when 1 and 3 of rust-table-sortable on /design: the components tab's table sorts by the head clicked, again
    /// reverses, and pages three rows a page, the tab kept in each link; its caption says the order.
    #[test]
    fn the_design_table_sorts_by_a_head_click_and_pages_its_rows() {
        let _budget = TestBudget::new();
        let page = crate::design::page("components", None, "tab=components&sort=Since&dir=desc");
        let since = r#"aria-sort="descending"><a href="?tab=components&#38;sort=Since">Since</a>"#;
        assert!(page.contains(since), "{page}");
        let words = crate::forms::text_of(&page);
        let newest = [
            "a pager for long queues",
            "Agent view reads it",
            "the old app's",
        ];
        let at: Vec<usize> = newest.iter().map(|w| words.find(w).expect(w)).collect();
        assert!(at.is_sorted(), "{words}");
        assert!(
            !words.contains("sortable, design system first"),
            "on page 2: {words}"
        );
        assert!(
            words.contains("5 rows; sorted by Since, descending; page 1 of 2. Next page: ?tab=components&sort=Since&dir=desc&page=2"),
            "{words}"
        );
        assert!(
            page.contains(r#"href="?tab=components&#38;sort=Since&#38;dir=desc&#38;page=2""#),
            "{page}"
        );
        let second = crate::forms::text_of(&crate::design::page(
            "components",
            None,
            "tab=components&sort=Since&dir=desc&page=2",
        ));
        assert!(
            second.contains("sortable, design system first") && second.contains("page 2 of 2"),
            "{second}"
        );
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

    /// Task rust-ui-controls: a switch is one checkbox with the switch role in its label, its track drawn beside it; checked
    /// only when asked, so a form sends it as it shows.
    #[test]
    fn a_switch_is_a_checkbox_with_the_switch_role_checked_only_when_asked() {
        let _budget = TestBudget::new();
        let (html, text) = example("switch");
        let track = r#"<span class="switch-track" aria-hidden="true"><span class="switch-thumb"></span></span>"#;
        assert_eq!(
            html,
            format!(
                r#"<label class="switch"><input class="switch-input" type="checkbox" role="switch" id="c-switch-on" name="notify" checked=true/>{track}Notify me</label><label class="switch"><input class="switch-input" type="checkbox" role="switch" id="c-switch-off" name="digest"/>{track}Daily digest</label>"#
            )
        );
        assert_eq!(text, "Notify me Daily digest\n");
    }

    /// Task rust-ui-controls: a toolbar is its role, label and orientation, its buttons never submit, a separator runs across
    /// its line; a vertical one's separators lie flat.
    #[test]
    fn a_toolbar_is_its_buttons_grouped_and_separated_along_its_orientation() {
        let _budget = TestBudget::new();
        let (html, text) = example("toolbar");
        assert_eq!(
            html,
            r#"<div role="toolbar" id="c-toolbar" class="toolbar" aria-label="Selection" aria-orientation="horizontal" data-orientation="horizontal"><div role="group" class="toolbar-group"><button type="button" class="toolbar-button">B</button><button type="button" class="toolbar-button">I</button></div><div class="toolbar-separator" role="separator" aria-orientation="vertical"></div><button type="button" id="c-toolbar-comment" class="toolbar-button">Comment</button></div>"#
        );
        assert_eq!(text, "B\tI\nComment\n");
        let vertical = dioxus_ssr::render_element(rsx! {
            Toolbar { id: "t", label: "T", vertical: true, ToolbarSeparator {} ToolbarGroup { title: "Skills", ToolbarButton { "a" } } }
        });
        assert!(
            vertical.contains(r#"data-orientation="vertical""#),
            "{vertical}"
        );
        assert!(
            vertical.contains(r#"role="separator" aria-orientation="horizontal""#),
            "{vertical}"
        );
        assert!(
            vertical.contains(r#"<span class="toolbar-title">Skills</span>"#),
            "{vertical}"
        );
    }

    /// Task rust-ui-controls: a hover card is its trigger then its content; closed, the content is hidden and no part of the
    /// text, and its look shows it on a hover or a focus (its own or its holder's), after a wait, with no script, letting a
    /// click through to what it covers.
    #[test]
    fn a_hover_card_shows_its_content_on_a_hover_and_hides_it_from_the_text_closed() {
        let _budget = TestBudget::new();
        let (html, text) = example("hover_card");
        assert_eq!(
            html,
            r#"<div class="hover-card" data-state="open"><span class="badge li-tag">AAA-12</span><div class="hover-card-content" data-state="open"><b class="title">Port the board</b><span class="description">The old board drifts from the design system.</span></div></div>"#
        );
        assert_eq!(
            text,
            "AAA-12\nPort the board The old board drifts from the design system.\n"
        );
        let closed =
            dioxus_ssr::render_element(rsx! { HoverCard { content: rsx! { "more" }, "face" } });
        assert!(closed.contains(r#"data-state="closed" hidden"#), "{closed}");
        assert_eq!(crate::forms::text_of(&closed), "face\n");
        for rule in [
            "pointer-events: none; }",
            ".hover-card:hover > .hover-card-content, .hover-card:focus-within > .hover-card-content, :focus-visible > .hover-card > .hover-card-content { display: block; animation: hover-card-in 100ms 500ms both; }",
        ] {
            assert!(CSS.contains(rule), "{rule}");
        }
    }

    /// Task rust-ui-controls: keys are `kbd`s, a chord's inside one group, each its own word in the text.
    #[test]
    fn keys_are_kbds_a_chord_grouped_each_key_a_word() {
        let _budget = TestBudget::new();
        let (html, text) = example("kbd");
        assert_eq!(
            html,
            r#"<kbd class="kbd-group"><kbd class="kbd">⌘</kbd><kbd class="kbd">K</kbd></kbd><kbd class="kbd">Esc</kbd>"#
        );
        assert_eq!(text, "⌘ K Esc\n");
        assert_eq!(source("kbd"), "https://rust-ui.com/docs/components/kbd");
    }

    /// Done-when 3 of rust-notion-one-menu: the menu's Search or ask field on /design, its HTML a search form to /search with
    /// ⌘ K in it, its text the field's name and its keys.
    #[test]
    fn ask_is_a_search_form_named_search_or_ask_with_its_keys() {
        let _budget = TestBudget::new();
        let (html, text) = example("ask");
        assert_eq!(
            html,
            r#"<form class="form" id="c-ask" role="search" method="get" action="/search"><div class="ask"><label><span class="sr-only">Search or ask</span><input class="input" name="q" placeholder="Search or ask" type="search"/></label><kbd class="kbd-group"><kbd class="kbd">⌘</kbd><kbd class="kbd">K</kbd></kbd></div></form>"#
        );
        assert_eq!(text, "Search or ask ⌘ K\n");
        let page = crate::design::page("components", None, "");
        assert!(
            page.contains(r#"id="c-ask""#) && page.contains(&source("ask")),
            "on /design"
        );
    }

    /// Done-when 1 of rust-ui-bottom-nav: a bottom nav is a labelled nav holding one grid row, each item its icon over its
    /// name, the shown one marked; More is a focusable label of the checkbox that shows the rest, so it needs no script;
    /// its looks keep rust-ui's ideas, and its page is rust-ui's.
    #[test]
    fn a_bottom_nav_is_a_row_of_icons_over_names_the_shown_one_marked_more_last() {
        let _budget = TestBudget::new();
        let (html, text) = example("bottom_nav");
        assert!(
            html.starts_with(r#"<nav class="bottom-nav" id="c-bottom-nav" aria-label="Pages"><div class="bottom-nav-grid"><a class="bottom-nav-item" href="/inbox" aria-current="page"><svg class="ic""#),
            "{html}"
        );
        assert!(
            html.contains(r#"</svg><span class="bottom-nav-label">Inbox</span></a><a class="bottom-nav-item" href="/today"><svg"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<label class="bottom-nav-item" role="button" tabindex="0" id="c-bottom-nav-more" for="c-bottom-nav-fold" aria-controls="c-bottom-nav-rest"><svg"#),
            "{html}"
        );
        assert!(
            html.ends_with(r#"<span class="bottom-nav-label">More</span></label></div></nav>"#),
            "{html}"
        );
        assert_eq!(text, "Inbox\tToday\tMail\tMore\n");
        for rule in [
            ".bottom-nav-grid { display: grid; grid-auto-flow: column; grid-auto-columns: minmax(0, 1fr);",
            "padding-bottom: env(safe-area-inset-bottom, 0px);",
            ".bottom-nav-item:hover, .bottom-nav-item[aria-current] { color: var(--color-primary); }",
            ".bottom-nav-item:active { transform: scale(0.98); }",
            "-webkit-tap-highlight-color: transparent; -webkit-touch-callout: none;",
        ] {
            assert!(CSS.contains(rule), "{rule}");
        }
        assert_eq!(
            source("bottom_nav"),
            "https://rust-ui.com/docs/components/bottom-nav"
        );
        let notice = include_str!("../../NOTICE-rust-ui");
        assert!(
            notice.contains("BottomNav, BottomNavItem and BottomNavMore"),
            "{notice}"
        );
    }

    /// Task rust-ui-media: dioxus-components' AspectRatio, its two boxes, the outer one's padding its height.
    #[test]
    fn an_aspect_ratio_is_a_box_padded_to_its_height_what_it_holds_inside() {
        let _budget = TestBudget::new();
        let (html, text) = example("aspect_ratio");
        assert_eq!(
            html,
            r#"<div class="aspect-ratio" style="padding-bottom: 56.25%"><div class="aspect-ratio-content"><span class="badge chip">16:9</span></div></div>"#
        );
        assert_eq!(text, "16:9\n");
        let square = |ratio: f64| dioxus_ssr::render_element(rsx! { AspectRatio { ratio, "x" } });
        assert_eq!(square(0.0), square(1.0), "no positive ratio is a square");
        assert!(square(f64::NAN).contains("padding-bottom: 100%"));
        for rule in [
            ".aspect-ratio { position: relative; width: 100%; height: 0; }",
            ".aspect-ratio-content { position: absolute; inset: 0;",
        ] {
            assert!(CSS.contains(rule), "{rule}");
        }
    }

    /// Task rust-ui-media: rust-ui's Image on an AspectRatio, lazy and async, no words of its own.
    #[test]
    fn an_image_is_held_to_its_ratio_loaded_lazily_and_says_nothing() {
        let _budget = TestBudget::new();
        let (html, text) = example("image");
        assert_eq!(
            html,
            format!(
                r#"<div class="aspect-ratio" style="padding-bottom: 75%"><div class="aspect-ratio-content"><img class="image" loading="lazy" decoding="async" src="{PIXEL}" alt="A green square"/></div></div>"#
            )
        );
        assert_eq!(text, "");
        assert!(
            CSS.contains(
                ".image { display: block; width: 100%; height: 100%; object-fit: cover; }"
            )
        );
    }

    /// Task rust-ui-media: rust-ui's Attachment, a document its icon on an AspectRatio square, a picture its Image upright;
    /// its name then its kind as words; the tile its link, never a script's.
    #[test]
    fn an_attachment_is_its_media_name_and_kind_a_picture_upright_a_script_no_link() {
        let _budget = TestBudget::new();
        let (html, text) = example("attachment");
        assert!(html.starts_with(r#"<div class="attachment-group"><a class="attachment" id="c-attachment-pdf" data-orientation="horizontal" href="/mail/blob/b1?name=slots.pdf" title="slots.pdf"><div class="attachment-media"><div class="aspect-ratio" style="padding-bottom: 100%"><div class="aspect-ratio-content"><svg class="ic""#), "{html}");
        assert!(html.contains(r#"<span class="attachment-content"><b class="attachment-title">slots.pdf</b><span class="attachment-description">PDF</span></span></a>"#), "{html}");
        assert!(html.contains(&format!(r#"<div class="attachment" id="c-attachment-png" data-orientation="vertical"><div class="attachment-media"><div class="aspect-ratio" style="padding-bottom: 75%"><div class="aspect-ratio-content"><img class="image" loading="lazy" decoding="async" src="{PIXEL}" alt=""/>"#)), "{html}");
        assert!(html.contains(r#"<div class="attachment" id="c-attachment-none" data-orientation="horizontal"><div class="attachment-media">"#), "{html}");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(
            html.contains(r#"<span class="attachment-description">text/plain</span>"#),
            "no extension: the MIME type"
        );
        assert_eq!(text, "slots.pdf PDF\nroom.png PNG\nnotes text/plain\n");
        for (kind, picture) in [
            ("image/jpeg", true),
            ("Image/PNG", true),
            ("image/webp; name=a.webp", true),
            ("image/svg+xml", false),
            ("image/svg+xml; charset=utf-8", false),
            ("image/tiff", false),
            ("application/pdf", false),
        ] {
            assert_eq!(is_picture(kind), picture, "{kind}");
        }
        // only this app's pictures or its own bytes are fetched, never another host's; an extension is letters and digits
        let tile = |name: &str, href: &str| {
            dioxus_ssr::render_element(
                rsx! { Attachment { id: "t", name, kind: "image/png", href, picture: true } },
            )
        };
        let remote = tile("p.png", "https://track.example/p.png");
        assert!(
            !remote.contains("<img") && remote.contains(r#"href="https://track.example/p.png""#),
            "{remote}"
        );
        assert!(tile("Minutes 10.30 am", "/x").contains(">image/png<"));
        assert_eq!(
            source("attachment"),
            "https://rust-ui.com/docs/components/attachment"
        );
        assert_eq!(
            source("aspect_ratio"),
            "https://dioxuslabs.github.io/dioxus-components/component/?name=aspect_ratio"
        );
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
        let page = crate::design::page("components", None, "");
        let text = crate::forms::text_of(&page);
        // HTML over a column only where a table or a graph shows its text beside it (task design-agent-forms)
        assert_eq!(
            page.matches(r#"<span class="ds-side">HTML</span>"#).count(),
            crate::design::families::FORMED.len(),
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
            let (html, words) = forms(example, "");
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
