//! The design system page: /design's six families in Dioxus on design.json's tokens, each state light and dark side by
//! side, then Linear-like cards and Notion-like comments to sit beside the evidence screens (@nha, 2026-11-06: « this will
//! allow knowin we can style things. »). Moved from rust/ui-spike/dioxus-page, since retired (task rust-design-page;
//! @nha, 2026-11-08: « please build /design (design system) from the spike »): dash serve renders it at /design, dash ui
//! --design draws it in the window.
use blitz_dom::{BaseDocument, local_name};
use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub mod catalog;
pub(crate) mod families;
pub use families::{Avatar, Badge, rect};
pub(crate) mod linear;
mod notion;
pub(crate) mod notion_doc;

use crate::THEME;
/// The spike's base components (buttons, badges, avatars), which its own screens share.
pub const CSS: &str = include_str!("base.css");
pub const DESIGN_CSS: &str = include_str!("design.css");
/// The web's handlers for the families' live parts (Stage's pick): a server render runs none.
/// Blocks its port: no dioxus-liveview runs, so a web page is a dioxus-ssr picture with no handlers
/// (rfc-rust-unified#ui-lib's drift; liveview's own future is open upstream,
/// https://github.com/DioxusLabs/dioxus/issues/2699); the window runs these handlers already.
const DESIGN_JS: &str = include_str!("design.js");
/// A Linear card and its column: /design's and the board's one look.
pub const CARDS_CSS: &str = include_str!("cards.css");

type Family = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

/// DesignLive's @families, in its order: slug, title, what it holds, its sections. The test keeps the two the same.
pub const FAMILIES: [Family; 6] = [
    (
        "layout",
        "Layout",
        "The rules every page follows, the shell and the parts a page is framed with.",
        &[
            ("guidelines", "Guidelines"),
            ("heroui", "From HeroUI"),
            ("shell", "Shell and menu"),
            ("bell", "Notification bell"),
            ("shared", "One per thing"),
            ("cards", "Cards"),
            ("colors", "Colours"),
        ],
    ),
    (
        "data",
        "Data",
        "How a quantity is shown, then the parts that show it: tables, charts, timelines, lists.",
        &[
            ("data", "Presenting data"),
            ("charts", "Charts"),
            ("stats", "Figures as visuals"),
            ("lists", "Lists of things"),
            ("records", "Records, after Twenty"),
            ("knowledge", "Knowledge entry"),
        ],
    ),
    (
        "forms",
        "Forms",
        "What he presses, picks and types: actions, toggles, fields, the text editor.",
        &[
            ("buttons", "Actions"),
            ("group", "Toggle group"),
            ("field", "Form field"),
            ("text", "Text editor"),
        ],
    ),
    (
        "feedback",
        "Feedback",
        "What tells him where a thing stands, and his comments and approvals on it.",
        &[
            ("stage", "Stage"),
            ("wait", "Wait badge"),
            ("chips", "Badges and chips"),
            ("reading", "Chances and reasons"),
            ("comments", "Comments"),
            ("comments-principles", "Comments on HeroUI's principles"),
            ("handoff", "Handoff and approval"),
        ],
    ),
    (
        "mail",
        "Mail",
        "The parts /mail, /mail/t/<thread> and /mail/compose draw with: the frame, the list's bar, the inbox's row, a conversation's head and mails, the reply box and a new mail.",
        &[("thread", "Thread"), ("composer", "Composer")],
    ),
    (
        "board",
        "Board",
        "The agents' work: the board, a project's issues, a task's page and its card, a terminal it runs in.",
        &[
            ("lanes", "Board"),
            ("issues", "A project's issues"),
            ("tasks", "Task page"),
            ("work-card", "Step or task card"),
            ("agents", "Agent lanes"),
            ("terminal", "Terminal"),
        ],
    ),
];

/// The bar's tabs past the families, each in the theme its evidence screen is in: dark or not.
/// Notion's editor is the Documents tab's first part, not a tab of its own: one document component, its types under it
/// (@nha, 2026-11-09: « Aim to simplify and unify not create one-offs every time. »); `?tab=notion` still opens it.
const SHOWN: [(&str, &str, bool); 3] = [
    ("linear", "Linear cards", false),
    ("components", "Components", false),
    ("documents", "Documents", false),
];

/// Every name /design shows a component or view under: its families and their sections, its other tabs, the components tab's
/// entries and the documents tab's types, « Task brief » as task-brief. A Rust task names one as the layer it changes (task
/// rust-task-names-layer).
pub fn names() -> Vec<String> {
    let families = FAMILIES
        .iter()
        .flat_map(|f| std::iter::once(f.0).chain(f.3.iter().map(|s| s.0)))
        .chain(
            crate::component::registered()
                .iter()
                .flat_map(|c| c.sections.iter().map(|s| s.id)),
        );
    let tabs = SHOWN
        .iter()
        .map(|s| s.0)
        .chain(catalog::shown().map(|c| c.slug));
    let mut names: Vec<String> = families.chain(tabs).map(String::from).collect();
    names.extend(
        crate::documents::TYPES
            .iter()
            .map(|t| t.0.to_lowercase().replace(' ', "-")),
    );
    names.sort();
    names.dedup();
    names
}

