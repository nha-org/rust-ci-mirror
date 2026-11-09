//! Section 11's flat dispatch: a key's exact entry, else the one default, no hierarchy. Each kind of extension point is a
//! `Dispatch`; our crates fill them at link time (`Compiled`), files in extensions.d at start (`Registry::add`).
use std::collections::BTreeMap;

/// The key of the default entry. A key like any other: taken twice, it is refused like any other.
pub const DEFAULT_KEY: &str = "*";

pub trait Extract {
    fn extract(&self, input: &[u8]) -> Result<String, String>;
}

/// An MCP tool. Its schema, arguments and JSON answers are JSON text, so the core takes no JSON library.
pub trait Tool {
    fn description(&self) -> &str;
    fn input_schema(&self) -> &str;
    fn call(&self, arguments: &str) -> Result<String, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    /// The start of its body, as the store bounds it.
    pub body: String,
}

pub trait Rerank {
    /// The ids in their new order. An id left out keeps its place behind the named ones: a hook orders, it cannot hide.
    fn rerank(&self, query: &str, hits: &[Hit]) -> Result<Vec<String>, String>;
}

/// A slot's line: a part of another page, as a line in the daily's « today » (rfc-rust-plugins-ui section 3, the Slot port).
/// What it counts and where it opens are known before its count, so a line still reading is its card in its loading state,
/// never a line of text a count replaces (task rust-today-doc; @nha, 2026-11-09: « This is a card loading state. same element. »).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// What it says, its count first: « 3 to send ». `Ok(None)`: its first read has not ended, so no count yet, never a zero;
    /// `Err`: why it has none.
    pub says: Result<Option<String>, String>,
    /// What it counts, in a few words.
    pub what: String,
    /// The list it opens.
    pub href: String,
}

/// A plugin's part of another page, registered under `<page>/<order>-<plugin>` (« today/10-drafts »): the page reads every
/// key under its name in key order, never a plugin by name.
pub trait Slot {
    fn line(&self) -> Line;
}

/// `inventory::submit!` this from a crate: the `defmethod` analogue. An entry serving several keys names each one.
pub struct Compiled {
    pub origin: &'static str,
    pub keys: &'static [&'static str],
    pub extractor: fn() -> Box<dyn Extract>,
}

inventory::collect!(Compiled);

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

    /// In key order, the order a chain of hooks runs in.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, &E)> {
        self.0
            .iter()
            .map(|(key, (origin, entry))| (key.as_str(), origin.as_str(), &**entry))
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
            .filter(|(key, _, _)| key.starts_with(&prefix))
            .map(|(_, origin, slot)| (origin, slot.line()))
            .collect()
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
        for (key, origin, hook) in self.reranks.iter() {
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
    use dash_meter::memory::TestBudget;

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

    #[test]
    fn dispatch_is_flat_the_exact_key_else_the_one_default() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_key_taken_twice_refuses_the_later_contribution_whole() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_page_reads_the_slots_under_its_name_alone() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_hook_orders_hits_but_cannot_hide_or_invent_them() {
        let _budget = TestBudget::new();
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
}
