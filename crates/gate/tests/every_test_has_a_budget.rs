//! Section 4's « 50 MB by default » holds for every test: `#[dash_test]` opens each with its TestBudget, so there is no line
//! to forget, and a bare `#[test]`, the one way left to run without it, fails here (task rust-test-budget-attr; three tasks
//! lost a commit to the old « every test needs TestBudget » line on 2026-11-12).
use dash_gate::graph::{Lexeme, lexed};
use dash_gate::rust_files;
use dash_meter::dash_test;
use std::path::Path;

/// `source`'s code tokens, a string literal an empty one, a comment none (/code-review: one copy for both rules).
fn code_tokens(source: &str) -> Vec<String> {
    lexed(source)
        .into_iter()
        .map(|l| match l {
            Lexeme::Code(c) => c,
            Lexeme::Str(_) => String::new(),
        })
        .collect()
}

/// How many test attributes with no budget `source` holds as code: `#[test]` under any path (`::core::prelude::v1::test`,
/// `tokio::test`). One in a comment or a string is none (/code-review).
fn bare_tests(source: &str) -> usize {
    let code = code_tokens(source);
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

/// Under a second, in ms, a duration's tokens from `i`: `Duration::from_millis(300)`, `from_micros`, `from_nanos`,
/// `from_secs_f64(0.5)`, or `as_millis()`'s bare number after a `<`. None for a second or more, or for a name.
fn sub_second_at(code: &[String], i: usize) -> Option<f64> {
    let at = |j: usize| code.get(j).map_or("", String::as_str);
    let num = |s: &str| s.replace('_', "").parse::<f64>().ok();
    let ms = match (at(i), at(i + 1)) {
        ("from_millis", "(") if at(i + 3) == ")" => num(at(i + 2))?,
        ("from_micros", "(") if at(i + 3) == ")" => num(at(i + 2))? / 1e3,
        ("from_nanos", "(") if at(i + 3) == ")" => num(at(i + 2))? / 1e6,
        ("from_secs_f64" | "from_secs_f32", "(") if at(i + 3) == "." => {
            num(&format!("{}.{}", at(i + 2), at(i + 4)))? * 1e3
        }
        _ => return None,
    };
    (ms < 1000.0).then_some(ms)
}

/// The functions in `source` that assert an elapsed time under a second with no `#[wall_budget(..)]` among their attributes:
/// an `assert!` whose condition, clause by clause, bounds a value above by a sub-second duration (`took < from_millis(300)`,
/// `from_millis(300) > took`), a name bound to one, or an `as_millis() <` under 1000. A lower bound (`>= from_millis(50)`) is
/// safe on a slow runner, so it passes. ponytail: a bound computed (`from_millis(3 * 100)`), passed to a helper or a timeout
/// is not seen; a nested fn's assert reads the nested fn's attributes, a false refusal its message names.
fn unnamed_wall_bounds(source: &str) -> Vec<String> {
    let code = code_tokens(source);
    let at = |i: usize| code.get(i).map_or("", String::as_str);
    // a name bound to one, with where: `let limit = Duration::from_millis(300);`, `const BUDGET: Duration = ..` (/code-review)
    let names: Vec<(usize, &str, bool)> = (0..code.len())
        .filter(|&j| sub_second_at(&code, j).is_some())
        .filter_map(|j| {
            let eq = (0..j).rev().find(|&k| {
                !(at(k) == "::" || at(k).chars().all(|c| c.is_alphanumeric() || c == '_'))
            })?;
            let name = if at(eq.saturating_sub(2)) == ":" {
                eq.saturating_sub(3)
            } else {
                eq.saturating_sub(1)
            };
            let kw = at(name.saturating_sub(1));
            (at(eq) == "=" && matches!(kw, "let" | "mut" | "const" | "static"))
                .then(|| (name, at(name), matches!(kw, "const" | "static")))
        })
        .collect();
    // a named fn, never a `fn(..)` pointer type
    let named = |j: usize| {
        at(j) == "fn"
            && at(j + 1).chars().all(|c| c.is_alphanumeric() || c == '_')
            && !at(j + 1).is_empty()
    };
    let mut bad = vec![];
    for i in 0..code.len() {
        if !(matches!(at(i), "assert" | "debug_assert") && at(i + 1) == "!" && at(i + 2) == "(") {
            continue;
        }
        // the condition: up to the first comma at the call's own depth
        let (mut depth, mut end) = (0, i + 3);
        while end < code.len() {
            match at(end) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" if depth == 0 => break,
                ")" | "]" | "}" => depth -= 1,
                "," if depth == 0 => break,
                _ => {}
            }
            end += 1;
        }
        let Some(f) = (0..i).rev().find(|&j| named(j)) else {
            continue;
        };
        // a `let` of the assert's own fn, a const anywhere; never a field (`PACE.stuck_limit` is no `let stuck_limit`)
        let bound_name = |j: usize| {
            at(j - 1) != "."
                && names
                    .iter()
                    .any(|&(at_, n, item)| n == at(j) && (item || (f..i).contains(&at_)))
        };
        // each clause of the condition on its own: `v.len() > 0 && took < from_millis(300)` hides no bound (/code-review)
        let mut clauses = vec![];
        let (mut from, mut depth) = (i + 3, 0);
        for j in i + 3..=end {
            match at(j) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                "&" | "|" if depth == 0 && at(j + 1) == at(j) => {
                    clauses.push(from..j);
                    from = j + 2;
                }
                _ => {}
            }
        }
        clauses.push(from..end);
        let upper = clauses.into_iter().any(|cond| {
            // a comparison, never `<<`, `->`, `=>` or a turbofish's `::<`
            let op = cond.clone().find(|&j| {
                matches!(at(j), "<" | ">")
                    && at(j + 1) != at(j)
                    && !matches!(at(j - 1), "<" | ">" | "-" | "=" | "::")
            });
            let Some(op) = op else { return false };
            // `as_millis() < 500`, or `as_millis() as u64 < 500`
            let millis = at(op.saturating_sub(3)) == "as_millis"
                || (at(op.saturating_sub(5)) == "as_millis" && at(op.saturating_sub(2)) == "as");
            cond.clone().any(|j| {
                let bound = sub_second_at(&code, j).is_some()
                    || bound_name(j)
                    || (j > op
                        && at(op) == "<"
                        && millis
                        && at(j)
                            .replace('_', "")
                            .parse::<u64>()
                            .is_ok_and(|n| n < 1000));
                bound && ((at(op) == "<" && j > op) || (at(op) == ">" && j < op))
            })
        });
        if !upper {
            continue;
        }
        // the item's attributes: back from `fn` to the end of what came before it
        let start = (0..f)
            .rev()
            .find(|&j| matches!(at(j), "}" | ";" | "{"))
            .map_or(0, |j| j + 1);
        if !(start..f).any(|j| at(j) == "wall_budget") {
            bad.push(at(f + 1).to_string());
        }
    }
    bad.dedup();
    bad
}

