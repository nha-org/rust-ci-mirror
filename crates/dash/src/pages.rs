//! Every page registers its route from its own module, `inventory::submit! { Page { .. } }` beside its drawing, and the server
//! builds its route table from those registrations: a new page edits no hub, so two features never meet in main.rs (task
//! rust-registry-no-hubs, rfc-rust-unified section 11's flat dispatch; @nha, 2026-11-09: « is it an orchestration failure or
//! is it because the architecture is not decoupled enough yet ? »). dash-gate's `hub_named_pages` refuses a page path in
//! main.rs, or one tasks.rs names that another module registers. This module reads no page module, so each can read it.
use dash_core::registry::{Dispatch, Registry};
use dash_surface::{self as surface, Drawing};
use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What a page reads, which says where the server answers it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reads {
    /// Nothing shared: on its connection's thread, no lock taken.
    Nothing,
    /// The board: on its connection's thread, the board's lock held.
    Board,
    /// The registry: on the serial route's thread, one at a time.
    Registry,
}

#[derive(Clone, Copy)]
pub struct Page {
    /// Its path, or `<prefix>/*` for each path one segment under the prefix: `/tasks/*` serves /tasks/<id>.
    pub path: &'static str,
    /// Its metric step, so a client's paths never mint one.
    pub step: &'static str,
    pub reads: Reads,
    /// The window draws it again at each ask, as a page that reads the store; any other once a visit.
    pub redraw: bool,
    pub serve: fn(&Asked) -> Response,
}

inventory::collect!(Page);

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

/// The repository root the tool runs against: `DASH_REPO`, or `~/repo/site_perso`.
pub(crate) fn repo() -> PathBuf {
    match std::env::var("DASH_REPO") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_else(|e| {
                eprintln!("HOME: {e}");
                std::process::exit(1);
            });
            PathBuf::from(home).join("repo/site_perso")
        }
    }
}

/// The data folder of the running tool, outside git: what `dash serve --port <n>` reads, and `make rust/serve` names.
pub fn live_data() -> PathBuf {
    repo().join("private/live/rust/data")
}

/// A page's request: its path and query, the form asked, and what its thread holds.
pub struct Asked<'a> {
    pub path: &'a str,
    pub query: &'a str,
    pub html: bool,
    /// The board, when the page reads it and this process holds one: `tasks::Board`, which `board` reads back, so this module
    /// names no page module.
    pub board: Option<&'a dyn Any>,
    pub registry: Option<&'a Registry>,
}

impl Asked<'_> {
    /// The board as its type; `None`: no board in this process. A board of another type is the server's bug, said at once:
    /// never « no port, no page » for a board the page was handed.
    pub fn board<B: Any>(&self) -> Option<&B> {
        self.board.map(|b| {
            b.downcast_ref().unwrap_or_else(|| {
                let wanted = std::any::type_name::<B>();
                panic!(
                    "{}: the board handed to the page is not a {wanted}",
                    self.path
                )
            })
        })
    }

    pub fn param(&self, name: &str) -> Option<String> {
        param(self.query, name)
    }

    /// The path's last segment decoded, a `/*` page's id. A path keeps its + (only a form's + is a space; /code-review of
    /// #1197).
    pub fn id(&self) -> String {
        segment_decoded(&self.path[self.path.rfind('/').map_or(0, |i| i + 1)..])
    }

    /// The drawing in the form asked; `None`: no board in this process.
    pub fn form(&self, read: Option<Result<Drawing, String>>) -> Response {
        let data = view_named(self.query) == Some("data");
        // the data view is the page's own tree, no order handed in (/code-review)
        let read = read.map(|r| {
            r.map(|d| {
                if data {
                    d
                } else {
                    surface::queried(d, self.query)
                }
            })
        });
        drawn(self.html, data, read)
    }

    pub fn failed(&self, e: String) -> Response {
        ("500 Internal Server Error", TEXT, e)
    }

    /// `HOME`, the logins' folder: no HOME is no login known, said on the page while the server stays up.
    pub fn home(&self) -> Result<std::path::PathBuf, Response> {
        std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .map_err(|e| self.failed(format!("HOME: {e}")))
    }

    /// The server's data folder: the one set at serve start, or the tool's live data when none was set (the same fallback the
    /// page modules used to take from the comments module).
    pub fn data(&self) -> PathBuf {
        serve_from_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| live_data())
    }
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

