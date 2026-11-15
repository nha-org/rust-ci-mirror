use crate::component::{Component, Section};
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "alert",
        classes: &["alert", "alert-title"],
        tags: &[],
        sections: &[Section {
            family: "feedback",
            id: "alert",
            title: "Alert",
            after: "",
            rows: &[
                ("A tool not installed", |_| rsx! { Alert { id: "ds-alert", title: "No hc to run", "dash shows terminals through herdr-core's hc, and cannot run it. Install hc at ~/.local/bin/hc, or set HC_BIN to its path in dash serve's environment, then reload this page." } }),
            ],
        }],
        heroui: Some(("alert", "ours", "banner")),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// Where a page stands when it cannot show its thing, its title then what to do about it (task rust-terminal-no-hc: /terminal
/// with no hc). A status, not an error: the page answers 200 and says so in the same place on the web, in the window and in
/// the text.
#[component]
pub fn Alert(
    #[props(into)] id: String,
    #[props(into)] title: String,
    children: Element,
) -> Element {
    rsx! {
        div { id, class: "alert", role: "status",
            b { class: "alert-title", "{title}" }
            p { {children} }
        }
    }
}
