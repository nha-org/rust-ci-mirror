//! The design system page: /design's six families in Dioxus on design.json's tokens, each state light and dark side by
//! side, then Linear-like cards and Notion-like comments to sit beside the evidence screens (@nha, 2026-11-06: « this will
//! allow knowin we can style things. »). Moved from rust/ui-spike/dioxus-page, which drives it from here (task
//! rust-design-page; @nha, 2026-11-08: « please build /design (design system) from the spike »): dash serve renders it at
//! /design, dash ui --design draws it in the window.
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
const DESIGN_SCRIPT: &str = include_str!("design.js");
/// A Linear card and its column: /design's and the board's one look.
pub const CARDS_CSS: &str = include_str!("cards.css");
pub use crate::components::CSS as COMPONENTS_CSS;

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
            ("table", "Table and filter bar"),
            ("charts", "Charts"),
            ("stats", "Figures as visuals"),
            ("timeline", "Lead timeline"),
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
        "The inbox's list, a thread's message and the composer.",
        &[("mail", "Mail")],
    ),
    (
        "board",
        "Board",
        "The agents' work: the board, a project's issues, a task's page and its card.",
        &[
            ("board", "Board"),
            ("issues", "A project's issues"),
            ("tasks", "Task page"),
            ("work-card", "Step or task card"),
            ("agents", "Agent lanes"),
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
        .flat_map(|f| std::iter::once(f.0).chain(f.3.iter().map(|s| s.0)));
    let tabs = SHOWN
        .iter()
        .map(|s| s.0)
        .chain(catalog::CATALOG.iter().map(|e| e.0));
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

/// A tab's index of its slug, `?tab=` on the web: the families, then SHOWN. Unknown is the first.
pub fn tab_of(slug: &str) -> usize {
    let slug = if slug == "notion" { "documents" } else { slug };
    let slugs = FAMILIES
        .iter()
        .map(|f| f.0)
        .chain(SHOWN.iter().map(|s| s.0));
    slugs
        .enumerate()
        .find(|(_, s)| *s == slug)
        .map_or(0, |(i, _)| i)
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

#[component]
fn FamilyPage(family: usize) -> Element {
    let (_, title, what, sections) = FAMILIES[family];
    rsx! {
        div { class: "ds-page",
            div { class: "ds-title", "{title}" }
            div { class: "ds-what", "{what}" }
            for (id, name) in sections.iter().copied() {
                section { id: "s-{id}", key: "{id}", class: "ds-section",
                    {families::head(name)}
                    {families::body(id)}
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
        style { {COMPONENTS_CSS} }
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
                i if i < FAMILIES.len() => rsx! { div { id: "ds-main", class: "ds-main", key: "{i}", FamilyPage { family: i } } },
                i if i == FAMILIES.len() => rsx! { linear::Linear {} },
                i if i == FAMILIES.len() + 1 => rsx! { catalog::Components {} },
                _ => rsx! { crate::documents::Documents { notion::Notion { web } } },
            }
            if web {
                script { dangerous_inner_html: DESIGN_SCRIPT }
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
    let pdf = crate::pdf::print(&page("notion", None))?;
    Ok(PRINTED.get_or_init(|| pdf).clone())
}

pub fn page(tab: &str, theme: Option<&str>) -> String {
    let tab = tab_of(tab);
    let dark = theme.map_or(dark_by_default(tab), |t| t == "dark");
    let mut dom = VirtualDom::new_with_props(
        Design,
        DesignProps {
            tab,
            dark,
            web: true,
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
    VirtualDom::new_with_props(FamilyPage, FamilyPageProps { family })
}

/// Each tone rings in its own colour, and a row of buttons leaves room between rings (@nha, 2026-11-09: « spacing looks wrong
/// on the focus line, and green around reject/lock ? sure ? »).
#[cfg(test)]
#[test]
fn each_button_tone_rings_in_its_own_colour_with_room_between() {
    let _budget = dash_meter::memory::TestBudget::new();
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
#[test]
fn stage_is_a_toggle_its_name_then_its_value_and_no_stepper() {
    let _budget = dash_meter::memory::TestBudget::new();
    let page = page("feedback", None);
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
#[test]
fn reading_shows_judging_as_brief_and_says_a_stopped_judge() {
    // a lead is judged within a pass of landing (task rust-judge-on-arrival): no resting « Not judged yet » row
    let _budget = dash_meter::memory::TestBudget::new();
    let page = page("feedback", None);
    let at = |id: &str| page.find(&format!(r#"id="s-{id}""#)).expect(id);
    let text = crate::words::text_of(&page[at("reading")..at("comments")]);
    assert!(text.contains("Judging\nJudging its chances..."), "{text}");
    assert!(
        text.contains("Not judged, the judge stopped\nNot judged: the lead chances watcher stopped. See /watchers."),
        "{text}"
    );
    assert!(!text.contains("Not judged yet"), "{text}");
}
