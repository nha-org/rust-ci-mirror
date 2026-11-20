//! The plugin switches (task rust-plugin-settings), as a plugin and the binary reach them: through the core's root.
use dash_core::plugin::serves;
use dash_core::registry::{Automation, Contribution, Does, Every, Line, Registry, Slot, Tool};
use dash_core::switches::*;
use dash_core::{Host, OpLog, Page, Reads, Tree, WallClock};
use dash_meter::dash_test;
use dash_sdk::Connection;
use std::path::{Path, PathBuf};

/// Why `origin`'s jobs do not run under `h`'s switches: the one off filter.
fn off(h: &dyn Host, origin: &str) -> Option<String> {
    read(h).stops(origin, Some(JOB))
}

// Entries of four stand-in plugin crates, so each origin is typed here as `submit!` would stamp it in that crate: a test
// stands in for several crates, which one test crate cannot hold.
inventory::submit! { Page { origin: "dash_alpha", path: "/alpha", step: "route_alpha", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "alpha".to_string()).into() } }
inventory::submit! { Pane { origin: "dash_alpha", name: "Alpha", href: "/alpha", order: 10, icon: "inbox", paths: &[], under: None } }
inventory::submit! { Page { origin: "dash_beta::pages", path: "/beta", step: "route_beta", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "beta".to_string()).into() } }
inventory::submit! { Pane { origin: "dash_beta::pages", name: "Beta", href: "/beta", order: 20, icon: "mail", paths: &["/beta-too"], under: None } }
inventory::submit! { Page { origin: "dash_under", path: "/under", step: "route_under", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "under".to_string()).into() } }
inventory::submit! { Pane { origin: "dash_under", name: "Under", href: "/under", order: 0, icon: "", paths: &[], under: Some("/beta") } }
inventory::submit! { Pane { origin: "dash_gamma", name: "Gamma", href: "/gamma", order: 30, icon: "board", paths: &[], under: None } }
// a plugin's page with no pane, as /drafts is: the pages it registers are its
inventory::submit! { Page { origin: "dash_delta", path: "/delta", step: "route_delta", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "delta".to_string()).into() } }
inventory::submit! { Page { origin: "dash_delta::one", path: "/delta/*", step: "route_delta_one", reads: Reads::Nothing, redraw: false, serve: |_| ("200 OK", dash_core::TEXT, "one".to_string()).into() } }

// a job the host's queue runs (dash_jobs), its plugin its origin's as an automation's is
inventory::submit! {
    dash_core::Recurring { origin: "dash_gamma::beat", model: "gamma-beat", every_ms: 60_000, file: "gamma.sqlite", schema: &[], start: |_| Err("never started here".into()) }
}

