//! Section 11's flat dispatch: a key's exact entry, else the one default, no hierarchy. Each kind of extension point is a
//! `Dispatch`; our crates fill them at link time (`Compiled`), files in extensions.d at start (`Registry::add`).
use std::collections::{BTreeMap, BTreeSet};

/// The key of the default entry. A key like any other: taken twice, it is refused like any other.
pub const DEFAULT_KEY: &str = "*";

pub trait Extract: Send + Sync {
    fn extract(&self, input: &[u8]) -> Result<String, String>;
}

/// An MCP tool. Its schema, arguments and JSON answers are JSON text, so the core takes no JSON library.
pub trait Tool: Send + Sync {
    fn description(&self) -> &str;
    fn input_schema(&self) -> &str;
    fn call(&self, arguments: &str) -> Result<String, String>;
    /// What a call spent or committed to, in the vendor's unit (credits, cents, a month's rate as Hostman's), read from its
    /// arguments and answer: the audit log keeps it, and `dash audit` sums it per tool. `Registry::call` asks it right after
    /// `call`, on the same thread, so a tool may keep it per thread (Hostman's).
    fn cost(&self, _arguments: &str, _answer: &Result<String, String>) -> f64 {
        0.0
    }
    /// A tool whose call changes the world (a send, a write) is asked each time; any other's repeat call is answered from
    /// the cache port (task rust-jev-tool: every tool cached by default, his words in rfc-rust-unified#tools).
    fn effects(&self) -> bool {
        false
    }
    /// The key a call spent on, by its name, never its secret, read from its answer: the budget port charges it.
    fn key(&self, _answer: &Result<String, String>) -> Option<String> {
        None
    }
    /// The arguments of the call that answers its usage left (credits, quota, spend against its cap), which `dash tools
    /// <name>` and /tools send through `Registry::call`, so the audit log keeps it too (service-tool step 3); None: it has none.
    fn usage(&self) -> Option<&str> {
        None
    }
}

/// One tool call as the audit log keeps it (task rust-audit-log; @nha, 2026-11-12: « the audit log shoud be a core
/// functionality imo, not specific to the hostman plugin. », « we should recall what works and what doesn't »).
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub tool: String,
    /// The origin that registered it: a crate, or a plugin's file.
    pub origin: String,
    pub asked: String,
    /// Its answer, or why it failed.
    pub answer: Result<String, String>,
    pub at_ms: i64,
    pub ms: i64,
    pub cost: f64,
}

/// The audit port: where every tool call is kept. The store is its adapter; a tool writes no line of it.
pub trait Audit {
    fn record(&self, call: &Call) -> Result<(), String>;
}

/// Answers already given, by tool and question: a repeat call of a tool with no effects pays nothing.
pub trait Cache {
    fn get(&self, key: &str) -> Option<String>;
    fn put(&self, key: &str, answer: &str) -> Result<(), String>;
}

/// Where each call's numbers go (its adapter sends them on), told whether the cache answered it.
pub trait Telemetry {
    fn call(&self, call: &Call, hit: bool);
}

/// What each key has spent, in its tool's unit: a tool holding several keys reads it to pick one under its cap.
pub trait Budget {
    fn charge(&self, key: &str, cost: f64) -> Result<(), String>;
    fn spent(&self, key: &str) -> Result<f64, String>;
}

/// Every port a call goes through: the clock (a function, so this module needs nothing above it), then where it is kept,
/// answered again, counted and charged.
pub struct Ports<'a> {
    pub now_ms: &'a dyn Fn() -> i64,
    pub audit: &'a dyn Audit,
    pub cache: &'a dyn Cache,
    pub telemetry: &'a dyn Telemetry,
    pub budget: &'a dyn Budget,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    /// The start of its body, as the store bounds it.
    pub body: String,
}

pub trait Rerank: Send + Sync {
    /// The ids in their new order. An id left out keeps its place behind the named ones: a hook orders, it cannot hide.
    fn rerank(&self, query: &str, hits: &[Hit]) -> Result<Vec<String>, String>;
}

pub use dash_sdk::{Line, Slot};

/// `inventory::submit!` this from a crate: the `defmethod` analogue. An entry serving several keys names each one.
pub struct Compiled {
    pub origin: &'static str,
    pub keys: &'static [&'static str],
    pub extractor: fn() -> Box<dyn Extract>,
}

inventory::collect!(Compiled);

