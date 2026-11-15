//! What a plugin registers and what the host hands it (ARCHITECTURE.md section 3, task rust-move-core-ports): a page, a
//! command, and the `Host` that gives both the clock, the repository, the data folder and the plugin's own database. The
//! crate root re-exports each, the one surface a plugin holds to (rule 18); the binary fills `Host` and serves the rest.
//! The core reads no clock, environment or file of its own: dash-gate's
//! `what_a_simulation_runs_reads_nothing_a_seed_does_not_decide` holds it.
use crate::oplog::{OpLog, WallClock};
use crate::registry::{Dispatch, Registry};
use crate::tasks::{Agent, Forge, Launcher, Policy, TaskList, TaskStore, Terminal};
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
    /// The task ports a command drives on the run store in `data`, its `--data`, the host's own tools; a host that keeps
    /// none says so.
    fn tasks(&self, _data: &Path) -> Result<TaskPorts, String> {
        Err("this host hands in no task ports".into())
    }
    /// A registered tool's answer to `arguments`, called through the registry, so its audit log keeps the call and the tool's
    /// own scope and caps hold (task rust-hostman-ci-wire): a plugin reaches another's tool by name, never by import.
    fn call(&self, tool: &str, _arguments: &str) -> Result<String, String> {
        Err(format!("this host calls no tool: {tool}"))
    }
    /// The forge's command line run with `args` under the host's login, its output: the forge's secrets stay the host's
    /// (task rust-hostman-ci-wire).
    fn forge(&self, _args: &[&str]) -> Result<String, String> {
        Err("this host asks no forge".into())
    }
}

/// A job the host's queue runs every `every_ms` (dash_jobs; task rust-hostman-ci-wire), registered from its own module with
/// `inventory::submit! { Recurring { .. } }`. `start` builds its run once, on the queue's thread; each run's rows go to the
/// plugin's own database, `file` under the host's data folder with `schema` applied.
pub struct Recurring {
    pub model: &'static str,
    pub every_ms: i64,
    pub file: &'static str,
    pub schema: &'static [&'static str],
    pub start: fn(&'static dyn Host) -> Result<Tick, String>,
}

/// One run of a recurring job: the rows it writes.
pub type Tick = Box<dyn Fn() -> Vec<crate::FieldWrite>>;

inventory::collect!(Recurring);

/// The task ports the host hands a command (task rust-orch-ports): the store, the list, the terminal, the forge, the agent,
/// the launcher, the policy, and the older app's database, where it files tasks today. The command times its own steps.
pub struct TaskPorts {
    pub store: Box<dyn TaskStore>,
    pub terminal: Box<dyn Terminal>,
    pub list: Box<dyn TaskList>,
    pub forge: Box<dyn Forge>,
    pub agent: Box<dyn Agent>,
    pub launcher: Box<dyn Launcher>,
    pub policy: Box<dyn Policy>,
    pub older_db: PathBuf,
}

/// A view's row (rfc-rust-unified section 6): its view (board, needs, docs), its key, then two fields.
pub type Row = (String, String, String, String);

/// The board view's columns, first to last: the words its rows' third field holds, and /tasks' statuses.
pub const BOARD: [&str; 4] = ["backlog", "todo", "in-progress", "in-review"];

/// The views port: a view's rows by its name, caught up with every log. The dataflow adapter (dash-views) is one; a plugin
/// reads a view through it, never the engine.
pub trait View {
    fn rows(&self, view: &str) -> Result<Vec<Row>, String>;
}

/// A UI tree of the design system's catalog elements (rfc-rust-unified section 10), built on the SDK alone: each element by
/// its id, its type, its props and its children. The design system checks it and draws it in every form.
#[derive(Default)]
pub struct Tree {
    elements: serde_json::Map<String, serde_json::Value>,
}

impl Tree {
    /// The element `id` of type `kind` (« issue-row »), its props none when `props` is null.
    pub fn add(&mut self, id: &str, kind: &str, props: serde_json::Value, children: &[String]) {
        let mut element = serde_json::json!({ "type": kind, "children": children });
        if !props.is_null() {
            element["props"] = props;
        }
        self.elements.insert(id.into(), element);
    }

