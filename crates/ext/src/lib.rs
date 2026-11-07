//! Extension of section 3: the loader of `extensions.d/` (section 12). Pods register into the same registry as the
//! compiled-in entries, with no rebuild. A change takes effect at the next start.
use dash_core::registry::Registry;
use std::path::Path;
use std::time::Duration;

pub mod pod;

const FOLDER: &str = "extensions.d";

/// How long one call into a pod may run. A pod past it is stopped and the call fails.
const CALL_LIMIT: Duration = Duration::from_secs(5);

/// Loads `<data>/extensions.d` in name order into `registry`, and answers each refused file as « file: reason ». A refused
/// file stays out whole and the others still load: one broken extension must not take the app down. `.json` is a pod's
/// manifest; any other file is left alone, so a pod can keep its program beside its manifest.
pub fn load(registry: &mut Registry, data: &Path) -> Vec<String> {
    let folder = data.join(FOLDER);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return vec![];
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();

    let mut refused = vec![];
    for path in paths {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let added = pod::load(&path, CALL_LIMIT).and_then(|c| registry.add(&name, c));
        if let Err(e) = added {
            refused.push(format!("{name}: {e}"));
        }
    }
    refused
}