/// When an automation runs: each day from a local `HH:MM` (a day the Mac slept through runs at its next pass), or every n minutes.
pub enum Every {
    Daily(&'static str),
    Minutes(i64),
}

/// What a run does: this binary's `dash` command (`Automation::args`), or a brief its task runs.
pub enum Does {
    Command(&'static [&'static str]),
    Brief(&'static str),
}

/// A recurring prompt or command the app's automations run (rfc-orchestration 8.2), shown on /watchers?tab=automations with its
/// runs. His own are the binary's; a plugin registers its own with `inventory::submit!` (the forge's reap, task
/// reap-agent-worktrees).
pub struct Automation {
    /// Its key, and the id of its one task.
    pub key: &'static str,
    /// Its plugin's name, whose job switch on /settings stops it (task rust-plugin-settings).
    pub plugin: &'static str,
    pub title: &'static str,
    pub why: &'static str,
    pub done_when: &'static str,
    pub epic: &'static str,
    pub every: Every,
    pub does: Does,
    /// The .mcp.json servers its task's session needs (`:run :mcp`).
    pub mcp: &'static [&'static str],
    /// A pause file in the run folder it also obeys, `YYYY-MM-DDTHH:MM` compared as text: the job-search pause
    /// (Dashboard.JobPause) for the morning run.
    pub pause_file: Option<&'static str>,
    /// Why it runs now rather than at its time, read each minute's pass: the disk short for the reap. None: at its time.
    pub urgent: Option<fn() -> Option<String>>,
}

inventory::collect!(Automation);

/// An urgent run's spacing: a short disk that a reap does not free reaps every 10 minutes, never every minute.
pub const URGENT_MS: i64 = 10 * 60_000;

impl Automation {
    /// Why it runs at `now_ms`, before its time: its `urgent` reason, once `URGENT_MS` passed since its last run `last_ms`.
    pub fn urgent_at(&self, last_ms: Option<i64>, now_ms: i64) -> Option<String> {
        // the spacing first: its reason (a `df` for the reap) is read only when it could run
        let spaced = last_ms.is_none_or(|at| now_ms - at >= URGENT_MS);
        spaced.then(|| self.urgent?()).flatten()
    }

    /// A command's words as the binary takes them, `<words[0]> --data <dir> <the rest>`, the shape every data command takes.
    pub fn args<'a>(words: &[&'a str], dir: &'a str) -> Vec<&'a str> {
        let (first, rest) = words
            .split_first()
            .map_or((None, &[][..]), |(f, r)| (Some(*f), r));
        (first.into_iter().chain(["--data", dir]))
            .chain(rest.iter().copied())
            .collect()
    }
}

/// A key's entry and where it came from, so what runs for a key is read, never deduced.
pub struct Dispatch<E: ?Sized>(BTreeMap<String, (String, Box<E>)>);

impl<E: ?Sized> Default for Dispatch<E> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<E: ?Sized> Dispatch<E> {
    pub fn get(&self, key: &str) -> Option<&E> {
        self.0.get(key).map(|(_, entry)| &**entry)
    }

    /// `key`'s entry while its plugin is not off whole on /settings (task rust-plugin-settings; /code-review: an off plugin's
    /// trees and lines still served).
    pub fn get_on(&self, key: &str) -> Option<&E> {
        let (origin, entry) = self.0.get(key)?;
        runs(origin).then_some(&**entry)
    }

    /// In key order, the order a chain of hooks runs in.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, &E)> {
        self.0
            .iter()
            .map(|(key, (origin, entry))| (key.as_str(), origin.as_str(), &**entry))
    }

    /// One origin's entries of one kind, refused whole when a key is taken: a dispatch outside `Registry`, as the binary's
    /// pages (task rust-registry-no-hubs), keeps its rule.
    pub fn add(
        &mut self,
        kind: &str,
        origin: &str,
        adding: Vec<(String, Box<E>)>,
    ) -> Result<(), String> {
        self.refuse_taken(kind, &adding)?;
        self.insert(origin, adding);
        Ok(())
    }

    fn refuse_taken(&self, kind: &str, adding: &[(String, Box<E>)]) -> Result<(), String> {
        for (i, (key, _)) in adding.iter().enumerate() {
            if let Some((origin, _)) = self.0.get(key) {
                return Err(format!("{kind} {key} is taken by {origin}"));
            }
            if adding[..i].iter().any(|(earlier, _)| earlier == key) {
                return Err(format!("{kind} {key} is registered twice in this file"));
            }
        }
        Ok(())
    }

    fn insert(&mut self, origin: &str, adding: Vec<(String, Box<E>)>) {
        for (key, entry) in adding {
            self.0.insert(key, (origin.to_string(), entry));
        }
    }
}

/// What one file or crate adds: it lands whole or not at all.
#[derive(Default)]
pub struct Contribution {
    pub extractors: Vec<(String, Box<dyn Extract>)>,
    pub tools: Vec<(String, Box<dyn Tool>)>,
    /// A slot's UI tree of catalog elements, as JSON text (section 12: a screen is data).
    pub ui: Vec<(String, Box<str>)>,
    pub reranks: Vec<(String, Box<dyn Rerank>)>,
    pub slots: Vec<(String, Box<dyn Slot>)>,
}

