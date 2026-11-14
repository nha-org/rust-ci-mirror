//! What a plugin registers and what the host hands it (ARCHITECTURE.md section 3, task rust-move-core-ports): a page, a
//! command, and the `Host` that gives both the clock, the repository, the data folder and the plugin's own database. The
//! crate root re-exports each, the one surface a plugin holds to (rule 18); the binary fills `Host` and serves the rest.
//! The core reads no clock, environment or file of its own: dash-gate's
//! `what_a_simulation_runs_reads_nothing_a_seed_does_not_decide` holds it.
use crate::oplog::{OpLog, WallClock};
use crate::registry::{Dispatch, Registry};
use std::any::Any;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The binary's side of every plugin: what the core and its plugins never read for themselves, so a test hands its own.
/// Its clock is the core's `WallClock` port, so a simulation's clock is a plugin's too.
pub trait Host: WallClock + Sync {
    /// The repository root the tool runs against.
    fn repo(&self) -> PathBuf;
    /// The folder the server keeps its data in, its `--data`.
    fn data(&self) -> PathBuf;
    /// The user's home folder, where the logins live; an error says why there is none.
    fn home(&self) -> Result<PathBuf, String>;
    /// A command's run under `dir`, reported by `command`'s name, so a stop at the cap names it.
    fn arm(&self, dir: &Path, command: &str);
    /// The plugin's own database `file` in `dir`, each of `schema` applied, as its op log (principle 8: each plugin owns
    /// its database).
    fn open(&self, dir: &Path, file: &str, schema: &[&str]) -> Result<Box<dyn OpLog>, String>;
}

/// A page's one source (the Rust tools' rule 4): the design system draws it as HTML, text or terminal, never the page.
pub enum Drawing {
    /// A UI tree of catalog elements, as JSON.
    Tree { title: String, tree: String },
    /// A page a component of its own draws whole, as /design: its text is its words.
    Page(String),
}

pub const TEXT: &str = "text/plain; charset=utf-8";
pub const HTML: &str = "text/html; charset=utf-8";
pub const JSON: &str = "application/json; charset=utf-8";

/// What a page answers: a drawing the host draws in the form asked, or a body as it is (a redirect, an error, a JSON reply).
pub enum Answer {
    Drawn(Drawing),
    /// Its status line, content type and text.
    Body(&'static str, &'static str, String),
}

impl From<(&'static str, &'static str, String)> for Answer {
    fn from((status, kind, body): (&'static str, &'static str, String)) -> Self {
        Answer::Body(status, kind, body)
    }
}

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

/// A page, registered from its own module with `inventory::submit! { Page { .. } }`: a new page edits no hub (rfc-rust-unified
/// section 11, flat dispatch).
#[derive(Clone, Copy)]
pub struct Page {
    /// Its path, or `<prefix>/*` for each path one segment under the prefix: `/tasks/*` serves /tasks/<id>.
    pub path: &'static str,
    /// Its metric step, so a client's paths never mint one.
    pub step: &'static str,
    pub reads: Reads,
    /// The window draws it again at each ask, as a page that reads the store; any other once a visit.
    pub redraw: bool,
    pub serve: fn(&Asked) -> Answer,
}

inventory::collect!(Page);

/// A page's request: its path and query, the form asked, what its thread holds, and the host.
pub struct Asked<'a> {
    pub path: &'a str,
    pub query: &'a str,
    pub html: bool,
    /// The board, when the page reads it and this process holds one: a type the core does not know, read back by `board`.
    pub board: Option<&'a dyn Any>,
    pub registry: Option<&'a Registry>,
    pub host: &'a dyn Host,
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

    /// The drawing, for the host to draw in the form asked; `None`: no board in this process.
    pub fn form(&self, read: Option<Result<Drawing, String>>) -> Answer {
        match read {
            Some(Ok(d)) => Answer::Drawn(d),
            Some(Err(e)) => self.failed(e),
            None => Answer::Body("404 Not Found", TEXT, "no port, no page".into()),
        }
    }

    pub fn failed(&self, e: String) -> Answer {
        Answer::Body("500 Internal Server Error", TEXT, e)
    }

    /// The logins' folder: no home is no login known, said on the page while the server stays up.
    pub fn home(&self) -> Result<PathBuf, Answer> {
        self.host.home().map_err(|e| self.failed(e))
    }

    /// The server's data folder.
    pub fn data(&self) -> PathBuf {
        self.host.data()
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
pub fn page(path: &str) -> Option<&'static Page> {
    let table = table();
    table.get(path).or_else(|| {
        let cut = path.rfind('/')?;
        table.get(&format!("{}*", &path[..=cut]))
    })
}

