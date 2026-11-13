//! /design?tab=components: each component of crate::components drawn, its HTML and its text side by side, so a page's two
//! forms are seen where they are made (task rust-design-components). Here, not in components.rs: it reads crate::forms,
//! which draws the menu with the components. Each component registers its own entry (crate::component).
use crate::component::Component;
use dioxus::prelude::*;

/// The components /design shows: every one registered with an example, by slug.
pub fn shown() -> impl Iterator<Item = &'static Component> {
    crate::component::registered()
        .iter()
        .copied()
        .filter(|c| c.example.is_some())
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
    let rows = shown().map(|c| {
        let example = c.example.expect("shown");
        super::families::specimen(
            c.slug,
            c.name,
            c.used,
            forms(example, &query),
            Some(c.source()),
        )
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
    use crate::component::{css, registered};
    use crate::components::*;
    use dash_meter::dash_test;

    fn entry(slug: &str) -> &'static Component {
        let found = registered().iter().copied().find(|c| c.slug == slug);
        found.expect(slug)
    }

    fn source(slug: &str) -> String {
        entry(slug).source()
    }

    /// Its example, or, for a component drawn on a family tab alone (the charts, task design-charts-tab), its first row there.
    fn example(slug: &str) -> (String, String) {
        let c = entry(slug);
        let first = || c.sections.first().and_then(|s| s.rows.first()).map(|r| r.1);
        forms(c.example.or_else(first).expect(slug), "")
    }

    #[dash_test]
    fn a_card_is_a_link_its_title_then_its_description() {
        let (html, text) = example("card");
        assert_eq!(
            html,
            r#"<a id="c-card" class="card" href="/tasks?project=site_perso"><b class="title">site_perso</b><span class="description">3 open, 1 in review</span></a><div id="c-card-reading" class="card skeleton" aria-busy="true" role="status" aria-label="Being read"><b class="title"></b><span class="description">drafts approved, not yet sent</span></div>"#
        );
        assert_eq!(
            text,
            "site_perso 3 open, 1 in review\ndrafts approved, not yet sent\n"
        );
    }

    #[dash_test]
    fn an_item_is_a_row_its_status_for_its_look_a_link_with_its_task() {
        let (html, text) = example("item");
        assert_eq!(
            html,
            r#"<div id="c-item" class="item access" data-status="works"><p class="head"><span class="badge chip">works</span><b class="name">fly</b></p><p class="usage"></p><p class="note"></p><p class="place"><span class="source"></span> </p></div><a id="c-row" class="item issue-row" data-task="a" href="/tasks/a"><b>a</b> Port the board</a>"#
        );
        assert_eq!(text, "works fly\na Port the board\n");
    }

    #[dash_test]
    fn a_dropdown_menu_is_its_trigger_then_a_menu_of_its_items() {
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
    #[dash_test]
    fn the_design_table_sorts_by_a_head_click_and_pages_its_rows() {
        let page = crate::markup(&crate::design::page(
            "components",
            None,
            "tab=components&sort=Since&dir=desc",
        ));
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
        let second = crate::forms::text_of(&crate::markup(&crate::design::page(
            "components",
            None,
            "tab=components&sort=Since&dir=desc&page=2",
        )));
        assert!(
            second.contains("sortable, design system first") && second.contains("page 2 of 2"),
            "{second}"
        );
    }

    #[dash_test]
    fn a_badge_is_its_look_and_its_words() {
        let (html, text) = example("badge");
        assert_eq!(
            html,
            r#"<span class="badge chip">works</span><span class="badge count">4</span><span class="badge li-tag">rust</span>"#
        );
        assert_eq!(text, "works 4 rust\n");
    }

    #[dash_test]
    fn an_avatar_is_the_initials() {
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
    /// name and logo kept, a grey line for each access to come (shown in Live's example, the part that refills it: Skeleton's
    /// own module draws no Tool, which draws a Skeleton, so the two modules hold no loop; task rust-components-register).
    #[dash_test]
    fn a_skeleton_is_busy_grey_lines_and_a_reading_tool_is_one() {
        let (html, text) = example("skeleton");
        let fly = include_str!("../logos/fly.svg");
        let lines = r#"<span class="line"></span><span class="line"></span></div>"#;
        assert_eq!(
            html,
            format!(
                r#"<div id="c-skeleton" class="skeleton" aria-busy="true" role="status" aria-label="Usage: being read">{lines}"#
            )
        );
        assert_eq!(text, "");
        assert_eq!(html.matches(crate::components::BUSY).count(), 1, "{html}");
        let (live, _) = example("live");
        let tool = format!(
            r#"<div id="c-live-reading" class="tool skeleton" aria-busy="true" role="status" aria-label="fly: being read"><p class="head"><span class="avatar logo">{fly}</span><b class="title">fly</b></p>{lines}"#
        );
        assert!(live.contains(&tool), "{live}");
        assert_eq!(live.matches(crate::components::BUSY).count(), 1, "{live}");
    }

    /// The live part redraws itself from the page's URL in HTML, only while a busy card is in it, and stops.
    #[dash_test]
    fn a_live_part_refreshes_itself_while_a_card_in_it_is_busy() {
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
        // the reading tool in it keeps its name, its accesses still to come
        assert_eq!(text, "fly\nworks fly\nfly\n");
    }

    #[dash_test]
    fn a_button_with_no_look_is_the_button_class_alone_its_attributes_passed() {
        let (html, text) = example("button");
        assert_eq!(
            html,
            r#"<button class="button">Search</button><button class="button quiet" type="button">Cancel</button><button class="button icon" aria-label="Menu" type="button">☰</button>"#
        );
        assert_eq!(text, "Search\tCancel\t☰\n");
    }

    /// A field with no value has no words: its text is nothing, its query is the URL's (`/search?q=`).
    #[dash_test]
    fn an_input_is_its_kind_name_and_placeholder() {
        let (html, text) = example("input");
        assert_eq!(
            html,
            r#"<input class="input" name="q" placeholder="Search tasks" type="search"/>"#
        );
        assert_eq!(text, "");
    }

    /// Its words are its value: empty, so its text is nothing.
    #[dash_test]
    fn a_textarea_is_its_name_rows_and_placeholder() {
        let (html, text) = example("textarea");
        assert_eq!(
            html,
            r#"<textarea class="textarea" name="text" placeholder="Your comment" required=true rows="3"></textarea>"#
        );
        assert_eq!(text, "");
    }

    /// A fold is its line, its words under it, and open only when asked: a present `open`, even « false », opens it.
    #[dash_test]
    fn a_fold_is_closed_unless_asked_open() {
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
    #[dash_test]
    fn a_frame_runs_no_script_and_loads_nothing_remote() {
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
    #[dash_test]
    fn a_frame_follows_the_theme_unless_the_mail_has_colours() {
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
    #[dash_test]
    fn a_form_is_its_fields_and_button_posted_unless_asked_get() {
        let (html, text) = example("form");
        assert_eq!(
            html,
            format!(
                r#"<form class="form" id="c-form-search" role="search" method="get" action="/search"><input class="input" name="q" placeholder="Search tasks" type="search"/><button class="button">Search</button></form><form class="form" id="c-form-upload" method="post" action="/settings/restore" enctype="multipart/form-data"><div class="dropzone"><label class="dropzone-area"><input class="dropzone-input" type="file" name="file" accept=".zip" required=true/><svg class="ic" width="16" height="16" viewBox="0 0 24 24"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12"></path></svg><span class="dropzone-text"><span class="dropzone-label">Drop a .zip file here, or choose one</span><span class="dropzone-file" aria-live="polite">No file chosen</span></span></label><button class="button">Read a backup</button><script>{DROPZONE_JS}</script></div></form>"#
            )
        );
        assert_eq!(
            text,
            "Search\nDrop a .zip file here, or choose one No file chosen\tRead a backup\n"
        );
    }

    /// Done-when 2 and 3 of settings-upload-styled: an upload is the File upload's drop zone, never the browser's bare picker:
    /// its input covers the zone's label, its icon and words, then the file chosen, then what sends it, inside the zone. The
    /// form's example and /design's File upload draw the same field, and /settings' restore node draws it too (screen.rs).
    #[dash_test]
    fn a_file_upload_is_a_drop_zone_its_file_named_and_its_submit_inside() {
        let (html, _) = example("dropzone");
        let zone = r#"<label class="dropzone-area"><input class="dropzone-input" type="file" name="file" accept=".zip" required=true/><svg class="ic""#;
        assert!(
            html.starts_with(&format!(
                r#"<div class="dropzone" id="c-file-upload">{zone}"#
            )),
            "{html}"
        );
        assert!(
            html.contains(&format!(r#"<span class="dropzone-text"><span class="dropzone-label">Drop a .zip file here, or choose one</span><span class="dropzone-file" aria-live="polite">No file chosen</span></span></label><button class="button">Read a backup</button><script>{DROPZONE_JS}</script></div>"#)),
            "{html}"
        );
        // the zone's script is asserted in place above; its other field on the markup alone (task rust-test-markup-only)
        assert!(crate::markup(&html).contains(r#"<input class="dropzone-input" type="file" name="files" accept=".png,.pdf" multiple=true/>"#), "{html}");
        let (form, _) = example("form");
        assert!(
            form.contains(&format!(
                r#"enctype="multipart/form-data"><div class="dropzone">{zone}"#
            )),
            "{form}"
        );
    }

    /// Done-when 3 of settings-upload-styled, the text form: an agent reads a zone as its words, the file chosen, then what
    /// sends it, a tab apart; no script, no picker. /design's Components tab shows that text under the zone's HTML.
    #[dash_test]
    fn a_file_upload_s_text_form_is_its_words_its_file_then_its_submit() {
        let (_, text) = example("dropzone");
        assert_eq!(
            text,
            "Drop a .zip file here, or choose one No file chosen\tRead a backup\nAttach images or PDFs: drop them here, or choose No file chosen\nAttach images or PDFs\n",
            "the compact one, a comment box's paper clip, reads as its name (task rust-doc-parity-rest)"
        );
        // several types read as words, never the raw list (/code-review)
        assert_eq!(
            drop_words(".png, .pdf"),
            "Drop a .png or .pdf file here, or choose one"
        );
        let tab = dioxus_ssr::render_element(rsx! { Components { query: String::new() } });
        let at = tab.find("File upload").expect("/design's File upload");
        let shown = &tab[at..];
        assert!(
            shown.contains("Drop a .zip file here, or choose one No file chosen"),
            "its text form shown on /design: {shown:.3000}"
        );
    }

    /// Done-when 3 of settings-upload-styled, after rust-ui's files: File upload is rust-ui's Dropzone, its page linked from
    /// /design, its notice kept, its ideas kept: a dashed area with an icon and words, tinted while a file is dragged over it
    /// (rust-ui's drag counter), a drop anywhere in it, each file named with rust-ui's size_display units.
    #[dash_test]
    fn a_file_upload_follows_rust_ui_s_dropzone() {
        assert_eq!(
            source("dropzone"),
            "https://rust-ui.com/docs/components/dropzone"
        );
        let tab = dioxus_ssr::render_element(rsx! { Components { query: String::new() } });
        let shown = &tab[tab.find("File upload").expect("/design's File upload")..];
        assert!(
            shown.contains("Built on rust-ui&#39;s Dropzone, which dioxus-components lacks.")
                && shown.contains("https://rust-ui.com/docs/components/dropzone"),
            "{shown:.3000}"
        );
        assert!(
            include_str!("../../NOTICE-rust-ui")
                .contains("FileUpload and DROPZONE_JS in src/components/each/dropzone.rs and their looks in src/components.css take their ideas from\nrust-ui's Dropzone")
        );
        for rule in [
            ".dropzone { position: relative;",
            "border: 1px dashed var(--color-base-300);",
            ".dropzone:hover, .dropzone[data-dragging] { border-color: var(--color-primary);",
            ".dropzone-area > .ic { flex: none; width: 24px; height: 24px; color: var(--color-primary); }",
            ".dropzone-input { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0;",
        ] {
            assert!(css().contains(rule), "{rule}");
        }
        for idea in [
            "z.dataset.over=(+z.dataset.over||0)+1",
            "document.addEventListener('drop'",
            "['KB','MB','GB']",
        ] {
            assert!(DROPZONE_JS.contains(idea), "{idea}");
        }
    }

    /// Task rust-tasks-views-toggle: a toggle is its options as links, the shown one marked, each its icon then its name.
    #[dash_test]
    fn a_toggle_is_its_options_each_an_icon_and_a_name_the_shown_one_marked() {
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
    #[dash_test]
    fn a_switch_is_a_checkbox_with_the_switch_role_checked_only_when_asked() {
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
    #[dash_test]
    fn a_toolbar_is_its_buttons_grouped_and_separated_along_its_orientation() {
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
    #[dash_test]
    fn a_hover_card_shows_its_content_on_a_hover_and_hides_it_from_the_text_closed() {
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
            assert!(css().contains(rule), "{rule}");
        }
    }

    /// Task rust-ui-controls: keys are `kbd`s, a chord's inside one group, each its own word in the text.
    #[dash_test]
    fn keys_are_kbds_a_chord_grouped_each_key_a_word() {
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
    #[dash_test]
    fn ask_is_a_search_form_named_search_or_ask_with_its_keys() {
        let (html, text) = example("ask");
        assert_eq!(
            html,
            r#"<form class="form" id="c-ask" role="search" method="get" action="/search"><div class="ask"><label><span class="sr-only">Search or ask</span><input class="input" name="q" placeholder="Search or ask" type="search"/></label><kbd class="kbd-group"><kbd class="kbd">⌘</kbd><kbd class="kbd">K</kbd></kbd></div></form>"#
        );
        assert_eq!(text, "Search or ask ⌘ K\n");
        let page = crate::markup(&crate::design::page("components", None, ""));
        assert!(
            page.contains(r#"id="c-ask""#) && page.contains(&source("ask")),
            "on /design"
        );
    }

    /// Done-when 1 of rust-ui-bottom-nav: a bottom nav is a labelled nav holding one grid row, each item its icon over its
    /// name, the shown one marked; More is a focusable label of the checkbox that shows the rest, so it needs no script;
    /// its looks keep rust-ui's ideas, and its page is rust-ui's.
    #[dash_test]
    fn a_bottom_nav_is_a_row_of_icons_over_names_the_shown_one_marked_more_last() {
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
            assert!(css().contains(rule), "{rule}");
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

    /// Done-when 1 of rust-charts-rust-ui: rust-ui's area, bar and line charts are entries on /design, each its rust-ui page,
    /// under its MIT notice; on the Charts tab since design-charts-tab.
    #[dash_test]
    fn charts_are_rust_ui_area_bar_line_each_its_entry_page_and_notice() {
        let page = crate::markup(&crate::design::page("charts", None, ""));
        let notice = include_str!("../../NOTICE-rust-ui");
        assert!(
            notice.contains("AreaChart, BarChart, LineChart") && notice.contains("MIT License"),
            "{notice}"
        );
        for (slug, name, page_of, id) in [
            ("area_chart", "AreaChart", "area-chart", "area"),
            ("bar_chart", "BarChart", "bar-chart", "bars"),
            ("line_chart", "LineChart", "line-chart", "line"),
        ] {
            assert_eq!(entry(slug).name, name);
            assert_eq!(
                source(slug),
                format!("https://rust-ui.com/charts/{page_of}")
            );
            assert!(
                page.contains(&format!(r#"id="s-{id}""#)) && page.contains(&source(slug)),
                "{slug} on /design"
            );
        }
    }

    /// Done-when 2 of rust-charts-rust-ui: each chart is an SVG the server drew, no script, nothing fetched (no URL, no
    /// src), and its text is its values as a table: the labels' column, then a column a series.
    #[dash_test]
    fn a_chart_is_svg_drawn_on_the_server_its_text_its_values_as_a_table() {
        for (slug, text) in [
            (
                "area_chart",
                "Week\tSent\tReplied\nW1\t6\t2\nW2\t8\t3\nW3\t4\t1\nW4\t11\t5\n",
            ),
            (
                "bar_chart",
                "Month\tNet\nJan\t2,500\nFeb\t-230\nMar\t1,800\n",
            ),
            ("line_chart", "Day\tReplies\nMon\t3\nTue\t5\nWed\t2\n"),
        ] {
            let (html, words) = example(slug);
            assert!(html.contains(r#" class="chart"><svg class="chart-svg" aria-hidden="true" viewBox="0 0 600 220">"#), "{html}");
            for remote in [
                "<script", "http", "src=", "href=", "<image", "<use", "@import",
            ] {
                assert!(!html.contains(remote), "{slug}: {remote} in {html}");
            }
            assert!(
                html.contains(r#"<table class="sr-only"><thead><tr><th>"#),
                "{html}"
            );
            assert_eq!(words, text, "{slug}");
        }
        let (bar, _) = example("bar_chart");
        assert!(
            bar.contains(r#"<path fill="currentColor" class="chart-bar neg""#)
                && bar.contains("<title>Feb: -230</title>"),
            "{bar}"
        );
        let (area, _) = example("area_chart");
        assert!(
            area.contains(r#"fill="url(#c-area-fade0)""#) && area.contains(r#"class="chart-key""#),
            "{area}"
        );
        // each mark's colour an attribute Blitz follows, a series' token from the CSS a browser reads (/code-review)
        assert!(
            area.contains(r#"stop-color="currentColor" stop-opacity="0.8""#),
            "{area}"
        );
        let line = &area[area.find(r#"class="chart-line""#).expect("a line")..];
        assert!(
            line[..line.find('>').expect("its end")]
                .contains(r#"fill="none" stroke="currentColor""#),
            "{area}"
        );
        assert_eq!(
            bar.matches("<path").count(),
            bar.matches(r#"<path fill="currentColor""#).count(),
            "{bar}"
        );
        for rule in [
            ".chart-s0 { color: var(--color-primary); }",
            "@media (max-width: 700px) { .chart-tick { font-size: 22px; }",
        ] {
            assert!(css().contains(rule), "{rule}");
        }
    }

    /// Task rust-ui-media: dioxus-components' AspectRatio, its two boxes, the outer one's padding its height.
    #[dash_test]
    fn an_aspect_ratio_is_a_box_padded_to_its_height_what_it_holds_inside() {
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
            assert!(css().contains(rule), "{rule}");
        }
    }

    /// Task rust-ui-media: rust-ui's Image on an AspectRatio, lazy and async, no words of its own.
    #[dash_test]
    fn an_image_is_held_to_its_ratio_loaded_lazily_and_says_nothing() {
        let (html, text) = example("image");
        assert_eq!(
            html,
            format!(
                r#"<div class="aspect-ratio" style="padding-bottom: 75%"><div class="aspect-ratio-content"><img class="image" loading="lazy" decoding="async" src="{PIXEL}" alt="A green square"/></div></div>"#
            )
        );
        assert_eq!(text, "");
        assert!(
            css().contains(
                ".image { display: block; width: 100%; height: 100%; object-fit: cover; }"
            )
        );
    }

    /// Task rust-ui-media: rust-ui's Attachment, a document its icon on an AspectRatio square, a picture its Image upright;
    /// its name then its kind as words; the tile its link, never a script's.
    #[dash_test]
    fn an_attachment_is_its_media_name_and_kind_a_picture_upright_a_script_no_link() {
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

    #[dash_test]
    fn tabs_are_links_the_current_one_marked() {
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
    #[dash_test]
    fn the_design_page_lists_every_component_with_both_forms() {
        let page = crate::markup(&crate::design::page("components", None, ""));
        let text = crate::forms::text_of(&page);
        // HTML over a column only where a table or a graph shows its text beside it (task design-agent-forms)
        assert_eq!(
            page.matches(r#"<span class="ds-side">HTML</span>"#).count(),
            shown().filter(|c| c.formed).count(),
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
        for c in shown() {
            let (slug, name, used) = (c.slug, c.name, c.used);
            let (html, words) = forms(c.example.expect(slug), "");
            let section = crate::design::families::specimen_html(
                slug,
                name,
                used,
                (&html, &words),
                Some(&source(slug)),
            );
            assert!(
                page.contains(&crate::markup(&section)),
                "{section} in {page}"
            );
            for line in words.lines() {
                assert!(text.contains(line), "{slug}'s text {line:?} in {text}");
            }
            let owned = registered().iter().any(|o| {
                o.tags.iter().any(|t| html.starts_with(&format!("<{t} ")))
                    || o.classes
                        .iter()
                        .any(|c| html.contains(&format!(r#"class="{c}"#)))
            });
            assert!(owned, "{slug} draws nothing a component registers: {html}");
        }
    }
}
