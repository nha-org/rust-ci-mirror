//! The Rust lexer the gate's source rules read (graph re-exports it for html, inline_js, build_paths and dash's codemap), and
//! the paths outside rust/ the gate's tests read. A leaf: it uses no other module, so a module in the crate's loop may take
//! it without joining graph to the loop (task rust-gate-reads-trigger; feature_loops_only_go_down).
use std::collections::BTreeSet;
use std::path::Path;

/// The paths outside rust/ dash-gate's tests read, so the pre-commit's rust-gate runs on a commit of one of them alone
/// (636d9230f doubled the port map's surfaces.rs row in private/docs and shipped red, task rust-gate-reads-trigger): each
/// string literal of `sources` (the gate's .rs files, their paths under `top`, lexed by [`lexed`]: comments out,
/// raw strings whole, a literal holding an escape dropped) that names a file or folder outside rust/, its leading `{top}/`
/// and `../` and anything from a later `{` placeholder dropped. A folder is kept (the hook reads its own entries), never a
/// top-level one alone or one written with a trailing `/`, both prefix checks; a path
/// `exists` denies (a scratch fixture), an absolute one or one climbing with `..` is left out. The hook passes every path
/// and matches its staged names, so a missing one costs nothing there. Read from the gate's text, never a hand list; src/ too, since its helpers
/// read for the tests (ratchet's port order). The whole-tree lint test reads more, which the pre-commit's lint step checks.
/// ponytail: a path the tests build in pieces is not seen; one helper recording each read would see it.
pub fn reads_past_rust(
    top: &Path,
    sources: &[String],
    exists: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let path = |lit: &str| {
        // a leading `{top}/` is the folder the path is under: what follows it is the path; any other `/` starts an absolute one
        let lit = match lit.strip_prefix('{') {
            Some(rest) => rest.split_once("}/").map_or("", |(_, path)| path),
            None => lit,
        };
        // a trailing `/` as written is a prefix check (`starts_with("private/docs/")`), never a read (/code-review of #1624);
        // one left by a cut placeholder (`bin/ci/{name}.sh`) is its folder's file
        if lit.ends_with('/') {
            return None;
        }
        let lit = lit.split('{').next().unwrap_or_default();
        let lit = lit.trim_start_matches("../");
        let lit = lit.trim_end_matches('/');
        let first = lit.split('/').next().unwrap_or_default();
        let ok = lit
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
            && lit.split('/').all(|p| !matches!(p, "" | "." | ".."))
            && ![".git", "rust"].contains(&first)
            // a top-level folder alone is a prefix check (`starts_with("private/")`), never a read: every commit would run it
            && !(lit == first && top.join(lit).is_dir());
        ok.then(|| lit.to_string())
    };
    let mut found = BTreeSet::new();
    for file in sources {
        let text = std::fs::read_to_string(top.join(file)).unwrap_or_default();
        found.extend(lexed(&text).iter().filter_map(|l| match l {
            Lexeme::Str(s) => path(s),
            Lexeme::Code(_) => None,
        }));
    }
    // each path probed once: `exists` may start a git per call
    found.into_iter().filter(|p| exists(p)).collect()
}

/// A token of code, or what a string literal holds.
#[derive(Clone)]
pub enum Lexeme {
    Code(String),
    Str(String),
}

/// The source's tokens and string literals in their order, comments and char literals gone.
pub fn lexed(source: &str) -> Vec<Lexeme> {
    let s: Vec<char> = source.chars().collect();
    let (mut out, mut i) = (vec![], 0);
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    while i < s.len() {
        let next = s.get(i + 1).copied();
        match s[i] {
            '/' if next == Some('/') => {
                while i < s.len() && s[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                let mut depth = 1;
                i += 2;
                while i < s.len() && depth > 0 {
                    match (s[i], s.get(i + 1)) {
                        ('/', Some('*')) => (depth, i) = (depth + 1, i + 2),
                        ('*', Some('/')) => (depth, i) = (depth - 1, i + 2),
                        _ => i += 1,
                    }
                }
            }
            '"' => {
                i += 1;
                let start = i;
                while i < s.len() && s[i] != '"' {
                    i += if s[i] == '\\' { 2 } else { 1 };
                }
                out.push(Lexeme::Str(s[start..i.min(s.len())].iter().collect()));
                i += 1;
            }
            // '\n' and '{' are literals; the quote of a lifetime is dropped and its name is a word like any other.
            '\'' if next == Some('\\') => {
                i += 3; // past the escaped character, a quote included
                while i < s.len() && s[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            '\'' if s.get(i + 2) == Some(&'\'') => i += 3,
            // `..` is its own token: `0..crate::a::N` starts a path, where `v.b::<u8>()` does not.
            c @ (':' | '.') if next == Some(c) => {
                out.push(Lexeme::Code(format!("{c}{c}")));
                i += 2;
            }
            c if word_char(c) => {
                let start = i;
                while i < s.len() && word_char(s[i]) {
                    i += 1;
                }
                let word: String = s[start..i].iter().collect();
                let hashes = s[i..].iter().take_while(|c| **c == '#').count();
                // A raw identifier is its name: `mod r#type;` is type.rs.
                if word == "r" && hashes == 1 && s.get(i + 1).is_some_and(|c| word_char(*c)) {
                    i += 1;
                    continue;
                }
                if matches!(word.as_str(), "r" | "br" | "cr") && s.get(i + hashes) == Some(&'"') {
                    let end = ["\"".to_string(), "#".repeat(hashes)].concat();
                    let end: Vec<char> = end.chars().collect();
                    i += hashes + 1;
                    let start = i;
                    while i < s.len() && !s[i..].starts_with(&end) {
                        i += 1;
                    }
                    out.push(Lexeme::Str(s[start..i.min(s.len())].iter().collect()));
                    i += end.len();
                }
                out.push(Lexeme::Code(word));
            }
            c if c.is_whitespace() => i += 1,
            c => {
                out.push(Lexeme::Code(c.to_string()));
                i += 1;
            }
        }
    }
    out
}
