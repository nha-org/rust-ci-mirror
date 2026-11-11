//! A crate's heavy test files, fetched into a cache, never kept in git (task rust-affected-tests; @nha, 2026-11-11: « There is
//! a real risk that the test time balloons with OCR or other data-heavy tasks »). Each crate lists its own in `fixtures.txt`
//! beside its Cargo.toml, a line each: `<sha256> <name> <url>`, `#` a comment. A test asks [`fixture`] for a name and gets a
//! path: the cached copy, else one fetched with curl, its sha256 checked before it lands. The cache is `DASH_FIXTURES`, else
//! the checkout's target/fixtures, which git ignores; a file is named by its hash, so two tests fetching it at once both land
//! the same bytes. ponytail: one cache per checkout, as the sandbox grants no shared folder; DASH_FIXTURES names one later.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The path of `name` from `crate_dir`'s fixtures.txt (a test passes `env!("CARGO_MANIFEST_DIR")`), fetched once.
pub fn fixture(crate_dir: &str, name: &str) -> PathBuf {
    let crate_dir = Path::new(crate_dir);
    let list = crate_dir.join("fixtures.txt");
    let text = std::fs::read_to_string(&list).unwrap_or_else(|e| panic!("{}: {e}", list.display()));
    let (sha, url) = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .find_map(|l| match l.split_whitespace().collect::<Vec<_>>()[..] {
            [sha, n, url] if n == name => Some((sha.to_string(), url.to_string())),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "{name} is not in {}: add `<sha256> {name} <url>`",
                list.display()
            )
        });
    // CARGO_TARGET_DIR when the run names one, as cargo builds there (/code-review)
    let cache = std::env::var_os("DASH_FIXTURES")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CARGO_TARGET_DIR").map(|t| PathBuf::from(t).join("fixtures")))
        .unwrap_or_else(|| crate_dir.join("../../target/fixtures"));
    let path = cache.join(format!("{sha}-{name}"));
    if path.is_file() {
        return path;
    }
    std::fs::create_dir_all(&cache).unwrap_or_else(|e| panic!("{}: {e}", cache.display()));
    // its own name until checked: a cut fetch or a wrong file never reads as the fixture. Per call, not per process: cargo
    // test's threads share a pid, and two fetching one fixture wrote one file (/code-review)
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = cache.join(format!("{sha}-{name}.{}.{call}", std::process::id()));
    let fetched = std::process::Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(&part)
        .arg(&url)
        .status();
    if !fetched.as_ref().is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_file(&part);
        panic!("{name}: curl could not fetch {url} ({fetched:?})");
    }
    let bytes = std::fs::read(&part).unwrap_or_else(|e| panic!("{}: {e}", part.display()));
    let got: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != sha {
        let _ = std::fs::remove_file(&part);
        panic!(
            "{name}: {url} gave sha256 {got}, fixtures.txt holds {sha}: fix the line or the file it names"
        );
    }
    std::fs::rename(&part, &path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    path
}