#[derive(Default)]
pub struct Registry {
    pub extractors: Dispatch<dyn Extract>,
    pub tools: Dispatch<dyn Tool>,
    pub ui: Dispatch<str>,
    pub reranks: Dispatch<dyn Rerank>,
    pub slots: Dispatch<dyn Slot>,
}

impl Registry {
    /// Every crate's entries that the binary links.
    pub fn with_compiled() -> Result<Self, String> {
        let mut registry = Self::default();
        for compiled in inventory::iter::<Compiled> {
            let extractors = compiled
                .keys
                .iter()
                .map(|key| (key.to_string(), (compiled.extractor)()))
                .collect();
            registry.add(
                compiled.origin,
                Contribution {
                    extractors,
                    ..Default::default()
                },
            )?;
        }
        Ok(registry)
    }

    /// A key already taken refuses the whole contribution, so the first file by name order keeps it (section 11).
    pub fn add(&mut self, origin: &str, c: Contribution) -> Result<(), String> {
        self.extractors.refuse_taken("extractor", &c.extractors)?;
        self.tools.refuse_taken("tool", &c.tools)?;
        self.ui.refuse_taken("ui", &c.ui)?;
        self.reranks.refuse_taken("rerank", &c.reranks)?;
        self.slots.refuse_taken("slot", &c.slots)?;
        // every origin whose entries the switches gate is listed, a slot's or a tree's alone too (/code-review); extractors
        // run whatever the switches say, so they list nothing
        if !(c.tools.is_empty() && c.ui.is_empty() && c.reranks.is_empty() && c.slots.is_empty()) {
            seen(origin, !c.tools.is_empty());
        }
        self.extractors.insert(origin, c.extractors);
        self.tools.insert(origin, c.tools);
        self.ui.insert(origin, c.ui);
        self.reranks.insert(origin, c.reranks);
        self.slots.insert(origin, c.slots);
        Ok(())
    }

    /// Each line registered under `page`, in key order, with its origin: one that fails is its error, so the page names it
    /// and still shows the others.
    pub fn slot_lines(&self, page: &str) -> Vec<(&str, Line)> {
        let prefix = format!("{page}/");
        self.slots
            .iter()
            .filter(|(key, origin, _)| key.starts_with(&prefix) && runs(origin))
            .map(|(_, origin, slot)| (origin, slot.line()))
            .collect()
    }

    /// The one way a tool is called, whatever registered it: a repeat question to a tool with no effects is the cache's
    /// answer, and every call, a hit too, is timed, counted, charged to its key and kept. None: no such tool. The second
    /// result is the ports' own failures, which never hide the answer.
    pub fn call(
        &self,
        name: &str,
        arguments: &str,
        ports: &Ports,
    ) -> Option<(Result<String, String>, Result<(), String>)> {
        let (_, (origin, tool)) = self.tools.0.get_key_value(name)?;
        // an off plugin's tools stop (task rust-plugin-settings): refused before the tool runs, named, and kept like any
        // call (/code-review: a refusal returned before the audit left no trace of what was asked)
        let on = tool_on(origin);
        let cached = format!("{name}\n{arguments}");
        let at_ms = (ports.now_ms)();
        let hit = (on && !tool.effects())
            .then(|| ports.cache.get(&cached))
            .flatten();
        let fresh = hit.is_none();
        let answer = if on {
            hit.map_or_else(|| tool.call(arguments), Ok)
        } else {
            let plugin = crate::switches::plugin_of(origin);
            Err(format!(
                "{name}: the {plugin} plugin's tools are off on /settings"
            ))
        };
        // the tool's time alone, before its cost is read (/code-review)
        let ms = (ports.now_ms)() - at_ms;
        let mut failed = vec![];
        if let (true, false, Ok(a)) = (fresh, tool.effects(), &answer) {
            failed.extend(ports.cache.put(&cached, a).err());
        }
        let call = Call {
            tool: name.to_string(),
            origin: origin.clone(),
            asked: arguments.to_string(),
            cost: if fresh && on {
                tool.cost(arguments, &answer)
            } else {
                0.0
            },
            ms,
            at_ms,
            answer,
        };
        ports.telemetry.call(&call, !fresh);
        if let Some(key) = tool.key(&call.answer) {
            failed.extend(ports.budget.charge(&key, call.cost).err());
        }
        failed.extend(ports.audit.record(&call).err());
        let recorded = if failed.is_empty() {
            Ok(())
        } else {
            Err(failed.join("; "))
        };
        Some((call.answer, recorded))
    }

    pub fn extract(&self, key: &str, input: &[u8]) -> Result<String, String> {
        self.extractors
            .get(key)
            .or_else(|| self.extractors.get(DEFAULT_KEY))
            .ok_or(format!("no extractor for {key} and no default"))?
            .extract(input)
    }

