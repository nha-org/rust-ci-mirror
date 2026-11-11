use super::parts::{DOTS, icon};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "dropdown_menu",
        classes: &["dropdown-menu", "dropdown-menu-content", "dropdown-menu-item"],
        tags: &[],
        name: "DropdownMenu",
        used: "The Notion top bar's « ... »: Export PDF, Export Markdown. Its trigger opens it, a click elsewhere closes it (Escape too on the web); closed, its items are no part of the text.",
        example: Some(|_| {
            rsx! {
                DropdownMenu { id: "c-menu", open: true, label: "Page menu", trigger: icon(DOTS),
                    DropdownMenuItem { id: "c-menu-pdf", "Export PDF" }
                    DropdownMenuItem { id: "c-menu-md", href: "data:text/markdown;charset=utf-8,%23%20Plan", download: "plan.md", "Export Markdown" }
                }
            }
        }),
        ..Component::BARE
    }
}

/// A menu its trigger opens: DioxusLabs/dioxus-components' DropdownMenu (MIT OR Apache-2.0, notice kept in
/// NOTICE-dioxus-components), rebuilt on its primitive's markup since dioxus-primitives needs Dioxus 0.7: the trigger a button
/// with `aria-haspopup` and `aria-expanded`, the content labelled by it, `data-state` open or closed on all three. Its items
/// are actions, so a `menu` of `menuitem`s where upstream has a listbox of options (/code-review). `open` draws it open;
/// `toggle` is the window's handler (an Option: a default handler needs a runtime, which a catalog example's props lack), and
/// the web's page script flips the same attributes (the Notion top bar's « ... », task notion-page-menu-export).
#[component]
pub fn DropdownMenu(
    #[props(into)] id: String,
    open: bool,
    label: &'static str,
    trigger: Element,
    toggle: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    let state = if open { "open" } else { "closed" };
    rsx! {
        div { id: "{id}", class: "dropdown-menu", "data-state": state,
            button { id: "{id}-trigger", class: "button icon", r#type: "button", "aria-label": label,
                "aria-haspopup": "menu", "aria-expanded": if open { "true" } else { "false" }, "data-state": state,
                onclick: move |e| {
                    if let Some(toggle) = toggle {
                        toggle.call(e);
                    }
                },
                {trigger}
            }
            div { id: "{id}-content", class: "dropdown-menu-content", role: "menu", "aria-labelledby": "{id}-trigger",
                "data-state": state, hidden: !open,
                {children}
            }
        }
    }
}

/// One item of a `DropdownMenu`: a link, `download` the file name it saves as; with no `href`, a button whose action its page's
/// handler runs, so a key reaches it too (/code-review).
#[component]
pub fn DropdownMenuItem(
    #[props(into)] id: String,
    #[props(into)] href: Option<String>,
    download: Option<&'static str>,
    children: Element,
) -> Element {
    match href {
        Some(href) => {
            rsx! { a { id, class: "dropdown-menu-item", role: "menuitem", href, download, {children} } }
        }
        None => {
            rsx! { button { id, class: "dropdown-menu-item", role: "menuitem", r#type: "button", {children} } }
        }
    }
}
