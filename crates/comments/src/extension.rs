//! The extension's version (task extension-self-reload, rfc-chrome-extension section 8): a SHA-256 of what his Chrome loads
//! from the unpacked folder, read from the disk at each request as Chrome reads it at each reload. Any merged change to
//! that code changes it, with no number to bump by hand; a README, a test or a .DS_Store never reloads him (a reload ends
//! a picker he is typing in, /code-review). The extension keeps the first one it reads after loading and reloads
//! itself once the app serves another (private/dashboard/extension/version.js).
use dash_core::{Asked, JSON, Page, Reads, json};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The folder Chrome loads, under the repository.
pub const FOLDER: &str = "private/dashboard/extension";

/// Whether Chrome loads `name` as the extension's: its modules, pages (the DevTools panel's) and manifest, at the folder's
/// top, as extension_browser.rs copies them (a module in a subfolder would fail that test's import first).
pub fn shipped(name: &str) -> bool {
    name.ends_with(".js") || name.ends_with(".html") || name == "manifest.json"
}

/// The hex SHA-256 of each shipped file in `dir`, by name, each name and body length-prefixed so no two folders hash alike.
pub fn version(dir: &Path) -> Result<String, String> {
    let said = |e: std::io::Error| format!("{}: {e}", dir.display());
    let mut files = vec![];
    for entry in std::fs::read_dir(dir).map_err(said)? {
        let entry = entry.map_err(said)?;
        if entry.file_type().map_err(said)?.is_file()
            && shipped(&entry.file_name().to_string_lossy())
        {
            files.push(entry.path());
        }
    }
    files.sort();
    let mut h = Sha256::new();
    for path in files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .as_encoded_bytes()
            .to_vec();
        let body = std::fs::read(&path).map_err(said)?;
        for part in [name, body] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part);
        }
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `GET /extension/version`'s status, type and body for the repository `repo`, `{"version": "<hex>"}`: JSON, which the
/// host serves as is (a text page gets its menu appended). A folder not read is a 500, which the extension reads as no
/// change.
pub fn answer(repo: &Path) -> (&'static str, &'static str, String) {
    match version(&repo.join(FOLDER)) {
        Ok(v) => ("200 OK", JSON, json!({ "version": v }).to_string()),
        Err(e) => ("500 Internal Server Error", dash_core::TEXT, e),
    }
}

inventory::submit! {
    Page { path: "/extension/version", step: "route_extension_version", reads: Reads::Nothing, redraw: false,
        serve: |a: &Asked| answer(&a.host.repo()).into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    #[dash_test]
    fn any_change_to_a_file_of_the_folder_changes_the_version_and_a_missing_folder_is_a_500() {
        let scratch = TestDir::new("extension-version");
        let dir = scratch.0.join(FOLDER);
        std::fs::create_dir_all(dir.join("sub")).expect("dir");
        std::fs::write(dir.join("background.js"), "a").expect("write");
        let (status, kind, first) = answer(&scratch.0);
        assert_eq!((status, kind), ("200 OK", JSON));
        let v: serde_json::Value = serde_json::from_str(&first).expect("json");
        assert_eq!(v["version"].as_str().map(str::len), Some(64));
        assert_eq!(
            answer(&scratch.0).2,
            first,
            "the same folder, the same version"
        );
        // a subfolder, a README, a test or Finder's file is no code Chrome loads: no reload for him
        for f in [
            "sub/x.js",
            "README.md",
            "version_test.mjs",
            ".DS_Store",
            "package.json",
        ] {
            std::fs::write(dir.join(f), "x").expect("write");
        }
        assert_eq!(answer(&scratch.0).2, first);
        std::fs::write(dir.join("manifest.json"), "{}").expect("manifest");
        let shipped = answer(&scratch.0).2;
        assert_ne!(shipped, first, "the manifest is shipped");
        let first = shipped;
        std::fs::write(dir.join("background.js"), "b").expect("edit");
        let edited = answer(&scratch.0).2;
        assert_ne!(edited, first, "an edit");
        std::fs::write(dir.join("version.js"), "").expect("add");
        let added = answer(&scratch.0).2;
        assert_ne!(added, edited, "a new file, even empty");
        std::fs::write(dir.join("panel.html"), "").expect("page");
        assert_ne!(
            answer(&scratch.0).2,
            added,
            "a page Chrome loads, the DevTools panel's"
        );
        assert_eq!(
            answer(&scratch.0.join("gone")).0,
            "500 Internal Server Error"
        );
    }
}
