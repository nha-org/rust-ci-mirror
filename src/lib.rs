//! The tracer bullet of rfc-rust-unified section 17: one crate, one test, one green run on the mirror's CI.

/// The ping answer the Elixir dash gives (`dash ping`), the first command the Rust app will answer too.
pub fn ping() -> &'static str {
    "pong"
}

#[cfg(test)]
mod tests {
    #[test]
    fn ping_answers_pong() {
        assert_eq!(super::ping(), "pong");
    }
}