/// A tab's index of its slug, `?tab=` on the web: the families, then SHOWN, then the family holding a section of that slug
/// (`?tab=charts` is Data's, task rust-charts-rust-ui). Unknown is the first.
pub fn tab_of(slug: &str) -> usize {
    let slug = if slug == "notion" { "documents" } else { slug };
    let slugs = FAMILIES
        .iter()
        .map(|f| f.0)
        .chain(SHOWN.iter().map(|s| s.0));
    let holding = || {
        FAMILIES.iter().position(|f| {
            f.3.iter().any(|s| s.0 == slug) || crate::component::sections(f.0).any(|s| s.id == slug)
        })
    };
    slugs
        .enumerate()
        .find(|(_, s)| *s == slug)
        .map(|(i, _)| i)
        .or_else(holding)
        .unwrap_or(0)
}

fn slug(tab: usize) -> &'static str {
    FAMILIES
        .get(tab)
        .map_or_else(|| SHOWN[tab - FAMILIES.len()].0, |f| f.0)
}

/// A tab's theme when the URL names none.
pub fn dark_by_default(tab: usize) -> bool {
    tab >= FAMILIES.len() && SHOWN[tab - FAMILIES.len()].2
}

/// Blitz focuses a field and no button, and a click's default then clears the focus (blitz-dom events/pointer.rs:815, handle_click). So a
/// page's root focuses the button, in a task: that runs after the default, which stays whole, a form's submit included. Off every control
/// the default still blurs, as in a browser. The web has no such document: a browser does it itself.
pub fn focus_clicked_button(e: MouseEvent) {
    let Some(document) = try_consume_context::<Rc<RefCell<BaseDocument>>>() else {
        return;
    };
    let p = e.client_coordinates();
    let button = {
        let doc = document.borrow();
        let mut at = doc
            .hit(p.x as f32, p.y as f32)
            .and_then(|hit| doc.get_node(hit.node_id));
        while let Some(node) = at.filter(|n| !n.is_focussable()) {
            at = node.parent.and_then(|id| doc.get_node(id));
        }
        at.filter(|n| n.data.is_element_with_tag_name(&local_name!("button")))
            .map(|n| n.id)
    };
    if let Some(id) = button {
        spawn(async move {
            document.borrow_mut().set_focus_to(id);
        });
    }
}

/// The families whose examples show their HTML beside their text form: tables, charts and boards, how agents see them
/// (task design-agent-forms). The others keep light beside dark.
const FORMED: [&str; 2] = ["data", "board"];

/// A new mail's head on /design: From shown, the rest fields.
fn new_mail_fields() -> Vec<crate::documents::Field> {
    let field = |name: &str, label: &str, value: &str| crate::documents::Field {
        name: name.into(),
        label: label.into(),
        value: value.into(),
        required: name == "to",
    };
    vec![
        field("", "From", "nicolas@klar.im"),
        field("to", "To", ""),
        field("cc", "Cc", ""),
        field("bcc", "Bcc", ""),
        field("subject", "Subject", ""),
    ]
}

/// A comment of a page's rail, from his words to the agent's answer: on the whole page when `quote` is empty.
fn rail_thread(quote: &str, files: Vec<(String, String, String)>) -> crate::documents::Thread {
    let said = |who: &str, words: &str| crate::documents::Said {
        who: who.into(),
        when: String::new(),
        words: words.into(),
    };
    crate::documents::Thread {
        id: String::new(),
        quote: quote.into(),
        said: vec![
            said("Nicolas", "Can the table keep its filter?"),
            said("Comment agent", "Waiting for the agent."),
        ],
        open: true,
        files,
    }
}

/// A family's sections in order: its own (FAMILIES), and each section a component draws there (crate::component::sections)
/// after the one it names, "" first; one naming a section the family lacks goes last. No list here names a component (task
/// rust-components-register).
fn sections_of(
    family: usize,
) -> Vec<(
    &'static str,
    &'static str,
    Option<&'static crate::component::Section>,
)> {
    let (slug, _, _, own) = FAMILIES[family];
    let mut all: Vec<_> = own.iter().map(|&(id, name)| (id, name, None)).collect();
    let mut left: Vec<_> = crate::component::sections(slug).collect();
    // inserted first one by one, so those after "" end in id order
    left.sort_by_key(|s| std::cmp::Reverse(s.id));
    while !left.is_empty() {
        let before = left.len();
        left.retain(|s| {
            let at = match s.after {
                "" => Some(0),
                after => all.iter().position(|a| a.0 == after).map(|i| i + 1),
            };
            at.map(|i| all.insert(i, (s.id, s.title, Some(*s))))
                .is_none()
        });
        if left.len() == before {
            all.extend(left.drain(..).map(|s| (s.id, s.title, Some(s))));
        }
    }
    all
}

