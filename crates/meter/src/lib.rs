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
/// The crate's folder comes from the run (cargo sets CARGO_MANIFEST_DIR for each test it runs), never only from the build:
/// sccache hands every worktree the same compiled meter, its env! naming the worktree that built it first, so on 2026-11-12
/// every test here read a removed worktree's path and stopped as if on the mirror (task rust-merge-gate-port).
pub fn monorepo() -> Option<std::path::PathBuf> {
    let crate_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into());
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
