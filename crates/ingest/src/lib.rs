//! Extension of section 3: extractors, each a registry entry (section 11). This one is the compiled-in path: a crate adds
//! its keys with `inventory`, with no central list. The binary only has to link it.
use dash_core::registry::{Compiled, DEFAULT_KEY, Extract};

struct Text;

impl Extract for Text {
    fn extract(&self, input: &[u8]) -> Result<String, String> {
        // A byte that is not text refuses the file: a lossy copy would be stored for good and searched.
        std::str::from_utf8(input)
            .map(str::to_owned)
            .map_err(|e| format!("not text: {e}"))
    }
}

inventory::submit! {
    Compiled {
        origin: "crate dash-ingest",
        keys: &["text/plain", "text/markdown", DEFAULT_KEY],
        extractor: || Box::new(Text),
    }
}

#[cfg(test)]
mod tests {
    use dash_core::registry::Registry;
    use dash_meter::memory::TestBudget;

    #[test]
    fn its_keys_are_in_the_registry_with_no_central_list() {
        let _budget = TestBudget::new();
        let r = Registry::with_compiled().expect("no key taken twice");
        assert_eq!(
            r.listing(),
            [
                "extractor * crate dash-ingest",
                "extractor text/markdown crate dash-ingest",
                "extractor text/plain crate dash-ingest",
            ]
        );
        assert_eq!(r.extract("text/plain", b"hi").unwrap(), "hi");
        assert!(r.extract("image/png", b"hi \xff").is_err(), "not text");
        assert_eq!(r.extract("image/png", b"hi").unwrap(), "hi", "the default");
    }
}