/// Each section a component draws lands on a family's tab, after a section that tab holds, under an id no other section
/// of /design holds; Mail's tab reads its frame, bar, thread row and head, then its own (/code-review of 114248eb4).
#[cfg(test)]
#[dash_meter::dash_test]
fn each_components_section_lands_where_it_names_once() {
    let mut ids = vec![];
    for (family, f) in FAMILIES.iter().enumerate() {
        let all = sections_of(family);
        for s in crate::component::sections(f.0) {
            let placed = s.after.is_empty() || all.iter().any(|a| a.0 == s.after);
            assert!(placed, "{} follows {}, not on {}", s.id, s.after, f.0);
        }
        ids.extend(all.iter().map(|a| a.0));
    }
    for s in crate::component::registered()
        .iter()
        .flat_map(|c| c.sections)
    {
        assert!(
            FAMILIES.iter().any(|f| f.0 == s.family),
            "{} on {}",
            s.id,
            s.family
        );
    }
    let drawn = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), drawn, "a section id twice");
    let mail = FAMILIES.iter().position(|f| f.0 == "mail").expect("mail");
    let order: Vec<&str> = sections_of(mail).iter().map(|a| a.0).collect();
    assert_eq!(
        order,
        [
            "mail-shell",
            "mail-bar",
            "thread-row",
            "mail-head",
            "thread",
            "composer"
        ]
    );
}

#[component]
fn FamilyPage(
    family: usize,
    #[props(default)] dark: bool,
    #[props(default)] query: String,
) -> Element {
    let (slug, title, what, _) = FAMILIES[family];
    let forms = FORMED.contains(&slug);
    let how = families::Rows { forms, dark, query };
    let sides = if forms {
        ["HTML", "Text"]
    } else {
        ["Light", "Dark"]
    };
    rsx! {
        div { class: "ds-page",
            div { class: "ds-title", "{title}" }
            div { class: "ds-what", "{what}" }
            for (id, name, drawn) in sections_of(family) {
                section { id: "s-{id}", key: "{id}", class: if families::WIDE.contains(&id) { "ds-section ds-wide" } else { "ds-section" },
                    {families::heads(name, sides)}
                    if let Some(drawn) = drawn {
                        for (label, row) in drawn.rows.iter().copied() {
                            {how.row(label, || row(&how.query))}
                        }
                    // documents draws Composer and reads families: a families row of it would be a module loop (dash-gate)
                    } else if id == "composer" {
                        {how.row("Reply", || rsx! { crate::documents::Composer { id: "ds-composer", action: "/mail/t/a/reply", send: "Send", placeholder: "Reply to Ada Lovelace" } })}
                        {how.row("New mail, its formatting bar", || rsx! { crate::documents::Composer { id: "ds-compose-new", action: "/mail/compose", send: "Send", placeholder: "Message", format: true, fields: new_mail_fields() } })}
                        // a page's comment box, every page's rail (task rust-comment-files-filter)
                        {how.row("A page's comment, its images and PDFs attached", || rsx! {
                            crate::documents::Composer { id: "ds-notes-box", action: "/compass", send: "Comment", placeholder: "Your comment", attach: true }
                        })}
                    } else if id == "comments" {
                        {families::body(id, &how)}
                        // a page's rail, as every page's sidebar draws it (forms.rs's notes; task rust-comment-files-filter):
                        // documents' threads and Composer, here for the same module loop
                        {how.row("Open and All, each with its count", || rsx! {
                            crate::components::Toggle { id: "ds-notes-filter", label: "Which comments",
                                crate::components::ToggleOption { href: "?tab=feedback&comments=open#s-comments", current: true, icon: rsx! {}, "Open · 1" }
                                crate::components::ToggleOption { href: "?tab=feedback&comments=all#s-comments", current: false, icon: rsx! {}, "All · 2" }
                            }
                        })}
                        {how.row("On the whole page, or its passage, with its files", || crate::documents::threads(&[
                            rail_thread("", vec![]),
                            rail_thread("each component with its states",
                                vec![("brief.pdf".into(), "/compass/files/ds/0-brief.pdf".into(), "application/pdf".into())]),
                        ], true))}
                        {how.row("Picking an element", || rsx! { crate::components::Compass { id: "ds-pick", picking: true } })}
                    } else {
                        {families::body(id, &how)}
                    }
                }
            }
        }
    }
}

/// `tab` and `dark` start the page; `web`: a server render, which runs no handler, so each tab and the theme are links
/// that change the URL (`?tab=`, `?theme=`), where the window keeps its signals.
#[derive(Props, Clone, PartialEq, Default)]
pub struct DesignProps {
    pub tab: usize,
    pub dark: bool,
    pub web: bool,
    /// The URL's query, which orders the components tab's table (task rust-table-sortable).
    pub query: String,
}

