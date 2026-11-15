//! /settings' plugins (task rust-plugin-settings; @nha, 2026-11-14: « Can settings enable/disable plugins ? », « Can settings
//! reorder plugins in the menu ? »): each plugin on or off, whole or one capability, and the menu's order, each a Dioxus
//! handler, run natively in the window and through the page's liveview on the web (its island), never a script or a form.
use super::switch::Switch;
use crate::component::{Component, Terminal};
use dash_core::switches;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "plugin-switches",
        classes: &["plugin-switches", "plugin-row", "plugin-order"],
        tags: &[],
        name: "PluginSwitches",
        used: "/settings' plugins: each on or off, whole or one capability (its pane, its tools, its jobs), and the menu's \
               order for this user, each through its Dioxus handler.",
        example: Some(|_| rsx! { PluginSwitches { id: "c-plugin-switches" } }),
        terminal: Some(Terminal::Words),
        css: ".plugin-switches { display: grid; gap: var(--spacing-s2); }\n\
              .plugin-row, .plugin-order { display: flex; flex-wrap: wrap; align-items: center; gap: var(--spacing-s3); }\n\
              .plugin-row > .switch:first-child { min-width: 10rem; font-weight: 600; }",
        ..Component::BARE
    }
}

/// The island /settings' liveview runs: its plugins and the menu's order.
pub const PLUGINS_ISLAND: &str = "plugins";

/// Each plugin, its switch first then one a capability, then the menu's top entries in this user's order, each with Up and
/// Down. A handler writes through the core's switches, and the list is drawn again from what they then hold; a write
/// refused is said under the list. Settings stays on: its page turns the others back on.
#[component]
pub fn PluginSwitches(#[props(into)] id: String) -> Element {
    let mut writes = use_signal(|| 0u32);
    let mut said = use_signal(String::new);
    // a refused write draws its rows anew: the switch the browser flipped is set back, its value unchanged sending no patch
    // (/code-review)
    let mut refused = use_signal(|| 0u32);
    // read so each write draws the list again
    let _ = writes();
    let mut done = move |r: Result<(), String>| {
        if r.is_err() {
            refused += 1;
        }
        said.set(r.err().unwrap_or_default());
        writes += 1;
    };
    let plugins = dash_core::registry::plugins();
    let menu = switches::menu();
    let last = menu.len().saturating_sub(1);
    rsx! {
        div { id, class: "plugin-switches",
            h2 { "Plugins" }
            for p in plugins {
                div { class: "plugin-row", key: "{p.name}/{refused}",
                    // a sub-entry off with its parent: its switch waits for the parent's
                    Switch { id: format!("plugin-{}", p.name), name: "plugin", checked: p.on,
                        disabled: p.name == switches::SETTINGS || p.held_by.is_some(),
                        oninput: {
                            let name = p.name.clone();
                            move |e: FormEvent| done(switches::switch(&name, None, e.checked()))
                        },
                        "{p.name}"
                    }
                    if let Some(parent) = p.held_by {
                        span { "off with {parent}" }
                    }
                    for (capability, on) in p.capabilities {
                        Switch { id: format!("plugin-{}-{capability}", p.name), name: "capability", checked: on,
                            disabled: p.name == switches::SETTINGS,
                            oninput: {
                                let name = p.name.clone();
                                move |e: FormEvent| done(switches::switch(&name, Some(capability), e.checked()))
                            },
                            "{capability}"
                        }
                    }
                }
            }
            h2 { "Menu" }
            for (n, (pane, _)) in menu.into_iter().enumerate() {
                div { class: "plugin-order", key: "{pane.href}",
                    span { "{pane.name}" }
                    button { class: "button", r#type: "button", id: "pane-{pane.plugin()}-up", disabled: n == 0,
                        onclick: move |_| done(switches::moved(pane.href, true)), "Up"
                    }
                    button { class: "button", r#type: "button", id: "pane-{pane.plugin()}-down", disabled: n == last,
                        onclick: move |_| done(switches::moved(pane.href, false)), "Down"
                    }
                }
            }
            if !said().is_empty() {
                p { role: "alert", "{said}" }
            }
        }
    }
}

/// The plugins and the menu as an agent reads them: a plugin a line, its state and each capability's, then the menu's order.
pub fn plugin_lines() -> Vec<String> {
    let state = |on: bool| if on { "on" } else { "off" };
    let mut lines = vec!["Plugins".to_string()];
    for p in dash_core::registry::plugins() {
        let caps: Vec<String> = (p.capabilities.iter())
            .map(|(c, on)| format!("{c} {}", state(*on)))
            .collect();
        let on = match p.held_by {
            Some(parent) => format!("off with {parent}"),
            None => state(p.on).into(),
        };
        lines.push(format!("{}\t{on}\t{}", p.name, caps.join(", ")));
    }
    lines.push("Menu".into());
    lines.extend(
        switches::menu()
            .iter()
            .map(|(pane, _)| format!("{}\t{}", pane.name, pane.href)),
    );
    lines
}
