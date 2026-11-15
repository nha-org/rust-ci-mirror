//! The plugin switches (task rust-plugin-settings), as a plugin and the binary reach them: through the core's root.
use dash_core::registry::{
    Automation, Contribution, Does, Every, Line, Registry, Slot, Tool, jobs_on, off, plugins,
};
use dash_core::switches::*;
use dash_core::{FieldWrite, Host, Op, OpLog, Page, Reads, WallClock};
use dash_meter::dash_test;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

dash_core::pane! { "Alpha", 10, "inbox", Page { path: "/alpha", step: "route_alpha", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "alpha".to_string()).into() } }
dash_core::pane! { "Beta", 20, "mail", ["/beta-too"], Page { path: "/beta", step: "route_beta", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "beta".to_string()).into() } }
dash_core::pane! { "Under", under "/beta", Page { path: "/under", step: "route_under", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "under".to_string()).into() } }
dash_core::pane! { "Gamma", 30, "board", "/gamma" }
// a plugin's page with no pane, as /drafts is: the paths under its name are its
inventory::submit! { Page { path: "/delta", step: "route_delta", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "delta".to_string()).into() } }

// a job the host's queue runs (dash_jobs), its plugin named as an automation's is
inventory::submit! {
    dash_core::Recurring { model: "gamma-beat", plugin: "gamma", every_ms: 60_000, file: "gamma.sqlite", schema: &[], start: |_| Err("never started here".into()) }
}

inventory::submit! {
    Automation {
        key: "beta-sweep",
        plugin: "beta",
        title: "",
        why: "",
        done_when: "",
        epic: "",
        every: Every::Minutes(60),
        does: Does::Command(&["beta"]),
        mcp: &[],
        pause_file: None,
        urgent: None,
    }
}

/// A host whose plugin database is a log kept across opens, as a file is, and whose user is `home`'s last part.
struct Kept(Arc<StdMutex<Vec<Op>>>, &'static str);

struct Shared(Arc<StdMutex<Vec<Op>>>);

impl OpLog for Shared {
    fn write(&mut self, writes: &[FieldWrite]) -> Result<Vec<Op>, String> {
        let mut ops = self.0.lock().unwrap();
        let new: Vec<Op> = (writes.iter())
            .map(|w| Op {
                stamp: ops.len() as i64 + 1,
                writer: "t".into(),
                write: w.clone(),
            })
            .collect();
        ops.extend(new.iter().cloned());
        Ok(new)
    }
    fn apply(&mut self, _: &[Op]) -> Result<usize, String> {
        unreachable!()
    }
    fn ops_after(&self, _: Option<(i64, &str)>, _: i64) -> Result<Vec<Op>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn state_hash(&self) -> Result<u64, String> {
        unreachable!()
    }
}

/// The test's clock, which a test moves on.
static NOW: AtomicI64 = AtomicI64::new(0);

impl WallClock for Kept {
    fn now_ms(&self) -> i64 {
        NOW.load(Ordering::SeqCst)
    }
}

impl Host for Kept {
    fn repo(&self) -> PathBuf {
        "/repo".into()
    }
    fn data(&self) -> PathBuf {
        "/data".into()
    }
    fn home(&self) -> Result<PathBuf, String> {
        Ok(Path::new("/Users").join(self.1))
    }
    fn arm(&self, _: &Path, _: &str) {}
    fn log(&self, dir: &Path, file: &str, schema: &[&str]) -> Result<Box<dyn OpLog>, String> {
        assert_eq!(
            (dir, file, schema),
            (Path::new("/data"), FILE, &[SCHEMA][..])
        );
        Ok(Box::new(Shared(Arc::clone(&self.0))))
    }
}

fn hosted_as(user: &'static str) -> Arc<StdMutex<Vec<Op>>> {
    let ops = Arc::new(StdMutex::new(vec![]));
    hosted(
        Box::leak(Box::new(Kept(Arc::clone(&ops), user))),
        Path::new("/data"),
    );
    ops
}

fn hrefs(menu: &[(&'static Pane, Vec<&'static Pane>)]) -> Vec<&'static str> {
    menu.iter().map(|(p, _)| p.href).collect()
}

