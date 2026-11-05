//! The core of section 3: the data model and the ports. It depends on nothing of ours (crates/gate holds that).

/// The ping answer the Elixir dash gives (`dash ping`), the first command the Rust app will answer too.
pub fn ping() -> &'static str {
    "pong"
}

pub struct Scanned {
    pub rows: i64,
    pub bytes: i64,
}

/// The store port: another store is another adapter, and nothing above it changes.
pub trait Store {
    /// One full read, so the whole store passes through its cache.
    fn scan(&self) -> Result<Scanned, String>;
}

#[cfg(test)]
mod tests {
    #[test]
    fn ping_answers_pong() {
        let _budget = dash_meter::memory::TestBudget::new();
        assert_eq!(super::ping(), "pong");
    }
}
