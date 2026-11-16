//! The build-path rule (task rust-no-build-path-at-runtime), a module of its own: it reads [`graph::lexed`], and graph
//! reads the crate root, so the root holding it would be a module loop.
use crate::graph::{self, Lexeme};
use std::path::Path;

/// Each tracked .rs file that bakes the build's crate folder into code run outside a test, with its count: `env!` or
/// `option_env!` of `CARGO_MANIFEST_DIR`. A test is a file under tests/ or benches/, a tests.rs, a file under `#![cfg(test)]`
/// or an item under a `#[cfg(...)]` naming `test` and no `not`. Vendored crates are theirs. sccache hands every worktree the
/// same compiled crate, its path naming whichever worktree built it first: dash-meter's sent every worktree's tests to a
/// removed one, where they stopped as if on the mirror (rust-merge-gate-port, 4ad1f3f16, 2026-11-12). Fix: read the path at
/// run time (`std::env::var_os`, the binary's folder). Read on [`graph::lexed`]'s tokens, so comments and strings never
/// count (/code-review of PR 1534: the line reader missed `cfg(all(test, ..))` and a `{ // why` opener).
pub fn build_paths_at_run_time(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let mut bad = vec![];
    for (_, file) in files {
        let test_file = file
            .split('/')
            .any(|p| matches!(p, "tests" | "benches" | "tests.rs" | "vendor"));
        if !file.ends_with(".rs") || test_file {
            continue;
        }
        let text =
            std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let lex = graph::lexed(&text);
        let code = |i: usize| match lex.get(i) {
            Some(Lexeme::Code(c)) => c.as_str(),
            _ => "",
        };
        // the `]` closing the attribute whose `[` is at `open`, and whether it is a cfg naming test and no not
        let attribute = |open: usize| {
            let (mut depth, mut j) = (0, open);
            while j < lex.len() {
                match code(j) {
                    "[" => depth += 1,
                    "]" if depth == 1 => break,
                    "]" => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            let words: Vec<&str> = (open + 1..j).map(code).collect();
            let test =
                words.first() == Some(&"cfg") && words.contains(&"test") && !words.contains(&"not");
            (j, test)
        };
        let (mut uses, mut depth, mut i) = (0, 0, 0);
        while i < lex.len() {
            match code(i) {
                "#" if code(i + 1) == "!" && code(i + 2) == "[" => {
                    let (end, test) = attribute(i + 2);
                    // the file's own, before any brace: the whole file is a test
                    if test && depth == 0 {
                        uses = 0;
                        break;
                    }
                    i = end;
                }
                "#" if code(i + 1) == "[" => {
                    let (end, test) = attribute(i + 1);
                    i = end;
                    if test {
                        // the item: to a `;` or `,` at its own level, or its block's closing `}`; a `}` or `)` it did not
                        // open ends it unread (a last field)
                        let mut level = 0;
                        while i + 1 < lex.len() {
                            i += 1;
                            match code(i) {
                                "{" | "(" | "[" => level += 1,
                                "}" | ")" | "]" if level == 0 => {
                                    i -= 1;
                                    break;
                                }
                                "}" if level == 1 && code(i + 1) != "," => break,
                                "}" | ")" | "]" => level -= 1,
                                ";" | "," if level == 0 => break,
                                _ => {}
                            }
                        }
                    }
                }
                "{" => depth += 1,
                "}" => depth -= 1,
                "env" | "option_env"
                    if code(i + 1) == "!"
                        && code(i + 2) == "("
                        && matches!(lex.get(i + 3), Some(Lexeme::Str(s)) if s == concat!("CARGO_", "MANIFEST_DIR")) =>
                {
                    uses += 1
                }
                _ => {}
            }
            i += 1;
        }
        if uses > 0 {
            bad.push(format!(
                "{file}: {uses} uses of the build's crate folder at run time; read it when the code runs"
            ));
        }
    }
    bad
}
