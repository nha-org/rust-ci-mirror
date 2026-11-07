//! Tooling, outside the rings of section 3: the memory reading, budgets and cap of section 4. A crate with tests takes it as a
//! dev-dependency; only the binary, which the graph gate exempts, takes it as a dependency, for /memory and its allocator.
pub mod budgets;
pub mod cap;
pub mod memory;

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
