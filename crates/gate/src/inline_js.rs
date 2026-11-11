//! Our inline scripts in rust/crates, whose bytes only go down to none (task rust-components-no-js; @nha, 2026-11-11: « we
//! want pure rust components as much as possible. », « I thought we did not use JS / TS at all ? »). A component's
//! behaviour is Rust, Dioxus handlers run natively in the window and through dioxus-liveview on the web (rust/CLAUDE.md
//! rule 6, rfc-rust-unified#ui-lib). Each script is a `const <NAME>_JS: &str`, a literal or an `include_str!` of a .js file,
//! the comment above it naming what blocks its port after [`MARK`] with a link; a page puts one on only by that name, a
//! `script` element's `dangerous_inner_html` or a script tag holding `{NAME_JS}` in a format string. Their bytes, as the
//! source holds them, are counted against the one number in [`HELD`]: more fails, fewer fails until lowered in the same
//! commit, and `dash scripts count --check --base` refuses the number rising past its base's (ratchet.rs).
//! ponytail: it reads the source by tokens, so a script built at run time (`format!`, a variable) passes; an rsx parse if one
//! slips by.
use crate::graph::{Lexeme, lexed};
use crate::rust_files;
use std::path::Path;

/// The held byte count, from rust/.
pub const HELD: &str = "inline-js-held.txt";

/// What the comment above each script says before what blocks its port.
pub const MARK: &str = "Blocks its port:";

/// Built with concat!: the gate's own source holds no tag literal (html::html_in_strings).
const OPEN: &str = concat!("<", "script");
const CLOSE: &str = concat!("<", "/script>");

/// One script: where its const is, its name and its bytes.
#[derive(Debug, PartialEq)]
pub struct Script {
    pub at: String,
    pub name: String,
    pub bytes: usize,
}

fn script_name(name: &str) -> bool {
    name.ends_with("_JS") && name.len() > 3
}

/// The extensions of a script file, which only a NAME_JS const may bring in.
const SCRIPT_FILES: [&str; 4] = ["js", "mjs", "cjs", "ts"];

/// Every script under `root`'s crates and what breaks the rule: a script with no blocker named, a value neither a literal nor
/// an `include_str!`, a page's script put on by anything but a counted script's name, a script file no script includes. A
/// tests/ folder and each inline `#[cfg(test)] mod` are not read: a test's own markup is no page.
pub fn inline_scripts(root: &Path) -> (Vec<Script>, Vec<String>) {
    let (mut scripts, mut bad, mut included, mut put_on) = (vec![], vec![], vec![], vec![]);
    let mut files = rust_files(&root.join("crates"));
    files.sort();
    for file in files {
        let name = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        if name.contains("/tests/") {
            continue;
        }
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(e) => {
                bad.push(format!("{name}: {e}"));
                continue;
            }
        };
        let lex = without_test_modules(lexed(&source));
        let code = |i: usize| match lex.get(i) {
            Some(Lexeme::Code(c)) => c.as_str(),
            _ => "",
        };
        for i in 0..lex.len() {
            // a script: `const NAME_JS: &str = <literal or include_str!("x.js")>`, a static too
            if ["const", "static"].contains(&code(i)) && script_name(code(i + 1)) {
                let id = code(i + 1).to_string();
                // a declaration with no value (a trait's) is no script: its impl's const is
                let Some(eq) = (i..lex.len()).find(|&j| ["=", ";"].contains(&code(j))) else {
                    continue;
                };
                if code(eq) == ";" {
                    continue;
                }
                let raw = ["r", "br", "cr"].contains(&code(eq + 2));
                let bytes = match (lex.get(eq + 1), lex.get(eq + 4)) {
                    (Some(Lexeme::Str(s)), _) => Some(if raw { s.len() } else { unescaped_len(s) }),
                    (_, Some(Lexeme::Str(p))) if code(eq + 1) == "include_str" => {
                        let js = file.parent().unwrap_or(root).join(p);
                        included.push(std::fs::canonicalize(&js).unwrap_or(js.clone()));
                        std::fs::metadata(&js).map(|m| m.len() as usize).ok()
                    }
                    _ => None,
                };
                match bytes {
                    Some(bytes) => scripts.push(Script {
                        at: name.clone(),
                        name: id.clone(),
                        bytes,
                    }),
                    None => bad.push(format!(
                        "{name}: {id} is neither a literal nor an include_str! of a file there"
                    )),
                }
                if !names_its_blocker(&source, &id) {
                    bad.push(format!(
                        "{name}: {id}: the comment above it names what blocks its port, « {MARK} the Dioxus or Blitz gap, its https link »"
                    ));
                }
            }
            // a page's script element, after any token: `script { dangerous_inner_html: NAME_JS }`, a path to it allowed
            if code(i) == "script" && code(i + 1) == "{" {
                let value: Vec<&str> = (i + 4..lex.len())
                    .map(code)
                    .take_while(|t| !["}", ","].contains(t))
                    .collect();
                let last = value.last().copied().unwrap_or("");
                if code(i + 2) != "dangerous_inner_html" || code(i + 3) != ":" || !script_name(last)
                {
                    bad.push(format!(
                        "{name}: a script element puts on {:?}, never a NAME_JS script",
                        value.concat()
                    ));
                } else {
                    put_on.push((name.clone(), last.to_string()));
                }
            }
            // a page's script in a format string: the tag holding `{NAME_JS}` alone, any case.
            // ponytail: a plain string passes too, and ships the braces as text; a format! parse if one does.
            if let Some(Lexeme::Str(s)) = lex.get(i) {
                let lower = s.to_ascii_lowercase();
                for (at, _) in lower.match_indices(OPEN) {
                    let rest = &s[at + OPEN.len()..];
                    let named = rest
                        .strip_prefix(">{")
                        .and_then(|r| r.split_once('}'))
                        .filter(|(n, after)| {
                            script_name(n) && after.to_ascii_lowercase().starts_with(CLOSE)
                        });
                    match named {
                        Some((n, _)) => put_on.push((name.clone(), n.to_string())),
                        None => {
                            let shown: String = rest.chars().take(30).collect();
                            bad.push(format!(
                                "{name}: a string writes {OPEN}{shown}, never {OPEN}>{{NAME_JS}}{CLOSE}"
                            ));
                        }
                    }
                }
            }
        }
    }
    // a name put on a page is a counted script, never a let or a const outside rust/crates
    for (at, used) in put_on {
        if !scripts.iter().any(|s| s.name == used) {
            bad.push(format!(
                "{at}: {used} is put on a page but is no counted NAME_JS const or static in rust/crates"
            ));
        }
    }
    // read from the folders, never git: an untracked file counts, and no hook's GIT_DIR changes what is seen
    for js in script_files(&root.join("crates")) {
        let canonical = std::fs::canonicalize(&js).unwrap_or(js.clone());
        if !included.contains(&canonical) {
            let path = js.strip_prefix(root).unwrap_or(&js).display();
            bad.push(format!(
                "{path}: a script no NAME_JS const includes, so it is not counted"
            ));
        }
    }
    (scripts, bad)
}

