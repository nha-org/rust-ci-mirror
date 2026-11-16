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

/// What a component reads the host's switches from, a root context the binary provides on each tree it draws: a
/// component is handed nothing else of the binary's. None provided (a test, a catalog example): every plugin on.
#[derive(Clone)]
pub enum Hosted {
    /// A page's server render: the switches as its host held them then, and its user.
    Read(std::sync::Arc<dash_core::switches::Switches>, String),
    /// A live tree (the window, a liveview island): the host itself, read at each draw and written through.
    Live(&'static dyn dash_core::Host),
}

impl Hosted {
    /// `host`'s switches now, and its user; no host: every plugin on.
    pub fn read(host: Option<&dyn dash_core::Host>) -> Self {
        match host {
            Some(h) => Self::Read(dash_core::switches::now(h), dash_core::switches::user(h)),
            None => Self::Read(Default::default(), "me".into()),
        }
    }

    /// The switches to draw from, and whose menu.
    pub fn switches(&self) -> (std::sync::Arc<dash_core::switches::Switches>, String) {
        match self {
            Self::Read(switches, user) => (switches.clone(), user.clone()),
            Self::Live(h) => (dash_core::switches::now(*h), dash_core::switches::user(*h)),
        }
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
    // none provided (a test, the catalog's example): every plugin on, and a write refused
    let hosted = try_consume_context::<Hosted>().unwrap_or_else(|| Hosted::read(None));
    let (held, user) = hosted.switches();
    let (plugins, menu) = (dash_core::registry::plugins_in(&held), held.menu(&user));
    // a write goes through a live tree's host alone: a server render's switches are a reading
    let live = match hosted {
        Hosted::Live(h) => Some(h),
        Hosted::Read(..) => None,
    };
    let write = move |w: &dyn Fn(&dyn dash_core::Host) -> Result<(), String>| match live {
        Some(h) => w(h),
        None => Err(switches::UNKEPT.to_string()),
    };
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
                            move |e: FormEvent| done(write(&|h| switches::switch(h, &name, None, e.checked())))
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
                                move |e: FormEvent| done(write(&|h| switches::switch(h, &name, Some(capability), e.checked())))
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
                        onclick: move |_| done(write(&|h| switches::moved(h, pane.href, true))), "Up"
                    }
                    button { class: "button", r#type: "button", id: "pane-{pane.plugin()}-down", disabled: n == last,
                        onclick: move |_| done(write(&|h| switches::moved(h, pane.href, false))), "Down"
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
pub fn plugin_lines(host: Option<&dyn dash_core::Host>) -> Vec<String> {
    let (held, user) = Hosted::read(host).switches();
    let (plugins, menu) = (dash_core::registry::plugins_in(&held), held.menu(&user));
    let state = |on: bool| if on { "on" } else { "off" };
    let mut lines = vec!["Plugins".to_string()];
    for p in plugins {
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
        menu.iter()
            .map(|(pane, _)| format!("{}\t{}", pane.name, pane.href)),
    );
    lines
}
