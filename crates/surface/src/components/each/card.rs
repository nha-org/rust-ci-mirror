use super::parts::joined;
use super::skeleton::Skeleton;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "card",
        classes: &["card", "li-card", "description"],
        tags: &[],
        name: "Card",
        used: "Only where a card was approved: the drafts' previews and the board's Linear cards (look li-card); a list of things is a List or a Table (@nha, 2026-11-09: « Cards are not the default UI element »); reading, it greys as a Skeleton.",
        example: Some(|_| {
            rsx! {
                Card { id: "c-card", href: "/tasks?project=site_perso".to_string(),
                    CardTitle { "site_perso" }
                    CardDescription { "3 open, 1 in review" }
                }
                Card { id: "c-card-reading", reading: true, CardDescription { "drafts approved, not yet sent" } }
            }
        }),
        ..Component::BARE
    }
}

/// A card: a link when it has `href`. `task`: the task it shows, `data-task`, the comment box's anchor (crate::forms).
/// `draggable`: a board's card his drag moves to another column (Board).
/// `reading`: its loading state, the same card under the same id, greyed with a grey line where its title will be: the page's
/// LIVE_JS swaps in the filled card in place (task rust-today-doc; @nha, 2026-11-09: « This is a card loading state. same
/// element. »). `children` are what is known already, its description.
#[component]
pub fn Card(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default)] href: Option<String>,
    #[props(default, into)] task: Option<String>,
    #[props(default)] draggable: bool,
    #[props(default)] reading: bool,
    children: Element,
) -> Element {
    let class = joined("card", look);
    let draggable = draggable.then_some("true");
    if reading {
        return rsx! { Skeleton { id, what: "", lines: 0, look: class, CardTitle {} {children} } };
    }
    match href {
        Some(href) => rsx! { a { id, class, "data-task": task, draggable, href, {children} } },
        None => rsx! { div { id, class, "data-task": task, draggable, {children} } },
    }
}

#[component]
pub fn CardTitle(children: Element) -> Element {
    rsx! { b { class: "title", {children} } }
}

#[component]
pub fn CardDescription(children: Element) -> Element {
    rsx! { span { class: "description", {children} } }
}
