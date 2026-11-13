use super::parts::joined;
use crate::component::Component;
use dioxus::prelude::*;

/// Whether a toolbar's buttons run down (its separators across) or along.
#[derive(Clone, Copy)]
struct Vertical(bool);

inventory::submit! {
    Component {
        slug: "toolbar",
        classes: &["toolbar", "toolbar-button", "toolbar-group", "toolbar-separator", "toolbar-title"],
        tags: &[],
        name: "Toolbar",
        used: "The Notion tab's bar over a pick: its marks, Comment and the skills, a press on it keeping the pick.",
        example: Some(|_| {
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
        }),
        menu: true,
        ..Component::BARE
    }
}

/// Buttons acting on one thing: dioxus-components' Toolbar (notice in NOTICE-dioxus-components), its `role="toolbar"`,
/// orientation and separators kept. A press on it never reaches what is under it nor moves the focus, so the selection it
/// acts on stays (the Notion tab's bar over a pick). ponytail: no arrow-key focus moves, Tab reaches each button; the
/// primitive's roving focus when a page needs it.
#[component]
pub fn Toolbar(
    #[props(into)] id: String,
    label: &'static str,
    #[props(default)] look: &'static str,
    #[props(default)] vertical: bool,
    children: Element,
) -> Element {
    use_context_provider(|| Vertical(vertical));
    let orientation = if vertical { "vertical" } else { "horizontal" };
    rsx! {
        div { id, class: joined("toolbar", look), role: "toolbar", "aria-label": label,
            "aria-orientation": orientation, "data-orientation": orientation,
            // nor takes the focus from it (/code-review): the web's page script does the same
            onmousedown: move |e| {
                e.prevent_default();
                e.stop_propagation();
            },
            {children}
        }
    }
}

/// One button of a `Toolbar`; `onclick` its action (an Option, as DropdownMenu's `toggle`, so a catalog example needs no
/// runtime). `title` names it on a hover and to a screen reader when its face is a sign; `mark`, its `data-format`, the
/// formatting a Composer's bar puts on the selection.
#[component]
pub fn ToolbarButton(
    #[props(into)] id: Option<String>,
    #[props(default)] look: &'static str,
    #[props(default)] title: Option<&'static str>,
    #[props(default)] mark: Option<&'static str>,
    onclick: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    rsx! {
        button { id, class: joined("toolbar-button", look), r#type: "button", title, "aria-label": title, "data-format": mark,
            onclick: move |e| {
                if let Some(onclick) = onclick {
                    onclick.call(e);
                }
            },
            {children}
        }
    }
}

/// A line between a `Toolbar`'s parts, across its run: a vertical toolbar's runs across.
#[component]
pub fn ToolbarSeparator() -> Element {
    let vertical = try_use_context::<Vertical>().is_some_and(|v| v.0);
    let orientation = if vertical { "horizontal" } else { "vertical" };
    rsx! { div { class: "toolbar-separator", role: "separator", "aria-orientation": orientation } }
}

/// Buttons of a `Toolbar` kept together, `title` its words over them.
#[component]
pub fn ToolbarGroup(
    #[props(default)] look: &'static str,
    title: Option<&'static str>,
    children: Element,
) -> Element {
    rsx! {
        div { class: joined("toolbar-group", look), role: "group",
            if let Some(title) = title {
                span { class: "toolbar-title", "{title}" }
            }
            {children}
        }
    }
}
