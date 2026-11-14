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
/// `from_millis(300) > took`), a name bound to one, or a duration read as a number (`as_millis()`, `as_secs_f64() * 1000.0`,
/// or a name bound to one, the store's old `at(100) < 200.0`) under a literal worth less than a second. A lower bound
/// (`>= from_millis(50)`) is safe on a slow runner, so it passes. ponytail: a bound computed (`from_millis(3 * 100)`), passed
/// to a helper or a timeout is not seen; a nested fn's assert reads the nested fn's attributes, a false refusal its message
/// names.
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
    // A number literal at `k`, its type suffix dropped (`500`, `200.0`, `0.5f64`, `1e3`, `500u128`), with the tokens it takes.
    let number = |k: usize| {
        let bare = |t: &str| {
            let t = t.replace('_', "");
            let suffixes = [
                "u128", "u64", "u32", "u16", "u8", "usize", "i128", "i64", "i32", "i16", "i8",
                "isize", "f64", "f32",
            ];
            (suffixes.iter())
                .find_map(|x| {
                    t.strip_suffix(x)
                        .filter(|n| !n.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or(t)
        };
        let whole = bare(at(k)).parse::<f64>().ok()?;
        let frac = (at(k + 1) == ".")
            .then(|| bare(at(k + 2)))
            .filter(|f| f.chars().all(|c| c.is_ascii_digit()) && !f.is_empty());
        match frac {
            Some(f) => Some((format!("{}.{f}", bare(at(k))).parse::<f64>().ok()?, 3)),
            None => Some((whole, 1)),
        }
    };
    // The milliseconds one unit of a duration read as a number is worth, in `range`'s tokens: `as_millis()` 1, `as_micros()`
    // 0.001, `as_secs_f64()` 1000, each `* N` after it dividing it by N and each `/ N` multiplying it (`as_secs_f64() * 1000.0` is 1,
    // `as_millis() as f64 / 1000.0` is 1000). ponytail: the first `as_*` and every literal factor in the range count, whichever
    // term they belong to (`a.as_secs_f64() + b * 1000.0` reads as ms); split such a sum when the rule misreads one.
    let unit_of = |range: std::ops::Range<usize>| {
        let (at_, mut unit) = range.clone().find_map(|k| match at(k) {
            "as_millis" => Some((k, 1.0)),
            "as_micros" => Some((k, 1e-3)),
            "as_nanos" => Some((k, 1e-6)),
            "as_secs_f64" | "as_secs_f32" => Some((k, 1e3)),
            _ => None,
        })?;
        // the factors after the conversion: an index's `* q / 100` before it scales no duration
        for k in at_..range.end {
            match (at(k), number(k + 1)) {
                ("*", Some((n, _))) if n != 0.0 => unit /= n,
                ("/", Some((n, _))) => unit *= n,
                _ => {}
            }
        }
        Some(unit)
    };
    // a `let` bound to a duration read as a number, with where and its unit: `let at = |q| took[q].as_secs_f64() * 1000.0;`.
    // ponytail: a plain name only; `if let Some(ms) = ..` or a tuple pattern is not seen
    let read_names: Vec<(usize, &str, f64)> = (0..code.len())
        .filter(|&j| at(j) == "let")
        .filter_map(|j| {
            let name = if at(j + 1) == "mut" { j + 2 } else { j + 1 };
            let eq = (name..code.len()).find(|&k| matches!(at(k), "=" | ";"))?;
            let mut depth = 0;
            let end = (eq..code.len()).find(|&k| {
                match at(k) {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
                depth < 0 || (depth == 0 && at(k) == ";")
            })?;
            let unit = (at(eq) == "=").then(|| unit_of(eq..end)).flatten()?;
            Some((name, at(name), unit))
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
            // the side bounded above and the side bounding it, past a `<=`'s `=`
            let after = if at(op + 1) == "=" { op + 2 } else { op + 1 };
            let (low, high) = if at(op) == "<" {
                (cond.start..op, after..cond.end)
            } else {
                (after..cond.end, cond.start..op)
            };
            // a duration read as a number under a bare literal worth less than a second: `took.as_millis() as u64 < 500`,
            // `at(100) < 200.0` where `at` reads one
            // the whole bounding side one literal, or one cast: `500`, `200.0`, `0.5f64`, `500 as u128`
            let literal = number(high.start).and_then(|(n, took)| {
                let rest = high.end.saturating_sub(high.start + took);
                (rest == 0 || (rest == 2 && at(high.start + took) == "as")).then_some(n)
            });
            let unit = unit_of(low.clone()).or_else(|| {
                low.clone().find_map(|j| {
                    (at(j.saturating_sub(1)) != ".")
                        .then(|| {
                            read_names
                                .iter()
                                .find(|&&(p, n, _)| n == at(j) && (f..i).contains(&p))
                        })
                        .flatten()
                        .map(|&(_, _, u)| u)
                })
            });
            if let (Some(n), Some(u)) = (literal, unit)
                && n * u < 1000.0
            {
                return true;
            }
            cond.clone().any(|j| {
                let bound = sub_second_at(&code, j).is_some() || bound_name(j);
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
        "assert!(took.as_secs_f64() * 1000.0 < 200.0);",
        "assert!(took.as_secs_f64() < 0.5);",
        "assert!(200 > took.as_millis());",
        // the store's load test before task rust-load-test-no-wall-clock: a closure reads each write's time in ms
        "let at = |q: usize| took[(took.len() - 1) * q / 100].as_secs_f64() * 1000.0; assert!(at(100) < 200.0, \"{line}\");",
        "assert!(took.as_millis() < 500 as u128);",
        "assert!(took.as_millis() < 200u128);",
        "assert!(took.as_secs_f64() < 0.5f64);",
        "assert!(took.as_secs_f64() * 1e6 < 500_000.0);",
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
        "assert!(took.as_secs_f64() < 2.0);",
        "assert!(took.as_secs_f64() * 1000.0 < 1500.0);",
        "assert!(took.as_millis() > 200);",
        "let at = |q: usize| took[q].as_secs_f64() * 1000.0; assert!(at(100) >= 200.0);",
        "let n = rows.len(); assert!(n < 200);",
        "assert!(took.as_millis() as f64 / 1000.0 < 2.0);",
        "assert!(took.as_millis() < 1500u128);",
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
