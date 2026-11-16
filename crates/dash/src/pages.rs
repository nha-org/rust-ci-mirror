//! Every page registers its route from its own module, `inventory::submit! { Page { .. } }` beside its drawing, and the server
//! builds its route table from those registrations: a new page edits no hub, so two features never meet in main.rs (task
//! rust-registry-no-hubs, rfc-rust-unified section 11's flat dispatch; @nha, 2026-11-09: « is it an orchestration failure or
//! is it because the architecture is not decoupled enough yet ? »). dash-gate's `hub_named_pages` refuses a page path in
//! main.rs, or one tasks.rs names that another module registers. This module reads no page module, so each can read it.
use dash_core::Answer;
use dash_surface as surface;
use std::path::PathBuf;
use std::sync::OnceLock;

// the SDK's page types, the core's since task rust-move-core-ports: the binary's modules still name them here
pub use dash_core::{Asked, HTML, JSON, Page, Reads, TEXT, page as find, param, segment_decoded};

pub(crate) use dash_process::repo;

/// The data folder of the running tool, outside git: what `dash serve --port <n>` reads, and `make rust/serve` names.
pub fn live_data() -> PathBuf {
    repo().join("private/live/rust/data")
}

pub type Response = (&'static str, &'static str, String);

/// The binary's host (ARCHITECTURE.md section 3): the clock, the checkout, the data folder and each plugin's own database,
/// the one place a page or a plugin's command reads them. Built once, in `main`, from the process's flags, and handed to
/// each command and page: no static holds it or a folder of its (task untangle-one-host; ARCHITECTURE.md section 6, « the
/// `Host` handed in over a static »).
#[derive(Clone)]
pub struct Binary {
    /// The folder `--data` names; none: the tool's live data, resolved at its first read and kept (`dash help` reads no
    /// checkout; a request reads it many times).
    data: Option<PathBuf>,
    live: OnceLock<PathBuf>,
    /// Whether the plugin switches are read from that folder: a command run on a data folder, the server, the window; a
    /// worktree's scratch server names the live folder only to copy from it, so it keeps none (/code-review: hosting opened
    /// plugins.sqlite read-write in the live folder).
    switched: bool,
}

/// A test's hosts: built as `main` builds the process's one.
#[cfg(test)]
mod testing {
    impl super::Binary {
        /// A test's host on `dir`, no switches kept: every plugin on.
        pub fn on(dir: &std::path::Path) -> &'static Self {
            Self::leaked(Some(dir.to_path_buf()), false)
        }

        /// A test's host as `dash serve` has it on `dir`, its switches kept there.
        pub fn served(dir: &std::path::Path) -> &'static Self {
            Self::leaked(Some(dir.to_path_buf()), true)
        }
    }
}

impl Binary {
    pub fn new(data: Option<PathBuf>, switched: bool) -> Self {
        Self {
            data,
            live: OnceLock::new(),
            switched,
        }
    }

    /// The same host for the whole process: `main`'s one, and a test's.
    pub fn leaked(data: Option<PathBuf>, switched: bool) -> &'static Self {
        Box::leak(Box::new(Self::new(data, switched)))
    }

    /// `--data`'s folder, else the tool's live data.
    pub fn folder(&self) -> PathBuf {
        let live = || self.live.get_or_init(live_data);
        self.data.as_ref().unwrap_or_else(live).clone()
    }

    /// Where the plugin switches are kept (`Host::switched`).
    pub fn switches(&self) -> Option<PathBuf> {
        self.switched.then(|| self.folder())
    }
}

/// The mail server's transport as `who`, which `dash serve` sets from /mail's: the host's `signed_in` (task
/// draft-from-is-address), kept here since the host's module takes no page's.
pub static SIGN_IN: OnceLock<dash_core::SignIn> = OnceLock::new();

/// A page's answer as the server sends it: a drawing in the form asked (`?view=data` its own tree as JSON, no order handed
/// in), a body as it is.
pub fn answered(a: &Asked, answer: Answer) -> Response {
    match answer {
        Answer::Body(status, kind, body) => (status, kind, body),
        Answer::Drawn(d) => {
            let data = view_named(a.query) == Some("data");
            let d = if data {
                d
            } else {
                surface::queried(d, a.query)
            };
            drawn_for(Some(a.host), a.html, data, Some(Ok(d)))
        }
    }
}

pub use dash_store::tasks::{args_of, form_decoded};

pub use dash_core::form::{encoded, form_of, form_parts, parts, view_named, wants_html};

/// A page's one drawing as HTML or as its text, `html` asking which: the one place a page's form is picked (task
/// rust-page-forms; @nha, 2026-11-08: « this will be a rule for every page. /mail tasks tools status etc. »). `None`: no board.
pub fn drawn(html: bool, data: bool, read: Option<Result<surface::Drawing, String>>) -> Response {
    drawn_for(None, html, data, read)
}

