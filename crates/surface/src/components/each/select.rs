use super::avatar::Avatar;
use super::form::Form;
use super::input::Input;
use super::parts::initials_of;
use super::place::{PartHost, placed};
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
            }
        }),
        ..Component::BARE
    }
}

/// One of an issue's properties as Linear's sidebar sets it (board_live.ex's `property_menu`; task
/// rust-board-buttons-new-issue): its value, a tap opens its choices, each a button posting `field` and its value to `action`.
/// A `<details>`, so it opens with no script; the choice shown is checked. Its field filters the choices as he types, as
/// Linear's menus do (task rust-issue-sidebar-menus). `many`: each choice turns one value on or off (labels), the others
/// kept, posting `on` 1 or 0 beside it. `initials`: each choice opens on its name's avatar (assignee). `numbered`: each choice
/// ends on its place from 0, and that digit picks it while the menu has focus, as the older app's assignee and « Remind me »
/// menus (its `data-n`). A live part (`placed`): the filter and the digits are `MenuLive`'s Rust handlers, run by the page's
/// liveview and in the window (task rust-js-port-small).
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
    let current = dioxus_ssr::render_element(current);
    let (field, label) = (field.to_string(), label.to_string());
    let part = MenuPart {
        id,
        action,
        field,
        label,
        current,
        choices,
        many,
        initials,
        numbered,
    };
    placed("Menu", &part, rsx! { MenuLive { part: part.clone() } })
}

/// A PropertyMenu as its renderer draws it again: `current` the value shown, as HTML.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MenuPart {
    pub id: String,
    pub action: String,
    pub field: String,
    pub label: String,
    pub current: String,
    pub choices: Vec<(String, String, bool)>,
    pub many: bool,
    pub initials: bool,
    pub numbered: bool,
}

/// Whether a choice stays shown under the filter's words `q`: its value or its name holds them, case aside (the digit is no
/// word of it).
pub fn kept(q: &str, value: &str, name: &str) -> bool {
    let q = q.trim().to_lowercase();
    q.is_empty() || format!("{value}: {name}").to_lowercase().contains(&q)
}

/// The choice a key picks in a numbered menu: a digit alone, pressed once (no repeat, no modifier, not composing), naming a
/// choice the filter shows. The form's id, `<menu>-<n>`.
pub fn picked(m: &MenuPart, q: &str, key: &Key, plain: bool) -> Option<String> {
    let Key::Character(c) = key else { return None };
    let n: usize = c
        .parse()
        .ok()
        .filter(|_| plain && m.numbered && c.len() == 1)?;
    let (value, name, _) = m.choices.get(n)?;
    kept(q, value, name).then(|| format!("{}-{n}", m.id))
}

/// A PropertyMenu with its Rust handlers (task rust-js-port-small; they replaced FILTER_JS): the filter's words hide the
/// choices without them; a digit, pressed while the menu holds focus but not in its filter, sends the numbered choice it
/// names, through the host. With no host (a page's picture) the filter still filters, the digits send nothing.
/// ponytail: a digit with focus on the page's body (the script's « one menu open ») picks nothing: liveview hears only its own
/// elements, and Safari leaves focus there on a click of the summary; a tab to the menu first, until the page is live whole.
#[component]
pub fn MenuLive(part: MenuPart) -> Element {
    let host = try_consume_context::<PartHost>();
    let mut q = use_signal(String::new);
    let m = part.clone();
    let onkeydown = move |e: KeyboardEvent| {
        let plain = !e.is_auto_repeating() && !e.is_composing() && e.modifiers().is_empty();
        if let (Some(host), Some(form)) = (&host, picked(&m, &q.peek(), &e.key(), plain)) {
            e.prevent_default();
            (host.submit)(&form);
        }
    };
    let MenuPart {
        id,
        action,
        field,
        label,
        current,
        choices,
        many,
        initials,
        numbered,
    } = part;
    let role = if many {
        "menuitemcheckbox"
    } else {
        "menuitemradio"
    };
    rsx! {
        details { id: "{id}", class: "prop-menu", onkeydown,
            summary { title: "{label}", dangerous_inner_html: current }
            div { class: "prop-menu-list", role: "menu", "aria-label": "{label}",
                // for agents alone: where a pick posts, each choice's value and the one set
                span { class: "text-only", "{label}: POST {action} {field}=" if many { " with on=1 to turn one on, on=0 off" } }
                // its own keys are words, never a pick; no `value` bound: over liveview a patch of an older value would undo a
                // letter typed since (/code-review)
                Input { r#type: "search", class: "prop-menu-filter", placeholder: "{label}", "aria-label": "{label}",
                    autocomplete: "off", oninput: move |e: FormEvent| q.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| e.stop_propagation() }
                for (n, (value, name, on)) in choices.into_iter().enumerate() {
                    Form { id: "{id}-{n}", action: action.clone(), hidden: !kept(&q(), &value, &name),
                        Input { r#type: "hidden", name: "{field}", value: "{value}" }
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
