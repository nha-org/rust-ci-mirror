//! Extension of section 3: the loader of `extensions.d/` (section 12). Steel scripts and pods register into the same
//! registry as the compiled-in entries, with no rebuild. A change takes effect at the next start.
use dash_core::registry::Registry;
use std::path::Path;
use std::time::Duration;

pub mod pod;
pub mod script;

pub const FOLDER: &str = "extensions.d";

/// How long one call into an extension may run, its load included. A script or pod past it is stopped and the call fails.
pub const CALL_LIMIT: Duration = Duration::from_secs(5);

/// Loads `<data>/extensions.d` in name order into `registry`, and answers each refused file as « file: reason ». A refused
/// file stays out whole and the others still load: one broken extension must not take the app down. `.scm` is a script,
/// `.json` a pod's manifest; any other file is left alone, so a pod can keep its program beside its manifest. `check_ui` is
/// the renderer's own check of a UI tree, passed in since a tree that cannot draw must refuse its file too, and this crate
/// takes only the core.
pub fn load(
    registry: &mut Registry,
    data: &Path,
    check_ui: impl Fn(&str) -> Result<String, String>,
) -> Vec<String> {
    let folder = data.join(FOLDER);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return vec![];
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();

    let mut base = None; // No script, no engine: its 33 MB are paid only when one exists.
    let mut refused = vec![];
    for path in paths {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let contribution = match path.extension().and_then(|e| e.to_str()) {
            Some("scm") => {
                let base = base.get_or_insert_with(script::base_engine);
                script::load(base, &path, CALL_LIMIT)
            }
            Some("json") => pod::load(&path, &folder, CALL_LIMIT),
            _ => continue,
        };
        let added = contribution.and_then(|c| {
            for (slot, tree) in &c.ui {
                check_ui(tree).map_err(|e| format!("ui {slot}: {e}"))?;
            }
            registry.add(&name, c)
        });
        if let Err(e) = added {
            refused.push(format!("{name}: {e}"));
        }
    }
    refused
}