    /// Each hook in key order. A hook that fails or answers an id it was not given is left out and named: search
    /// still answers, in the order before it.
    pub fn rerank(&self, query: &str, mut hits: Vec<Hit>) -> (Vec<Hit>, Vec<String>) {
        let mut errors = vec![];
        for (key, origin, hook) in self.reranks.iter().filter(|(_, o, _)| runs(o)) {
            match hook
                .rerank(query, &hits)
                .and_then(|ids| reorder(&hits, &ids))
            {
                Ok(reordered) => hits = reordered,
                Err(e) => errors.push(format!("rerank {key} ({origin}): {e}")),
            }
        }
        (hits, errors)
    }

    /// Every key with its entry's origin, one a line.
    pub fn listing(&self) -> Vec<String> {
        fn lines<E: ?Sized>(kind: &str, d: &Dispatch<E>) -> Vec<String> {
            d.iter()
                .map(|(key, origin, _)| format!("{kind} {key} {origin}"))
                .collect()
        }
        [
            lines("extractor", &self.extractors),
            lines("rerank", &self.reranks),
            lines("slot", &self.slots),
            lines("tool", &self.tools),
            lines("ui", &self.ui),
        ]
        .concat()
    }
}

/// Whether an entry from `origin` runs: its plugin is not off whole on /settings. ponytail: extractors are left out, no
/// plugin with a row on /settings registers one; filter `extract` too when one does.
fn runs(origin: &str) -> bool {
    crate::switches::now().on(crate::switches::plugin_of(origin), None)
}

/// Whether a tool from `origin` answers: its plugin and its tools on. tools/list and `call` both ask it, so an off tool is
/// neither offered nor run (/code-review: tools/list still offered it).
pub fn tool_on(origin: &str) -> bool {
    crate::switches::now().on(
        crate::switches::plugin_of(origin),
        Some(crate::switches::TOOL),
    )
}

/// The registry's plugins, each with whether it registered a tool, so /settings lists it and its tool switch. ponytail: only
/// added to, so an extensions.d file removed while the server runs stays listed until a restart; key it by registry build
/// once files come and go at run time.
static ORIGINS: std::sync::Mutex<BTreeMap<String, bool>> = std::sync::Mutex::new(BTreeMap::new());

/// The registry noted `origin`'s entries, tools among them or not.
fn seen(origin: &str, tools: bool) {
    if let Ok(mut origins) = ORIGINS.lock() {
        *origins
            .entry(crate::switches::plugin_of(origin).to_string())
            .or_default() |= tools;
    }
}

/// Why job `a` does not run: its plugin or its jobs are off on /settings, said where its runs are read (the Rust tools' rule 5:
/// a watcher that stops says so); none: it runs.
pub fn off(a: &Automation) -> Option<String> {
    jobs_off(a.plugin)
}

/// Why `plugin`'s jobs, an `Automation` or a `Recurring` one, do not run: it or its jobs off on /settings, or the switches
/// unread, each named (/code-review: an unread file blamed /settings, where every switch showed on); none: they run.
pub fn jobs_off(plugin: &str) -> Option<String> {
    let switches = crate::switches::now();
    match switches.on(plugin, Some(crate::switches::JOB)) {
        true => None,
        false if switches.unread => Some(format!(
            "off: the plugin switches ({}) do not read, so every job waits until they do",
            crate::switches::FILE
        )),
        false => Some(format!(
            "off: the {plugin} plugin's jobs are off on /settings"
        )),
    }
}

/// Whether `plugin`'s jobs run.
pub fn jobs_on(plugin: &str) -> bool {
    jobs_off(plugin).is_none()
}

/// A plugin as /settings lists it: its name, whether it is on, and each capability it has with its own switch.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub name: String,
    pub on: bool,
    /// The plugin whose pane its pane sits under, while that one hides it: off with it, its own switch moot meanwhile
    /// (/code-review: a sub-entry said on while its page answered 404).
    pub held_by: Option<&'static str>,
    pub capabilities: Vec<(&'static str, bool)>,
}

