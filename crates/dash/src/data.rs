//! `dash data copy <dir>`: a consistent copy of the live data folder, for a task's checks (task rust-live-copy; @nha,
//! 2026-11-06: « slow is smooth and smooth is fast. we aim for slow but systemic improvement. »). A task's sandbox reads the
//! live store and writes none of it, so rust-window-in-process and rust-pools-fable each built a copy of their own. Each
//! database goes through VACUUM INTO, as reference.rs copies the older app's while it writes; journals, locks and the telemetry switch stay behind.
use std::io::Write;
use std::path::Path;

/// `from`'s databases and plain files copied into `to`, made if missing: the names copied, sorted. Refuses `to` = `from`,
/// whose databases the copy would first remove.
pub fn copy(from: &Path, to: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(to).map_err(said(to))?;
    if std::fs::canonicalize(to).map_err(said(to))?
        == std::fs::canonicalize(from).map_err(said(from))?
    {
        return Err(format!(
            "{}: the live folder itself, never a copy",
            to.display()
        ));
    }
    let mut names = vec![];
    for entry in std::fs::read_dir(from).map_err(said(from))? {
        let entry = entry.map_err(said(from))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let journal = [".lock", "-wal", "-shm", "-journal"]
            .iter()
            .any(|e| name.ends_with(e));
        // the telemetry switch stays live's: a check on the copy never reports as the running tool
        if journal
            || name == "telemetry"
            || !entry.file_type().map_err(said(&entry.path()))?.is_file()
        {
            continue;
        }
        let (source, copy) = (entry.path(), to.join(&name));
        match name.ends_with(".sqlite") {
            true => dash_store::older::snapshot(&source, &copy)?,
            // its bytes, never its mode: a read-only live file would stay read-only here, and a second copy would fail on it
            false => {
                let bytes = std::fs::read(&source).map_err(said(&source))?;
                match std::fs::remove_file(&copy) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(said(&copy)(e));
                    }
                    _ => std::fs::write(&copy, bytes).map_err(said(&copy))?,
                }
            }
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// An IO error named by its path.
fn said(at: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{}: {e}", at.display())
}

inventory::submit! { crate::usage::Command::usage("dash data copy <dir>") }
/// `dash data copy <dir>`: the live folder copied, then the line a check's `--data` takes. Exit 1 on a failure.
pub fn run(to: &Path, mut out: std::fs::File) {
    let from = crate::tasks::live_data();
    match copy(&from, to) {
        Ok(names) => {
            let line = format!(
                "copied {} into {}: {}",
                from.display(),
                to.display(),
                names.join(" ")
            );
            writeln!(out, "{line}\nnext: dash <command> --data {}", to.display()).expect("stdout");
        }
        Err(why) => {
            eprintln!("data copy: {why}");
            std::process::exit(1);
        }
    }
}
