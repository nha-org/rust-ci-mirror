//! Section 4's « 50 MB by default » holds only for a test that declares its budget, so a test with none fails here.
use dash::memory::TestBudget;
use std::path::Path;

fn sources(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

// ponytail: counts per file, so a test with two guards hides one with none; a #[test] wrapper macro if that ever bites.
#[test]
fn every_test_declares_a_budget() {
    let _budget = TestBudget::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![];
    sources(&root.join("src"), &mut files);
    sources(&root.join("tests"), &mut files);
    let mut bad = vec![];
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read");
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect();
        let (tests, budgets) = (
            code.matches("#[test]").count(),
            code.matches("let _budget = ").count(),
        );
        if tests > budgets {
            bad.push(format!(
                "{}: {tests} tests, {budgets} budgets",
                file.display()
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "a test with no TestBudget: {}",
        bad.join("; ")
    );
}
