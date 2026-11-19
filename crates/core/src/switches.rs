//! Which plugins run and how the menu orders them (task rust-plugin-settings; @nha, 2026-11-14: « Can settings enable/disable
//! plugins ? », « Can settings reorder plugins in the menu ? »): rfc-superapp-plugins section 1, « A plugin can be turned on
//! or off, whole or capability by capability ». Each pane registers its menu entry beside its page (`pane!`), so the sidebar
//! lists the registry's entries, never a list of its own. A plugin's name is its entries' origin, the module `submit!`
//! stamped, read by `plugin_of` alone (rfc-superapp-plugins section 3): never a path, a URL or a name typed by hand; its
//! capabilities are its pane, its tools and its jobs. What is off and each user's order are the plugin runtime's own
//! database, plugins.sqlite in the data folder, plain SQL opened through the host (ARCHITECTURE.md section 3, decision 8),
//! read through the `switches` View where the host holds one, else its table at once: no cache, no lock, no static
//! (task untangle-switches-ports, section 6 B3). A plugin's settings and status are its own Elements under `settings/` and
//! `status/`, which `pane!` and `settings!` register beside it; /settings draws them and names no plugin.
use dash_sdk::{Host, Tree, json};
use std::collections::{BTreeMap, BTreeSet};

/// A plugin's entry in the menu, registered beside its page with `pane!`.
#[derive(Debug)]
pub struct Pane {
    /// The module that registered it, stamped by `pane!`: its plugin is `plugin_of`'s reading of it.
    pub origin: &'static str,
    pub name: &'static str,
    /// The page it opens.
    pub href: &'static str,
    /// Its place when its user ordered none: lower first.
    pub order: u16,
    /// A design-system icon, by name (dash_surface's `forms::named_icon`).
    pub icon: &'static str,
    /// The other paths it stands for: a page under one is its too (/graph is Tasks').
    pub paths: &'static [&'static str],
    /// The entry it sits under, by its href; none: a top-level entry.
    pub under: Option<&'static str>,
}

inventory::collect!(Pane);

impl Pane {
    pub fn plugin(&self) -> String {
        plugin_of(self.origin)
    }
}

/// A plugin's capabilities, each switched on its own.
pub const PANE: &str = "pane";
pub const TOOL: &str = "tool";
pub const JOB: &str = "job";

/// The plugin whose page turns the others back on: never off.
pub const SETTINGS: &str = "settings";

/// The plugin runtime's database in the data folder, one row a switch or a user's order.
pub const FILE: &str = "plugins.sqlite";
pub const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS switch (id TEXT PRIMARY KEY, value TEXT) WITHOUT ROWID;";
/// The view of its rows (dash_views): each row its id, then its value.
pub const VIEW: &str = "switches";

/// What is off, `<plugin>` or `<plugin>/<capability>`, and each user's order of the menu's top entries, by href.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Switches {
    pub off: BTreeSet<String>,
    pub order: BTreeMap<String, Vec<String>>,
    /// The database did not read and no read came before: every job stops, the rest stays on (/code-review).
    pub unread: bool,
}

impl Switches {
    /// The switches the rows hold, each its id and value.
    pub fn of(rows: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut switches = Self::default();
        for (row, value) in rows {
            switches.take(&row, &value);
        }
        switches
    }

    /// A row's value: `on/<key>` « on » or « off », `order/<user>` the hrefs a line each.
    fn take(&mut self, row: &str, v: &str) {
        if let Some(key) = row.strip_prefix("on/") {
            match v {
                "off" => self.off.insert(key.to_string()),
                _ => self.off.remove(key),
            };
        } else if let Some(user) = row.strip_prefix("order/") {
            let hrefs = v.lines().map(String::from).collect();
            self.order.insert(user.to_string(), hrefs);
        }
    }

    /// The one off filter (ARCHITECTURE.md section 4 point 8): why an entry `origin` registered is not dispatched, its
    /// plugin or that capability off, or the switches unread for a job; none: it runs. Pages, tools, jobs, slots and
    /// reranks all ask it, so an off plugin's entries each stop the same way.
    pub fn stops(&self, origin: &str, capability: Option<&str>) -> Option<String> {
        let plugin = plugin_of(origin);
        if self.on(&plugin, capability) {
            return None;
        }
        // the switch that holds it is named: the plugin's whole one first, which /settings shows above its capabilities'
        // (/code-review: a plugin off whole was said « its tools are off » while its tool switch showed on)
        Some(match capability {
            Some(JOB) if self.unread => {
                format!(
                    "the plugin switches ({FILE}) do not read, so every job waits until they do"
                )
            }
            _ if self.off.contains(&plugin) => format!("the {plugin} plugin is off on /settings"),
            Some(c) => format!("the {plugin} plugin's {c}s are off on /settings"),
            None => format!("the {plugin} plugin is off on /settings"),
        })
    }