/// Every plugin this process knows, by name: its pane, the tools the registry took from it, its jobs.
pub fn plugins() -> Vec<Plugin> {
    let switches = crate::switches::now();
    let mut found: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    let mut held: BTreeMap<&'static str, &'static str> = BTreeMap::new();
    for pane in inventory::iter::<crate::switches::Pane> {
        let parent = (pane.under).and_then(|h| {
            inventory::iter::<crate::switches::Pane>
                .into_iter()
                .find(|p| p.href == h)
        });
        if let Some(parent) = parent.filter(|p| !switches.shows(p)) {
            held.insert(pane.plugin(), parent.plugin());
        }
        found
            .entry(pane.plugin().into())
            .or_default()
            .insert(crate::switches::PANE);
    }
    for (origin, tools) in ORIGINS.lock().map(|t| t.clone()).unwrap_or_default() {
        let caps = found.entry(origin).or_default();
        if tools {
            caps.insert(crate::switches::TOOL);
        }
    }
    let recurring = inventory::iter::<dash_sdk::Recurring>
        .into_iter()
        .map(|r| r.plugin);
    for plugin in inventory::iter::<Automation>
        .into_iter()
        .map(|a| a.plugin)
        .chain(recurring)
    {
        found
            .entry(plugin.into())
            .or_default()
            .insert(crate::switches::JOB);
    }
    found
        .into_iter()
        .map(|(name, capabilities)| Plugin {
            held_by: held.get(name.as_str()).copied(),
            on: switches.on(&name, None) && !held.contains_key(name.as_str()),
            capabilities: capabilities
                .into_iter()
                // its own switch, whatever the plugin's: a plugin on again finds each capability as it was
                .map(|c| (c, !switches.off.contains(&format!("{name}/{c}"))))
                .collect(),
            name,
        })
        .collect()
}

/// Ports that keep, cache, count and charge nothing: a server whose calls another app keeps (a worktree's surface).
pub struct Untracked;

impl Audit for Untracked {
    fn record(&self, _call: &Call) -> Result<(), String> {
        Ok(())
    }
}

impl Cache for Untracked {
    fn get(&self, _key: &str) -> Option<String> {
        None
    }
    fn put(&self, _key: &str, _answer: &str) -> Result<(), String> {
        Ok(())
    }
}

impl Telemetry for Untracked {
    fn call(&self, _call: &Call, _hit: bool) {}
}

impl Budget for Untracked {
    fn charge(&self, _key: &str, _cost: f64) -> Result<(), String> {
        Ok(())
    }
    fn spent(&self, _key: &str) -> Result<f64, String> {
        Ok(0.0)
    }
}

impl<'a> Ports<'a> {
    /// The clock alone: `Untracked` for every other port.
    pub fn untracked(now_ms: &'a dyn Fn() -> i64) -> Self {
        Ports {
            now_ms,
            audit: &Untracked,
            cache: &Untracked,
            telemetry: &Untracked,
            budget: &Untracked,
        }
    }
}

/// Every port in memory, a test double: the calls kept with whether the cache answered, the cache, each key's spend. `full`
/// fails each record, as a full disk would.
#[cfg(any(test, feature = "memory-log"))]
#[derive(Default)]
pub struct Memory {
    pub calls: std::sync::Mutex<Vec<(Call, bool)>>,
    pub kept: std::sync::Mutex<Vec<Call>>,
    pub cache: std::sync::Mutex<BTreeMap<String, String>>,
    pub spent: std::sync::Mutex<BTreeMap<String, f64>>,
    pub full: bool,
}

#[cfg(any(test, feature = "memory-log"))]
impl Memory {
    /// The ports on this memory, the clock given.
    pub fn ports<'a>(&'a self, now_ms: &'a dyn Fn() -> i64) -> Ports<'a> {
        Ports {
            now_ms,
            audit: self,
            cache: self,
            telemetry: self,
            budget: self,
        }
    }
}

#[cfg(any(test, feature = "memory-log"))]
impl Audit for Memory {
    fn record(&self, call: &Call) -> Result<(), String> {
        if self.full {
            return Err("disk full".into());
        }
        self.kept.lock().unwrap().push(call.clone());
        Ok(())
    }
}

#[cfg(any(test, feature = "memory-log"))]
impl Cache for Memory {
    fn get(&self, key: &str) -> Option<String> {
        self.cache.lock().unwrap().get(key).cloned()
    }
    fn put(&self, key: &str, answer: &str) -> Result<(), String> {
        self.cache.lock().unwrap().insert(key.into(), answer.into());
        Ok(())
    }
}

#[cfg(any(test, feature = "memory-log"))]
impl Telemetry for Memory {
    fn call(&self, call: &Call, hit: bool) {
        self.calls.lock().unwrap().push((call.clone(), hit));
    }
}

#[cfg(any(test, feature = "memory-log"))]
impl Budget for Memory {
    fn charge(&self, key: &str, cost: f64) -> Result<(), String> {
        *self.spent.lock().unwrap().entry(key.into()).or_default() += cost;
        Ok(())
    }
    fn spent(&self, key: &str) -> Result<f64, String> {
        Ok(self
            .spent
            .lock()
            .unwrap()
            .get(key)
            .copied()
            .unwrap_or_default())
    }
}