/// The page at `path`: its exact key, else its prefix's `/*`. Flat: two lookups, no walk up the path.
pub fn find(path: &str) -> Option<&'static Page> {
    let table = table();
    table.get(path).or_else(|| {
        let cut = path.rfind('/')?;
        table.get(&format!("{}*", &path[..=cut]))
    })
}

pub const TEXT: &str = "text/plain; charset=utf-8";
pub const HTML: &str = "text/html; charset=utf-8";
pub const JSON: &str = "application/json; charset=utf-8";

pub type Response = (&'static str, &'static str, String);

/// `+` and `%XX` as a form sends them; a bad escape stays as it is.
pub fn form_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit)) // from_str_radix alone takes a leading +
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
                continue;
            }
            (b'+', _) => out.push(b' '),
            (byte, _) => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One segment of a path decoded: its + stays a plus, as only a form's + is a space (/code-review of #1197).
pub fn segment_decoded(segment: &str) -> String {
    form_decoded(&segment.replace('+', "%2B"))
}

/// The query's first `name=` value, decoded as a form sends it.
pub fn param(query: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&prefix))
        .map(form_decoded)
}

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

// POSIX's tzset, which the libc crate does not bind on every target.
unsafe extern "C" {
    fn tzset();
}

/// The local time now, as `YYYY-MM-DDTHH:MM`, the hook's `date +%Y-%m-%dT%H:%M`.
pub(crate) fn local_now() -> String {
    // SAFETY: localtime_r writes only into `tm`, which outlives the call. tzset first: localtime_r need not read TZ again,
    // and `dash orch` runs for days, across a change of hour.
    // tzset writes libc's zone globals: one caller at a time, the automations' pass and a /watchers view among them
    static TZ: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = TZ.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let tm = unsafe {
        tzset();
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// Done-when 1 of rust-registry-no-hubs: the route table is the registrations, each path once, and each page is found by
    /// its path, its step the one route_step names; nothing under a page's own path is another's.
    #[test]
    fn the_route_table_is_built_from_each_module_s_registration() {
        let _budget = TestBudget::new();
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

    /// A board of another type is said, never read as no board: `None` is only a process with no board.
    #[test]
    fn a_board_of_another_type_fails_loudly() {
        let _budget = TestBudget::new();
        fn asked(board: Option<&dyn Any>) -> Asked<'_> {
            Asked {
                path: "/tasks",
                query: "",
                html: false,
                board,
                registry: None,
            }
        }
        assert!(asked(None).board::<String>().is_none());
        let wrong = std::panic::catch_unwind(|| asked(Some(&7u8)).board::<String>().cloned());
        let said = wrong.expect_err("a panic");
        let said = said.downcast_ref::<String>().expect("its words");
        assert!(said.contains("/tasks") && said.contains("String"), "{said}");
        assert_eq!(asked(Some(&7u8)).board::<u8>(), Some(&7));
    }

    /// The data folder set at serve start is the one a page reads, and live data otherwise.
    #[test]
    fn the_data_folder_set_at_serve_start_is_the_one_a_page_reads() {
        let _budget = TestBudget::new();
        fn asked() -> Asked<'static> {
            Asked {
                path: "/settings",
                query: "",
                html: false,
                board: None,
                registry: None,
            }
        }
        // before anything is set: the fallback
        assert_eq!(asked().data(), live_data());
        let dir = dash_meter::TestDir::new("pages-data");
        serve_from(&dir.0);
        // after: the set folder
        assert_eq!(asked().data(), dir.0);
    }
}
