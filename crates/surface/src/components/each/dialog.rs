use super::button::Button;
use super::form::Form;
use super::input::Input;
use super::parts::{PLUS, icon};
use super::textarea::Textarea;
use crate::component::Component;
use dioxus::prelude::*;

/// Where a `NewIssue` form posts: `dash task file` files its words on the board's project (dash main.rs's `new_issue`).
pub const NEW_ISSUE: &str = "/tasks/new";

inventory::submit! {
    Component {
        slug: "dialog",
        classes: &["dialog", "dialog-title", "dialog-actions"],
        tags: &[],
        name: "Dialog",
        used: "The board's « + New issue »: a pill that opens the browser's own modal, no script, its form a title and a description posted to /tasks/new.",
        example: Some(|_| rsx! { NewIssue { id: "c-new-issue", project: None } }),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A modal over the page: dioxus-components' Dialog (notice in NOTICE-dioxus-components) on the browser's own `<dialog>`,
/// opened and closed by a button's `commandfor` and `command`, so no script runs (task rust-board-buttons-new-issue).
/// `closedby="closerequest"`: Esc closes it, a stray click outside never does, so a tap beside a long description keeps it
/// (/code-review of PR 1395).
#[component]
pub fn Dialog(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { dialog { id, class: "dialog", "aria-label": label, "closedby": "closerequest", {children} } }
}

/// Linear's « + New issue », top right of the board (board_live.ex's `new-issue-button`; @nha, 2026-11-10: « there should be
/// a "new issue" button »): a pill that opens a `Dialog` of its form, a title and a description, posted to `NEW_ISSUE`.
/// `project`: the board's, sent with them; none files on site_perso's. Its text form says where it posts and what.
#[component]
pub fn NewIssue(#[props(into)] id: String, project: Option<String>) -> Element {
    let dialog = format!("{id}-dialog");
    rsx! {
        button { id, class: "button pill", r#type: "button", "commandfor": "{dialog}", "command": "show-modal",
            {icon(PLUS)} "New issue"
        }
        Dialog { id: "{dialog}", label: "New issue",
            Form { id: "{dialog}-form", action: NEW_ISSUE,
                b { class: "dialog-title", "New issue" }
                Input { name: "title", placeholder: "Issue title", required: true, "autofocus": "autofocus", aria_label: "Title" }
                Textarea { name: "description", placeholder: "Add description...", rows: 4, aria_label: "Description" }
                if let Some(project) = project {
                    Input { r#type: "hidden", name: "project", value: "{project}" }
                }
                div { class: "dialog-actions",
                    button { class: "button quiet", r#type: "button", "commandfor": "{dialog}", "command": "close", "Cancel" }
                    Button { "Create issue" }
                }
            }
        }
    }
}