/// The lexemes less each inline `#[cfg(test)] mod name { … }`, by its braces: an out-of-line `mod tests;` hides nothing, and
/// code after a test module is still read.
fn without_test_modules(lex: Vec<Lexeme>) -> Vec<Lexeme> {
    let code = |i: usize| match lex.get(i) {
        Some(Lexeme::Code(c)) => c.as_str(),
        _ => "",
    };
    let attr = ["#", "[", "cfg", "(", "test", ")", "]"];
    let (mut kept, mut i) = (vec![], 0);
    while i < lex.len() {
        let mut j = i + attr.len();
        if attr.iter().enumerate().all(|(k, t)| code(i + k) == *t) {
            if code(j) == "pub" {
                j += 1;
            }
            if code(j) == "mod" && code(j + 2) == "{" {
                let mut depth = 0;
                j += 2;
                while j < lex.len() {
                    match code(j) {
                        "{" => depth += 1,
                        "}" => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                i = j + 1;
                continue;
            }
        }
        kept.push(lex[i].clone());
        i += 1;
    }
    kept
}

/// A plain literal's length as Rust builds it: `\` and a newline drop the indent after it, an escape is one character.
/// ponytail: `\u{…}` counts one byte whatever its UTF-8 length; none of our scripts holds one.
fn unescaped_len(s: &str) -> usize {
    let (mut n, mut chars) = (0, s.chars().peekable());
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\n') => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                Some('x') => {
                    chars.next();
                    chars.next();
                    n += 1;
                }
                Some('u') => {
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                    }
                    n += 1;
                }
                _ => n += 1,
            },
            c => n += c.len_utf8(),
        }
    }
    n
}

/// Every script file under `dir`, build output and tests/ folders left out.
fn script_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = vec![];
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|n| n != "target" && n != "tests")
                {
                    dirs.push(path);
                }
            } else if path
                .extension()
                .is_some_and(|e| SCRIPT_FILES.iter().any(|x| e == *x))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Whether the comment right above the declaration of `id` names, after [`MARK`], what blocks its port with a link. Comment
/// lines are read as one text, so a MARK wrapped across two lines still counts.
fn names_its_blocker(source: &str, id: &str) -> bool {
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let decls = [format!("const {id}:"), format!("static {id}:")];
    let Some(at) = lines
        .iter()
        .position(|l| !l.starts_with("//") && decls.iter().any(|d| l.contains(d.as_str())))
    else {
        return false;
    };
    let mut above: Vec<&str> = lines[..at]
        .iter()
        .rev()
        .take_while(|l| l.starts_with("//") || l.starts_with("#["))
        .filter(|l| l.starts_with("//"))
        .map(|l| l.trim_start_matches('/').trim())
        .collect();
    above.reverse();
    let comment = above.join(" ");
    comment
        .find(MARK)
        .is_some_and(|m| comment[m..].contains("https://"))
}

/// The scripts' bytes against the number held: the line to print, or why it fails.
pub fn verdict(bytes: usize, held: Option<usize>) -> Result<String, String> {
    match held {
        None => Err(format!("rust/{HELD}: no count, write {bytes}")),
        Some(h) if bytes > h => Err(format!(
            "{bytes} inline script bytes in rust/crates, {h} held: write the behaviour as Dioxus handlers in Rust \
             (rust/CLAUDE.md rule 6), never a script"
        )),
        Some(h) if bytes < h => Err(format!(
            "{bytes} inline script bytes in rust/crates, {h} held: lower it to {bytes} in rust/{HELD}, in this commit"
        )),
        Some(h) => Ok(format!(
            "{bytes} inline script bytes in rust/crates, {h} held"
        )),
    }
}
