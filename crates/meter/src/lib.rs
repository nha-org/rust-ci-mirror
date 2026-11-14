//! Tooling, outside the rings of section 3: the memory reading, budgets and cap of section 4. A crate with tests takes it as a
//! dev-dependency; only the binary, which the graph gate exempts, takes it as a dependency, for /memory and its allocator.
pub mod budgets;
pub mod cap;
pub mod fixtures;
pub mod memory;

/// Every test's attribute: `#[test]` with its [`memory::TestBudget`] first (task rust-test-budget-attr).
pub use dash_test_macro::dash_test;
/// A sub-second wall-clock bound's named budget (dash-gate's `a_sub_second_wall_bound_names_its_budget`).
pub use dash_test_macro::wall_budget;
// The macro names `::dash_meter`, so this crate's own tests take it too.
extern crate self as dash_meter;

/// Every line a test's stand-in listener hears until one quiet stretch of its read timeout.
pub fn drain(socket: &std::net::UdpSocket) -> Vec<String> {
    let mut lines = vec![];
    let mut buf = [0u8; 512];
    while let Ok(n) = socket.recv(&mut buf) {
        lines.push(String::from_utf8_lossy(&buf[..n]).into_owned());
    }
    lines
}

/// The monorepo around rust/, or None on the public mirror, which holds rust/ alone, after a line saying the test stops there.
/// Told by rust/ itself sitting in it, so a monorepo that moved a folder a test reads still fails (PR 1426, 2026-11-11: its
/// tests reached private/dashboard/bin/dash, missing on the mirror). The one reach past rust/: the gate's
/// `run_time_paths_past_rust` refuses any other.
/// The crate's folder comes from the run alone (cargo sets CARGO_MANIFEST_DIR for each test it runs), never from the build:
/// sccache hands every worktree the same compiled meter, its env! naming the worktree that built it first, so on 2026-11-12
/// every test here read a removed worktree's path and stopped as if on the mirror (task rust-merge-gate-port; dash-gate's
/// build_paths_at_run_time refuses the build's).
pub fn monorepo() -> Option<std::path::PathBuf> {
    let Some(crate_dir) = std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from)
    else {
        println!("CARGO_MANIFEST_DIR is unset: a test binary run outside cargo; this test stops");
        return None;
    };
    let top = crate_dir.join("../../..");
    if top.join("rust/Cargo.toml").is_file() {
        Some(top)
    } else {
        println!(
            "no monorepo around rust/: the public mirror, which holds rust/ alone; this test stops"
        );
        None
    }
}

/// A test's own folder, gone at its end even when the test fails. A leftover of an earlier run under the same pid is
/// cleared first: it held a 186 MB store and a fixture would have added to it.
pub struct TestDir(pub std::path::PathBuf);

impl TestDir {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("dash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        Self(dir)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A stand-in script a test runs, kept under the temp folder at a path named by its text's hash and written once: macOS scans
/// each new executable on its first run, 8 to 21 s under a load of 80 to 140 against 0.02 s for the second (task
/// rust-front-door-filing-race, 2026-11-12: the front door's filing missed its 20 s on it). Per-test paths reach the script
/// through its environment, so its text, and the one scan, stay the same run after run.
/// ponytail: the first run after the file is gone (a new text, a reboot, a purged $TMPDIR) still pays the one scan inside
/// its test; a nextest setup script warming every stand-in is the upgrade if that first run keeps failing.
pub fn stand_in(script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let name = &fixtures::sha256(script.as_bytes())[..16];
    let dir = std::env::temp_dir().join("dash-stand-ins");
    let path = dir.join(name);
    let ready = |p: &std::path::Path| {
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o777 == 0o755)
            && std::fs::read_to_string(p).is_ok_and(|kept| kept == script)
    };
    if ready(&path) {
        return path;
    }
    // written beside it under a name of this call's own, then linked in: a test running it meanwhile never reads half a
    // script, and of two callers racing the first copy stays, never swapped for a second unscanned one (/code-review)
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::fs::create_dir_all(&dir).expect("the stand-ins' folder");
    let fresh = dir.join(format!(".{name}-{}-{call}", std::process::id()));
    std::fs::write(&fresh, script).expect("the stand-in");
    std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    // a kept file that is not this text or not executable (a broken earlier write) is replaced
    if std::fs::hard_link(&fresh, &path).is_err() && !ready(&path) {
        std::fs::rename(&fresh, &path).expect("the stand-in in place");
    }
    let _ = std::fs::remove_file(&fresh);
    path
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    /// The same text is the same file, never written again, so macOS scans it once; another text is another file. Never
    /// run here: this text is new each run, and its first run is the scan the helper spares every other test.
    #[crate::dash_test]
    fn a_stand_in_is_written_once_per_text_and_executable() {
        let script = format!("#!/bin/sh\necho {}\n", std::process::id());
        let first = super::stand_in(&script);
        let written = std::fs::metadata(&first).expect("kept");
        let again = super::stand_in(&script);
        assert_eq!(first, again);
        let kept = std::fs::metadata(&again).expect("kept");
        let when = |m: &std::fs::Metadata| m.modified().expect("its time");
        assert_eq!(
            (kept.ino(), when(&kept)),
            (written.ino(), when(&written)),
            "never rewritten"
        );
        let other = super::stand_in(&format!("{script}echo other\n"));
        assert_ne!(other, first);
        let kept = std::fs::metadata(&first).expect("kept");
        assert_eq!(kept.mode() & 0o777, 0o755, "executable");
        assert_eq!(std::fs::read_to_string(&first).expect("its text"), script);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&other);
    }
}
