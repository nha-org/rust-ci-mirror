//! The host's side of the plugin API (ARCHITECTURE.md section 3): the pages the host serves with the off
//! filter, and what only its own pages read. A plugin takes dash-sdk; the binary fills `Host` and serves the rest.
//! The core reads no clock, environment or file of its own: dash-gate's
//! `what_a_simulation_runs_reads_nothing_a_seed_does_not_decide` holds it.
use crate::registry::Dispatch;
use crate::switches::{Pane, Switches};
use dash_sdk::{Asked, Host};
use std::any::Any;
use std::sync::OnceLock;

/// The board view's columns, first to last: the words its rows' third field holds, and /tasks' statuses.
pub const BOARD: [&str; 4] = ["backlog", "todo", "in-progress", "in-review"];

/// A page's or plugin's warm-up after a restart: `start` begins its reads, handed the host as its page is, `reading` says
/// one still runs (task rust-warmup-queue). Registered beside the page, as its `Page` is; `dash serve` runs each one at a time.
pub struct Warmer {
    pub page: &'static str,
    pub start: fn(&dyn Host),
    pub reading: fn() -> bool,
}

inventory::collect!(Warmer);

pub use dash_sdk::{Page, Reads};

/// A pane and its page, registered together from the page's module, each origin stamped as `submit!` stamps it, its
/// plugin's settings and status Elements beside them (`settings!`):
/// `pane!("Mail", 30, "mail", Page { .. })`, the paths it also stands for in brackets before the page; `pane!("Comments",
/// under "/agents", Page { .. })` a sub-entry, ordered in its parent's by its path; `pane!("Projects", 50, "board",
/// "/projects")` an entry whose page is the older app's, with no page of ours.
#[macro_export]
macro_rules! pane {
    ($name:literal, $order:literal, $icon:literal, $href:literal) => {
        ::inventory::submit! { $crate::switches::Pane { origin: ::core::module_path!(), name: $name, href: $href, order: $order, icon: $icon, paths: &[], under: None } }
        $crate::settings! { $name }
    };
    ($name:literal, under $parent:literal, $($page:ident)::+ { $($field:tt)* }) => {
        const _: () = {
            const PAGE: $crate::plugin::Page = $($page)::+ { origin: ::core::module_path!(), $($field)* };
            ::inventory::submit! { PAGE }
            ::inventory::submit! {
                $crate::switches::Pane { origin: PAGE.origin, name: $name, href: PAGE.path, order: 0, icon: "", paths: &[], under: Some($parent) }
            }
        };
        $crate::settings! { $name }
    };
    ($name:literal, $order:literal, $icon:literal, [$($path:literal),*], $($page:ident)::+ { $($field:tt)* }) => {
        const _: () = {
            const PAGE: $crate::plugin::Page = $($page)::+ { origin: ::core::module_path!(), $($field)* };
            ::inventory::submit! { PAGE }
            ::inventory::submit! {
                $crate::switches::Pane { origin: PAGE.origin, name: $name, href: PAGE.path, order: $order, icon: $icon, paths: &[$($path),*], under: None }
            }
        };
        $crate::settings! { $name }
    };
    ($name:literal, $order:literal, $icon:literal, $($page:ident)::+ { $($field:tt)* }) => {
        $crate::pane! { $name, $order, $icon, [], $($page)::+ { $($field)* } }
    };
}

/// What the page was handed, as its type: the board for a `Reads::Board` page, else the registry; `None`: this process holds none. A
/// held value of another type is the server's bug, said at once: never « no port, no page » for what the page was handed.
/// Host-side, as a generic: the sdk's `Asked` holds it as `dyn Any` (ARCHITECTURE.md section 7 gotcha 2).
pub fn held<'a, T: Any>(asked: &Asked<'a>) -> Option<&'a T> {
    asked.held.map(|h| {
        h.downcast_ref().unwrap_or_else(|| {
            let wanted = std::any::type_name::<T>();
            panic!("{}: what the page was handed is not a {wanted}", asked.path)
        })
    })
}

/// Every registered page by its path, through the core's dispatch: a path taken twice is refused, the first kept and the
/// refusal logged (the registry's rule, section 11).
fn table() -> &'static Dispatch<Page> {
    static TABLE: OnceLock<Dispatch<Page>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Dispatch::default();
        for page in inventory::iter::<Page> {
            let entry = vec![(page.path.to_string(), Box::new(*page))];
            if let Err(e) = table.add("page", page.origin, entry) {
                eprintln!("{e}");
            }
        }
        table
    })
}

/// The page registered at `path`, on or off: its exact key, else its prefix's `/*`. Flat: two lookups, no walk up the path.
pub fn registered(path: &str) -> Option<&'static Page> {
    let table = table();
    table.get(path).or_else(|| {
        let cut = path.rfind('/')?;
        table.get(&format!("{}*", &path[..=cut]))
    })
}

/// The page served at `path`: the one registered there, none while the pane holding it is off on /settings (task
/// rust-plugin-settings): an off plugin's pages stop.
pub fn page(host: &dyn Host, path: &str) -> Option<&'static Page> {
    registered(path).filter(|_| serves(host, path))
}

// what a pane stands for, read here beside the page table: switches.rs reads no path (untangle-plugin-identity)
impl Pane {
    /// `path` is its href, one of its paths, or a page under one: /tasks/<id> is Tasks'.
    pub fn holds(&self, path: &str) -> bool {
        let under = |p: &str| path == p || path.strip_prefix(p).is_some_and(|r| r.starts_with('/'));
        under(self.href) || self.paths.iter().any(|p| under(p))
    }
}