    pub fn on(&self, plugin: &str, capability: Option<&str>) -> bool {
        !(self.unread && capability == Some(JOB))
            && !self.off.contains(plugin)
            && capability.is_none_or(|c| !self.off.contains(&format!("{plugin}/{c}")))
    }

    /// A pane shows: its plugin and its pane on, and its parent's too, a sub-entry living under it (/code-review: a
    /// sub-entry whose parent was off served on with no entry leading to it).
    pub fn shows(&self, pane: &Pane) -> bool {
        let parent = pane
            .under
            .and_then(|h| inventory::iter::<Pane>.into_iter().find(|p| p.href == h));
        self.on(&pane.plugin(), Some(PANE)) && parent.is_none_or(|p| self.shows(p))
    }

    /// Every top entry, on or off, in `user`'s order then their own: an href the order lacks (a pane added since) after the
    /// ordered ones.
    fn ranked(&self, user: &str) -> Vec<&'static Pane> {
        let mut top: Vec<&Pane> = (inventory::iter::<Pane>.into_iter())
            .filter(|p| p.under.is_none())
            .collect();
        top.sort_by_key(|p| (p.order, p.href));
        let order = self.order.get(user).map(Vec::as_slice).unwrap_or_default();
        top.sort_by_key(|p| order.iter().position(|h| h == p.href).unwrap_or(usize::MAX));
        top
    }

    /// The menu `user` sees: the top entries that show, in the user's order, each with its sub-entries that show, in theirs.
    pub fn menu(&self, user: &str) -> Vec<(&'static Pane, Vec<&'static Pane>)> {
        let mut subs: Vec<&Pane> = (inventory::iter::<Pane>.into_iter())
            .filter(|p| p.under.is_some() && self.shows(p))
            .collect();
        subs.sort_by_key(|p| (p.order, p.href));
        let top = self.ranked(user).into_iter().filter(|p| self.shows(p));
        top.map(|t| {
            (
                t,
                subs.iter()
                    .copied()
                    .filter(|p| p.under == Some(t.href))
                    .collect(),
            )
        })
        .collect()
    }
}

/// A write through a host that keeps no switches (`Host::switched`).
pub const UNKEPT: &str = "this host keeps no switches";

/// The switches as the host holds them: its View's rows when it holds views (a page's, caught up with every commit), else
/// plugins.sqlite's table read now; none kept is every plugin on. A read that fails stops every job and leaves the rest on,
/// said: a broken file never hides the app, and never starts a job turned off. ponytail: a host whose views do not follow
/// plugins.sqlite reads every plugin on; each host's views follow it once the CLI asks serve (section 5 point 9).
pub fn read(host: &dyn Host) -> Switches {
    if host.switched().is_none() {
        return Switches::default();
    }
    let rows = match host.view() {
        Ok(view) => (view.rows(VIEW)).map(|rows| {
            rows.into_iter()
                .map(|(_, id, value, _)| (id, value))
                .collect()
        }),
        Err(_) => table(host),
    };
    rows.map(Switches::of).unwrap_or_else(|e| {
        eprintln!("the plugin switches do not read, so every job stops: {e}");
        Switches {
            unread: true,
            ..Switches::default()
        }
    })
}

/// The switches plugins.sqlite holds now, read from its table, never a View: what a move reads before it writes.
pub fn kept(host: &dyn Host) -> Result<Switches, String> {
    host.switched().ok_or(UNKEPT)?;
    table(host).map(Switches::of)
}

/// The table's rows, each its id and value.
fn table(host: &dyn Host) -> Result<Vec<(String, String)>, String> {
    let e = |e: dash_sdk::rusqlite::Error| e.to_string();
    let db = host.open(FILE, &[SCHEMA])?;
    let mut read = db
        .prepare("SELECT id, value FROM switch WHERE value IS NOT NULL")
        .map_err(e)?;
    let rows = read
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(e)?;
    rows.collect::<Result<_, _>>().map_err(e)
}

