//! `dash worktree lease|leave|list --repo <checkout>`: the run pool of `dash_forge` for every worktree maker, herdr's delegates
//! and reviewers (herdr/session.sh) and another repository's (klar's), each repository's pool its own (task reap-kept-worktrees).
use dash_forge::GitHub;
use std::io::Write;
use std::path::{Path, PathBuf};

inventory::submit! { crate::usage::Command::usage("dash worktree lease --repo <checkout> --path <new folder> --branch <b> --base <ref>") }
inventory::submit! { crate::usage::Command::usage("dash worktree leave --repo <checkout> <worktree>") }
inventory::submit! { crate::usage::Command::usage("dash worktree list --repo <checkout>") }

/// Its exit: 0 done, 1 failed, 2 a misuse, 3 no slot free (a lease's caller then makes its own worktree).
pub fn command(args: &[&str], home: &Path, mut out: impl Write) -> i32 {
    let said = match args {
        [
            "lease",
            "--repo",
            repo,
            "--path",
            path,
            "--branch",
            branch,
            "--base",
            base,
        ] => forge(repo, home).and_then(|f| lease(&f, &absolute(path), branch, base)),
        ["leave", "--repo", repo, path] => forge(repo, home).and_then(|f| {
            let path = absolute(path);
            f.leave_at(&path).map(|()| (0, format!("left: {path}\n")))
        }),
        ["list", "--repo", repo] => forge(repo, home)
            .and_then(|f| f.rows())
            .map(|rows| (0, list(&rows))),
        _ => Ok((2, String::new())),
    };
    match said {
        Ok((2, _)) => {
            eprintln!("usage: dash worktree lease|leave|list --repo <checkout> (dash help)");
            2
        }
        Ok((code, text)) => {
            out.write_all(text.as_bytes()).expect("stdout");
            code
        }
        Err(e) => {
            eprintln!("dash worktree: {e}");
            1
        }
    }
}

fn forge(repo: &str, home: &Path) -> Result<GitHub, String> {
    GitHub::on(Path::new(repo), home)
}

/// The folder as given, made absolute from here: git's worktree commands run in the repository, never in the caller's folder.
fn absolute(path: &str) -> String {
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|d| d.join(&path))
            .unwrap_or(path)
    };
    path.to_string_lossy().into_owned()
}

fn lease(forge: &GitHub, path: &str, branch: &str, base: &str) -> Result<(i32, String), String> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    Ok(match forge.lease_at(path, branch, base)? {
        true => (0, format!("leased: {path} on {branch}, from the pool\n")),
        false => (3, "no slot free: make a fresh worktree\n".into()),
    })
}

/// One line each, its state first, then how many sit unheld past a day: the number the reap brings to 0.
fn list(rows: &[dash_forge::Row]) -> String {
    let line = |r: &dash_forge::Row| {
        let branch = if r.branch.is_empty() {
            "(detached)"
        } else {
            &r.branch
        };
        format!(
            "{}\t{}h\t{branch}\t{}\n",
            r.state,
            r.idle.as_secs() / 3600,
            r.path
        )
    };
    let unheld = rows.iter().filter(|r| r.state == "unheld").count();
    let rows: String = rows.iter().map(line).collect();
    format!("{rows}{unheld} unheld past a day\n")
}