/// Task rust-store-rebuild-write-flake, done-when 3: a test asserting an elapsed time under 1 s names its budget with
/// `#[dash_meter::wall_budget("<why a shared runner meets it>")]`, or this fails. Such bounds reddened PRs 1287, 1476, 1496,
/// 1498, 1510 and 1525 on a shared runner; a bound worth keeping says why, and a check measures what it guards instead
/// (bytes read, ops waited, a memory budget) where it can.
#[dash_test]
fn a_sub_second_wall_bound_names_its_budget() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut bad = vec![];
    for file in rust_files(&crates) {
        let found = unnamed_wall_bounds(&std::fs::read_to_string(&file).expect("read"));
        if !found.is_empty() {
            bad.push(format!("{}: {}", file.display(), found.join(", ")));
        }
    }
    assert!(
        bad.is_empty(),
        "a test asserting an elapsed time under 1 s fails on a busy shared runner: measure what it guards (bytes read, ops \
         waited, its memory budget) with a loose wall-clock ceiling, or name its budget with \
         #[dash_meter::wall_budget(\"<the bound and why a shared runner meets it>\")]. Unnamed: {}",
        bad.join("; ")
    );
    // The plants: each upper bound under a second trips it; a named one, a lower bound, a second or more and a quote do not.
    let t = |body: &str| unnamed_wall_bounds(&format!("#[dash_test]\nfn t() {{ {body} }}"));
    for tripped in [
        "assert!(took < Duration::from_millis(300), \"{took:?}\");",
        "assert!(start.elapsed() <= Duration::from_millis(999));",
        "assert!(Duration::from_millis(200) > took);",
        "assert!(took < std::time::Duration::from_secs_f64(0.5));",
        "assert!(took < Duration::from_micros(500_000));",
        "assert!(took.as_millis() < 500, \"slow\");",
        "debug_assert!(at.elapsed() < Duration::from_nanos(1_000));",
        "assert!(v.len() > 0 && took < Duration::from_millis(300));",
        "assert!(a < b && Duration::from_millis(200) > took);",
        "let limit = Duration::from_millis(300); assert!(took < limit);",
        "const BUDGET: Duration = Duration::from_millis(200); assert!(took < BUDGET);",
        "assert!(took.as_millis() as u64 < 500);",
    ] {
        assert_eq!(t(tripped), ["t"], "{tripped}");
    }
    for passed in [
        "assert!(took < Duration::from_secs(5));",
        "assert!(took < Duration::from_millis(1000));",
        "assert!(took.as_millis() < 2000);",
        "assert!(queued.elapsed() >= before + Duration::from_millis(50));",
        "assert!(waited(&s, Duration::from_millis(200)).is_ok());",
        "assert_eq!(waited(&s, Duration::from_millis(200)), Ok(None));",
        "// assert!(took < Duration::from_millis(300));",
        "let s = \"assert!(took < Duration::from_millis(300))\";",
        "assert!(v.len() < 3, \"{:?}\", Duration::from_millis(5));",
        "let poll = Duration::from_millis(50); assert!(took >= poll);",
        "assert!(f::<u8>() > 0 || took < Duration::from_secs(2));",
        "let limit = Duration::from_millis(9); assert!(took < PACE.limit);",
    ] {
        assert!(
            t(passed).is_empty(),
            "a clean plant trips the rule: narrow unnamed_wall_bounds to upper bounds under 1 s in code: {passed}"
        );
    }
    assert!(
        unnamed_wall_bounds(
            "fn a() { let limit = Duration::from_millis(9); }\nfn b() { assert!(took < limit); }"
        )
        .is_empty(),
        "another fn's let trips the rule: read a let in the assert's own fn alone"
    );
    let named = "#[dash_test]\n#[dash_meter::wall_budget(\"/tasks's page budget\")]\nfn t() { \
                 assert!(took < Duration::from_millis(300)); }";
    assert!(
        unnamed_wall_bounds(named).is_empty(),
        "a named bound trips the rule: read #[wall_budget] among the fn's attributes"
    );
}
