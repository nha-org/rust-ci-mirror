use super::badge::Badge;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "aspect_ratio",
        classes: &["aspect-ratio", "aspect-ratio-content"],
        tags: &[],
        name: "AspectRatio",
        used: "Holds a picture's room while it loads: under each Image and an Attachment's media square.",
        example: Some(|_| {
            rsx! {
                AspectRatio { ratio: 16.0 / 9.0, Badge { look: "chip", "16:9" } }
            }
        }),
        menu: true,
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A box held to `ratio`, its width over its height: dioxus-components' AspectRatio (notice in NOTICE-dioxus-components), its
/// two boxes kept, the outer one's height its padding as upstream's, so what it holds takes its room before it loads. A ratio
/// that is no positive number is a square.
#[component]
pub fn AspectRatio(#[props(default = 1.0)] ratio: f64, children: Element) -> Element {
    let ratio = if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    };
    let pad = 100.0 / ratio;
    rsx! {
        div { class: "aspect-ratio", style: "padding-bottom: {pad}%",
            div { class: "aspect-ratio-content", {children} }
        }
    }
}
