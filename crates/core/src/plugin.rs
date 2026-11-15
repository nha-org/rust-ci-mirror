//! The host's side of the plugin API (ARCHITECTURE.md section 3): the pages the host serves with the off
//! filter, and what only its own pages read. A plugin takes dash-sdk; the binary fills `Host` and serves the rest.
//! The core reads no clock, environment or file of its own: dash-gate's
//! `what_a_simulation_runs_reads_nothing_a_seed_does_not_decide` holds it.
use crate::registry::Dispatch;
use dash_sdk::Asked;
use std::any::Any;
use std::sync::OnceLock;

/// The board view's columns, first to last: the words its rows' third field holds, and /tasks' statuses.
pub const BOARD: [&str; 4] = ["backlog", "todo", "in-progress", "in-review"];

/// A page's or plugin's warm-up after a restart: `start` begins its reads, `reading` says one still runs (task
/// rust-warmup-queue). Registered beside the page, as its `Page` is; `dash serve` runs each one at a time.
pub struct Warmer {
    pub page: &'static str,
    pub start: fn(),
    pub reading: fn() -> bool,
}

inventory::collect!(Warmer);

pub use dash_sdk::{Page, Reads};

/// A pane and its page, registered together from the page's module: `pane!("Mail", 30, "mail", Page { .. })`, the paths it
/// also stands for in brackets before the page; `pane!("Comments", under "/agents", Page { .. })` a sub-entry, ordered in
/// its parent's by its path; `pane!("Projects", 50, "board", "/projects")` an entry whose page is the older app's, with no
/// page of ours.
#[macro_export]
macro_rules! pane {
    ($name:literal, $order:literal, $icon:literal, $href:literal) => {
        ::inventory::submit! { $crate::switches::Pane { name: $name, href: $href, order: $order, icon: $icon, paths: &[], under: None } }
    };
    ($name:literal, under $parent:literal, $page:expr) => {
        const _: () = {
            const PAGE: $crate::plugin::Page = $page;
            ::inventory::submit! { PAGE }
            ::inventory::submit! {
                $crate::switches::Pane { name: $name, href: PAGE.path, order: 0, icon: "", paths: &[], under: Some($parent) }
            }
        };
    };
    ($name:literal, $order:literal, $icon:literal, [$($path:literal),*], $page:expr) => {
        const _: () = {
            const PAGE: $crate::plugin::Page = $page;
            ::inventory::submit! { PAGE }
            ::inventory::submit! {
                $crate::switches::Pane { name: $name, href: PAGE.path, order: $order, icon: $icon, paths: &[$($path),*], under: None }
            }
        };
    };
    ($name:literal, $order:literal, $icon:literal, $page:expr) => {
        $crate::pane! { $name, $order, $icon, [], $page }
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
            if let Err(e) = table.add("page", page.step, entry) {
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
pub fn page(path: &str) -> Option<&'static Page> {
    registered(path).filter(|_| crate::switches::serves(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use dash_sdk::StandIn;

    inventory::submit! { Warmer { page: "/a-test-page", start: || {}, reading: || false } }

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
