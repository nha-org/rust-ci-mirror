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
    load_with(registry, data, &|_| Ok(()))
}

/// Like `load`, but each pod's UI trees are checked by `check_ui` before the pod lands; a tree that cannot be drawn
/// refuses the whole pod, named by its file.
pub fn load_with(
    registry: &mut Registry,
    data: &Path,
    check_ui: &dyn Fn(&str) -> Result<(), String>,
) -> Vec<String> {
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
        let added = pod::load(&path, CALL_LIMIT).and_then(|c| {
            for (key, tree) in &c.ui {
                check_ui(tree).map_err(|e| format!("{key}: {e}"))?;
            }
            registry.add(&name, c)
        });
        if let Err(e) = added {
            refused.push(format!("{name}: {e}"));
        }
    }
    refused
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::registry::Registry;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    #[test]
    fn an_mcp_pod_in_extensions_d_adds_an_extractor_found_by_flat_dispatch() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("ext-flat");
        let folder = dir.0.join("extensions.d");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("30-rot13.json"), pod::sample_manifest()).expect("manifest");
        let mut r = Registry::default();
        let refused = load(&mut r, &dir.0);
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(r.extract("text/x-rot13", b"hello").unwrap(), "uryyb");
        assert!(
            r.listing()
                .contains(&"extractor text/x-rot13 30-rot13.json".to_string())
        );
    }

    /// Part 2 of rust-registry-ext: a pod is added while the binary runs, with no rebuild. One process: no pod, no extractor;
    /// the pod's file written, the next load finds it; the binary's file unchanged throughout.
    #[test]
    fn a_pod_written_while_the_binary_runs_is_found_at_the_next_load() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("ext-no-rebuild");
        let exe = std::env::current_exe().expect("this binary");
        let built = exe.metadata().and_then(|m| m.modified()).expect("its time");
        let mut before = Registry::default();
        assert!(load(&mut before, &dir.0).is_empty());
        assert!(
            before.extract("text/x-rot13", b"hello").is_err(),
            "no pod, no extractor"
        );
        let folder = dir.0.join("extensions.d");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("40-rot13.json"), pod::sample_manifest()).expect("manifest");
        let mut after = Registry::default();
        assert!(load(&mut after, &dir.0).is_empty());
        assert_eq!(after.extract("text/x-rot13", b"hello").unwrap(), "uryyb");
        assert_eq!(
            exe.metadata().and_then(|m| m.modified()).expect("its time"),
            built,
            "no rebuild"
        );
    }
}
