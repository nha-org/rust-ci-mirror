//! /settings' plugin listing and drawer (task untangle-switches-ports), with stand-in plugins of their own. A file of its
//! own on purpose: a test binary shares one inventory, so a stand-in pane registered in the library's unit tests joins
//! every page's menu there (forms::tests::every_page_carries_one_menu_and_marks_where_it_is went red on PR 1694).
use dash_core::registry::Automation;
use dash_core::registry::{Does, Every};
use dash_core::switches::{JOB, PANE, Pane, TOOL, read};
use dash_core::{Connection, OpLog, WallClock};
use dash_core::{Host, Tree};
use dash_meter::dash_test;
use dash_surface::components::{plugins_in, settings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// a stand-in plugin crate's entries, origins typed as `submit!` stamps them there: zeta's pane and job, eta's pane under it
inventory::submit! { Pane { origin: "dash_zeta", name: "Zeta", href: "/zeta-test", order: 900, icon: "", paths: &[], under: None } }
inventory::submit! { Pane { origin: "dash_eta", name: "Eta", href: "/eta-test", order: 0, icon: "", paths: &[], under: Some("/zeta-test") } }
inventory::submit! {
    Automation { origin: "dash_zeta", key: "zeta-sweep", title: "", why: "", done_when: "", epic: "", every: Every::Minutes(60),
        does: Does::Command(&["zeta"]), mcp: &[], pause_file: None, urgent: None }
}

/// A host keeping its switches in a folder of its own, plain SQL.
struct Kept(PathBuf);

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
        Ok("/Users/nha".into())
    }
    fn arm(&self, _: &Path, _: &str) {}
    fn open(&self, file: &str, schema: &[&str]) -> Result<Connection, String> {
        let db = Connection::open(self.0.join(file)).map_err(|e| e.to_string())?;
        db.execute_batch(&schema.concat())
            .map_err(|e| e.to_string())?;
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
    fn log(&self, _: &Path, f: &str, _: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Err(format!("{f}: no op log"))
    }
}

/// Done-when 2 of untangle-switches-ports: /settings lists each plugin by what it registered (its pane, its jobs, the
/// registry's tools), a sub-entry held off with its parent, and draws a row each from its Element, a pod's by the core's
/// draw, then the menu's order and every status.
#[dash_test]
fn settings_lists_each_plugin_by_what_it_registered_and_draws_its_element() {
    let dir = dash_meter::TestDir::new("surface-settings");
    let h = &Kept(dir.0.clone());
    let registered = BTreeMap::from([("zeta".to_string(), true), ("pod".to_string(), true)]);
    let find = |name: &str| {
        let listed = plugins_in(&read(h), &registered);
        listed.into_iter().find(|p| p.name == name).expect(name)
    };
    assert_eq!(
        find("zeta").capabilities,
        [(JOB, true), (PANE, true), (TOOL, true)]
    );
    dash_core::switches::switch(h, "zeta", None, false).unwrap();
    let eta = find("eta");
    assert_eq!(
        (eta.on, eta.held_by),
        (false, Some("zeta".to_string())),
        "held off with its parent"
    );
    let mut tree = Tree::default();
    assert_eq!(
        settings(h, &registered, "s", &mut tree),
        ["s-settings", "s-menu", "s-status"]
    );
    let dash_core::Drawing::Tree { tree, .. } = tree.drawing("Settings", "s") else {
        panic!("a tree")
    };
    let tree: serde_json::Value = serde_json::from_str(&tree).unwrap();
    let el = |id: &str| tree["elements"][id].clone();
    assert_eq!(el("plugin-zeta")["props"]["on"], false, "its row drawn off");
    assert_eq!(
        el("plugin-eta")["props"]["disabled"],
        true,
        "a held one waits for its parent"
    );
    assert_eq!(
        el("plugin-pod-tool")["props"]["key"],
        "pod/tool",
        "a pod's, by the core's draw"
    );
    assert_eq!(
        el("s-status-zeta")["props"]["text"],
        "zeta: off, job on, pane on, tool on"
    );
}