/// A command, registered from its own module with `inventory::submit! { Command { .. } }`; `dash help` prints every usage
/// line sorted, and one with `run` needs no arm in the binary's match.
pub struct Command {
    /// One usage line, `dash <words> <arguments>`.
    pub usage: &'static str,
    /// The words naming it after `dash`, given with `run`.
    pub words: &'static [&'static str],
    /// What answers the arguments after its words, the host handed in; none while the binary's match answers it.
    // ponytail: a writer, never a file (the core reads none), so a command handing its output to a child process stays an
    // arm of the binary's match (`dash doctor`); a host method that runs a child is the upgrade when a plugin needs one.
    pub run: Option<Run>,
}

/// A command's answer to its arguments, the host handed in, its lines written to the writer.
pub type Run = fn(&dyn Host, &[&str], &mut dyn Write);

impl Command {
    pub const fn usage(usage: &'static str) -> Command {
        Command {
            usage,
            words: &[],
            run: None,
        }
    }
}

inventory::collect!(Command);

/// One segment of a path decoded: its + stays a plus, as only a form's + is a space (/code-review of #1197).
pub fn segment_decoded(segment: &str) -> String {
    crate::form::form_decoded(&segment.replace('+', "%2B"))
}

/// The query's first `name=` value, decoded as a form sends it.
pub fn param(query: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&prefix))
        .map(crate::form::form_decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// A test's host: each call it should not get panics, so the test shows what it read.
    struct Bare;

    impl WallClock for Bare {
        fn now_ms(&self) -> i64 {
            7
        }
    }

    impl Host for Bare {
        fn repo(&self) -> PathBuf {
            PathBuf::from("/repo")
        }
        fn data(&self) -> PathBuf {
            PathBuf::from("/data")
        }
        fn home(&self) -> Result<PathBuf, String> {
            Err("HOME: unset".into())
        }
        fn arm(&self, _: &Path, _: &str) {}
        fn open(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
            Err("no database here".into())
        }
    }

    fn asked(board: Option<&dyn Any>) -> Asked<'_> {
        Asked {
            path: "/tasks/a+b%20c",
            query: "show=x%2By&view=text",
            html: false,
            board,
            registry: None,
            host: &Bare,
        }
    }

    /// A board of another type is said, never read as no board: `None` is only a process with no board.
    #[dash_test]
    fn a_board_of_another_type_fails_loudly() {
        assert!(asked(None).board::<String>().is_none());
        let wrong = std::panic::catch_unwind(|| asked(Some(&7u8)).board::<String>().cloned());
        let said = wrong.expect_err("a panic");
        let said = said.downcast_ref::<String>().expect("its words");
        assert!(said.contains("/tasks") && said.contains("String"), "{said}");
        assert_eq!(asked(Some(&7u8)).board::<u8>(), Some(&7));
    }

    /// Done-when 3: what a page reads of the world comes from the host it was handed, its id and params from its request.
    #[dash_test]
    fn a_page_reads_the_world_through_its_host() {
        let a = asked(None);
        assert_eq!(a.data(), PathBuf::from("/data"));
        assert!(
            matches!(a.home(), Err(Answer::Body(s, _, e)) if s.starts_with("500") && e == "HOME: unset")
        );
        assert_eq!(a.id(), "a+b c", "a path keeps its +");
        assert_eq!(a.param("show").as_deref(), Some("x+y"));
        assert!(matches!(a.form(None), Answer::Body(s, ..) if s.starts_with("404")));
        let drawn = a.form(Some(Ok(Drawing::Page("p".into()))));
        assert!(matches!(drawn, Answer::Drawn(Drawing::Page(p)) if p == "p"));
        assert!(
            matches!(("308 Permanent Redirect", TEXT, "/x".to_string()).into(), Answer::Body(s, _, b) if s.starts_with("308") && b == "/x")
        );
    }
}
