//! The Rust app of rfc-rust-unified: the tracer of section 17, grown by the memory harness of section 4.
pub mod budgets;
pub mod memory;
pub mod store;

/// The ping answer the Elixir dash gives (`dash ping`), the first command the Rust app will answer too.
pub fn ping() -> &'static str {
    "pong"
}

#[cfg(test)]
mod tests {
    #[test]
    fn ping_answers_pong() {
        let _budget = crate::memory::TestBudget::new();
        assert_eq!(super::ping(), "pong");
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
