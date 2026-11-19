//! /settings' live part (task untangle-switches-ports; ARCHITECTURE.md section 4 point 7): the Elements each plugin placed
//! under `settings/` and `status/`, drawn by the core as a tree of catalog switches and buttons, each keyed as
//! `dash_core::switches::set` reads it. Their Dioxus handlers write through the host, natively in the window and through the
//! page's liveview on the web (its island, drawn by crate::screen's `SettingsLive`), never a script or a form; the island then
//! draws the tree again from what the switches hold, so a switch takes effect at once. The design system names no plugin here.
use super::switch::Switch;
use dioxus::prelude::*;
use std::collections::BTreeMap;

use dash_core::registry::Automation;
use dash_core::switches::{
    JOB, PANE, PLACE, Pane, STATUS, Switches, TOOL, draw_settings, draw_status, plugin_of, read,
    user,
};
use dash_core::{Host, Tree, json, placed};
use std::collections::BTreeSet;

/// The host a live tree writes through, a root context the binary provides on the window and on each liveview: a component
/// is handed nothing else of the binary's. None provided (a server render, a test, a catalog example): a picture, no handler.
#[derive(Clone, Copy)]
pub struct Hosted(pub &'static dyn dash_core::Host);

/// Each plugin the binary's registry holds, with whether it registered a tool (`Registry::plugins`): /settings lists its
/// tool switch. The binary provides it beside `Hosted`; none: no tool switch drawn.
#[derive(Clone, Default)]
pub struct Registered(pub BTreeMap<String, bool>);

/// The island /settings' liveview runs.
pub const SETTINGS_ISLAND: &str = "settings";

/// /settings' plugins part, its elements by id, `plugins` the island around them: the core's drawing of every plugin's
/// settings (`dash_core::settings`), which the island's liveview draws again live.
pub fn settings_part(
    host: &dyn dash_core::Host,
    registered: &BTreeMap<String, bool>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut tree = dash_core::Tree::default();
    let parts = settings(host, registered, SETTINGS_ISLAND, &mut tree);
    let island = serde_json::json!({ "name": SETTINGS_ISLAND });
    tree.add("plugins", "island", island, &parts);
    let dash_core::Drawing::Tree { tree, .. } = tree.drawing("", "plugins") else {
        return serde_json::Map::new();
    };
    let tree: serde_json::Value = serde_json::from_str(&tree).unwrap_or_default();
    tree["elements"].as_object().cloned().unwrap_or_default()
}

/// What a write by a live setting told its island: each refused one, which draws the tree anew so the control the browser
/// flipped is set back (its value unchanged sends no patch), and the last refusal's words.
#[derive(Clone, Copy)]
pub(crate) struct Settled {
    pub(crate) writes: Signal<u32>,
    pub(crate) refused: Signal<u32>,
    pub(crate) said: Signal<String>,
}

impl Settled {
    fn done(mut self, written: Result<(), String>) {
        if written.is_err() {
            self.refused += 1;
        }
        self.said.set(written.err().unwrap_or_default());
        self.writes += 1;
    }
}

/// `setting` written through the live host, `on` its new state, and its island told; none live: refused, said.
fn write(live: (Option<Hosted>, Option<Settled>), setting: &str, on: bool) {
    let written = match live.0 {
        Some(Hosted(host)) => dash_core::switches::set(host, setting, on),
        None => Err(dash_core::switches::UNKEPT.to_string()),
    };
    if let Some(settled) = live.1 {
        settled.done(written);
    }
}

/// What a live setting writes through and tells, read where it is drawn.
fn live() -> (Option<Hosted>, Option<Settled>) {
    (try_consume_context(), try_consume_context())
}

/// A setting's switch: the catalog's Switch, its handler writing `setting` while its page runs live.
#[component]
pub fn SettingSwitch(
    #[props(into)] id: String,
    label: String,
    setting: String,
    on: bool,
    disabled: bool,
) -> Element {
    // live only inside its island, which redraws after a write and says a refusal; anywhere else a picture (/code-review)
    let live = live();
    rsx! {
        if live.0.is_some() && live.1.is_some() {
            Switch { id, name: "setting", checked: on, disabled,
                oninput: move |e: FormEvent| write(live, &setting, e.checked()), "{label}"
            }
        } else {
            Switch { id, name: "setting", checked: on, disabled, "{label}" }
        }
    }
}

/// A setting's button (a menu entry moved): the catalog's button, its handler writing `setting` while its page runs live.
#[component]
pub fn SettingButton(
    #[props(into)] id: String,
    label: String,
    setting: String,
    disabled: bool,
) -> Element {
    // live only inside its island, as a switch; anywhere else a picture
    let live = live();
    let disabled = disabled || live.1.is_none();
    rsx! {
        super::button::Button { r#type: "button", id, disabled,
            onclick: move |_| write(live, &setting, true), "{label}"
        }
    }
}

/// A plugin as /settings lists it: its name, whether it is on, and each capability it has with its own switch.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub name: String,
    pub on: bool,
    /// The plugin whose pane its pane sits under, while that one hides it: off with it, its own switch moot meanwhile
    /// (/code-review: a sub-entry said on while its page answered 404).
    pub held_by: Option<String>,
    pub capabilities: Vec<(&'static str, bool)>,
    /// Its panes' names in menu order, so /settings says what its switch hides: « fallback » is Projects and Status.
    pub panes: Vec<&'static str>,
}

/// Every plugin this process knows, by name, as `switches` hold them: its panes and jobs as registered, its tools as
/// `registered` lists them (a registry's `plugins`), each plugin there listed too.
pub fn plugins_in(switches: &Switches, registered: &BTreeMap<String, bool>) -> Vec<Plugin> {
    let mut found: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    // a plugin is held while one of its panes sits under a parent hidden: /settings says it off with that parent, as that
    // pane's pages answer 404 (/code-review: a sub-entry said on while its page answered 404)
    let mut held: BTreeMap<String, String> = BTreeMap::new();
    let mut panes: BTreeMap<String, Vec<&Pane>> = BTreeMap::new();
    for pane in inventory::iter::<Pane> {
        let parent =
            (pane.under).and_then(|h| inventory::iter::<Pane>.into_iter().find(|p| p.href == h));
        let plugin = pane.plugin();
        if let Some(parent) = parent.filter(|p| !switches.shows(p)) {
            held.entry(plugin.clone()).or_insert(parent.plugin());
        }
        panes.entry(plugin.clone()).or_default().push(pane);
        found.entry(plugin).or_default().insert(PANE);
    }
    for (plugin, tools) in registered {
        let caps = found.entry(plugin.clone()).or_default();
        if *tools {
            caps.insert(TOOL);
        }
    }
    let recurring = inventory::iter::<dash_core::Recurring>
        .into_iter()
        .map(|r| r.origin);
    for origin in inventory::iter::<Automation>
        .into_iter()
        .map(|a| a.origin)
        .chain(recurring)
    {
        found.entry(plugin_of(origin)).or_default().insert(JOB);
    }
    found
        .into_iter()
        .map(|(name, capabilities)| Plugin {
            panes: panes.remove(&name).map_or(vec![], |mut p| {
                p.sort_by_key(|p| (p.under.is_some(), p.order, p.href));
                p.into_iter().map(|p| p.name).collect()
            }),
            held_by: held.get(&name).cloned(),
            on: switches.on(&name, None) && !held.contains_key(&name),
            capabilities: capabilities
                .into_iter()
                // its own switch, whatever the plugin's: a plugin on again finds each capability as it was
                .map(|c| (c, !switches.off.contains(&format!("{name}/{c}"))))
                .collect(),
            name,
        })
        .collect()
}

/// One plugin as `plugins_in` lists it, `tools` whether the registry holds its tools: a plugin's own settings Element draws it.
pub fn plugin(switches: &Switches, name: &str, tools: bool) -> Plugin {
    let registered = BTreeMap::from([(name.to_string(), tools)]);
    let found = plugins_in(switches, &registered)
        .into_iter()
        .find(|p| p.name == name);
    found.unwrap_or_else(|| Plugin {
        name: name.into(),
        on: switches.on(name, None),
        held_by: None,
        capabilities: vec![],
        panes: vec![],
    })
}

/// A plugin as an element under `settings/` or `status/` is handed it, in its props: what `plugins_in` read, once a draw of
/// /settings (/code-review: each element read the switches and listed every plugin again).
fn props(p: &Plugin) -> serde_json::Value {
    let capabilities: Vec<_> = (p.capabilities.iter())
        .map(|(c, on)| json!([c, on]))
        .collect();
    json!({"plugin": p.name, "on": p.on, "held_by": p.held_by, "capabilities": capabilities, "panes": p.panes})
}

/// /settings' plugins, menu and status (ARCHITECTURE.md section 4 point 7): each plugin drawn by the Element it placed under
/// `settings/`, handed its name from its origin and whether `registered` lists its tools; the menu's top entries in this
/// user's order, each with Up and Down; then each under `status/`. A plugin with no Element of its own (an extensions.d pod
/// until its manifest brings one, phase D; a plugin crate whose jobs or tools have no pane, until the sdk carries
/// `settings!`, untangle-sdk-crate) is drawn by the core's. Names no plugin.
pub fn settings(
    host: &dyn Host,
    registered: &BTreeMap<String, bool>,
    id: &str,
    tree: &mut Tree,
) -> Vec<String> {
    // the switches read once, every plugin this process knows (its panes, its jobs, the registry's) listed once
    let switches = read(host);
    let mut listed: BTreeMap<String, Plugin> = (plugins_in(&switches, registered).into_iter())
        .map(|p| (p.name.clone(), p))
        .collect();
    let mut parts = vec![];
    for place in [PLACE, STATUS] {
        let mut own: BTreeMap<String, dash_core::Draw> = BTreeMap::new();
        for e in placed(place) {
            own.entry(plugin_of(e.origin)).or_insert(e.draw);
        }
        // a plugin that placed an Element of its own and registered nothing else is listed too
        for name in own.keys() {
            let tools = registered.get(name).copied().unwrap_or(false);
            listed
                .entry(name.clone())
                .or_insert_with(|| plugin(&switches, name, tools));
        }
        let core = if place == PLACE {
            draw_settings
        } else {
            draw_status
        } as dash_core::Draw;
        let mut rows = vec![];
        for (name, p) in &listed {
            let draw = own.get(name).copied().unwrap_or(core);
            let at = format!("{id}-{place}-{name}");
            match draw(host, &props(p), &at, tree) {
                Ok(ids) => rows.extend(ids),
                Err(e) => {
                    tree.add(&at, "text", json!({"text": format!("{name}: {e}")}), &[]);
                    rows.push(at);
                }
            }
        }
        if place == STATUS {
            parts.push(menu_order(
                &switches.menu(&user(host)),
                &format!("{id}-menu"),
                tree,
            ));
        }
        let section = format!("{id}-{place}");
        let title = if place == PLACE { "Plugins" } else { "Status" };
        tree.add(&section, "section", json!({"title": title}), &rows);
        parts.push(section);
    }
    parts
}

/// The menu's top entries in this user's order, each with Up and Down, the catalog's buttons keyed as `set` reads them.
fn menu_order(menu: &[(&'static Pane, Vec<&'static Pane>)], id: &str, tree: &mut Tree) -> String {
    let last = menu.len().saturating_sub(1);
    let mut rows = vec![];
    for (n, (pane, _)) in menu.iter().enumerate() {
        let slug = pane.href.trim_start_matches('/');
        let (row, name) = (format!("{id}-{slug}"), format!("{id}-{slug}-name"));
        let (up, down) = (format!("pane-{slug}-up"), format!("pane-{slug}-down"));
        tree.add(&name, "text", json!({"text": pane.name}), &[]);
        tree.add(
            &up,
            "button",
            json!({"label": "Up", "key": format!("up:{}", pane.href), "disabled": n == 0}),
            &[],
        );
        tree.add(
            &down,
            "button",
            json!({"label": "Down", "key": format!("down:{}", pane.href), "disabled": n == last}),
            &[],
        );
        tree.add(&row, "bar", json!(null), &[name, up, down]);
        rows.push(row);
    }
    tree.add(id, "section", json!({"title": "Menu"}), &rows);
    id.to_string()
}