/// Done-when 3 of rust-plugin-settings, the core's part: an order is kept per user, read back from the plugin database, and
/// another user's menu keeps the panes' own order; a pane registered since an order was kept comes after the ordered ones.
#[dash_test]
fn the_menu_s_order_is_kept_per_user() {
    let ops = hosted_as("nha");
    assert_eq!(
        hrefs(&menu()),
        ["/alpha", "/beta", "/gamma"],
        "the panes' own order"
    );
    assert_eq!(
        menu()[1].1.iter().map(|p| p.href).collect::<Vec<_>>(),
        ["/under"],
        "a sub-entry under its parent"
    );
    moved("/gamma", true).unwrap();
    moved("/gamma", true).unwrap();
    assert_eq!(hrefs(&menu()), ["/gamma", "/alpha", "/beta"]);
    moved("/gamma", true).unwrap();
    assert_eq!(
        hrefs(&menu()),
        ["/gamma", "/alpha", "/beta"],
        "the first stays first"
    );
    assert!(
        moved("/under", true).is_err(),
        "a sub-entry is no top entry"
    );
    let kept = kept(&Kept(Arc::clone(&ops), "nha")).unwrap();
    assert_eq!(
        kept.order["nha"],
        ["/gamma", "/alpha", "/beta"],
        "read back from the database"
    );
    assert_eq!(
        hrefs(&kept.menu("someone-else")),
        ["/alpha", "/beta", "/gamma"],
        "another user's own"
    );
    // an off pane keeps its place: Beta up passes Gamma, the next shown, and Alpha comes back in the middle, not last
    switch("alpha", Some(PANE), false).unwrap();
    moved("/beta", true).unwrap();
    assert_eq!(hrefs(&menu()), ["/beta", "/gamma"]);
    switch("alpha", Some(PANE), true).unwrap();
    assert_eq!(hrefs(&menu()), ["/beta", "/alpha", "/gamma"]);
    let mut partial = Switches::default();
    partial.order.insert("nha".into(), vec!["/beta".into()]);
    assert_eq!(
        hrefs(&partial.menu("nha")),
        ["/beta", "/alpha", "/gamma"],
        "an unordered pane after"
    );
}

struct Waiting;

impl Slot for Waiting {
    fn line(&self) -> Line {
        Line {
            says: Ok(Some("2 waiting".into())),
            what: "replies".into(),
            href: "/beta".into(),
        }
    }
}

struct Echo;

impl Tool for Echo {
    fn description(&self) -> &str {
        "echo"
    }
    fn input_schema(&self) -> &str {
        "{}"
    }
    fn call(&self, arguments: &str) -> Result<String, String> {
        Ok(arguments.into())
    }
}

