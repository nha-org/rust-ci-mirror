//! The host a plugin is handed (ARCHITECTURE.md section 3): what a plugin never reads for itself, so a test hands its own.
use crate::ops::{FieldWrite, OpLog, WallClock};
pub use rusqlite::Connection;

/// A view's row (rfc-rust-unified section 6): its view (board, needs, docs), its key, then two fields.
pub type Row = (String, String, String, String);

/// The views port: a view's rows by its name, caught up with every log. The dataflow adapter (dash-views) is one; a plugin
/// reads a view through it, never the engine.
pub trait View {
    fn rows(&self, view: &str) -> Result<Vec<Row>, String>;
}

use std::path::{Path, PathBuf};

/// The binary's side of every plugin. Its clock is the `WallClock` port, so a simulation's clock is a plugin's too.
pub trait Host: WallClock + Sync {
    /// The repository root the tool runs against.
    fn repo(&self) -> PathBuf;
    /// The folder the server keeps its data in, its `--data`.
    fn data(&self) -> PathBuf;
    /// The folder this process runs on, where its plugin switches are kept: a command's `--data`, the server's own. None
    /// (a test, a tool run on no data folder, a scratch server): every plugin on, no database opened to learn it, and a
    /// command that needs a folder says its usage.
    fn switched(&self) -> Option<PathBuf> {
        None
    }
    /// The user's home folder, where the logins live; an error says why there is none.
    fn home(&self) -> Result<PathBuf, String>;
    /// A command's run under `dir`, reported by `command`'s name, so a stop at the cap names it.
    fn arm(&self, dir: &Path, command: &str);
    /// The plugin's own database `file` in the host's data folder, each of `schema` applied: read it with plain SQL, write
    /// it through `transact` (ARCHITECTURE.md section 3, decision 8). Every table has a primary key.
    fn open(&self, file: &str, _schema: &[&str]) -> Result<Connection, String> {
        Err(format!("this host opens no database: {file}"))
    }
    /// `run`'s SQL on `db`, an `open`ed file, in one transaction, committed only when it answers `Ok`: a check and its write
    /// never cross another's. The host records the commit's changeset, the file's one event stream (section 5).
    fn transact(
        &self,
        _db: &Connection,
        _run: &mut dyn FnMut(&Connection) -> Result<(), String>,
    ) -> Result<(), String> {
        Err("this host writes no database".into())
    }
    /// The views a page reads (ARCHITECTURE.md section 3): a view's rows from memory, every plugin's commits in them. A
    /// host that holds none says so: a command's, a test's.
    fn view(&self) -> Result<&dyn View, String> {
        Err("this host holds no views".into())
    }
    /// The database `file` in `dir` as an op log: the plugins not on plain SQL yet (comments, mail, hostman-ci, the
    /// switches, the jobs); each moves to `open` with its rust-move task, and this goes with the last.
    fn log(&self, dir: &Path, file: &str, schema: &[&str]) -> Result<Box<dyn OpLog>, String>;
    /// A transport signed in as `who`, one of his addresses: a channel's send and its reads go through it (task
    /// draft-from-is-address); a host that signs in nowhere says so.
    fn signed_in(&self, who: &str) -> Result<Transport, String> {
        Err(format!("this host signs {who} in nowhere"))
    }
    /// `run` handed the host once `ms` have passed on its clock: a send's Undo window; a host that runs nothing later says so.
    fn after(&self, _ms: i64, _run: Later) -> Result<(), String> {
        Err("this host runs nothing later".into())
    }
    /// A registered tool's answer to `arguments`, called through the host's registry, so its audit log keeps the call and
    /// the tool's own scope and caps hold (task rust-hostman-ci-wire): a plugin reaches another's tool by name, never by
    /// import.
    fn call(&self, tool: &str, _arguments: &str) -> Result<String, String> {
        Err(format!("this host calls no tool: {tool}"))
    }
    /// The forge's command line run with `args` under the host's login, its output: the forge's secrets stay the host's
    /// (task rust-hostman-ci-wire).
    fn forge(&self, _args: &[&str]) -> Result<String, String> {
        Err("this host asks no forge".into())
    }
    /// A child process (ARCHITECTURE.md section 3, `run(Program) -> Ran`): its environment `program.given` alone with PATH,
    /// HOME and TMPDIR, in its own process group, the whole group killed at its limit. A plugin starts no process by itself.
    fn run(&self, program: &Program) -> Result<Ran, String> {
        Err(format!("this host runs no program: {}", program.bin))
    }
}

/// A program a plugin asks its host to run: the binary, its arguments, its folder, the only environment it gets beside
/// PATH, HOME and TMPDIR, its input, its time limit.
pub struct Program<'a> {
    pub bin: &'a str,
    pub args: &'a [&'a str],
    pub dir: Option<&'a std::path::Path>,
    pub given: &'a [(&'a str, &'a str)],
    pub stdin: Option<&'a str>,
    pub limit: std::time::Duration,
}

/// What a program answered: its exit code (-1 for a signal), stdout and stderr's end.
pub struct Ran {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A request to a server and its answer: the URL and the body posted, `None` a GET; the answer's body, or why none came.
pub type Transport = Box<dyn Fn(&str, Option<&str>) -> Result<String, String>>;

/// A way to open a transport as `who`, `Host::signed_in`'s shape, for a host that keeps it to hand in.
pub type SignIn = fn(&str) -> Result<Transport, String>;

/// What the host runs later (`Host::after`), handed the host then.
pub type Later = Box<dyn FnOnce(&dyn Host) + Send>;

/// A job the host's queue runs every `every_ms` (dash_jobs; task rust-hostman-ci-wire), registered from its own module with
/// `submit! { Recurring { .. } }`. `start` builds its run once, on the queue's thread; each run's rows go to the
/// plugin's own database, `file` under the host's data folder with `schema` applied.
pub struct Recurring {
    pub model: &'static str,
    /// The module that registered it, stamped by `submit!`: its plugin's job switch on /settings stops it (task
    /// rust-plugin-settings).
    pub origin: &'static str,
    pub every_ms: i64,
    pub file: &'static str,
    pub schema: &'static [&'static str],
    pub start: fn(&'static dyn Host) -> Result<Tick, String>,
}

/// One run of a recurring job: the rows it writes.
pub type Tick = Box<dyn Fn() -> Vec<FieldWrite>>;

inventory::collect!(Recurring);
