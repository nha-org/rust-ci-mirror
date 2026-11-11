use super::aspect_ratio::AspectRatio;
use super::parts::PIXEL;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "image",
        classes: &["image"],
        tags: &[],
        name: "Image",
        used: "A picture a mail carries on /mail/t/<thread>, inside its Attachment, cropped to its ratio. Built on rust-ui's Image, which dioxus-components lacks.",
        example: Some(|_| {
            rsx! {
                Image { src: PIXEL, alt: "A green square", ratio: 4.0 / 3.0 }
            }
        }),
        library: Some("https://rust-ui.com/docs/components/image"),
        menu: true,
        ..Component::BARE
    }
}

/// A picture held to its `ratio` while it loads, cropped to fill it: rust-ui's Image (notice in NOTICE-rust-ui), its `alt`,
/// lazy load and async decode kept, on `AspectRatio`. Its text form is nothing: what holds it names it.
#[component]
pub fn Image(
    #[props(into)] src: String,
    #[props(into)] alt: String,
    #[props(default = 1.0)] ratio: f64,
) -> Element {
    rsx! {
        AspectRatio { ratio,
            img { class: "image", src, alt, loading: "lazy", decoding: "async" }
        }
    }
}
