//! A fixture is fetched into the cache once, checked by its sha256, and never lands when the hash differs.
use dash_meter::TestDir;
use dash_meter::fixtures::fixture;
use dash_meter::memory::TestBudget;

#[test]
fn a_fixture_is_fetched_once_into_the_cache_and_checked() {
    let _budget = TestBudget::new();
    let dir = TestDir::new("fixtures");
    // the cache this test reads is the crate's default: a run naming another would move it (/code-review)
    for key in ["DASH_FIXTURES", "CARGO_TARGET_DIR"] {
        // SAFETY: nextest runs each test in its own process, this thread its only one
        unsafe { std::env::remove_var(key) };
    }
    let source = dir.0.join("source.pdf");
    std::fs::write(&source, "%PDF-1.4 a page").expect("source");
    let krate = dir.0.join("crates/pdf");
    std::fs::create_dir_all(&krate).expect("crate");
    let real: String = {
        use sha2::Digest;
        sha2::Sha256::digest(b"%PDF-1.4 a page")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let url = format!("file://{}", source.display());
    std::fs::write(
        krate.join("fixtures.txt"),
        format!(
            "# a comment\n{real} page.pdf {url}\n{} wrong.pdf {url}\n",
            "0".repeat(64)
        ),
    )
    .expect("list");
    let crate_dir = krate.to_string_lossy();
    let path = fixture(&crate_dir, "page.pdf");
    assert_eq!(
        path,
        krate.join(format!("../../target/fixtures/{real}-page.pdf"))
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("cached"),
        "%PDF-1.4 a page"
    );
    // the second ask reads the cache: the source is gone
    std::fs::remove_file(&source).expect("rm source");
    assert_eq!(
        fixture(&crate_dir, "page.pdf"),
        path,
        "the cached copy, never a second fetch"
    );
    // a cached copy changed since its fetch is fetched anew, never read
    std::fs::write(&source, "%PDF-1.4 a page").expect("source");
    std::fs::write(&path, "%PDF-1.4 cut").expect("a cut copy");
    assert_eq!(
        std::fs::read_to_string(fixture(&crate_dir, "page.pdf")).expect("refetched"),
        "%PDF-1.4 a page",
        "the bytes its sha256 names"
    );
    // a hash that differs refuses, and nothing lands under its name
    let refused = std::panic::catch_unwind(|| fixture(&crate_dir, "wrong.pdf"));
    assert!(refused.is_err(), "a wrong sha256 must refuse the file");
    let left: Vec<_> = std::fs::read_dir(path.parent().expect("cache"))
        .expect("cache")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("wrong"))
        .collect();
    assert_eq!(
        left,
        Vec::<String>::new(),
        "a refused fetch leaves no file in the cache. Found: {left:?}"
    );
}
