//! Which plugins run and how the menu orders them (task rust-plugin-settings; @nha, 2026-11-14: « Can settings enable/disable
//! plugins ? », « Can settings reorder plugins in the menu ? »): rfc-superapp-plugins section 1, « A plugin can be turned on
//! or off, whole or capability by capability ». Each pane registers its menu entry beside its page (`pane!`), so the sidebar
//! lists the registry's entries, never a list of its own. A plugin's name is its pane's path (« /mail »: mail) or its
//! registry origin without « dash- » or « .json » (« dash-jev », « jev.json »: jev); its capabilities are its pane, its tools and its jobs. What is off
//! and each user's order are the plugin runtime's own database, plugins.sqlite in the data folder, opened through the host:
//! the core reads no file of its own.
use dash_sdk::{FieldWrite, Host, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, TryLockError};

/// A plugin's entry in the menu, registered beside its page with `pane!`.
#[derive(Debug)]
pub struct Pane {
    pub name: &'static str,
    /// The page it opens; its plugin's name is this path's.
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
    pub fn plugin(&self) -> &'static str {
        self.href.trim_start_matches('/')
    }

    /// `path` is its href, one of its paths, or a page under one: /tasks/<id> is Tasks'.
    pub fn holds(&self, path: &str) -> bool {
        let under = |p: &str| path == p || path.strip_prefix(p).is_some_and(|r| r.starts_with('/'));
        under(self.href) || self.paths.iter().any(|p| under(p))
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
const TABLE: &str = "switch";
const FIELD: &str = "value";

/// What is off, `<plugin>` or `<plugin>/<capability>`, and each user's order of the menu's top entries, by href.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Switches {
    pub off: BTreeSet<String>,
    pub order: BTreeMap<String, Vec<String>>,
    /// The database did not read and no read came before: every job stops, the rest stays on (/code-review).
    pub unread: bool,
}

