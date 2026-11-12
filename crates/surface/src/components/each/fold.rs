use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "collapsible",
        classes: &["fold"],
        tags: &[],
        name: "Fold",
        used: "A conversation's mails on /mail/t/<thread>: the older ones folded to a line, the latest open.",
        example: Some(|_| {
            rsx! {
                Fold { id: "c-fold", open: false, summary: rsx! { "Ada · 2026-11-08 · Thanks for the call" },
                    pre { "Thanks for the call, see you Monday." }
                }
            }
        }),
        menu: true,
        ..Component::BARE
    }
}

/// A part folded to its one line, opened by a tap: a conversation's older mails (task rust-mail-thread). No `id`: a fold no link
/// names (a document's fold with no source id, whose export must not mint one).
#[component]
pub fn Fold(
    #[props(into)] id: Option<String>,
    /// A class beside `fold`: « wn », an RFC's writer notes.
    #[props(default)]
    look: &'static str,
    open: bool,
    summary: Element,
    children: Element,
) -> Element {
    // a present `open`, even « false », opens it: only an open fold carries one
    let open = open.then_some(true);
    // a static class when no look is given, so a fold's attributes keep their order (class first)
    match look {
        "" => rsx! { details { class: "fold", id, open, summary { {summary} } {children} } },
        look => {
            rsx! { details { class: "fold {look}", id, open, summary { {summary} } {children} } }
        }
    }
}
