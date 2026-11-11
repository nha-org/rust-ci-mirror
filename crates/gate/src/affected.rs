//! `dash ci affected <base> [-- <cargo arguments>]`: the workspace crates a change touches and every crate depending on them,
//! read from `cargo metadata` (task rust-affected-tests; @nha, 2026-11-11: « we can then skip CI if the crate doesn't change »).
//! The mirror's CI and the pre-push test those crates alone; a change to OCR runs no mail test. The diff is from the merge-base
//! with `<base>` to the working tree, or from `<base>` itself when the two share no history (the mirror's parentless commits).
//! A changed file under the workspace but in no crate (Cargo.lock, .config, ci/, vendor/) affects every crate; one outside
//! the workspace (site/, private/) none, as the mirror never holds it. dash-gate always runs once anything does: its tests
//! hold the whole workspace to its rules (rust/CLAUDE.md rule 2). Main's mirror run still tests the whole workspace.
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The crate whose tests read every other crate's files.
pub const ALWAYS: &str = "dash-gate";

/// The affected crates, sorted: from `cargo metadata --no-deps`'s JSON, the workspace's folder relative to the repository's
/// top (`""` when they are one, as on the mirror), and the changed paths relative to that top.
pub fn affected(
    metadata: &str,
    workspace: &str,
    changed: &[String],
) -> Result<Vec<String>, String> {
    let meta: Value = serde_json::from_str(metadata).map_err(|e| format!("cargo metadata: {e}"))?;
    let root = meta["workspace_root"]
        .as_str()
        .ok_or("cargo metadata: no workspace_root")?;
    let mut crates = vec![];
    for p in meta["packages"]
        .as_array()
        .ok_or("cargo metadata: no packages")?
    {
        let name = p["name"].as_str().unwrap_or_default().to_string();
        let manifest = Path::new(p["manifest_path"].as_str().unwrap_or_default());
        let dir = manifest
            .parent()
            .and_then(|d| d.strip_prefix(root).ok())
            .unwrap_or(Path::new(""));
        // every kind: a dev-dependency's change reaches the tests that use it
        let deps: BTreeSet<String> = p["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["path"].is_string())
            .filter_map(|d| d["name"].as_str().map(String::from))
            .collect();
        crates.push((name, dir.to_string_lossy().into_owned(), deps));
    }
    let prefix = if workspace.is_empty() {
        String::new()
    } else {
        format!("{}/", workspace.trim_end_matches('/'))
    };
    let mut hit = BTreeSet::new();
    for path in changed {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        // the longest crate folder holding it: a crate's folder never names another's
        match crates
            .iter()
            .filter(|(_, dir, _)| !dir.is_empty() && rel.starts_with(&format!("{dir}/")))
            .max_by_key(|c| c.1.len())
        {
            Some((name, _, _)) => hit.insert(name.clone()),
            // ponytail: any other workspace file runs everything (a README too); a list of inert files if that ever costs
            None => {
                return Ok(crates
                    .into_iter()
                    .map(|c| c.0)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect());
            }
        };
    }
    // each crate depending on one already hit, until none is added
    while let Some(next) = crates
        .iter()
        .find(|(name, _, deps)| !hit.contains(name) && deps.iter().any(|d| hit.contains(d)))
    {
        hit.insert(next.0.clone());
    }
    if !hit.is_empty() && crates.iter().any(|c| c.0 == ALWAYS) {
        hit.insert(ALWAYS.to_string());
    }
    Ok(hit.into_iter().collect())
}

/// `cargo <arguments>` with a `-p <crate>` for each affected one, before the first `--` (clippy's own flags follow it).
pub fn cargo_args(crates: &[String], args: &[String]) -> Vec<String> {
    let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let packages = crates.iter().flat_map(|c| ["-p".to_string(), c.clone()]);
    args[..at]
        .iter()
        .cloned()
        .chain(packages)
        .chain(args[at..].iter().cloned())
        .collect()
}

/// The workspace holding `dir`: its manifest, the repository's top and the workspace's folder relative to that top.
pub fn workspace(dir: &Path) -> Result<(PathBuf, PathBuf, String), String> {
    let top = PathBuf::from(run("git", dir, &["rev-parse", "--show-toplevel"])?.trim());
    // site_perso keeps it in rust/; the mirror holds rust/ at its top
    for folder in ["rust", ""] {
        let manifest = top.join(folder).join("Cargo.toml");
        if manifest.is_file() {
            return Ok((manifest, top, folder.to_string()));
        }
    }
    Err(format!(
        "{}: no Cargo.toml at its top or in rust/",
        top.display()
    ))
}

/// The affected crates of the repository holding `dir`, against `base`, with `cargo` (CARGO when a cargo runs us).
pub fn of_repo(dir: &Path, base: &str, cargo: &str) -> Result<Vec<String>, String> {
    let (manifest, top, folder) = workspace(dir)?;
    let manifest = manifest.to_string_lossy();
    let meta = run(
        cargo,
        &top,
        &[
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
            &manifest,
        ],
    )?;
    // unrelated histories (the mirror's main against its branch commit) have no merge-base: their trees are compared
    let from = run("git", &top, &["merge-base", base, "HEAD"]).unwrap_or_else(|_| base.to_string());
    let names = run(
        "git",
        &top,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--name-only",
            "--no-renames",
            from.trim(),
        ],
    )?;
    let changed: Vec<String> = names.lines().map(String::from).collect();
    affected(&meta, &folder, &changed)
}

/// Why the checkout holding `dir` is not `sha` as pushed: HEAD another commit, or a change to the workspace not committed.
/// None when it is: the pre-push then tests what leaves, in this warm worktree rather than a cold checkout.
pub fn not_as_pushed(dir: &Path, sha: &str) -> Result<Option<String>, String> {
    let (_, top, folder) = workspace(dir)?;
    let head = run("git", &top, &["rev-parse", "HEAD"])?;
    let pushed = run("git", &top, &["rev-parse", &format!("{sha}^{{commit}}")])?;
    if head.trim() != pushed.trim() {
        return Ok(Some(format!(
            "this checkout is at {}, not {sha} as pushed",
            head.trim()
        )));
    }
    let scope = if folder.is_empty() {
        ".".to_string()
    } else {
        folder
    };
    let dirty = run("git", &top, &["status", "--porcelain", "--", &scope])?;
    Ok((!dirty.trim().is_empty()).then(|| format!("{scope}/ has changes {sha} does not hold")))
}

fn run(program: &str, dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("{program} {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