#[allow(non_snake_case)]
pub fn Design(props: DesignProps) -> Element {
    let mut view = use_signal(|| props.tab);
    let mut dark = use_signal(|| props.dark);
    let web = props.web;
    let theme = if dark() {
        "app dark ds-root"
    } else {
        "app light ds-root"
    };
    let tabs = FAMILIES
        .iter()
        .map(|f| (f.0, f.1))
        .chain(SHOWN.iter().map(|s| (s.0, s.1)))
        .enumerate();
    let (current, other) = if dark() {
        ("dark", "light")
    } else {
        ("light", "dark")
    };
    let here = slug(view());
    rsx! {
        style { {THEME} }
        style { {CSS} }
        style { {crate::component::css()} }
        style { {CARDS_CSS} }
        style { {DESIGN_CSS} }
        div { class: theme, onclick: focus_clicked_button,
            div { class: "ds-bar",
                span { class: "ds-brand", "Design" }
                for (i, (slug, title)) in tabs {
                    if i == FAMILIES.len() {
                        span { class: "ds-gap" }
                    }
                    if web {
                        a { id: "{tab_id(slug)}", key: "{slug}", class: if view() == i { "ds-tab on" } else { "ds-tab" },
                            href: if i < FAMILIES.len() { format!("/design?tab={slug}&theme={current}") } else { format!("/design?tab={slug}") },
                            "{title}"
                        }
                    } else {
                        div { id: "{tab_id(slug)}", key: "{slug}", class: if view() == i { "ds-tab on" } else { "ds-tab" },
                            onclick: move |_| { view.set(i); if i >= FAMILIES.len() { dark.set(dark_by_default(i)); } }, "{title}"
                        }
                    }
                }
                span { class: "spring" }
                if web {
                    a { id: "ds-theme", class: "button quiet", href: "/design?tab={here}&theme={other}",
                        if dark() { "Light" } else { "Dark" }
                    }
                } else {
                    button { id: "ds-theme", class: "button quiet", onclick: move |_| dark.toggle(),
                        if dark() { "Light" } else { "Dark" }
                    }
                }
            }
            match view() {
                i if i < FAMILIES.len() => rsx! { div { id: "ds-main", class: "ds-main", key: "{i}", FamilyPage { family: i, dark: dark(), query: props.query.clone() } } },
                i if i == FAMILIES.len() => rsx! { linear::Linear {} },
                i if i == FAMILIES.len() + 1 => rsx! { catalog::Components { query: props.query.clone() } },
                _ => {
                    let rich = serde_json::json!({ "type": "doc", "content": notion_doc::rich() });
                    rsx! { crate::documents::Documents { notion::Notion { web,
                        rich: rsx! { crate::documents::DocView { doc: rich, view: crate::documents::View::Notion } } } } }
                }
            }
            if web {
                script { dangerous_inner_html: DESIGN_JS }
            }
        }
    }
}

/// The spike's ids: f-<family>, v-linear, v-notion (its drive script clicks them).
fn tab_id(slug: &str) -> String {
    if SHOWN.iter().any(|s| s.0 == slug) {
        format!("v-{slug}")
    } else {
        format!("f-{slug}")
    }
}

/// The whole web page at /design: the component's server render. `tab` and `theme` as the URL names them.
pub use notion::DOCUMENT_PDF;

/// The Notion document's PDF: its page printed (crate::pdf), once a process: the page is the same until a rebuild, so a
/// second click costs no Chrome (/code-review). A failed print is not kept: the next click tries again.
pub fn document_pdf() -> Result<Vec<u8>, String> {
    static PRINTED: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    if let Some(pdf) = PRINTED.get() {
        return Ok(pdf.clone());
    }
    let pdf = crate::pdf::print(&page("notion", None, ""))?;
    Ok(PRINTED.get_or_init(|| pdf).clone())
}

pub fn page(tab: &str, theme: Option<&str>, query: &str) -> String {
    let tab = tab_of(tab);
    let dark = theme.map_or(dark_by_default(tab), |t| t == "dark");
    let mut dom = VirtualDom::new_with_props(
        Design,
        DesignProps {
            tab,
            dark,
            web: true,
            query: query.into(),
        },
    );
    dom.rebuild_in_place();
    let body = dioxus_ssr::render(&dom);
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Design system</title></head><body style="margin:0">{body}</body></html>"#
    )
}

/// One family's page alone: the spike's test renders each without a window.
pub fn family_vdom(family: usize) -> VirtualDom {
    VirtualDom::new_with_props(
        FamilyPage,
        FamilyPageProps {
            family,
            dark: false,
            query: String::new(),
        },
    )
}

/// Each tone rings in its own colour, and a row of buttons leaves room between rings (@nha, 2026-11-09: « spacing looks wrong
/// on the focus line, and green around reject/lock ? sure ? »).
#[cfg(test)]
#[dash_meter::dash_test]
fn each_button_tone_rings_in_its_own_colour_with_room_between() {
    let css = DESIGN_CSS;
    // The one rule for a selector: a second further down would override it unseen (/code-review).
    let rule = |selector: &str| {
        let lines: Vec<&str> = css.lines().filter(|l| l.starts_with(selector)).collect();
        assert_eq!(lines.len(), 1, "one rule {selector}: {lines:?}");
        lines[0]
    };
    for (tone, token) in [
        ("", "primary"),
        (".accept", "success"),
        (".reject", "error"),
        (".warn", "warning"),
    ] {
        let line = rule(&format!(".button{tone}:focus, .button{tone}.is-focus {{"));
        assert!(
            line.contains(&format!("0 0 0 4px var(--color-{token})")),
            "{tone}: {line}"
        );
    }
    assert!(rule(".ds-flow.ds-buttons {").contains("gap: var(--spacing-s4)"));
}

