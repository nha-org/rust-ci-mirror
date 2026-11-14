//! Every page registers its route from its own module, `inventory::submit! { Page { .. } }` beside its drawing, and the server
//! builds its route table from those registrations: a new page edits no hub, so two features never meet in main.rs (task
//! rust-registry-no-hubs, rfc-rust-unified section 11's flat dispatch; @nha, 2026-11-09: « is it an orchestration failure or
//! is it because the architecture is not decoupled enough yet ? »). dash-gate's `hub_named_pages` refuses a page path in
//! main.rs, or one tasks.rs names that another module registers. This module reads no page module, so each can read it.
use dash_core::{Answer, OpLog};
use dash_surface as surface;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// the SDK's page types, the core's since task rust-move-core-ports: the binary's modules still name them here
pub use dash_core::{Asked, HTML, JSON, Page, Reads, TEXT, page as find, param, segment_decoded};

/// The folder `dash serve` keeps the pages' data in: its `--data`. A server concern, held here so no page module reads it
/// through the comments module (task rust-registry-no-hubs, RFC section 3).
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn serve_from(dir: &Path) {
    let _ = DATA_DIR.set(dir.to_path_buf());
}

pub fn serve_from_dir() -> Result<&'static Path, String> {
    DATA_DIR
        .get()
        .map(PathBuf::as_path)
        .ok_or_else(|| "no data folder in this process".to_string())
}

/// The repository root the tool runs against: `DASH_REPO`, or `~/repo/site_perso`. Under nextest (`NEXTEST` set, inherited by
/// the dash a test starts) an unset `DASH_REPO` panics naming the test: a test never reads the live checkout, it gets a
/// scratch one (task rust-tests-no-live-repo; rust-search-port's tests read the live repo and cost a rerun).
pub(crate) fn repo() -> PathBuf {
    if let Some(v) = std::env::var_os("DASH_REPO") {
        return PathBuf::from(v);
    }
    if std::env::var_os("NEXTEST").is_some() {
        let test = std::env::var("NEXTEST_TEST_NAME").unwrap_or_else(|_| "a test".into());
        panic!(
            "{test}: DASH_REPO is unset under nextest: give the test a scratch repo, never the live checkout"
        );
    }
    let home = std::env::var("HOME").unwrap_or_else(|e| {
        eprintln!("HOME: {e}");
        std::process::exit(1);
    });
    PathBuf::from(home).join("repo/site_perso")
}

/// The data folder of the running tool, outside git: what `dash serve --port <n>` reads, and `make rust/serve` names.
pub fn live_data() -> PathBuf {
    repo().join("private/live/rust/data")
}

pub type Response = (&'static str, &'static str, String);

/// The binary's host (ARCHITECTURE.md section 3): the clock, the checkout, the data folder and each plugin's own database,
/// the one place a page or a plugin's command reads them.
pub struct Binary;

pub static HOST: Binary = Binary;

impl dash_core::WallClock for Binary {
    fn now_ms(&self) -> i64 {
        dash_process::now_ms()
    }
}

impl dash_core::Host for Binary {
    fn repo(&self) -> PathBuf {
        repo()
    }
    /// The one set at serve start, or the tool's live data when none was set.
    fn data(&self) -> PathBuf {
        serve_from_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| live_data())
    }
    fn home(&self) -> Result<PathBuf, String> {
        std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|e| format!("HOME: {e}"))
    }
    fn arm(&self, dir: &Path, command: &str) {
        crate::reporting::arm(dir, command);
    }
    fn open(&self, dir: &Path, file: &str, schema: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Ok(Box::new(dash_store::replica(&dir.join(file), schema)?))
    }
}

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
            drawn(a.html, data, Some(Ok(d)))
        }
    }
}

/// `s` percent-encoded for a form, as bin/dash sends it.
pub fn encoded(s: &str) -> String {
    (s.bytes())
        .map(
            |b| match b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                true => (b as char).to_string(),
                false => format!("%{b:02X}"),
            },
        )
        .collect()
}