inventory::submit! {
    Automation {
        origin: "dash_beta",
        key: "beta-sweep",
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

/// A host whose plugin database is plugins.sqlite in a folder of its own, kept across opens as a file is, and whose user is
/// `home`'s last part: what the binary's host does, its changesets aside.
struct Kept(PathBuf, &'static str);

impl WallClock for Kept {
    fn now_ms(&self) -> i64 {
        0
    }
}

impl Host for Kept {
    fn repo(&self) -> PathBuf {
        "/repo".into()
    }
    fn data(&self) -> PathBuf {
        self.0.clone()
    }
    fn switched(&self) -> Option<PathBuf> {
        Some(self.0.clone())
    }
    fn home(&self) -> Result<PathBuf, String> {
        Ok(Path::new("/Users").join(self.1))
    }
    fn arm(&self, _: &Path, _: &str) {}
    fn open(&self, file: &str, schema: &[&str]) -> Result<Connection, String> {
        assert_eq!((file, schema), (FILE, &[SCHEMA][..]));
        let db = Connection::open(self.0.join(file)).map_err(|e| e.to_string())?;
        db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(db)
    }
    fn transact(
        &self,
        db: &Connection,
        run: &mut dyn FnMut(&Connection) -> Result<(), String>,
    ) -> Result<(), String> {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        run(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    fn log(&self, _: &Path, file: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Err(format!(
            "the switches open {file} as plain SQL, never as an op log"
        ))
    }
}

fn hosted_as(user: &'static str) -> (Kept, dash_meter::TestDir) {
    let dir = dash_meter::TestDir::new("core-switches");
    (Kept(dir.0.clone(), user), dir)
}

fn hrefs(menu: &[(&'static Pane, Vec<&'static Pane>)]) -> Vec<&'static str> {
    menu.iter().map(|(p, _)| p.href).collect()
}

/// Done-when 3 of rust-plugin-settings, the core's part: an order is kept per user, read back from the plugin database, and
/// another user's menu keeps the panes' own order; a pane registered since an order was kept comes after the ordered ones.
#[dash_test]
fn the_menu_s_order_is_kept_per_user() {
    let (h, _dir) = &hosted_as("nha");
    assert_eq!(
        hrefs(&menu(h)),
        ["/alpha", "/beta", "/gamma"],
        "the panes' own order"
    );
    assert_eq!(
        menu(h)[1].1.iter().map(|p| p.href).collect::<Vec<_>>(),
        ["/under"],
        "a sub-entry under its parent"
    );
    moved(h, "/gamma", true).unwrap();
    moved(h, "/gamma", true).unwrap();
    assert_eq!(hrefs(&menu(h)), ["/gamma", "/alpha", "/beta"]);
    moved(h, "/gamma", true).unwrap();
    assert_eq!(
        hrefs(&menu(h)),
        ["/gamma", "/alpha", "/beta"],
        "the first stays first"
    );
    assert!(
        moved(h, "/under", true).is_err(),
        "a sub-entry is no top entry"
    );
    let kept = kept(&Kept(h.0.clone(), "nha")).unwrap();
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
    switch(h, "alpha", Some(PANE), false).unwrap();
    moved(h, "/beta", true).unwrap();
    assert_eq!(hrefs(&menu(h)), ["/beta", "/gamma"]);
    switch(h, "alpha", Some(PANE), true).unwrap();
    assert_eq!(hrefs(&menu(h)), ["/beta", "/alpha", "/gamma"]);
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
    let (h, _dir) = &hosted_as("nha");
    let mut registry = Registry::default();
    let tools = vec![("beta-echo".to_string(), Box::new(Echo) as Box<dyn Tool>)];
    registry
        .add(Contribution {
            origin: "dash_beta::tool".into(),
            tools,
            slots: vec![("today/10-beta".into(), Box::new(Waiting) as Box<dyn Slot>)],
            ui: vec![("beta-card".into(), "{}".into())],
            ..Default::default()
        })
        .unwrap();
    let ports = dash_core::registry::Ports::untracked(h);
    let answers = |r: &Registry| r.call("beta-echo", "hi", &ports).unwrap().0;
    let jobs = || {
        (inventory::iter::<Automation>.into_iter())
            .filter(|a| off(h, a.origin).is_none())
            .map(|a| a.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(jobs(), ["beta-sweep"]);
    switch(h, "gamma", Some(JOB), false).unwrap();
    assert_eq!(
        off(h, "dash_gamma::beat"),
        Some("the gamma plugin's jobs are off on /settings".into()),
        "its job switch stops the queue's job (dash_jobs' pass asks it), that switch named"
    );
    assert!(
        dash_core::page(h, "/gamma").is_none() && serves(h, "/gamma"),
        "its pane still on, the older app's page"
    );
    switch(h, "gamma", Some(JOB), true).unwrap();
    assert!(
        dash_core::page(h, "/beta").is_some() && dash_core::page(h, "/beta-too/x").is_none(),
        "no page at /beta-too/x"
    );

    switch(h, "beta", Some(PANE), false).unwrap();
    assert!(
        dash_core::page(h, "/beta").is_none(),
        "its pane off: its page stops"
    );
    assert!(!serves(h, "/beta-too/x"), "and the paths it stands for");
    assert_eq!(
        answers(&registry),
        Ok("hi".into()),
        "its pane off, its tool still answers"
    );
    assert_eq!(
        hrefs(&menu(h)),
        ["/alpha", "/gamma"],
        "and its entry leaves the menu"
    );

    switch(h, "beta", None, false).unwrap();
    assert_eq!(
        answers(&registry),
        Err("beta-echo: the beta plugin is off on /settings".into()),
        "the switch that holds it named: the plugin's whole one"
    );
    assert!(jobs().is_empty(), "its jobs stop");
    assert!(
        registry.slot_lines(h, "today").is_empty() && registry.ui.get_on(h, "beta-card").is_none(),
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
        off(h, sweep.origin),
        Some("the beta plugin is off on /settings".into()),
        "and say so"
    );
    assert!(
        dash_core::page(h, "/under").is_none(),
        "a sub-entry stops with its parent"
    );

    switch(h, "beta", None, true).unwrap();
    switch(h, "beta", Some(PANE), true).unwrap();
    assert!(dash_core::page(h, "/beta").is_some());
    assert!(
        dash_core::page(h, "/under").is_some(),
        "and serves again with it"
    );
    assert_eq!(registry.slot_lines(h, "today").len(), 1, "its line back on");
    assert!(registry.ui.get_on(h, "beta-card").is_some());
    switch(h, "under", None, false).unwrap();
    assert!(
        dash_core::page(h, "/under").is_none(),
        "off, its page stops"
    );
    assert_eq!(menu(h)[1].1.len(), 0, "and its entry leaves its parent");
    assert_eq!(answers(&registry), Ok("hi".into()));
    assert_eq!(jobs(), ["beta-sweep"]);
    assert!(
        switch(h, SETTINGS, None, false).is_err(),
        "settings stays on"
    );

    switch(h, "delta", None, false).unwrap();
    assert!(
        dash_core::page(h, "/delta").is_none() && !serves(h, "/delta/x"),
        "a plugin with no pane: its pages stop too (/code-review)"
    );
    assert!(
        serves(h, "/delta/x/y"),
        "a path no entry registers is no plugin's, delta off too: its first segment names none (ARCHITECTURE.md section 6 \
         point 2)"
    );
    switch(h, "delta", None, true).unwrap();
    assert!(dash_core::page(h, "/delta").is_some());
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
    fn switched(&self) -> Option<PathBuf> {
        Some("/data".into())
    }
    fn home(&self) -> Result<PathBuf, String> {
        Err("none".into())
    }
    fn arm(&self, _: &Path, _: &str) {}
    fn open(&self, _: &str, _: &[&str]) -> Result<Connection, String> {
        Err("database is locked".into())
    }
    fn log(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Err("database is locked".into())
    }
}

/// Switches that never read: every job waits and says the file is why, never /settings, where each shows on; pages serve.
#[dash_test]
fn unread_switches_stop_the_jobs_and_name_the_file() {
    let h = &Broken;
    assert_eq!(
        off(h, "dash_beta"),
        Some(format!(
            "the plugin switches ({FILE}) do not read, so every job waits until they do"
        ))
    );
    assert!(
        dash_core::page(h, "/alpha").is_some(),
        "the app still shows"
    );
}

/// A host that keeps no switches (a test, a tool run on no data folder): every plugin on, and a switch is refused, named.
#[dash_test]
fn with_no_host_every_plugin_is_on() {
    let h = &dash_sdk::StandIn::default();
    assert!(dash_core::page(h, "/alpha").is_some());
    assert_eq!(
        switch(h, "alpha", None, false),
        Err("this host keeps no switches".into())
    );
    assert_eq!(user(h), "me");
}

/// Done-when 1 and 3 of untangle-switches-ports: a switch another process wrote (/settings in `dash serve`, read by `dash
/// orch` or `dash mcp`) holds at the next read, no cache in between; a host holding views reads the switches from its View.
#[dash_test]
fn a_switch_holds_at_the_next_read_and_a_view_is_read_first() {
    let (h, _dir) = &hosted_as("nha");
    assert!(dash_core::page(h, "/alpha").is_some());
    let theirs = Connection::open(h.0.join(FILE)).unwrap();
    theirs
        .execute(
            "INSERT INTO switch (id, value) VALUES ('on/alpha', 'off')",
            [],
        )
        .unwrap();
    assert!(dash_core::page(h, "/alpha").is_none(), "read at once");

    struct Viewed(Kept);
    impl dash_core::View for Viewed {
        fn rows(&self, view: &str) -> Result<Vec<dash_core::Row>, String> {
            assert_eq!(view, VIEW);
            Ok(vec![(
                VIEW.into(),
                "on/beta".into(),
                "off".into(),
                String::new(),
            )])
        }
    }
    impl WallClock for Viewed {
        fn now_ms(&self) -> i64 {
            0
        }
    }
    impl Host for Viewed {
        fn repo(&self) -> PathBuf {
            self.0.repo()
        }
        fn data(&self) -> PathBuf {
            self.0.data()
        }
        fn switched(&self) -> Option<PathBuf> {
            self.0.switched()
        }
        fn home(&self) -> Result<PathBuf, String> {
            self.0.home()
        }
        fn arm(&self, _: &Path, _: &str) {}
        fn open(&self, _: &str, _: &[&str]) -> Result<Connection, String> {
            Err("a host holding views reads them, never the file".into())
        }
        fn view(&self) -> Result<&dyn dash_core::View, String> {
            Ok(self)
        }
        fn log(&self, d: &Path, f: &str, s: &[&str]) -> Result<Box<dyn OpLog>, String> {
            self.0.log(d, f, s)
        }
    }
    let viewed = &Viewed(Kept(h.0.clone(), "nha"));
    assert!(
        dash_core::page(viewed, "/alpha").is_some() && dash_core::page(viewed, "/beta").is_none(),
        "its View's rows, not the file's"
    );
}

/// Done-when 2 of untangle-switches-ports, the core's part: a plugin's settings Element draws its row from the props
/// /settings hands it, each a catalog switch keyed as `set` reads it, and `set` writes what each key names.
#[dash_test]
fn a_settings_element_draws_its_props_and_set_writes_their_keys() {
    let (h, _dir) = &hosted_as("nha");
    let props = serde_json::json!({"plugin": "pod", "on": true, "held_by": null,
        "capabilities": [["tool", false]], "panes": []});
    let mut tree = Tree::default();
    assert_eq!(
        draw_settings(h, &props, "s", &mut tree),
        Ok(vec!["s".to_string()])
    );
    let dash_core::Drawing::Tree { tree, .. } = tree.drawing("Settings", "s") else {
        panic!("a tree")
    };
    let tree: serde_json::Value = serde_json::from_str(&tree).unwrap();
    let el = |id: &str| tree["elements"][id].clone();
    assert_eq!(
        (
            el("plugin-pod")["type"].clone(),
            el("plugin-pod")["props"]["key"].clone()
        ),
        ("switch".into(), "pod".into())
    );
    assert_eq!(
        el("plugin-pod-tool")["props"]["key"],
        "pod/tool",
        "a capability's switch, keyed as set reads it"
    );
    set(h, "pod/tool", true).unwrap();
    set(h, "pod", false).unwrap();
    set(h, "down:/alpha", true).unwrap();
    let kept = kept(h).unwrap();
    assert!(kept.off.contains("pod") && !kept.off.contains("pod/tool"));
    assert_eq!(kept.order["nha"][..2], ["/beta", "/alpha"]);
}

/// An origin's plugin: its crate's without « dash_ », an extensions.d file's without « .json », the binary's own entry its
/// top module's, so one plugin is one row whichever of its modules registered it.
#[dash_test]
fn an_origin_names_its_plugin() {
    assert_eq!(
        [
            "dash_jev",
            "dash_jev::tool::ask",
            "jev.json",
            "dash::mail::compose",
            "dash::doc_index",
            "dash_hostman_ci::live",
            "dash",
        ]
        .map(plugin_of),
        [
            "jev",
            "jev",
            "jev",
            "mail",
            "doc-index",
            "hostman-ci",
            "dash"
        ]
    );
}