    /// The tree under `root`, titled, for the host to draw.
    pub fn drawing(self, title: &str, root: &str) -> Drawing {
        let tree = serde_json::json!({ "root": root, "elements": self.elements });
        Drawing::Tree {
            title: title.into(),
            tree: tree.to_string(),
        }
    }
}

/// A plugin's lines another plugin reads by key, through the core (principle 7: plugins compose through the core, never by
/// import), registered from its own module with `inventory::submit! { ContextProvider { .. } }`.
pub struct ContextProvider {
    /// `<plugin>/<what>`, « launcher/parts ».
    pub key: &'static str,
    /// The lines about `about`, the host handed in.
    pub lines: fn(&dyn Host, &str) -> Result<Vec<String>, String>,
}

inventory::collect!(ContextProvider);

/// The lines the provider of `key` gives about `about`; no provider, or a key taken twice, is said.
pub fn context(host: &dyn Host, key: &str, about: &str) -> Result<Vec<String>, String> {
    let mut found = inventory::iter::<ContextProvider>().filter(|p| p.key == key);
    match (found.next(), found.next()) {
        (Some(p), None) => (p.lines)(host, about),
        (None, _) => Err(format!("no plugin provides {key}")),
        (Some(_), Some(_)) => Err(format!("{key} is provided twice")),
    }
}

/// A page's or plugin's warm-up after a restart: `start` begins its reads, `reading` says one still runs (task
/// rust-warmup-queue). Registered beside the page, as its `Page` is; `dash serve` runs each one at a time.
pub struct Warmer {
    pub page: &'static str,
    pub start: fn(),
    pub reading: fn() -> bool,
}

inventory::collect!(Warmer);

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

    inventory::submit! { Warmer { page: "/a-test-page", start: || {}, reading: || false } }
    inventory::submit! { ContextProvider { key: "test/echo", lines: |_, about| Ok(vec![about.into()]) } }
    inventory::submit! { ContextProvider { key: "test/twice", lines: |_, _| Ok(vec![]) } }
    inventory::submit! { ContextProvider { key: "test/twice", lines: |_, _| Ok(vec![]) } }

    /// Done-when 5 of rust-orch-ports: a warmer registers through the SDK, as a page does.
    #[dash_test]
    fn a_warmer_registers_through_the_sdk() {
        let pages: Vec<&str> = inventory::iter::<Warmer>().map(|w| w.page).collect();
        assert!(pages.contains(&"/a-test-page"), "{pages:?}");
    }

    /// A key no plugin provides, or two provide, is said; one provider answers.
    #[dash_test]
    fn a_context_key_reads_its_one_provider() {
        assert_eq!(context(&Bare, "test/echo", "x"), Ok(vec!["x".to_string()]));
        assert_eq!(
            context(&Bare, "test/none", "x"),
            Err("no plugin provides test/none".into())
        );
        assert_eq!(
            context(&Bare, "test/twice", "x"),
            Err("test/twice is provided twice".into())
        );
        assert!(
            Bare.tasks(Path::new("/d")).is_err(),
            "a host with no task ports says so"
        );
    }

    /// A tree's element keeps its type, its props and its children; a null props is none.
    #[dash_test]
    fn a_tree_built_on_the_sdk_is_the_catalog_s_json() {
        let mut tree = Tree::default();
        tree.add("page", "stack", serde_json::Value::Null, &["t".into()]);
        tree.add("t", "heading", crate::json!({"text": "Tasks"}), &[]);
        let Drawing::Tree { title, tree } = tree.drawing("Tasks", "page") else {
            panic!("a tree")
        };
        let tree: serde_json::Value = serde_json::from_str(&tree).expect("json");
        assert_eq!(title, "Tasks");
        assert_eq!(tree["root"], "page");
        assert_eq!(
            tree["elements"]["page"],
            crate::json!({"type": "stack", "children": ["t"]})
        );
        assert_eq!(tree["elements"]["t"]["props"]["text"], "Tasks");
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
