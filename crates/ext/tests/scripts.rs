//! Section 12's « one nextest test per script »: each `.scm` in extensions.d is its own test, in its own process with its
//! own budget, so a new script needs no new Rust. This speaks nextest's custom harness protocol
//! (nexte.st/docs/design/custom-test-harnesses): `--list --format terse`, then `--exact <name> --nocapture`.
use dash_ext::CALL_LIMIT;
use dash_ext::script::{STEEL_TEST_MB, run_tests};
use dash_meter::memory::TestBudget;
use std::path::PathBuf;

fn scripts() -> Vec<PathBuf> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("extensions.d");
    let entries = std::fs::read_dir(&folder).expect("extensions.d");
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "scm"))
        .collect();
    found.sort();
    found
}

fn test_name(script: &std::path::Path) -> String {
    format!(
        "script::{}",
        script.file_name().unwrap_or_default().to_string_lossy()
    )
}

fn passes(script: &std::path::Path) -> bool {
    let _budget = TestBudget::of(STEEL_TEST_MB);
    match run_tests(script, CALL_LIMIT) {
        Ok(passed) => {
            println!("{}: {}", script.display(), passed.join(", "));
            true
        }
        Err(e) => {
            eprintln!("{}: {e}", script.display());
            false
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            scripts()
                .iter()
                .for_each(|s| println!("{}: test", test_name(s)));
        }
        return;
    }
    let wanted = args
        .iter()
        .position(|a| a == "--exact")
        .and_then(|i| args.get(i + 1));
    let chosen: Vec<PathBuf> = scripts()
        .into_iter()
        .filter(|s| wanted.is_none_or(|w| *w == test_name(s)))
        .collect();
    assert!(!chosen.is_empty(), "no script is named {wanted:?}");
    let failed = chosen.iter().filter(|s| !passes(s)).count();
    std::process::exit(i32::from(failed > 0));
}
