//! Section 4's « 50 MB by default » holds for every test: `#[dash_test]` opens each with its TestBudget, so there is no line
//! to forget, and a bare `#[test]`, the one way left to run without it, fails here (task rust-test-budget-attr; three tasks
//! lost a commit to the old « every test needs TestBudget » line on 2026-11-12).
use dash_gate::graph::{Lexeme, lexed};
use dash_gate::rust_files;
use dash_meter::dash_test;
use std::path::Path;

/// How many test attributes with no budget `source` holds as code: `#[test]` under any path (`::core::prelude::v1::test`,
/// `tokio::test`). One in a comment or a string is none (/code-review).
fn bare_tests(source: &str) -> usize {
    let code: Vec<String> = lexed(source)
        .into_iter()
        .map(|l| match l {
            Lexeme::Code(c) => c,
            Lexeme::Str(_) => String::new(),
        })
        .collect();
    let at = |i: usize| code.get(i).map_or("", String::as_str);
    let word =
        |i: usize| at(i).chars().all(|c| c.is_alphanumeric() || c == '_') && !at(i).is_empty();
    (2..code.len())
        .filter(|&i| at(i) == "test" && at(i + 1) == "]")
        .filter(|&i| {
            // back over the path to the attribute's opening
            let mut start = i;
            while start >= 1 && at(start - 1) == "::" {
                start -= 1;
                if start >= 1 && word(start - 1) {
                    start -= 1;
                }
            }
            start >= 2 && at(start - 1) == "[" && at(start - 2) == "#"
        })
        .count()
}

#[dash_test]
fn every_test_has_a_budget() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut bad = vec![];
    for file in rust_files(&crates) {
        let n = bare_tests(&std::fs::read_to_string(&file).expect("read"));
        if n > 0 {
            bad.push(format!("{}: {n}", file.display()));
        }
    }
    assert!(
        bad.is_empty(),
        "write #[dash_test] (`use dash_meter::dash_test;`), never #[test]: it opens the test with its 50 MB budget, \
         #[dash_test(TestBudget::of(<mb>))] for another. Bare #[test]s: {}",
        bad.join("; ")
    );
    // The plants: one bare attribute trips it, a commented or quoted one and the attribute itself do not.
    assert_eq!(
        bare_tests(
            "#[test]\nfn a() {}\n# [ test ]\nfn b() {}\n#[::core::prelude::v1::test]\nfn c() {}\n#[tokio::test]\nfn d() {}"
        ),
        4
    );
    assert_eq!(
        bare_tests("#[cfg(test)]\nmod t {}\nfn f(v: &[u8], test: usize) -> u8 { v[test] }"),
        0
    );
    assert_eq!(
        bare_tests("// #[test]\nconst T: &str = \"#[test]\";\n#[dash_test]\nfn a() {}"),
        0
    );
}
