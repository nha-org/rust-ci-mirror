use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "stages",
        classes: &["stages"],
        tags: &[],
        name: "Stage",
        used: "An RFC's stage under its title on /v/, the rfc skill's `ol.stages`: each step a node on one line, those reached \
               filled, the current one named in bold (@nha, 2026-10-18: « This is a horizontal timeline. design a real UI \
               element here. »).",
        example: Some(|_| rsx! {
            Stages { steps: vec!["Draft".into(), "Review".into(), "Accepted".into(), "Shipped".into()], current: "Accepted" }
        }),
        css: include_str!("stages.css"),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A document's stage: `steps` in order, each up to `current` reached (`done`), `current` the step it is at. A `current` that
/// names no step reaches none. `label` is its `aria-label`, the doc's own word (« Stage » when empty). Its markup is the rfc
/// skill's, so `dash doc import` reads an export back to the same block.
#[component]
pub fn Stages(
    #[props(into)] id: Option<String>,
    steps: Vec<String>,
    #[props(into)] current: String,
    #[props(into, default)] label: String,
) -> Element {
    let label = if label.is_empty() {
        "Stage".to_string()
    } else {
        label
    };
    let at = steps.iter().position(|s| *s == current);
    let reached = |i: usize| at.is_some_and(|a| i < a);
    rsx! {
        ol { id, class: "stages", "aria-label": "{label}",
            for (i, step) in steps.iter().enumerate() {
                if Some(i) == at {
                    li { "aria-current": "step", "{step}" }
                } else if reached(i) {
                    li { class: "done", "{step}" }
                } else {
                    li { "{step}" }
                }
            }
        }
    }
}
