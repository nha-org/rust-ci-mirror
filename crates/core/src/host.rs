//! The host a plugin is handed (ARCHITECTURE.md section 3), a leaf below the plugin API and the switches that both read it
//! (task rust-plugin-settings: dash-gate's feature loops), re-exported by `plugin` and the crate root.
use crate::oplog::{OpLog, WallClock};
use crate::tasks::{Agent, Forge, Launcher, Policy, TaskList, TaskStore, Terminal};
use std::path::{Path, PathBuf};

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
    /// A transport signed in as `who`, one of his addresses: a channel's send and its reads go through it (task
    /// draft-from-is-address); a host that signs in nowhere says so.
    fn signed_in(&self, who: &str) -> Result<Transport, String> {
        Err(format!("this host signs {who} in nowhere"))
    }
    /// `run` handed the host once `ms` have passed on its clock: a send's Undo window; a host that runs nothing later says so.
    fn after(&self, _ms: i64, _run: Later) -> Result<(), String> {
        Err("this host runs nothing later".into())
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

/// A request to a server and its answer: the URL and the body posted, `None` a GET; the answer's body, or why none came.
pub type Transport = Box<dyn Fn(&str, Option<&str>) -> Result<String, String>>;

/// A way to open a transport as `who`, `Host::signed_in`'s shape, for a host that keeps it to hand in.
pub type SignIn = fn(&str) -> Result<Transport, String>;

/// What the host runs later (`Host::after`), handed the host then.
pub type Later = Box<dyn FnOnce(&dyn Host) + Send>;

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

/// A job the host's queue runs every `every_ms` (dash_jobs; task rust-hostman-ci-wire), registered from its own module with
/// `inventory::submit! { Recurring { .. } }`. `start` builds its run once, on the queue's thread; each run's rows go to the
/// plugin's own database, `file` under the host's data folder with `schema` applied.
pub struct Recurring {
    pub model: &'static str,
    /// Its plugin, by name: /settings stops its jobs (task rust-plugin-settings).
    pub plugin: &'static str,
    pub every_ms: i64,
    pub file: &'static str,
    pub schema: &'static [&'static str],
    pub start: fn(&'static dyn Host) -> Result<Tick, String>,
}

/// One run of a recurring job: the rows it writes.
pub type Tick = Box<dyn Fn() -> Vec<crate::FieldWrite>>;

inventory::collect!(Recurring);
