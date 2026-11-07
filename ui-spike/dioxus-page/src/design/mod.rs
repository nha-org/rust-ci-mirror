//! The design system page: /design's six families in Dioxus on design.json's tokens, each state light and dark side by
//! side, then Linear-like cards and Notion-like comments to sit beside the evidence screens. Plain screens prove behaviour
//! (lib.rs); this proves styling comes cheap (@nha, 2026-11-06: « this will allow knowin we can style things. »).
use crate::{CSS, THEME};
use dioxus::prelude::*;

mod families;
mod linear;
mod notion;
pub mod script;

pub const DESIGN_CSS: &str = include_str!("design.css");

type Family = (&'static str, &'static str, &'static str, &'static [(&'static str, &'static str)]);

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
        &[("buttons", "Actions"), ("group", "Toggle group"), ("field", "Form field"), ("text", "Text editor")],
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
    ("mail", "Mail", "The inbox's list, a thread's message and the composer.", &[("mail", "Mail")]),
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

#[derive(Clone, Copy, PartialEq)]
enum View {
    Family(usize),
    Linear,
    Notion,
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
                    div { class: "ds-head",
                        span { class: "ds-name", "{name}" }
                        span { class: "ds-side", "Light" }
                        span { class: "ds-side", "Dark" }
                    }
                    {families::body(id)}
                }
            }
        }
    }
}

#[allow(non_snake_case)]
pub fn DesignApp() -> Element {
    let mut view = use_signal(|| View::Family(0));
    let mut dark = use_signal(|| false);
    let theme = if dark() { "app dark ds-root" } else { "app light ds-root" };
    rsx! {
        style { {THEME} }
        style { {CSS} }
        style { {DESIGN_CSS} }
        div { class: theme,
            div { class: "ds-bar",
                span { class: "ds-brand", "Design" }
                for (i, (slug, title, _, _)) in FAMILIES.iter().copied().enumerate() {
                    div { id: "f-{slug}", key: "{slug}",
                        class: if view() == View::Family(i) { "ds-tab on" } else { "ds-tab" },
                        onclick: move |_| view.set(View::Family(i)), "{title}"
                    }
                }
                span { class: "ds-gap" }
                div { id: "v-linear", class: if view() == View::Linear { "ds-tab on" } else { "ds-tab" },
                    // Each opens in the theme its evidence screen is in.
                    onclick: move |_| { view.set(View::Linear); dark.set(false); }, "Linear cards"
                }
                div { id: "v-notion", class: if view() == View::Notion { "ds-tab on" } else { "ds-tab" },
                    onclick: move |_| { view.set(View::Notion); dark.set(true); }, "Notion comments"
                }
                span { class: "spring" }
                button { id: "ds-theme", class: "button quiet", onclick: move |_| dark.toggle(),
                    if dark() { "Light" } else { "Dark" }
                }
            }
            match view() {
                View::Family(i) => rsx! { div { id: "ds-main", class: "ds-main", key: "{i}", FamilyPage { family: i } } },
                View::Linear => rsx! { linear::Linear {} },
                View::Notion => rsx! { notion::Notion {} },
            }
        }
    }
}

pub fn vdom() -> VirtualDom {
    VirtualDom::new(DesignApp)
}

/// design.json's three faces in one collection: sans, then `monospace` and `cursive`.
pub fn font_ctx() -> blitz_dom::FontContext {
    use parley::fontique::{Blob, GenericFamily};
    let mut ctx = blitz_dom::build_single_font_ctx(ui_spike_common::DESIGN_FONT);
    for (face, generic) in [(ui_spike_common::MONO_FONT, GenericFamily::Monospace), (ui_spike_common::HAND_FONT, GenericFamily::Cursive)] {
        let decoded = blitz_dom::decode_font_bytes(face).into_owned();
        let families = ctx.collection.register_fonts(Blob::new(std::sync::Arc::new(decoded) as _), None);
        ctx.collection.set_generic_families(generic, families.iter().map(|(id, _)| *id));
    }
    ctx
}

/// One family's page alone: the test renders each without a window.
pub fn family_vdom(family: usize) -> VirtualDom {
    VirtualDom::new_with_props(FamilyPage, FamilyPageProps { family })
}