/// Whether `path` serves under `switches`: the plugins of the page and the post registered there are on, and each pane holding it
/// shows, a plugin with no pane (drafts) holding the pages it registered (/code-review: /drafts served with drafts off). A
/// path no entry registers (the older app's) is held by its panes alone: its first segment names no plugin (ARCHITECTURE.md
/// section 6 point 2).
pub fn serves_in(switches: &Switches, path: &str) -> bool {
    // the page's plugin and the post's, either method: a path both register stops with either (/code-review)
    let origins = [
        registered(path).map(|p| p.origin),
        dash_sdk::post(path).map(|p| p.origin),
    ];
    origins
        .into_iter()
        .flatten()
        .all(|o| switches.stops(o, None).is_none())
        && (inventory::iter::<Pane>.into_iter())
            .filter(|p| p.holds(path))
            .all(|p| switches.shows(p))
}

/// Whether `path` serves under the host's switches now.
pub fn serves(host: &dyn Host, path: &str) -> bool {
    serves_in(&crate::switches::read(host), path)
}

/// Why `path`'s page does not serve, its plugin off on /settings (the front door's 404, never sent on to the older app);
/// none: it serves.
pub fn stopped(host: &dyn Host, path: &str) -> Option<String> {
    (!serves(host, path)).then(|| format!("off: {path}: its plugin is off on /settings\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use dash_sdk::StandIn;

    inventory::submit! { Warmer { page: "/a-test-page", start: |_| {}, reading: || false } }
    crate::pane! { "Stamped", 999, "", Page { path: "/a-stamped-page", step: "route_a_stamped_page", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", crate::TEXT, String::new()).into() } }
    crate::submit! { crate::Command::usage("dash a-stamped-command") }
    crate::submit! { crate::Element { key: "test/stamped", draw: |_, _, _, _| Ok(vec![]) } }
    crate::submit! { crate::Post { path: "/a-stamped-post", step: "route_a_stamped_post", serve: |_, _| ("200 OK", crate::TEXT, String::new()).into() } }
    crate::submit! { crate::Recurring { model: "a-stamped-job", every_ms: 60_000, file: "x.sqlite", schema: &[], start: |_| Err("never".into()) } }
    crate::submit! {
        crate::registry::Automation {
            key: "a-stamped-automation", title: "", why: "", done_when: "", epic: "", every: crate::registry::Every::Minutes(60),
            does: crate::registry::Does::Command(&["x"]), mcp: &[], pause_file: None, urgent: None,
        }
    }

    fn asked<'a>(host: &'a StandIn, board: Option<&'a dyn Any>) -> Asked<'a> {
        Asked {
            path: "/tasks/x",
            query: "",
            html: false,
            held: board,
            host,
        }
    }

    /// Done-when 5 of rust-orch-ports: a warmer registers through the SDK, as a page does.
    #[dash_test]
    fn a_warmer_registers_through_the_sdk() {
        let pages: Vec<&str> = inventory::iter::<Warmer>().map(|w| w.page).collect();
        assert!(pages.contains(&"/a-test-page"), "{pages:?}");
    }

    /// Done-when 3 of untangle-plugin-identity (rfc-superapp-plugins section 3: « a plugin's identity is its registry
    /// origin »): each kind the core's macros register, a Page, Command, Pane, Element, Post, job, automation and a
    /// contribution's tools and slots, carries the module that registered it, and its plugin is its crate's, core here.
    #[dash_test]
    fn every_entry_s_plugin_is_its_crate() {
        use crate::switches::{Pane, plugin_of};
        fn one<T: 'static>(origin: impl Fn(&T) -> Option<&'static str>) -> &'static str
        where
            inventory::iter<T>: IntoIterator<Item = &'static T>,
        {
            let mut found = inventory::iter::<T>.into_iter().filter_map(origin);
            found.next().expect("registered")
        }
        let found = [
            one::<Page>(|p| (p.path == "/a-stamped-page").then_some(p.origin)),
            one::<Pane>(|p| (p.href == "/a-stamped-page").then_some(p.origin)),
            one::<crate::Command>(|c| (c.usage == "dash a-stamped-command").then_some(c.origin)),
            one::<crate::Element>(|e| (e.key == "test/stamped").then_some(e.origin)),
            one::<crate::Post>(|p| (p.path == "/a-stamped-post").then_some(p.origin)),
            one::<crate::Recurring>(|r| (r.model == "a-stamped-job").then_some(r.origin)),
            one::<crate::registry::Automation>(|a| {
                (a.key == "a-stamped-automation").then_some(a.origin)
            }),
        ];
        let tools = crate::contribution! { ui: vec![("t".into(), "{}".into())] };
        assert_eq!(
            found,
            [module_path!(); 7],
            "each stamped where it was registered"
        );
        assert_eq!(tools.origin, module_path!());
        assert!(
            found
                .iter()
                .chain([&tools.origin.as_str()])
                .all(|o| plugin_of(o) == "core")
        );
        let mut registry = crate::registry::Registry::default();
        assert!(
            registry
                .add(crate::registry::Contribution::default())
                .is_err(),
            "an origin left out is refused, never an unnamed plugin"
        );
    }

    /// What a page was handed, of another type, is said, never read as nothing: `None` is only a process that holds none.
    #[dash_test]
    fn a_board_of_another_type_fails_loudly() {
        let host = StandIn::default();
        assert!(held::<String>(&asked(&host, None)).is_none());
        let wrong = std::panic::catch_unwind(|| held::<String>(&asked(&host, Some(&7u8))).cloned());
        let said = wrong.expect_err("a panic");
        let said = said.downcast_ref::<String>().expect("its words");
        assert!(said.contains("/tasks") && said.contains("String"), "{said}");
        assert_eq!(held::<u8>(&asked(&host, Some(&7u8))), Some(&7));
    }
}
