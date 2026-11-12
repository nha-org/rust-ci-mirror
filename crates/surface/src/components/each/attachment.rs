use super::aspect_ratio::AspectRatio;
use super::image::Image;
use super::parts::{DOC, PIXEL, card_href, icon, local_path};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "attachment",
        classes: &["attachment", "attachment-group", "attachment-media", "attachment-content", "attachment-title", "attachment-description"],
        tags: &[],
        name: "Attachment",
        used: "A mail's files on /mail/t/<thread>, side by side: a document its icon, its name and kind, a picture shown upright. Built on rust-ui's Attachment, which dioxus-components lacks.",
        example: Some(|_| {
            rsx! {
                AttachmentGroup {
                    Attachment { id: "c-attachment-pdf", name: "slots.pdf", kind: "application/pdf", href: "/mail/blob/b1?name=slots.pdf" }
                    Attachment { id: "c-attachment-png", name: "room.png", kind: "image/png", href: PIXEL, picture: true }
                    Attachment { id: "c-attachment-none", name: "notes", kind: "text/plain", href: "javascript:alert(1)" }
                }
            }
        }),
        library: Some("https://rust-ui.com/docs/components/attachment"),
        menu: true,
        ..Component::BARE
    }
}

/// A file to open or download: rust-ui's Attachment (notice in NOTICE-rust-ui) on this system's tokens, built on
/// `AspectRatio`. Its media first, a document's icon in a square or, for a picture (`is_picture`), the picture itself as an
/// `Image`, the tile then upright as rust-ui's vertical one; then its name on one line and its kind, the name's extension
/// or else the MIME type. The whole tile is its link, as rust-ui's trigger; a link no page may open (`card_href`) leaves it
/// a tile. A picture is fetched only from this app (`local_path`) or drawn from its own bytes (a `data:image/` one, the
/// catalog's), never from another host, which would see it read (/code-review). ponytail: no size, no upload states nor
/// remove action, and a picture loads whole, no thumbnail; rust-ui's states when a page uploads, a thumbnail when one
/// weighs.
#[component]
pub fn Attachment(
    #[props(into)] id: String,
    #[props(into)] name: String,
    #[props(into)] kind: String,
    #[props(into)] href: String,
    #[props(default)] picture: bool,
) -> Element {
    let link = card_href(&Some(href.clone())).map(str::to_string);
    let bytes = href.starts_with("data:image/") && !href.starts_with("data:image/svg");
    let src = (picture && (local_path(&href) || bytes)).then_some(href);
    // an extension is letters and digits alone: « Minutes 10.30 am » has none (/code-review)
    let what = match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            ext.to_ascii_uppercase()
        }
        _ => kind,
    };
    let orientation = if src.is_some() {
        "vertical"
    } else {
        "horizontal"
    };
    let media = match src {
        // its name follows in words: the picture says it once more to no one (/code-review)
        Some(src) => rsx! { Image { src, alt: "", ratio: 4.0 / 3.0 } },
        None => {
            rsx! { AspectRatio { {icon(DOC)} } }
        }
    };
    let parts = rsx! {
        div { class: "attachment-media", {media} }
        span { class: "attachment-content",
            b { class: "attachment-title", "{name}" }
            span { class: "attachment-description", "{what}" }
        }
    };
    match link {
        Some(href) => rsx! {
            a { id, class: "attachment", "data-orientation": orientation, href, title: "{name}", {parts} }
        },
        None => rsx! { div { id, class: "attachment", "data-orientation": orientation, {parts} } },
    }
}

/// Attachments side by side, as rust-ui's AttachmentGroup: one row that scrolls along, each tile snapping to its start.
#[component]
pub fn AttachmentGroup(children: Element) -> Element {
    rsx! { div { class: "attachment-group", {children} } }
}