impl Switches {
    /// A row's last value: `on/<key>` « on » or « off », `order/<user>` the hrefs a line each.
    fn take(&mut self, row: &str, value: &Value) {
        let Value::Text(v) = value else { return };
        if let Some(key) = row.strip_prefix("on/") {
            match v.as_str() {
                "off" => self.off.insert(key.to_string()),
                _ => self.off.remove(key),
            };
        } else if let Some(user) = row.strip_prefix("order/") {
            let hrefs = v.lines().map(String::from).collect();
            self.order.insert(user.to_string(), hrefs);
        }
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
        self.on(pane.plugin(), Some(PANE)) && parent.is_none_or(|p| self.shows(p))
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

    /// Whether `path`'s page serves under these switches (`serves`).
    pub fn serves(&self, path: &str) -> bool {
        let mut holding = (inventory::iter::<Pane>.into_iter())
            .filter(|p| p.holds(path))
            .peekable();
        if holding.peek().is_none() {
            let first = path.trim_start_matches('/').split('/').next();
            return self.on(first.unwrap_or_default(), None);
        }
        holding.all(|p| self.shows(p))
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

/// Held by the one thread reading the database again: the others serve the last read meanwhile, so a busy file stalls one
/// request, never every one (/code-review: one slow request never stalls the others).
static READING: Mutex<()> = Mutex::new(());
/// The switches as last read, when, and at which write: read again past `FRESH_MS`, so a switch flipped by another process
/// (/settings in `dash serve`, the jobs in `dash orch`, the tools in `dash mcp`) holds there within it.
/// ponytail: one read per process, whichever folder it was read in: main builds one host and nextest runs each test in a
/// process of its own, so two folders never meet (/code-review of PR 1667). A per-host read once a test holds two hosts.
static READ: RwLock<Option<(i64, Arc<Switches>)>> = RwLock::new(None);
/// Bumped by each write: a read begun before it is never kept over it (/code-review).
static WRITES: AtomicU64 = AtomicU64::new(0);
/// A write through a host that keeps no switches (`Host::switched`).
pub const UNKEPT: &str = "this host keeps no switches";
/// How long a read serves before the database is read again.
pub const FRESH_MS: i64 = 5_000;

/// The switches held in the host's data folder.
pub fn kept(host: &dyn Host) -> Result<Switches, String> {
    kept_in(host, &host.data())
}

/// The switches held in `dir`: none kept is every plugin on. ponytail: the whole log replayed at
/// each read past `FRESH_MS`, a row a flip or a move, a few hundred in a lifetime; read the `switch` table's rows instead
/// once `OpLog` reads rows.
pub fn kept_in(host: &dyn Host, dir: &Path) -> Result<Switches, String> {
    let log = host.log(dir, FILE, &[SCHEMA])?;
    let mut switches = Switches::default();
    for op in log.ops_after(None, i64::MAX)? {
        switches.take(&op.write.row, &op.write.value);
    }
    Ok(switches)
}

/// The switches now, read again past `FRESH_MS`. A read that fails keeps the last one that did (/code-review: a busy or
/// broken file turned every off job and tool back on); with none before it, every job stops and the rest stays on, said: a
/// broken file never hides the app, and never starts a job turned off. ponytail: tools stay on there, as a sandboxed
/// session may read no live folder at all; stop them too once a read-only open of the database exists.
pub fn now(host: &dyn Host) -> Arc<Switches> {
    let Some(dir) = host.switched() else {
        return Arc::default();
    };
    let at = host.now_ms();
    let fresh = || {
        let held = READ.read().ok().and_then(|r| r.clone());
        match &held {
            Some((read_at, read)) if (0..FRESH_MS).contains(&(at - read_at)) => {
                Err(Arc::clone(read))
            }
            _ => Ok(held),
        }
    };
    let mut held = match fresh() {
        Err(read) => return read,
        Ok(held) => held,
    };
    let _reading = match READING.try_lock() {
        Ok(reading) => reading,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => match &held {
            Some((_, read)) => return Arc::clone(read),
            // nothing read yet: wait for the read under way, then take it
            None => {
                let reading = READING.lock().unwrap_or_else(|p| p.into_inner());
                // what the other read left is the last read now: a failed read here keeps it (/code-review: the
                // recheck's read was dropped, so a busy file stored « unread » over the good read just made)
                match fresh() {
                    Err(read) => return read,
                    Ok(left) => held = left,
                }
                reading
            }
        },
    };
    let writes = WRITES.load(Ordering::SeqCst);
    let read = kept_in(host, &dir).map(Arc::new).unwrap_or_else(|e| {
        eprintln!(
            "the plugin switches do not read, the last read kept, else every job stopped: {e}"
        );
        held.map_or_else(
            || {
                Arc::new(Switches {
                    unread: true,
                    ..Switches::default()
                })
            },
            |(_, last)| last,
        )
    });
    if let Ok(mut held) = READ.write()
        && WRITES.load(Ordering::SeqCst) == writes
    {
        *held = Some((at, Arc::clone(&read)));
    }
    read
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

fn write(host: &dyn Host, row: String, value: String) -> Result<(), String> {
    let dir = host.switched().ok_or(UNKEPT)?;
    let mut log = host.log(&dir, FILE, &[SCHEMA])?;
    let write = FieldWrite {
        table: TABLE.into(),
        row,
        field: FIELD.into(),
        value: Value::Text(value),
    };
    log.write(&[write])?;
    // read again at the next ask, from what the database now holds; a read begun before this one is not kept
    WRITES.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut held) = READ.write() {
        *held = None;
    }
    Ok(())
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

/// The user's menu now.
pub fn menu(host: &dyn Host) -> Vec<(&'static Pane, Vec<&'static Pane>)> {
    now(host).menu(&user(host))
}

/// Moves the top entry at `href` past the next one shown, up or down, in the user's order, kept for that user. The order
/// keeps every top entry, an off one too, so a pane turned on again comes back where it was (/code-review).
pub fn moved(host: &dyn Host, href: &str, up: bool) -> Result<(), String> {
    // read from the database, never the held read, which may predate another process's move (/code-review: a move made
    // within FRESH_MS elsewhere was written over)
    let dir = host.switched().ok_or(UNKEPT)?;
    let switches = kept_in(host, &dir)?;
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

/// A registry origin's plugin: « dash-jev » is jev, and so is extensions.d's « jev.json » (/code-review: a file's origin
/// listed apart from its pane's plugin).
pub fn plugin_of(origin: &str) -> &str {
    let name = origin.strip_prefix("dash-").unwrap_or(origin);
    name.strip_suffix(".json").unwrap_or(name)
}

/// Why `path`'s page does not serve, its plugin off on /settings (the front door's 404, never sent on to the older app);
/// none: it serves.
pub fn stopped(host: &dyn Host, path: &str) -> Option<String> {
    (!serves(host, path)).then(|| format!("off: {path}: its plugin is off on /settings\n"))
}

/// Whether `path`'s page serves: each pane holding it shows; held by none, the plugin its first segment names is on, a
/// plugin with no pane (drafts) holding the paths under its name (/code-review: /drafts served with drafts off).
pub fn serves(host: &dyn Host, path: &str) -> bool {
    now(host).serves(path)
}
