use super::avatar::Avatar;
use super::form::Form;
use super::input::Input;
use super::parts::initials_of;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "select",
        classes: &["prop-menu", "prop-menu-list", "prop-menu-item"],
        tags: &[],
        name: "PropertyMenu",
        used: "An issue's property he sets from its page, Linear's sidebar menus: status, priority, assignee, cycle, due date, reminder, labels, project; each pick posts its field to /tasks/<id>/set with no script, its field filters the choices as he types; labels tick on and off, the assignee's choices open on their avatar.",
        example: Some(|_| {
            rsx! {
                PropertyMenu { id: "c-prop", action: "/tasks/a-task/set", field: "priority", label: "Set priority",
                    current: rsx! { span { "High" } },
                    choices: vec![("1".into(), "Urgent".into(), false), ("2".into(), "High".into(), true)] }
                PropertyMenu { id: "c-prop-who", action: "/tasks/a-task/set", field: "assignee", label: "Assign to...", initials: true,
                    current: rsx! { span { "Rust" } },
                    choices: vec![("nicolas".into(), "Nicolas Ha".into(), false), ("rust".into(), "Rust".into(), true)] }
                PropertyMenu { id: "c-prop-labels", action: "/tasks/a-task/set", field: "labels", label: "Change or add labels...",
                    many: true, current: rsx! { span { "Labels" } },
                    choices: vec![("Bug".into(), "Bug".into(), true), ("Design".into(), "Design".into(), true), ("Docs".into(), "Docs".into(), false)] }
                {menu_filter()}
            }
        }),
        ..Component::BARE
    }
}

/// One of an issue's properties as Linear's sidebar sets it (board_live.ex's `property_menu`; task
/// rust-board-buttons-new-issue): its value, a tap opens its choices, each a button posting `field` and its value to `action`.
/// A `<details>`, so it opens with no script; the choice shown is checked. Its field filters the choices as he types, as
/// Linear's menus do (task rust-issue-sidebar-menus), the page drawing `menu_filter` once; without it every choice stays. `many`: each
/// choice turns one value on or off (labels), the others kept, posting `on` 1 or 0 beside it. `initials`: each choice opens on its name's avatar (assignee).
/// `numbered`: each choice ends on its place from 0, and that digit picks it while the menu is open, as the older app's
/// assignee and « Remind me » menus (its `data-n`).
#[component]
pub fn PropertyMenu(
    #[props(into)] id: String,
    #[props(into)] action: String,
    field: &'static str,
    label: &'static str,
    current: Element,
    /// Each choice: its value, its name, whether it is the one shown.
    choices: Vec<(String, String, bool)>,
    #[props(default)] many: bool,
    #[props(default)] initials: bool,
    #[props(default)] numbered: bool,
) -> Element {
    let role = if many {
        "menuitemcheckbox"
    } else {
        "menuitemradio"
    };
    rsx! {
        details { id, class: "prop-menu",
            summary { title: label, {current} }
            div { class: "prop-menu-list", role: "menu", "aria-label": label,
                // for agents alone: where a pick posts, each choice's value and the one set
                span { class: "text-only", "{label}: POST {action} {field}=" if many { " with on=1 to turn one on, on=0 off" } }
                Input { r#type: "search", class: "prop-menu-filter", placeholder: label, "aria-label": label,
                    autocomplete: "off" }
                for (n, (value, name, on)) in choices.into_iter().enumerate() {
                    Form { action: action.clone(),
                        Input { r#type: "hidden", name: field, value: "{value}" }
                        // the state he wants, never « flip what is stored »: a list read since may hold it already
                        // (/code-review of PR 1399)
                        if many {
                            Input { r#type: "hidden", name: "on", value: if on { "0" } else { "1" } }
                        }
                        button { class: "prop-menu-item", role, "aria-checked": if on { "true" } else { "false" },
                            span { class: "text-only", "{value}:" }
                            if initials {
                                span { "aria-hidden": "true", Avatar { initials: initials_of(&name), look: "li-who" } }
                            }
                            // its own box, as the older app's `flex-1 truncate` span: beside the avatar and the digit it
                            // is a word of its own, never « CClaude0 » (dash parity read it glued)
                            span { "{name}" }
                            if on {
                                span { class: "text-only", "(set)" }
                            }
                            if numbered {
                                span { class: "prop-menu-n", "aria-hidden": "true", "data-n": "{n}", "{n}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// PropertyMenu's filter script, drawn once by the page that holds the menus (/code-review of PR 1399: once a menu sent it
/// eight times on an issue page).
pub fn menu_filter() -> Element {
    rsx! { script { dangerous_inner_html: FILTER_JS } }
}

/// PropertyMenu's filter: a choice whose words lack his text is hidden, its digit no word of it. A digit picks the numbered
/// choice of the menu he is in (the one open, when focus is nowhere), never one the filter hid, never while he types in a
/// field, an editor or a select, and once per press (/code-review of rust-issue-sidebar-menus). One pair of listeners a page.
const FILTER_JS: &str = r#"(()=>{if(window.dashMenuFilter)return;window.dashMenuFilter=1;const words=b=>[...b.childNodes].filter(c=>!(c.classList&&c.classList.contains("prop-menu-n"))).map(c=>c.textContent).join(" ").toLowerCase();document.addEventListener("input",e=>{const f=e.target;if(!f.matches||!f.matches(".prop-menu-filter"))return;const q=f.value.trim().toLowerCase();f.closest(".prop-menu-list").querySelectorAll(".prop-menu-item").forEach(b=>{b.closest("form").hidden=!!q&&!words(b).includes(q)})});document.addEventListener("keydown",e=>{const t=e.target;if(e.repeat||e.isComposing||e.metaKey||e.ctrlKey||e.altKey||!/^[0-9]$/.test(e.key)||!t.closest||t.isContentEditable||t.matches("input,textarea,select"))return;const open=document.querySelectorAll("details.prop-menu[open]");const m=t.closest("details.prop-menu[open]")||(t===document.body&&open.length===1?open[0]:null);const n=m&&m.querySelector('.prop-menu-n[data-n="'+e.key+'"]');const f=n&&n.closest("form");if(!f||f.hidden)return;e.preventDefault();f.requestSubmit()})})()"#;