/// `drawn`, an element that reads the host's switches (/settings' plugins) drawn from `host`'s: a served page's.
pub fn drawn_for(
    host: Option<&dyn dash_core::Host>,
    html: bool,
    data: bool,
    read: Option<Result<surface::Drawing, String>>,
) -> Response {
    use surface::Drawing::{Page, Tree};
    let pretty = |tree: &str| {
        serde_json::from_str::<serde_json::Value>(tree)
            .and_then(|v| serde_json::to_string_pretty(&v))
    };
    let body = |d: surface::Drawing| match d {
        Tree { title, tree } if html => {
            surface::screen::page_for(host, &title, &tree).map(|b| (HTML, b))
        }
        // the view for a program (@nha, 2026-11-09: « if in the browser ideally pretty json »): the tree it is drawn from
        Tree { tree, .. } if data => pretty(&tree).map(|b| (JSON, b)).map_err(|e| e.to_string()),
        Tree { tree, .. } => surface::text_for(host, &tree).map(|b| (TEXT, b)),
        Page(page) if html => Ok((HTML, page)),
        Page(page) => Ok((TEXT, surface::forms::text_of(&page))),
    };
    match read.map(|read| read.and_then(body)) {
        Some(Ok((kind, body))) => ("200 OK", kind, body),
        Some(Err(e)) => ("500 Internal Server Error", TEXT, e),
        None => ("404 Not Found", TEXT, "no port, no page".into()),
    }
}