fn reorder(hits: &[Hit], ids: &[String]) -> Result<Vec<Hit>, String> {
    let mut named = Vec::with_capacity(hits.len());
    for (i, id) in ids.iter().enumerate() {
        if ids[..i].contains(id) {
            return Err(format!("it answered {id} twice"));
        }
        let hit = hits.iter().find(|h| &h.id == id);
        named.push(
            hit.ok_or(format!("it answered {id}, not one of its hits"))?
                .clone(),
        );
    }
    let rest = hits.iter().filter(|h| !ids.contains(&h.id)).cloned();
    named.extend(rest);
    Ok(named)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    struct Says(&'static str);

    impl Extract for Says {
        fn extract(&self, _input: &[u8]) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    fn says(key: &str, what: &'static str) -> Contribution {
        Contribution {
            extractors: vec![(key.to_string(), Box::new(Says(what)))],
            ..Default::default()
        }
    }

    #[dash_test]
    fn dispatch_is_flat_the_exact_key_else_the_one_default() {
        let mut r = Registry::default();
        assert!(r.extract("image/png", b"").is_err(), "no default yet");
        r.add("a", says("image/*", "wildcard is only a key"))
            .unwrap();
        r.add("b", says("image/png", "exact")).unwrap();
        assert!(r.extract("image/heic", b"").is_err(), "no hierarchy");
        r.add("c", says(DEFAULT_KEY, "default")).unwrap();
        assert_eq!(r.extract("image/png", b"").unwrap(), "exact");
        assert_eq!(r.extract("image/heic", b"").unwrap(), "default");
        assert_eq!(
            r.listing(),
            [
                "extractor * c",
                "extractor image/* a",
                "extractor image/png b"
            ]
        );
    }

    #[dash_test]
    fn a_key_taken_twice_refuses_the_later_contribution_whole() {
        let mut r = Registry::default();
        r.add("10-first", says("text/x", "first")).unwrap();
        let mut later = says("text/new", "later");
        later
            .extractors
            .push(("text/x".into(), Box::new(Says("later"))));
        let e = r.add("20-later", later).expect_err("refused");
        assert!(e.contains("text/x") && e.contains("10-first"), "{e}");
        assert_eq!(r.extract("text/x", b"").unwrap(), "first");
        assert_eq!(r.listing(), ["extractor text/x 10-first"], "nothing landed");

        let mut twice = says("text/y", "one");
        twice
            .extractors
            .push(("text/y".into(), Box::new(Says("two"))));
        assert!(r.add("30-twice", twice).is_err());
    }

    /// A dispatch outside the registry, as the binary's pages, keeps the rule: a taken key refuses the whole addition.
    #[dash_test]
    fn a_dispatch_of_its_own_refuses_a_taken_key_whole() {
        let mut d: Dispatch<&str> = Dispatch::default();
        d.add("page", "a", vec![("/x".into(), Box::new("first"))])
            .unwrap();
        let later = vec![
            ("/y".into(), Box::new("y")),
            ("/x".into(), Box::new("later")),
        ];
        let e = d.add("page", "b", later).expect_err("refused");
        assert!(e.contains("page /x is taken by a"), "{e}");
        assert_eq!(d.get("/x"), Some(&"first"));
        assert!(d.get("/y").is_none(), "nothing landed");
    }

    struct Counts(&'static str, usize);

    impl Slot for Counts {
        fn line(&self) -> Line {
            Line {
                says: Ok(Some(format!("{} {}", self.1, self.0))),
                what: String::new(),
                href: "/".into(),
            }
        }
    }

    #[dash_test]
    fn a_page_reads_the_slots_under_its_name_alone() {
        let mut r = Registry::default();
        let slot = |key: &str, what| Contribution {
            slots: vec![(key.to_string(), Box::new(Counts(what, 2)) as Box<dyn Slot>)],
            ..Default::default()
        };
        r.add("b", slot("today/b", "bees")).unwrap();
        r.add("a", slot("today/a", "ants")).unwrap();
        r.add("c", slot("todayish/c", "cats")).unwrap();
        assert!(r.add("d", slot("today/a", "again")).is_err(), "a key once");
        let says: Vec<(&str, String)> = r
            .slot_lines("today")
            .into_iter()
            .map(|(origin, line)| (origin, line.says.unwrap().unwrap()))
            .collect();
        assert_eq!(says, [("a", "2 ants".into()), ("b", "2 bees".into())]);
        assert!(r.listing().contains(&"slot today/a a".to_string()));
    }

    /// A tool that knows nothing of the log: it answers, or fails, and names a cost.
    struct Plain;

    impl Tool for Plain {
        fn description(&self) -> &str {
            "plain"
        }
        fn input_schema(&self) -> &str {
            "{}"
        }
        fn call(&self, arguments: &str) -> Result<String, String> {
            match arguments {
                "ok" => Ok("done".into()),
                _ => Err("refused".into()),
            }
        }
        fn cost(&self, _arguments: &str, answer: &Result<String, String>) -> f64 {
            if answer.is_ok() { 2.5 } else { 0.0 }
        }
    }

    /// The plain tool registered by a plugin, on a clock whose each read is 40 ms after the last.
    fn plain() -> (Registry, impl Fn() -> i64) {
        let mut r = Registry::default();
        let plain = Contribution {
            tools: vec![("plain".into(), Box::new(Plain) as Box<dyn Tool>)],
            ..Default::default()
        };
        r.add("20-plugin", plain).unwrap();
        let tick = std::cell::Cell::new(1_000);
        let clock = move || {
            tick.set(tick.get() + 40);
            tick.get()
        };
        (r, clock)
    }

    /// Task rust-audit-log, part 1: the port keeps each call, its tool, origin, question, answer or error, time and cost,
    /// with no line in the tool; a record that fails never hides the tool's answer.
    #[dash_test]
    fn the_tool_port_records_every_call_with_no_line_in_the_tool() {
        let (r, clock) = plain();
        let kept = Memory::default();
        let ports = kept.ports(&clock);
        assert!(r.call("nope", "ok", &ports).is_none(), "no tool");
        let (ok, recorded) = r.call("plain", "ok", &ports).unwrap();
        assert_eq!((ok, recorded), (Ok("done".into()), Ok(())));
        let (failed, _) = r.call("plain", "x", &ports).unwrap();
        assert_eq!(failed, Err("refused".into()));
        let call = |asked: &str, answer: Result<&str, &str>, at_ms, cost| Call {
            tool: "plain".into(),
            origin: "20-plugin".into(),
            asked: asked.into(),
            answer: answer.map(str::to_string).map_err(str::to_string),
            at_ms,
            ms: 40,
            cost,
        };
        assert_eq!(
            kept.kept.into_inner().unwrap(),
            [
                call("ok", Ok("done"), 1_040, 2.5),
                call("x", Err("refused"), 1_120, 0.0)
            ]
        );
        let full = Memory {
            full: true,
            ..Default::default()
        };
        let (answer, recorded) = r.call("plain", "ok", &full.ports(&clock)).unwrap();
        assert_eq!(
            (answer, recorded),
            (Ok("done".into()), Err("disk full".into()))
        );
    }

    /// A tool that counts its calls, answers each one anew, and names the key it spent on.
    struct Counted(std::sync::atomic::AtomicU32, bool);

    impl Tool for Counted {
        fn description(&self) -> &str {
            "counted"
        }
        fn input_schema(&self) -> &str {
            "{}"
        }
        fn call(&self, arguments: &str) -> Result<String, String> {
            let n = 1 + self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(format!("{arguments} #{n}"))
        }
        fn cost(&self, _arguments: &str, _answer: &Result<String, String>) -> f64 {
            1.5
        }
        fn effects(&self) -> bool {
            self.1
        }
        fn key(&self, _answer: &Result<String, String>) -> Option<String> {
            Some("k1".into())
        }
    }

    fn counted(effects: bool) -> Registry {
        let mut r = Registry::default();
        let tool = Box::new(Counted(Default::default(), effects)) as Box<dyn Tool>;
        let c = Contribution {
            tools: vec![("t".into(), tool)],
            ..Default::default()
        };
        r.add("30-plugin", c).unwrap();
        r
    }

    /// Done-when 2: a repeat question is the cache port's answer, the tool not asked again; another question is asked; a
    /// tool with effects is asked every time and nothing of it is cached.
    #[dash_test]
    fn a_repeat_call_is_the_cache_s_answer_a_tool_with_effects_is_asked_each_time() {
        let clock = || 0;
        let (r, memory) = (counted(false), Memory::default());
        let ports = memory.ports(&clock);
        let answers: Vec<_> = ["a", "a", "b", "a"]
            .map(|q| r.call("t", q, &ports).unwrap().0.unwrap())
            .into();
        assert_eq!(answers, ["a #1", "a #1", "b #2", "a #1"]);
        let (r, memory) = (counted(true), Memory::default());
        let ports = memory.ports(&clock);
        let answers: Vec<_> = ["a", "a"]
            .map(|q| r.call("t", q, &ports).unwrap().0.unwrap())
            .into();
        assert_eq!(answers, ["a #1", "a #2"]);
        assert!(
            memory.cache.lock().unwrap().is_empty(),
            "nothing of it cached"
        );
    }

    /// Two calls of one question, the second the cache's, on one memory.
    fn twice() -> Memory {
        let (r, memory, clock) = (counted(false), Memory::default(), || 7);
        for _ in 0..2 {
            assert_eq!(r.call("t", "q", &memory.ports(&clock)).unwrap().1, Ok(()));
        }
        memory
    }

    /// Done-when 3, telemetry: each call reaches it, the hit told apart, at no cost.
    #[dash_test]
    fn every_call_a_hit_too_reaches_telemetry() {
        let calls = twice().calls.into_inner().unwrap();
        let seen: Vec<_> = calls
            .iter()
            .map(|(c, hit)| (c.tool.as_str(), c.cost, *hit))
            .collect();
        assert_eq!(seen, [("t", 1.5, false), ("t", 0.0, true)]);
    }

    /// Done-when 3, audit log: the hit is kept too, its answer the cached one.
    #[dash_test]
    fn every_call_a_hit_too_is_kept_in_the_audit_log() {
        let kept = twice().kept.into_inner().unwrap();
        let answers: Vec<_> = kept.iter().map(|c| (c.answer.clone(), c.cost)).collect();
        assert_eq!(
            answers,
            [(Ok("q #1".into()), 1.5), (Ok("q #1".into()), 0.0)]
        );
    }

    /// Done-when 3, key budget: each call is charged to the key its answer names, the hit at nothing.
    #[dash_test]
    fn every_call_a_hit_too_is_charged_to_its_key() {
        let memory = twice();
        assert_eq!(memory.spent("k1"), Ok(1.5));
        assert_eq!(
            memory.kept.lock().unwrap().len(),
            2,
            "both calls reached the ports"
        );
    }

    struct Answers(Result<Vec<String>, String>);

    impl Rerank for Answers {
        fn rerank(&self, _query: &str, _hits: &[Hit]) -> Result<Vec<String>, String> {
            self.0.clone()
        }
    }

    fn hook(key: &str, answer: Result<Vec<&str>, &str>) -> Contribution {
        let answer = answer
            .map(|ids| ids.iter().map(|s| s.to_string()).collect())
            .map_err(str::to_string);
        Contribution {
            reranks: vec![(key.to_string(), Box::new(Answers(answer)))],
            ..Default::default()
        }
    }

    fn hits(ids: &[&str]) -> Vec<Hit> {
        ids.iter()
            .map(|id| Hit {
                id: id.to_string(),
                title: String::new(),
                body: String::new(),
            })
            .collect()
    }

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    #[dash_test]
    fn a_hook_orders_hits_but_cannot_hide_or_invent_them() {
        let mut r = Registry::default();
        r.add("x", hook("10-last-first", Ok(vec!["c"]))).unwrap();
        let (got, errors) = r.rerank("q", hits(&["a", "b", "c"]));
        assert_eq!((ids(&got), errors), (vec!["c", "a", "b"], vec![]));

        for (key, bad) in [
            ("20-invents", Ok(vec!["nope"])),
            ("30-twice", Ok(vec!["a", "a"])),
            ("40-fails", Err("boom")),
        ] {
            let mut r = Registry::default();
            r.add("y", hook(key, bad)).unwrap();
            let (got, errors) = r.rerank("q", hits(&["a", "b"]));
            assert_eq!(ids(&got), ["a", "b"], "{key} left the order alone");
            assert!(
                errors[0].contains(key) && errors[0].contains('y'),
                "{errors:?}"
            );
        }
    }

    /// Task reap-agent-worktrees, done-when 3's rule: an automation runs before its time once its reason holds and
    /// `URGENT_MS` passed since its last run; its command runs as the binary's `<word> --data <dir> <the rest>`.
    #[dash_test]
    fn an_urgent_reason_runs_it_early_spaced_and_its_command_takes_the_data_folder() {
        // how often the reason was read: never while the spacing holds it (a `df` each minute's pass, /code-review of c92b8d419)
        static READ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn short() -> Option<String> {
            READ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some("19.9 GB free, under 30 GB".into())
        }
        let reads = || READ.load(std::sync::atomic::Ordering::SeqCst);
        let reap = Automation {
            key: "reap",
            plugin: "forge",
            title: "",
            why: "",
            done_when: "",
            epic: "",
            every: Every::Minutes(60),
            does: Does::Command(&["orch", "--reap"]),
            mcp: &[],
            pause_file: None,
            urgent: Some(short),
        };
        let why = Some("19.9 GB free, under 30 GB".to_string());
        assert_eq!(reap.urgent_at(None, 0), why, "never ran: at once");
        assert_eq!(reap.urgent_at(Some(0), URGENT_MS - 1), None, "spaced");
        assert_eq!(reads(), 1, "the spacing held it: its reason unread");
        assert_eq!(reap.urgent_at(Some(0), URGENT_MS), why);
        let room = Automation {
            urgent: None,
            ..reap
        };
        assert_eq!(room.urgent_at(None, 0), None, "no reason: at its time");
        assert_eq!(
            Automation::args(&["orch", "--reap"], "d"),
            ["orch", "--data", "d", "--reap"]
        );
        assert_eq!(
            Automation::args(&["alerts"], "d"),
            ["alerts", "--data", "d"]
        );
    }
}
