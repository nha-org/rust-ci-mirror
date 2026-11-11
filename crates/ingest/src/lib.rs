//! Extension of section 3: extractors, each a registry entry (section 11). This one is the compiled-in path: a crate adds
//! its keys with `inventory`, with no central list. The binary only has to link it.
pub mod doc;
pub mod html;

use dash_core::registry::{Compiled, DEFAULT_KEY, Extract};
use std::fs::File;
use std::io::Read;

/// A file's bytes `dash ingest` takes. A read and a compiled-in extractor's copy of a file at this limit fit under the memory cap,
/// with room for the store's call: a test in crates/dash/tests/memory_budgets.rs holds that. A pod's call does not: its JSON can be
/// six times the file, so the cap stops it, unnamed (task rust-reads-bounded-rest).
pub const FILE_LIMIT: u64 = 32 * 1024 * 1024;

/// The file, or a refusal naming it past `FILE_LIMIT`. The read is what stops at the limit, not the size it says: a file can grow
/// after a look, and a pipe says none.
pub fn read_file(file: &str) -> Result<Vec<u8>, String> {
    let read = || {
        let file = File::open(file)?;
        // Room for all of it at once: a read that grows holds up to twice the file.
        let size = file.metadata()?.len().min(FILE_LIMIT) + 1;
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
        Ok::<_, std::io::Error>(bytes)
    };
    let bytes = read().map_err(|e| format!("{file}: {e}"))?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(format!("{file}: past {FILE_LIMIT} bytes, nothing stored"));
    }
    Ok(bytes)
}

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

    #[test]
    fn an_extractor_added_by_a_compiled_in_crate_is_found_by_flat_dispatch() {
        let _budget = TestBudget::new();
        let r = Registry::with_compiled().expect("no key taken twice");
        assert_eq!(r.extract("text/plain", b"hi").unwrap(), "hi");
        assert_eq!(r.extract("text/markdown", b"# hi").unwrap(), "# hi");
        assert_eq!(r.extract("application/octet-stream", b"hi").unwrap(), "hi");
        assert!(
            r.listing()
                .contains(&"extractor text/plain crate dash-ingest".to_string())
        );
    }
}
