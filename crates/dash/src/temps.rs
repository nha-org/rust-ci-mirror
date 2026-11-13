//! Claude's temp folders of ended sessions, queued for bin/prune-paths, which deletes them on the host at the next session start
//! (task rust-disk-caps: they held 26 GB on 2026-11-10, the disk red). `dash orch` runs [`sweep`] with its hourly reap.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long an ended session's folder stays untouched before it is queued.
const IDLE: Duration = Duration::from_secs(24 * 3600);

/// Each session folder under `root` (`<root>/<project, a cwd's slug>/<session id>`) whose session is no longer live: no record
/// in `sessions` (Claude's `<pid>.json` each, one folder for every login, which links it) names it with its process running, and
/// nothing in it changed for `idle`. The wait covers what the records miss: a record half written or not written yet, a run that
/// writes none, a session resumed after its sweep (/code-review of 487680671). ponytail: one resumed past `idle`, then idle
/// until the next start, may lose its folder; prune-paths keeps one a process holds a file in.
pub(crate) fn ended(root: &Path, sessions: &Path, idle: Duration) -> Result<Vec<PathBuf>, String> {
    let records =
        std::fs::read_dir(sessions).map_err(|e| format!("{}: {e}", sessions.display()))?;
    let mut live = vec![];
    for record in records.flatten() {
        let text = std::fs::read_to_string(record.path()).unwrap_or_default();
        let Ok(seen) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let pid = seen["pid"].as_i64().unwrap_or(0) as libc::pid_t;
        // 0: it runs; EPERM: it runs as another user
        let running = pid > 0
            && (unsafe { libc::kill(pid, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM));
        if let (true, Some(id)) = (running, seen["sessionId"].as_str()) {
            live.push(id.to_string());
        }
    }
    let dirs = |at: &Path| -> Vec<PathBuf> {
        let entries = std::fs::read_dir(at).into_iter().flatten().flatten();
        entries
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.is_symlink())
            .collect()
    };
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    // a session id is a UUID; other folders there (bash-edit-diff) are Claude's own
    let session =
        |n: &str| n.len() == 36 && [8, 13, 18, 23].iter().all(|&i| n.as_bytes()[i] == b'-');
    let projects = dirs(root).into_iter().filter(|p| name(p).starts_with('-'));
    let folders = projects.flat_map(|p| dirs(&p));
    let since = SystemTime::now() - idle;
    Ok(folders
        .filter(|f| session(&name(f)) && !live.contains(&name(f)) && changed(f) < since)
        .collect())
}

/// When anything in a folder last changed, itself included.
fn changed(dir: &Path) -> SystemTime {
    let at = |p: &Path| std::fs::symlink_metadata(p).and_then(|m| m.modified());
    let mut last = at(dir).unwrap_or(SystemTime::now());
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            last = last.max(at(&path).unwrap_or(SystemTime::now()));
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push(path);
            }
        }
    }
    last
}

/// `folders` not queued already, queued with `prune` (bin/prune-paths) run in `repo`: what it said, or why not.
pub(crate) fn queue(repo: &Path, prune: &Path, folders: &[PathBuf]) -> Result<String, String> {
    let request =
        std::fs::read_to_string(repo.join(".claude/run/prune.request")).unwrap_or_default();
    let new: Vec<&PathBuf> = (folders.iter())
        .filter(|f| !request.lines().any(|l| Path::new(l) == f.as_path()))
        .collect();
    if new.is_empty() {
        return Ok(String::new());
    }
    let out = std::process::Command::new(prune)
        .args(&new)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("{}: {e}", prune.display()))?;
    let said =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    match out.status.success() {
        true => Ok(format!(
            "{} ended sessions' temp folders queued\n",
            new.len()
        )),
        false => Err(format!("{}: {}", prune.display(), said.trim())),
    }
}

/// [`ended`] under Claude's temp root (`/tmp/claude-<uid>`), [`queue`]d from `repo`: what to print, one line.
pub(crate) fn sweep(repo: &Path, home: &Path) -> String {
    let root = std::fs::canonicalize("/tmp")
        .unwrap_or_else(|_| "/tmp".into())
        .join(format!("claude-{}", unsafe { libc::getuid() }));
    let found = ended(&root, &home.join(".claude/sessions"), IDLE);
    let said = found.and_then(|folders| queue(repo, &repo.join("bin/prune-paths"), &folders));
    said.unwrap_or_else(|e| format!("temps: {e}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    /// Task rust-disk-caps, done-when 3: the temp folder of a session no longer live is queued with bin/prune-paths (a stand-in
    /// here, which queues as it does); a live session's stays, Claude's own folders too, and a folder queued already is not again.
    #[dash_test]
    fn an_ended_session_s_temp_folder_is_queued_with_prune_paths() {
        let dir = TestDir::new("temps");
        let (root, sessions, repo) = (
            dir.0.join("tmp"),
            dir.0.join("sessions"),
            dir.0.join("repo"),
        );
        let (live, gone, never) = (
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222",
            "33333333-3333-3333-3333-333333333333",
        );
        for at in [live, gone, never] {
            std::fs::create_dir_all(root.join("-Users-x-repo").join(at).join("scratchpad"))
                .expect("a folder");
        }
        std::fs::create_dir_all(root.join("bash-edit-diff").join(gone)).expect("Claude's own");
        std::fs::create_dir_all(&sessions).expect("records");
        let mut ended_child = std::process::Command::new("true")
            .spawn()
            .expect("a process");
        ended_child.wait().expect("ended");
        let record = |pid: u32, id: &str| format!(r#"{{"pid":{pid},"sessionId":"{id}"}}"#);
        std::fs::write(sessions.join("1.json"), record(std::process::id(), live)).expect("live");
        std::fs::write(sessions.join("2.json"), record(ended_child.id(), gone)).expect("ended");

        let busy = ended(&root, &sessions, Duration::from_secs(3600)).expect("read");
        assert!(
            busy.is_empty(),
            "a folder changed within the hour stays: {busy:?}"
        );
        std::fs::write(sessions.join("3.json"), "half written").expect("a bad record");
        std::fs::write(sessions.join(".DS_Store"), "").expect("a stray file");
        let mut found = ended(&root, &sessions, Duration::ZERO).expect("a bad record is skipped");
        found.sort();
        let at = |id: &str| root.join("-Users-x-repo").join(id);
        assert_eq!(
            found,
            [at(gone), at(never)],
            "the ended and the unrecorded, never the live one"
        );

        let prune = dir.0.join("prune-paths");
        let queues = "#!/bin/sh\nmkdir -p .claude/run && printf '%s\\n' \"$@\" >> .claude/run/prune.request\n";
        std::fs::write(&prune, queues).expect("a stand-in");
        std::fs::set_permissions(&prune, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("runnable");
        std::fs::create_dir_all(&repo).expect("repo");
        assert_eq!(
            queue(&repo, &prune, &found),
            Ok("2 ended sessions' temp folders queued\n".into())
        );
        let request =
            std::fs::read_to_string(repo.join(".claude/run/prune.request")).expect("queued");
        assert_eq!(
            request,
            format!("{}\n{}\n", at(gone).display(), at(never).display())
        );
        assert_eq!(
            queue(&repo, &prune, &found),
            Ok(String::new()),
            "queued once"
        );
        assert!(at(gone).exists(), "queueing deletes nothing");
    }
}