/// A form bin/dash would send for `args` run in `cwd`.
pub fn form_of(cwd: &std::path::Path, args: &[&str]) -> String {
    let mut form = format!("cwd={}", encoded(&cwd.display().to_string()));
    for a in args {
        form.push_str(&format!("&args%5B%5D={}", encoded(a)));
    }
    form
}

pub use dash_store::tasks::{args_of, form_decoded};

pub use dash_core::form::{form_parts, parts};

/// `?view=`'s one of our three, in any case; any other value is the page's own.
pub fn view_named(query: &str) -> Option<&'static str> {
    match param(query, "view")
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => Some("html"),
        Some("text") => Some("text"),
        Some("data") => Some("data"),
        _ => None,
    }
}

/// A page's form (rust/CLAUDE.md point 4): `?view=html|text` first, in any case, else HTML when the Accept header names it,
/// else the text. Any other view, an empty one included, leaves it to the header.
pub fn wants_html(accept: &str, query: &str) -> bool {
    let view = param(query, "view").map(|v| v.trim().to_lowercase());
    match view.as_deref() {
        Some("html") => true,
        Some("text" | "data") => false,
        _ => accept.contains("text/html"),
    }
}

/// A page's one drawing as HTML or as its text, `html` asking which: the one place a page's form is picked (task
/// rust-page-forms; @nha, 2026-11-08: « this will be a rule for every page. /mail tasks tools status etc. »). `None`: no board.
pub fn drawn(html: bool, data: bool, read: Option<Result<surface::Drawing, String>>) -> Response {
    use surface::Drawing::{Page, Tree};
    let pretty = |tree: &str| {
        serde_json::from_str::<serde_json::Value>(tree)
            .and_then(|v| serde_json::to_string_pretty(&v))
    };
    let body = |d: surface::Drawing| match d {
        Tree { title, tree } if html => surface::screen::page(&title, &tree).map(|b| (HTML, b)),
        // the view for a program (@nha, 2026-11-09: « if in the browser ideally pretty json »): the tree it is drawn from
        Tree { tree, .. } if data => pretty(&tree).map(|b| (JSON, b)).map_err(|e| e.to_string()),
        Tree { tree, .. } => surface::text(&tree).map(|b| (TEXT, b)),
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
    // SAFETY: time only reads the clock.
    let tm = dash_store::older_handoff::local_tm(unsafe { libc::time(std::ptr::null_mut()) });
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// Unix time `t` as this Mac's local time, `YYYY-MM-DDTHH:MM:SS`.
pub(crate) fn local_at(t: i64) -> String {
    let tm = dash_store::older_handoff::local_tm(t as libc::time_t);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// How far his zone is ahead of UTC now, in milliseconds: Paris 3_600_000 in winter.
pub(crate) fn utc_offset_ms() -> i64 {
    // SAFETY: time only reads the clock.
    let tm = dash_store::older_handoff::local_tm(unsafe { libc::time(std::ptr::null_mut()) });
    tm.tm_gmtoff * 1000
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
            let found = find(&asked).unwrap_or_else(|| panic!("{asked} not found"));
            assert_eq!(found.path, page.path);
            let line = format!("GET {asked}?view=text HTTP/1.1");
            assert_eq!(crate::route_step(&line), page.step, "{line}");
        }
        assert!(find("/no-such-page").is_none() && find("/tasks/a/b").is_none());
        assert_eq!(find("/").map(|p| p.step), Some("route_page"));
    }

    /// The data folder set at serve start is the one a page reads, and live data otherwise.
    #[dash_test]
    fn the_data_folder_set_at_serve_start_is_the_one_a_page_reads() {
        fn asked() -> Asked<'static> {
            Asked {
                path: "/settings",
                query: "",
                html: false,
                board: None,
                registry: None,
                host: &HOST,
            }
        }
        let dir = dash_meter::TestDir::new("pages-data");
        // nextest runs each test in its own process (rust/CLAUDE.md rule 1): nothing else reads DASH_REPO
        unsafe { std::env::set_var("DASH_REPO", dir.0.join("repo")) };
        // before serve_from: DASH_REPO's live data folder
        assert_eq!(asked().data(), dir.0.join("repo/private/live/rust/data"));
        serve_from(&dir.0);
        // after: the set folder
        assert_eq!(asked().data(), dir.0);
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
}