/// The local time now, as `YYYY-MM-DDTHH:MM`, the hook's `date +%Y-%m-%dT%H:%M`.
pub(crate) fn local_now() -> String {
    let mut now = dash_store::older_handoff::local_iso(dash_process::now_s() as i64);
    now.truncate(16);
    now
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 1 of rust-registry-no-hubs: the route table is the registrations, each path once, and each page is found by
    /// its path, its step the one route_step names; nothing under a page's own path is another's.
    #[dash_test]
    fn the_route_table_is_built_from_each_module_s_registration() {
        let registered: Vec<&Page> = inventory::iter::<Page>.into_iter().collect();
        // the 19 this PR moved out of main.rs, a floor: a new page raises it with no edit here
        assert!(registered.len() >= 19, "{} pages", registered.len());
        let mut paths: Vec<&str> = registered.iter().map(|p| p.path).collect();
        paths.sort_unstable();
        let before = paths.len();
        paths.dedup();
        assert_eq!(paths.len(), before, "a path registered twice: {paths:?}");
        for page in registered {
            let asked = page.path.replace('*', "an-id");
            let found = find(crate::pages::Binary::on("/data".as_ref()), &asked)
                .unwrap_or_else(|| panic!("{asked} not found"));
            assert_eq!(found.path, page.path);
            let line = format!("GET {asked}?view=text HTTP/1.1");
            assert_eq!(
                crate::route_step(crate::pages::Binary::on("/data".as_ref()), &line),
                page.step,
                "{line}"
            );
        }
        assert!(
            find(crate::pages::Binary::on("/data".as_ref()), "/no-such-page").is_none()
                && find(crate::pages::Binary::on("/data".as_ref()), "/tasks/a/b").is_none()
        );
        assert_eq!(
            find(crate::pages::Binary::on("/data".as_ref()), "/").map(|p| p.step),
            Some("route_page")
        );
    }

    /// Done-when 1 of untangle-one-host: the host holds the data folder, as `main` built it from the flags, and a page
    /// reads it there: the folder `--data` names, else the live one; its switches kept in a named folder, never a scratch
    /// server's (/code-review: hosting opened plugins.sqlite read-write in the live folder).
    #[dash_test]
    fn the_host_holds_the_data_folder_a_page_reads() {
        use dash_core::Host;
        let dir = dash_meter::TestDir::new("pages-data");
        // nextest runs each test in its own process (rust/CLAUDE.md rule 1): nothing else reads DASH_REPO
        unsafe { std::env::set_var("DASH_REPO", dir.0.join("repo")) };
        let asked = |host: &'static Binary| Asked {
            path: "/settings",
            query: "",
            html: false,
            held: None,
            host,
        };
        // none named: DASH_REPO's live data folder, no switches kept
        let live = asked(Binary::leaked(None, false)).host;
        assert_eq!(live.data(), dir.0.join("repo/private/live/rust/data"));
        assert_eq!(live.switched(), None);
        // named: that folder, its switches there
        let named = asked(Binary::served(&dir.0)).host;
        assert_eq!(
            (named.data(), named.switched()),
            (dir.0.clone(), Some(dir.0.clone()))
        );
        // a scratch server names the live folder only to copy from it
        assert_eq!(Binary::on(&dir.0).switched(), None);
    }

    /// Done-when 1 of rust-tests-no-live-repo: under nextest, an unset DASH_REPO panics naming the test, never the live
    /// checkout.
    #[dash_test]
    fn an_unset_dash_repo_under_nextest_panics_naming_the_test() {
        let test =
            std::env::var("NEXTEST_TEST_NAME").expect("run under nextest (rust/CLAUDE.md rule 1)");
        // its own process: nothing else reads DASH_REPO, and the expected panic prints nothing
        unsafe { std::env::remove_var("DASH_REPO") };
        std::panic::set_hook(Box::new(|_| {}));
        let said = std::panic::catch_unwind(repo).expect_err("a panic, never ~/repo/site_perso");
        let said = said.downcast_ref::<String>().expect("its words");
        assert!(
            said.contains(&test) && said.contains("DASH_REPO is unset"),
            "{said}"
        );
        // outside nextest (dash itself), the default stays ~/repo/site_perso: here a scratch HOME
        let home = dash_meter::TestDir::new("pages-home");
        unsafe {
            std::env::remove_var("NEXTEST");
            std::env::set_var("HOME", &home.0);
        }
        assert_eq!(repo(), home.0.join("repo/site_perso"));
    }

    /// Done-when 1 of rust-plugin-settings: the sidebar renders the registry's entries, each pane registered beside its page
    /// (`dash_core::pane!`), the older app's two with no page of ours, Design by the design system itself; the menu's const
    /// list is gone from the design system.
    #[dash_test]
    fn the_sidebar_renders_the_registry_s_panes_each_registered_with_its_page() {
        let menu = dash_core::switches::menu(crate::pages::Binary::on("/data".as_ref()));
        let named: Vec<(&str, Vec<&str>)> = (menu.iter())
            .map(|(p, subs)| (p.name, subs.iter().map(|s| s.name).collect()))
            .collect();
        let expected = [
            ("Inbox", vec![]),
            ("Today", vec![]),
            ("Mail", vec![]),
            ("Tasks", vec![]),
            ("Projects", vec![]),
            ("Plan", vec![]),
            ("Documents", vec![]),
            ("Insights", vec![]),
            ("Agents", vec!["Comments"]),
            ("Tools", vec!["Watchers"]),
            ("Status", vec![]),
            ("Design", vec!["Code"]),
            ("Settings", vec![]),
        ];
        assert_eq!(named, expected, "the panes, in their own order");
        for (pane, subs) in &menu {
            let icon = surface::forms::named_icon(pane.icon);
            assert!(
                icon.is_some(),
                "{}: no icon is named {}",
                pane.name,
                pane.icon
            );
            let ours = !matches!(pane.href, "/projects" | "/status");
            assert_eq!(
                find(crate::pages::Binary::on("/data".as_ref()), pane.href).is_some(),
                ours,
                "{}: its page registered with it",
                pane.name
            );
            assert!(
                subs.iter()
                    .all(|s| find(crate::pages::Binary::on("/data".as_ref()), s.href).is_some()),
                "{}'s sub-entries",
                pane.name
            );
        }
        // the sidebar as drawn, read back as a document: its links in order, each its words and target
        let (_, drawn) = surface::parity::parts(&surface::forms::with_menu(
            crate::pages::Binary::on("/data".as_ref()),
            "",
            "/mail",
            "",
        ));
        let links: Vec<(String, String)> = (drawn.into_iter())
            .filter_map(|item| Some((item.label, item.href?)))
            .collect();
        let wanted: Vec<(String, String)> = (menu.iter())
            .flat_map(|(pane, subs)| std::iter::once(*pane).chain(subs.iter().copied()))
            .map(|p| (p.name.to_string(), p.href.to_string()))
            .collect();
        let entries: Vec<(String, String)> = (links.into_iter())
            .filter(|(_, href)| wanted.iter().any(|(_, h)| h == href))
            .collect();
        assert_eq!(
            entries, wanted,
            "each pane's link, its name, in the menu's order"
        );
        let forms = include_str!("../../surface/src/forms.rs");
        assert!(!forms.contains("const MENU:"), "no list of the menu's own");
        // the design system's tests link no binary and keep a copy each, held here to the real panes (/code-review: a pane's
        // order changed here left both copies passing on the old one); Design is the design system's own, in neither
        let copies = [
            ("forms.rs", forms),
            (
                "menu_panes.rs",
                include_str!("../../surface/tests/common/menu_panes.rs"),
            ),
        ];
        for (file, copy) in copies {
            let copy: String = copy.split_whitespace().collect();
            for pane in dash_core::switches::menu(crate::pages::Binary::on("/data".as_ref()))
                .iter()
                .flat_map(|(p, subs)| std::iter::once(*p).chain(subs.iter().copied()))
            {
                if pane.href == "/design" {
                    continue;
                }
                let paths: Vec<String> = pane.paths.iter().map(|p| format!("{p:?}")).collect();
                let under = pane.under.map_or("None".into(), |u| format!("Some({u:?})"));
                let entry = format!(
                    "({:?},{:?},{},{:?},[{}],{under})",
                    pane.name,
                    pane.href,
                    pane.order,
                    pane.icon,
                    paths.join(",")
                );
                assert!(
                    copy.contains(&entry),
                    "{file}: no {entry}, the binary's pane"
                );
            }
        }
    }
}