/// Whose order the menu takes: the host's user, by their home folder's name.
pub fn user(host: &dyn Host) -> String {
    let home = host.home().ok();
    let name = home
        .as_deref()
        .and_then(|h| h.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    name.unwrap_or_else(|| "me".into())
}

/// One row written in one transaction, its changeset the View's next diff (`Host::transact`).
fn write(host: &dyn Host, row: String, value: String) -> Result<(), String> {
    host.switched().ok_or(UNKEPT)?;
    let db = host.open(FILE, &[SCHEMA])?;
    let upsert = "INSERT INTO switch (id, value) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET value = excluded.value";
    host.transact(&db, &mut |db| {
        db.execute(upsert, [&row, &value])
            .map(drop)
            .map_err(|e| e.to_string())
    })
}

/// Turns `plugin` on or off, whole or one capability. Settings itself stays on: its page turns the others back on.
pub fn switch(
    host: &dyn Host,
    plugin: &str,
    capability: Option<&str>,
    on: bool,
) -> Result<(), String> {
    if plugin == SETTINGS {
        return Err("settings stays on: its page turns the others back on".into());
    }
    let key = capability.map_or(plugin.to_string(), |c| format!("{plugin}/{c}"));
    write(
        host,
        format!("on/{key}"),
        if on { "on" } else { "off" }.into(),
    )
}

/// What a settings control writes, by its key (the catalog's switch and button, drawn from the Elements under
/// `settings/`): `<plugin>` or `<plugin>/<capability>` turned on or off, `up:<href>` and `down:<href>` a menu entry moved.
pub fn set(host: &dyn Host, key: &str, on: bool) -> Result<(), String> {
    if let Some(href) = key.strip_prefix("up:") {
        return moved(host, href, true);
    }
    if let Some(href) = key.strip_prefix("down:") {
        return moved(host, href, false);
    }
    let (plugin, capability) = match key.split_once('/') {
        Some((plugin, capability)) => (plugin, Some(capability)),
        None => (key, None),
    };
    switch(host, plugin, capability, on)
}

/// The user's menu now.
pub fn menu(host: &dyn Host) -> Vec<(&'static Pane, Vec<&'static Pane>)> {
    read(host).menu(&user(host))
}

/// Moves the top entry at `href` past the next one shown, up or down, in the user's order, kept for that user. The order
/// keeps every top entry, an off one too, so a pane turned on again comes back where it was (/code-review).
pub fn moved(host: &dyn Host, href: &str, up: bool) -> Result<(), String> {
    // read from the table, never a View, which may lag a move made a moment ago elsewhere
    let switches = kept(host)?;
    let user = user(host);
    let ranked = switches.ranked(&user);
    let mut hrefs: Vec<&str> = ranked.iter().map(|p| p.href).collect();
    let shown: Vec<usize> = (ranked.iter().enumerate())
        .filter(|(_, p)| switches.shows(p))
        .map(|(i, _)| i)
        .collect();
    let at = (shown.iter())
        .position(|&i| hrefs[i] == href)
        .ok_or(format!("{href} is no entry of the menu"))?;
    let to = if up { at.checked_sub(1) } else { Some(at + 1) };
    if let Some(&to) = to.and_then(|to| shown.get(to)) {
        hrefs.swap(shown[at], to);
    }
    write(host, format!("order/{user}"), hrefs.join("\n"))
}

/// A plugin's settings and status, its own Elements under `settings/` and `status/` (ARCHITECTURE.md section 4 point 7),
/// registered from the module that holds its jobs or tools; `pane!` registers them beside a pane.
#[macro_export]
macro_rules! settings {
    () => {
        $crate::settings! { "" }
    };
    ($name:literal) => {
        ::inventory::submit! { $crate::Element { origin: ::core::module_path!(), key: ::core::concat!("settings/", ::core::module_path!(), "/", $name), draw: $crate::switches::draw_settings } }
        ::inventory::submit! { $crate::Element { origin: ::core::module_path!(), key: ::core::concat!("status/", ::core::module_path!(), "/", $name), draw: $crate::switches::draw_status } }
    };
}

/// The place a plugin's settings are drawn in, and its status's.
pub const PLACE: &str = "settings";
pub const STATUS: &str = "status";

/// The plugin its props hand an element: its name, whether it is on, the parent it is held off with, each capability with
/// its state, its panes' names.
type Handed<'a> = (
    &'a str,
    bool,
    Option<&'a str>,
    Vec<(&'a str, bool)>,
    Vec<&'a str>,
);

fn handed(props: &serde_json::Value) -> Result<Handed<'_>, String> {
    let name = props["plugin"]
        .as_str()
        .ok_or("no plugin named in its props")?;
    let capabilities = (props["capabilities"].as_array().into_iter().flatten())
        .filter_map(|c| Some((c[0].as_str()?, c[1].as_bool()?)))
        .collect();
    let panes = (props["panes"].as_array().into_iter().flatten())
        .filter_map(|p| p.as_str())
        .collect();
    Ok((
        name,
        props["on"].as_bool().unwrap_or(true),
        props["held_by"].as_str(),
        capabilities,
        panes,
    ))
}

