use super::parts::{LINK, icon};
use super::place::{Clip, Copied, PartHost, placed};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "copy",
        classes: &["copies"],
        tags: &[],
        name: "CopyButton",
        used: "An issue page's round copy buttons, as Linear's: Copy link (after the page's origin), Copy ID, Copy branch name.",
        example: Some(|_| {
            rsx! { Copies { CopyButton { id: "c-copy", label: "Copy link", text: "/tasks/a-task", origin: true, icon: icon(LINK) } } }
        }),
        ..Component::BARE
    }
}

/// Copies `text` on a tap, Linear's round copy buttons over an issue (board_live.ex's `copy_button`: Copy link, Copy ID,
/// Copy branch name; task rust-board-buttons-new-issue). `origin`: `text` is a path, copied after the page's origin, so the
/// link works from any address the page was opened at. Its label is its title and its text form. A live part (`placed`): its
/// tap is `CopyLive`'s Rust handler, run by the page's liveview and in the window (task rust-js-port-small).
#[component]
pub fn CopyButton(
    #[props(into)] id: String,
    label: &'static str,
    #[props(into)] text: String,
    #[props(default)] origin: bool,
    icon: Element,
) -> Element {
    let icon = Some(dioxus_ssr::render_element(icon));
    let label = label.to_string();
    let part = CopyPart {
        id,
        label,
        text,
        origin,
        icon,
        select: None,
    };
    placed("Copy", &part, rsx! { CopyLive { part: part.clone() } })
}

inventory::submit! {
    Component {
        slug: "copy-text",
        classes: &[],
        tags: &[],
        name: "CopyText",
        used: "A draft's « Copy the text »: the message copied whole, its words then say Copied, or Copy failed with the text selected.",
        example: Some(|_| rsx! { p { id: "c-copy-words", "Hello, a short message." } Copies { CopyText { id: "c-copy-text", text: "Hello, a short message.", select: "c-copy-words" } } }),
        ..Component::BARE
    }
}

/// Copies a message's whole text on a tap, for him to paste where it goes (the older draft page's « Copy the text »; task
/// rust-draft-state-edit): its words then say « Copied », or « Copy failed, the text is selected » with the element `select`
/// names selected, so he copies it by hand. A live part, as CopyButton.
#[component]
pub fn CopyText(
    #[props(into)] id: String,
    #[props(into)] text: String,
    #[props(into)] select: String,
) -> Element {
    let label = "Copy the text".to_string();
    let part = CopyPart {
        id,
        label,
        text,
        origin: false,
        icon: None,
        select: Some(select),
    };
    placed("Copy", &part, rsx! { CopyLive { part: part.clone() } })
}

/// CopyButtons side by side.
#[component]
pub fn Copies(children: Element) -> Element {
    rsx! { div { class: "copies", {children} } }
}

/// A CopyButton or a CopyText as its renderer draws it again: `icon` the round button's drawing, as HTML; none, a button of
/// its words.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CopyPart {
    pub id: String,
    /// The round button's title and name; a button of words, its words.
    pub label: String,
    pub text: String,
    pub origin: bool,
    pub icon: Option<String>,
    pub select: Option<String>,
}

/// What a copy says once tapped, as its host did it: the round button in its title, the words in its words.
fn said(copied: Copied) -> &'static str {
    match copied {
        Copied::Yes => "Copied",
        Copied::Selected => "Copy failed, the text is selected",
        Copied::No => "Not copied",
    }
}

/// A copy's button with its Rust handler (task rust-js-port-small; it replaced COPY_JS): a tap asks the host to write the
/// clipboard, then says whether it did. With no host (a page's picture) a tap does nothing.
/// ponytail: « Copied » stays until the next tap, where the script put the title back after 2.5 s; a timer when he asks.
#[component]
pub fn CopyLive(part: CopyPart) -> Element {
    let host = try_consume_context::<PartHost>();
    let mut told = use_signal(|| None::<&'static str>);
    let CopyPart {
        id,
        label,
        text,
        origin,
        icon,
        select,
    } = part;
    let clip = Clip {
        text: text.clone(),
        origin,
        select,
    };
    let onclick = move |_| {
        let (Some(host), clip) = (host.clone(), clip.clone()) else {
            return;
        };
        spawn(async move {
            let copied = (host.copy)(clip).await;
            told.set(Some(said(copied)));
        });
    };
    let shown = told().map_or(label.clone(), str::to_string);
    match icon {
        Some(icon) => rsx! {
            button { id, class: "button icon round", r#type: "button", title: "{shown}", "aria-label": "{label}", onclick,
                span { class: "text-only", "{label}: {text}" }
                span { "aria-hidden": "true", dangerous_inner_html: icon }
            }
        },
        None => rsx! { button { id, class: "button", r#type: "button", onclick, "{shown}" } },
    }
}