/// Done-when 2 of rust-plugin-settings, the core's part: an off plugin's pages, tools and jobs stop, its sub-entry's pages
/// and the paths it stands for too; one capability off stops that one alone (rfc-superapp-plugins section 1: « turning a
/// pane off leaves the plugin's tool answering »); settings never goes off; on again, each serves again.
#[dash_test]
fn an_off_plugin_s_pages_tools_and_jobs_stop() {
    hosted_as("nha");
    let mut registry = Registry::default();
    let tools = vec![("beta-echo".to_string(), Box::new(Echo) as Box<dyn Tool>)];
    registry
        .add(
            "dash-beta",
            Contribution {
                tools,
                slots: vec![("today/10-beta".into(), Box::new(Waiting) as Box<dyn Slot>)],
                ui: vec![("beta-card".into(), "{}".into())],
                ..Default::default()
            },
        )
        .unwrap();
    let clock = || 0;
    let ports = dash_core::registry::Ports::untracked(&clock);
    let answers = |r: &Registry| r.call("beta-echo", "hi", &ports).unwrap().0;
    let beta = plugins().into_iter().find(|p| p.name == "beta").unwrap();
    assert_eq!(
        beta.capabilities,
        [(JOB, true), (PANE, true), (TOOL, true)],
        "its pane, its tools, its jobs"
    );
    let jobs = || {
        (inventory::iter::<Automation>.into_iter())
            .filter(|a| off(a).is_none())
            .map(|a| a.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(jobs(), ["beta-sweep"]);
    let gamma = || plugins().into_iter().find(|p| p.name == "gamma").unwrap();
    assert_eq!(
        gamma().capabilities,
        [(JOB, true), (PANE, true)],
        "a queue's job listed with its plugin"
    );
    switch("gamma", Some(JOB), false).unwrap();
    assert!(
        !jobs_on("gamma"),
        "its job switch stops the queue's job (dash_jobs' pass asks it)"
    );
    assert_eq!(gamma().capabilities, [(JOB, false), (PANE, true)]);
    assert!(
        dash_core::page("/gamma").is_none() && serves("/gamma"),
        "its pane still on, the older app's page"
    );
    switch("gamma", Some(JOB), true).unwrap();
    assert!(
        dash_core::page("/beta").is_some() && dash_core::page("/beta-too/x").is_none(),
        "no page at /beta-too/x"
    );

    switch("beta", Some(PANE), false).unwrap();
    assert!(
        dash_core::page("/beta").is_none(),
        "its pane off: its page stops"
    );
    assert!(!serves("/beta-too/x"), "and the paths it stands for");
    assert_eq!(
        answers(&registry),
        Ok("hi".into()),
        "its pane off, its tool still answers"
    );
    assert_eq!(
        hrefs(&menu()),
        ["/alpha", "/gamma"],
        "and its entry leaves the menu"
    );

    switch("beta", None, false).unwrap();
    assert_eq!(
        answers(&registry),
        Err("beta-echo: the beta plugin's tools are off on /settings".into())
    );
    assert!(jobs().is_empty(), "its jobs stop");
    assert!(
        registry.slot_lines("today").is_empty() && registry.ui.get_on("beta-card").is_none(),
        "its lines on another page and its trees too (/code-review)"
    );
    assert!(
        dash_core::plugin::registered("/beta").is_some(),
        "its page still registered, read by what names Rust's pages (alerts)"
    );
    let sweep = inventory::iter::<Automation>
        .into_iter()
        .find(|a| a.key == "beta-sweep")
        .unwrap();
    assert_eq!(
        off(sweep),
        Some("off: the beta plugin's jobs are off on /settings".into()),
        "and say so"
    );
    assert!(
        dash_core::page("/under").is_none(),
        "a sub-entry stops with its parent"
    );
    let under = plugins().into_iter().find(|p| p.name == "under").unwrap();
    assert_eq!(
        (under.on, under.held_by),
        (false, Some("beta")),
        "and /settings says it is off with it (/code-review)"
    );

    switch("beta", None, true).unwrap();
    switch("beta", Some(PANE), true).unwrap();
    assert!(dash_core::page("/beta").is_some());
    assert!(
        dash_core::page("/under").is_some(),
        "and serves again with it"
    );
    assert_eq!(registry.slot_lines("today").len(), 1, "its line back on");
    assert!(registry.ui.get_on("beta-card").is_some());
    switch("under", None, false).unwrap();
    assert!(dash_core::page("/under").is_none(), "off, its page stops");
    assert_eq!(menu()[1].1.len(), 0, "and its entry leaves its parent");
    assert_eq!(answers(&registry), Ok("hi".into()));
    assert_eq!(jobs(), ["beta-sweep"]);
    assert!(switch(SETTINGS, None, false).is_err(), "settings stays on");

    switch("delta", None, false).unwrap();
    assert!(
        dash_core::page("/delta").is_none() && !serves("/delta/x"),
        "a plugin with no pane: its pages stop too (/code-review)"
    );
    switch("delta", None, true).unwrap();
    assert!(dash_core::page("/delta").is_some());
}

/// A host whose switches database never opens.
struct Broken;

impl WallClock for Broken {
    fn now_ms(&self) -> i64 {
        0
    }
}

impl Host for Broken {
    fn repo(&self) -> PathBuf {
        "/repo".into()
    }
    fn data(&self) -> PathBuf {
        "/data".into()
    }
    fn home(&self) -> Result<PathBuf, String> {
        Err("none".into())
    }
    fn arm(&self, _: &Path, _: &str) {}
    fn log(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Err("database is locked".into())
    }
}

/// Switches that never read: every job waits and says the file is why, never /settings, where each shows on; pages serve.
#[dash_test]
fn unread_switches_stop_the_jobs_and_name_the_file() {
    hosted(&Broken, Path::new("/data"));
    assert_eq!(
        dash_core::registry::jobs_off("beta"),
        Some(format!(
            "off: the plugin switches ({FILE}) do not read, so every job waits until they do"
        ))
    );
    assert!(dash_core::page("/alpha").is_some(), "the app still shows");
}

/// A command hosts the switches of the folder its `--data` names; a scratch server hosts none, the live folder not its own.
#[dash_test]
fn a_scratch_server_hosts_no_switches() {
    dash_core::switches::hosted_on_data(&Broken, &["mcp", "--scratch", "--data", "/live"]);
    assert_eq!(dash_core::registry::jobs_off("beta"), None, "none hosted");
    dash_core::switches::hosted_on_data(&Broken, &["watchers", "--data", "/data"]);
    assert!(dash_core::registry::jobs_off("beta").is_some(), "hosted");
}

/// No host (a test, a tool run outside the server): every plugin on, and a switch is refused, named.
#[dash_test]
fn with_no_host_every_plugin_is_on() {
    assert!(dash_core::page("/alpha").is_some());
    assert_eq!(
        switch("alpha", None, false),
        Err("no host keeps the switches in this process".into())
    );
    assert_eq!(user(), "me");
}

/// A switch another process wrote (/settings in `dash serve`, read by `dash orch` or `dash mcp`) holds here once the read is
/// `FRESH_MS` old, and a read is served from memory until then.
#[dash_test]
fn a_switch_another_process_wrote_holds_within_fresh_ms() {
    let ops = hosted_as("nha");
    assert!(dash_core::page("/alpha").is_some());
    let theirs = FieldWrite {
        table: "switch".into(),
        row: "on/alpha".into(),
        field: "value".into(),
        value: dash_core::Value::Text("off".into()),
    };
    Shared(Arc::clone(&ops)).write(&[theirs]).unwrap();
    NOW.store(FRESH_MS - 1, Ordering::SeqCst);
    assert!(
        dash_core::page("/alpha").is_some(),
        "read from memory, within FRESH_MS"
    );
    NOW.store(FRESH_MS, Ordering::SeqCst);
    assert!(dash_core::page("/alpha").is_none(), "read again past it");
}

/// An origin's plugin: a crate's without « dash- », an extensions.d file's without « .json », so one plugin is one row.
#[dash_test]
fn an_origin_names_its_plugin() {
    assert_eq!(
        ["dash-jev", "jev.json", "mail"].map(plugin_of),
        ["jev", "jev", "mail"]
    );
}