/// A plugin's settings row: its switch, the panes it hides in its words, then a switch a capability, each the catalog's
/// switch keyed as `set` reads it. Settings itself stays on; a plugin held off with its pane's parent waits for it.
pub fn draw_settings(
    _: &dyn Host,
    props: &serde_json::Value,
    id: &str,
    tree: &mut Tree,
) -> Result<Vec<String>, String> {
    let (name, on, held_by, capabilities, panes) = handed(props)?;
    let fixed = name == SETTINGS;
    let label = match panes.as_slice() {
        [] => name.to_string(),
        panes => format!("{name}: {}", panes.join(", ")),
    };
    let whole = format!("plugin-{name}");
    let mut row = vec![whole.clone()];
    let switch =
        json!({"label": label, "key": name, "on": on, "disabled": fixed || held_by.is_some()});
    tree.add(&whole, "switch", switch, &[]);
    if let Some(parent) = held_by {
        let held = format!("{whole}-held");
        tree.add(
            &held,
            "text",
            json!({"text": format!("off with {parent}")}),
            &[],
        );
        row.push(held);
    }
    for (capability, on) in capabilities {
        let at = format!("{whole}-{capability}");
        let key = format!("{name}/{capability}");
        let switch = json!({"label": capability, "key": key, "on": on, "disabled": fixed});
        tree.add(&at, "switch", switch, &[]);
        row.push(at);
    }
    tree.add(id, "bar", json!(null), &row);
    Ok(vec![id.to_string()])
}

/// A plugin's status line: on or off, each capability's state after it.
pub fn draw_status(
    _: &dyn Host,
    props: &serde_json::Value,
    id: &str,
    tree: &mut Tree,
) -> Result<Vec<String>, String> {
    let (name, on, held_by, capabilities, _) = handed(props)?;
    let state = |on: bool| if on { "on" } else { "off" };
    let on = held_by.map_or(state(on).to_string(), |parent| format!("off with {parent}"));
    let caps: String = (capabilities.iter())
        .map(|(c, on)| format!(", {c} {}", state(*on)))
        .collect();
    tree.add(
        id,
        "text",
        json!({"text": format!("{name}: {on}{caps}")}),
        &[],
    );
    Ok(vec![id.to_string()])
}

/// A registry origin's plugin, the one reading of identity (rfc-superapp-plugins section 3): its crate without « dash_ »,
/// « dash_jev::tool » is jev, and an extensions.d file's name without « .json », « jev.json » is jev too (/code-review: a
/// file's origin listed apart from its pane's plugin). The binary wires the plugins, so an entry of its own is its top
/// module's, « dash::mail::x » mail: a feature not yet moved to its crate (ARCHITECTURE.md section 6 B14, phase C), named
/// as that crate will be.
pub fn plugin_of(origin: &str) -> String {
    // an extensions.d file keeps the name its rows were stored under: without « dash- » and « .json », as typed
    if let Some(file) = origin.strip_suffix(".json") {
        return file.strip_prefix("dash-").unwrap_or(file).to_string();
    }
    let mut path = origin.split("::");
    let krate = path.next().unwrap_or_default();
    let name = match (krate, path.next()) {
        ("dash", Some(module)) => module,
        _ => krate,
    };
    name.strip_prefix("dash_").unwrap_or(name).replace('_', "-")
}

#[cfg(test)]
mod tests {
    use dash_meter::dash_test;

    /// A crate's entries are its plugin, the binary's its top module's; an extensions.d file keeps the name its rows were
    /// stored under before plugins had a crate: « dash-foo.json » foo, « my_ext.json » my_ext (the lane's /code-review of
    /// PR 1688).
    #[dash_test]
    fn plugin_of_reads_a_crate_a_module_and_a_file() {
        let cases = [
            ("dash_jev::tool", "jev"),
            ("dash_doc_index", "doc-index"),
            ("dash::mail::x", "mail"),
            ("jev.json", "jev"),
            ("dash-foo.json", "foo"),
            ("my_ext.json", "my_ext"),
        ];
        for (origin, plugin) in cases {
            assert_eq!(super::plugin_of(origin), plugin, "{origin}");
        }
    }
}