#[cfg(test)]
#[dash_meter::dash_test]
fn stage_is_a_toggle_its_name_then_its_value_and_no_stepper() {
    let page = crate::markup(&page("feedback", None, ""));
    assert!(
        !page.contains("ds-stage") && !page.contains("ds-stop"),
        "no stepper left"
    );
    let at = |id: &str| page.find(&format!(r#"id="s-{id}""#)).expect(id);
    let section = &page[at("stage")..at("wait")];
    assert!(
        section.contains(r#"<summary class="ds-pill on">Review</summary>"#),
        "{section}"
    );
    assert!(
        section.contains("<details class=\"ds-toggle\" open"),
        "the open row is open"
    );
    let text = crate::words::text_of(section);
    assert!(
        text.contains("Closed\nStage Review\nDraft\tReview\tAccepted\tSent\n"),
        "{text}"
    );
}

#[cfg(test)]
#[dash_meter::dash_test]
fn reading_shows_judging_as_brief_and_says_a_stopped_judge() {
    // a lead is judged within a pass of landing (task rust-judge-on-arrival): no resting « Not judged yet » row
    let page = crate::markup(&page("feedback", None, ""));
    let at = |id: &str| page.find(&format!(r#"id="s-{id}""#)).expect(id);
    let text = crate::words::text_of(&page[at("reading")..at("comments")]);
    assert!(text.contains("Judging\nJudging its chances..."), "{text}");
    assert!(
        text.contains("Not judged, the judge stopped\nNot judged: the lead chances watcher stopped. See /watchers."),
        "{text}"
    );
    assert!(!text.contains("Not judged yet"), "{text}");
}

/// Done-when 1 of rust-mail-design-parts: the Mail tab's one title, its sections named for the parts /mail draws with, the
/// thread row the component /mail's list is (@nha, 2026-11-09: « double-title. Are the elements actually used in /mail ? »).
#[cfg(test)]
#[dash_meter::dash_test]
fn the_mail_tab_has_one_title_and_a_section_per_part() {
    let page = crate::markup(&page("mail", None, ""));
    let text = crate::words::text_of(&page);
    assert_eq!(
        text.lines().filter(|l| l.trim() == "Mail").count(),
        1,
        "the title alone, no section of that name: {text}"
    );
    let at = |id: &str| page.find(&format!(r#"id="s-{id}""#)).expect(id);
    let heads: Vec<&str> = page
        .split(r#"<span class="ds-name">"#)
        .skip(1)
        .map(|s| &s[..s.find('<').unwrap_or(0)])
        .collect();
    assert_eq!(
        heads,
        [
            "Frame",
            "List bar",
            "Thread row",
            "Conversation head",
            "Thread",
            "Composer"
        ]
    );
    // task rust-mail-parity: the frame, its bar and a conversation's head, and New mail's formatting bar, each its component
    assert!(page[at("mail-shell")..at("mail-bar")].contains(r#"class="mail-shell""#));
    assert!(page[at("mail-bar")..at("thread-row")].contains(r#"class="mail-bar""#));
    assert!(page[at("mail-head")..at("thread")].contains(r#"class="mail-head""#));
    // six marks, drawn on the light side and the dark
    assert_eq!(
        page[at("composer")..].matches(r#"data-format="#).count(),
        12
    );
    let rows = &page[at("thread-row")..at("thread")];
    assert!(
        rows.contains(r#"class="thread-row unread""#) && rows.contains(r#"class="thread-row""#),
        "{rows}"
    );
    assert!(page[at("thread")..at("composer")].contains(r#"class="fold""#));
    assert!(page[at("composer")..].contains(r#"class="editor""#));
}

/// Done-when 1 of rust-timeline-component: /design's data tab draws the Timeline component, its first event « 3 days ago »
/// with its day beside it in the text form, and the documents tab's CRM entry draws its events with it.
#[cfg(test)]
#[dash_meter::dash_test]
fn design_shows_the_timeline_and_the_crm_entry_uses_it() {
    // the day read before and after the render: a midnight between them fails nothing
    let before = crate::components::today();
    let data = crate::markup(&page("data", None, ""));
    let text = crate::words::text_of(&data);
    let days = [before, crate::components::today()].map(|t| dash_core::date(t - 3));
    assert!(
        data.contains(r#"<ol class="timeline"><li><details class="when">"#),
        "{data}"
    );
    assert!(
        days.iter()
            .any(|day| text.contains(&format!("3 days ago\t{day}\nContract draft received\n"))),
        "{text}"
    );
    let documents = crate::markup(&page("documents", None, ""));
    assert!(
        documents.contains(r#"<h2>Timeline</h2><ol class="timeline">"#),
        "{documents}"
    );
}

/// Done-when 1 of rust-gantt-component: /design's data tab draws the Gantt under the Timeline, its five rows, their bars, the
/// today line, dotted arrows and a milestone; its text form there is the Mermaid gantt (done-when 2).
#[cfg(test)]
#[dash_meter::dash_test]
fn design_shows_the_gantt() {
    let data = crate::markup(&page("data", None, ""));
    let at = data
        .find(r#"<div class="gantt" id="c-gantt"><svg class="gantt-svg""#)
        .expect("the Gantt on /design");
    let gantt = &data[at..at + data[at..].find("</pre>").expect("its text")];
    let count = |s: &str| gantt.matches(s).count();
    assert_eq!(
        [
            count(r#"class="gantt-label""#),
            count("<rect"),
            count(r#"<g class="gantt-today"><line"#),
            count(r#"stroke-dasharray="3 3""#),
            count(r#"class="gantt-milestone"#)
        ],
        [5, 4, 1, 4, 1],
        "rows, bars, today, dotted arrows, milestone: {gantt}"
    );
    let text = crate::words::text_of(&data);
    assert!(text.contains("gantt\n  dateFormat YYYY-MM-DD\n"), "{text}");
    assert!(text.contains("  /plan ships :milestone, ships, "), "{text}");
}

/// Done-when 2 of rust-ui-controls: the Notion tab's bar over a pick is the design system's Toolbar, a vertical one whose
/// Comment is a button. Its search's Cmd K moved to the menu's Ask (task rust-notion-one-menu), tested in catalog.rs.
#[cfg(test)]
#[dash_meter::dash_test]
fn the_notion_selection_bar_is_the_toolbar() {
    let page = crate::markup(&page("notion", None, ""));
    for part in [
        r#"<div role="toolbar" id="no-tool" class="toolbar no-tool" aria-label="Selection" aria-orientation="vertical" data-orientation="vertical"><button type="button" class="toolbar-button">Bulleted list</button><div class="toolbar-separator" role="separator" aria-orientation="horizontal"></div><div role="group" class="toolbar-group no-tool-row"><button type="button" class="toolbar-button no-icon">A</button>"#,
        r#"<button type="button" id="no-comment" class="toolbar-button">"#,
        r#"<div role="group" class="toolbar-group"><span class="toolbar-title">Skills</span><button type="button" class="toolbar-button">"#,
    ] {
        assert!(page.contains(part), "{part} in {page}");
    }
    assert!(
        !page.contains("Cmd K") && !page.contains("no-tool-sep"),
        "{page}"
    );
}

/// The Notion tab's toggle is a closed `details`, its summary its head and its lines inside it: the browser's own element opens
/// and closes it, no script of ours (task rust-browser-tests-fewer moved it here from notion_browser's Chrome; @nha,
/// 2026-11-11: « small tests > big slow tests »).
#[cfg(test)]
#[dash_meter::dash_test]
fn the_notion_toggle_is_a_closed_details_its_lines_inside() {
    let page = crate::markup(&page("notion", None, ""));
    let open = r#"<details id="no-toggle" class="no-toggle"><summary id="no-toggle-head">Open questions</summary>"#;
    let at = page
        .find(open)
        .unwrap_or_else(|| panic!("{open} in {page}"));
    let toggle = &page[at..at + page[at..].find("</details>").expect("its end")];
    for q in notion::QUESTIONS {
        assert!(
            toggle.contains(&format!(
                r#"<div class="no-line d1"><span class="no-bullet"></span>{q}</div>"#
            )),
            "{q} inside: {toggle}"
        );
    }
}

/// Done-when 1 of rust-doc-rich-blocks: the Notion document on /design holds bold words, a code block and an image, each in
/// the model (notion_doc's `rich`), drawn by DocView, and read in the text form the way Markdown writes them.
#[cfg(test)]
#[dash_meter::dash_test]
fn the_notion_document_shows_bold_words_a_code_block_and_an_image_in_both_forms() {
    let page = crate::markup(&page("notion", None, ""));
    for html in [
        "Each frame has a budget of <strong>16 ms</strong>, measured in the window and on the web.",
        "<pre class=\"doc-text\"><code>cargo nextest run -p dash-ui frame_time\n  --run-ignored all</code></pre>",
        r#"<figure><img src="data:image/svg+xml;charset=utf-8,%3Csvg"#,
        r#"alt="Frame time over the last 120 frames"/></figure>"#,
    ] {
        assert!(page.contains(html), "{html} in {page}");
    }
    let text = crate::forms::text_of(&page);
    let rich = "Selection\n\
                Each frame has a budget of **16 ms**, measured in the window and on the web.\n\
                ```\n\
                cargo nextest run -p dash-ui frame_time\n  --run-ignored all\n\
                ```\n\
                ![Frame time over the last 120 frames]\n\
                Open questions\n";
    assert!(text.contains(rich), "{rich} in {text}");
}

/// Done-when 1 to 3 of design-agent-forms: each example of the Data and Board tabs shows its HTML beside its text form, the
/// text read off that same render; the leads table takes the page's filter and page, and both forms state its total and the
/// next page's URL (@nha, 2026-11-09: « Is there pagination for agents ? filters ? »).
#[cfg(test)]
#[dash_meter::dash_test]
fn data_and_board_examples_show_their_html_beside_their_text_and_a_table_pages_filtered() {
    for tab in ["data", "board"] {
        let page = crate::markup(&page(tab, None, &format!("tab={tab}")));
        let rows = page.matches(r#"<div class="ds-row">"#).count();
        assert!(rows > 3, "{tab}: {page}");
        assert_eq!(
            page.matches(r#"<pre class="ds-cell ds-code ds-text" aria-hidden="true">"#)
                .count(),
            rows,
            "{tab}: a text beside each example"
        );
        assert!(
            page.contains(r#"<span class="ds-side">HTML</span><span class="ds-side">Text</span>"#),
            "{tab}"
        );
    }
    let first = crate::markup(&page("data", None, "tab=data&filter=applied"));
    let at = |id: &str| first.find(&format!(r#"id="s-{id}""#)).expect(id);
    let table = &first[at("table")..at("charts")];
    // the HTML: the filter field holding the words, the page's other keys riding along, the caption, the pager's next
    for part in [
        r#"<form class="table-filter" method="get" role="search"><input type="hidden" name="tab" value="data"/><input class="input" value="applied" aria-label="Filter the table" name="filter" placeholder="Filter rows" type="search"/></form>"#,
        r#"<caption>4 of 8 rows match « applied »; page 1 of 2.<span class="sr-only"> Next page: ?tab=data&#38;filter=applied&#38;page=2</span></caption>"#,
        r#"data-kind="next" href="?tab=data&#38;filter=applied&#38;page=2""#,
    ] {
        assert!(table.contains(part), "{part} in {table}");
    }
    // the text beside it, read off that same HTML, and the page's own text form, which says it once
    let lines = "4 of 8 rows match « applied »; page 1 of 2. Next page: ?tab=data&filter=applied&page=2\nCompany\tStage\tRate\nNorthwind\tApplied\t600\nInitech\tApplied\t550\nStark\tApplied\t750\n";
    let pre = &table[table.find("<pre").expect("its text")..];
    let beside = crate::words::every_word(&pre[..pre.find("</pre>").expect("its end") + 6]);
    assert!(beside.starts_with(lines), "{beside}");
    let text = crate::words::text_of(&first);
    assert!(text.contains(lines), "{text}");
    let once = crate::words::text_of(table);
    assert_eq!(once.matches("Northwind").count(), 1, "{once}");
    // the next page, as an agent follows the URL: the last match, no further page
    let second = crate::markup(&page("data", None, "tab=data&filter=applied&page=2"));
    let at = |id: &str| second.find(&format!(r#"id="s-{id}""#)).expect(id);
    let second = crate::words::text_of(&second[at("table")..at("charts")]);
    assert!(
        second.contains("4 of 8 rows match « applied »; page 2 of 2.\nCompany\tStage\tRate\nWayne\tApplied\t680\n"),
        "{second}"
    );
    assert!(
        !second.contains("Northwind") && !second.contains("Next page"),
        "{second}"
    );
    // one filter field a page; a filter that keeps no row says so (/code-review)
    assert_eq!(table.matches(r#"role="search""#).count(), 1, "{table}");
    let none = crate::markup(&page("data", None, "tab=data&filter=zzz"));
    let at = |id: &str| none.find(&format!(r#"id="s-{id}""#)).expect(id);
    let none = crate::words::text_of(&none[at("table")..at("charts")]);
    assert!(
        none.contains(
            "0 of 8 rows match « zzz ».\nCompany\tStage\tRate\nNo lead matches this filter.\n"
        ),
        "{none}"
    );
}

/// Each row of a /design section drawn HTML beside text: its label, its HTML cell and the text beside it (task
/// design-agent-forms).
#[cfg(test)]
fn beside(page: &str, id: &str) -> Vec<(String, String, String)> {
    let section = &page[page.find(&format!(r#"id="s-{id}""#)).expect(id)..];
    let section = &section[..section.find("</section>").expect("its end")];
    let pre = r#"<pre class="ds-cell ds-code ds-text" aria-hidden="true">"#;
    (section.split(r#"<div class="ds-row">"#).skip(1))
        .map(|row| {
            let label = &row[row.find('>').expect("a label") + 1..];
            let label = label[..label.find('<').expect("its end")].to_string();
            let cell = &row[row.find(r#"ds-cell">"#).expect("its HTML") + 9..];
            let at = cell
                .find(pre)
                .unwrap_or_else(|| panic!("{id} {label}: no text beside"));
            let text = &cell[at..];
            let text = crate::words::every_word(&text[..text.find("</pre>").expect("its end") + 6]);
            (label, cell[..at].to_string(), text)
        })
        .collect()
}

/// The fix to design-agent-forms' report: each graph example, the Data tab's charts and figures and the Components tab's
/// Diagram and Treemap, and each board example shows its HTML beside its text, that text the one an agent reads off the
/// HTML, its numbers in it.
#[cfg(test)]
#[dash_meter::dash_test]
fn each_graph_and_board_example_shows_its_html_beside_its_text_with_its_numbers() {
    let (data, board) = (
        crate::markup(&page("data", None, "")),
        crate::markup(&page("board", None, "")),
    );
    let components = crate::markup(&page("components", None, ""));
    let graphs = [
        (
            &data,
            "charts",
            "Funnel",
            "Stage\tLeads\nOpened\t100\nApplied\t62\n",
        ),
        (
            &data,
            "charts",
            "Per week",
            "Week\tApplied\nW1\t6\nW2\t8\nW3\t4\nW4\t11\n",
        ),
        (
            &data,
            "charts",
            "An area",
            "Week\tSent\tReplied\nW1\t6\t2\n",
        ),
        (
            &data,
            "charts",
            "A line",
            "Day\tReplies\nDay 1\t3\nDay 2\t5\n",
        ),
        (&data, "stats", "Share bar", "8 of 12 done, 4 left\n"),
        (
            &data,
            "stats",
            "Sparkline",
            "Replies\nDay\tReplies\nDay 1\t3\n",
        ),
        (
            &components,
            "diagram",
            "/deps&#39; crate graph",
            "flowchart LR\n",
        ),
        (
            &components,
            "treemap",
            "/deps&#39; code map",
            "store: 900 lines",
        ),
        (&board, "lanes", "Lanes", "Needs you 2 Working 3 Ready 1\n"),
    ];
    for (page, id, label, says) in graphs {
        let rows = beside(page, id);
        let (_, html, text) = (rows.iter())
            .find(|r| r.0.starts_with(label))
            .unwrap_or_else(|| panic!("{id} {label} in {rows:?}"));
        assert_eq!(
            *text,
            crate::words::text_of(html),
            "{id} {label}: read off its HTML"
        );
        assert!(text.contains(says), "{id} {label}: {says:?} in {text}");
        assert!(html.contains('<'), "{id} {label}: its HTML drawn");
    }
    // every row of the charts and figures, not only those named
    for id in ["charts", "stats"] {
        assert!(beside(&data, id).len() >= 3, "{id}");
    }
}

/// Done-when 3 of rust-charts-rust-ui: /design's charts section and the sparkline are the design system's charts, each row
/// an SVG chart, and the hand-drawn specimens (bars, funnel tracks, the bare sparkline) are gone from the page and its styles.
#[cfg(test)]
#[dash_meter::dash_test]
fn the_charts_section_and_the_sparkline_are_the_design_systems_charts() {
    let served = page("data", None, "");
    // the live check's URL, /design?tab=charts, opens the Data tab that holds them
    assert_eq!(page("charts", None, ""), served);
    let data = crate::markup(&served);
    assert_eq!(
        (tab_of("charts"), tab_of("table"), tab_of("nope")),
        (1, 1, 0)
    );
    let rows = beside(&data, "charts");
    let charted: Vec<&str> = (rows.iter())
        .filter(|r| r.1.contains(r#" class="chart"><svg class="chart-svg""#))
        .map(|r| r.0.as_str())
        .collect();
    assert_eq!(
        charted,
        ["Funnel", "Per week", "An area, two series", "A line"]
    );
    let (_, spark, _) = (beside(&data, "stats").into_iter())
        .find(|r| r.0 == "Sparkline")
        .expect("a sparkline");
    assert!(spark.contains(r#"<div id="ds-replies" class="chart spark"><svg class="chart-svg" aria-hidden="true" viewBox="0 0 160 44">"#), "{spark}");
    assert!(
        !spark.contains("chart-dot") && !spark.contains("chart-tick"),
        "a line alone: {spark}"
    );
    for gone in [
        "ds-funnel",
        "ds-track",
        "ds-fill",
        "ds-bars",
        "ds-column",
        "ds-spark",
    ] {
        assert!(!data.contains(gone), "{gone} on the Data tab");
        assert!(
            !include_str!("design.css").contains(&format!(".{gone}")),
            "{gone} in design.css"
        );
    }
}

/// Every colour on the design page comes from design.json through theme.css: no hex, rgb or hsl of its own (moved from
/// rust/ui-spike, task rust-ui-spike-retire).
#[cfg(test)]
#[dash_meter::dash_test]
fn the_design_css_has_no_colour_of_its_own() {
    assert!(
        DESIGN_CSS.contains("var(--color-primary)"),
        "the read is of the wrong file"
    );
    let raw: Vec<&str> = DESIGN_CSS
        .lines()
        .filter(|l| !l.trim_start().starts_with("/*"))
        .filter(|l| {
            l.contains("rgb(")
                || l.contains("rgba(")
                || l.contains("hsl(")
                || l.split(": #").count() > 1
                || l.split(" #")
                    .skip(1)
                    .any(|c| c.starts_with(|x: char| x.is_ascii_hexdigit()))
        })
        .collect();
    assert!(raw.is_empty(), "{raw:#?}");
}

/// Each section of a family outside FORMED draws a state in the light theme and in the dark one (moved from rust/ui-spike);
/// FORMED's show their text form instead, held by data_and_board_examples_show_their_html_beside_their_text_and_a_table_pages_filtered.
#[cfg(test)]
#[dash_meter::dash_test]
fn each_section_has_a_state_in_both_themes() {
    let themed: Vec<_> = (FAMILIES.iter().enumerate())
        .filter(|(_, (slug, ..))| !FORMED.contains(slug))
        .collect();
    assert!(themed.len() >= 4, "the families were read");
    for (i, (_, _, _, sections)) in themed {
        let mut vdom = family_vdom(i);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        for (id, _) in *sections {
            let from = html
                .find(&format!("id=\"s-{id}\""))
                .unwrap_or_else(|| panic!("no section {id}"));
            let rest = &html[from + 1..];
            let section = &rest[..rest.find("id=\"s-").unwrap_or(rest.len())];
            assert!(
                section.contains("app light ds-cell") && section.contains("app dark ds-cell"),
                "{id} draws no state"
            );
        }
    }
}
