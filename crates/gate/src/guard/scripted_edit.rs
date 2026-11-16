//! Refuses a scripted edit of a .rs file that would change nothing, or that meets conflict markers: `sed -i`, `perl -pi` and
//! a python script's `.replace(...)` or `re.sub(...)`, each pattern read against the file before the call runs. cargo fmt
//! rewraps the code, so a pattern written from memory matches nothing and the edit says nothing: three tasks lost turns on it
//! on 2026-11-09 (task rust-scripted-edit-fmt; @nha, 2026-10-29: « You fixed the mistake but didn't find the cause and what
//! could actually fix that class of errors right ? »). The Edit tool fails loudly when its text is missing, so the refusal
//! names it. A pattern it cannot read (a backreference, a script file, a variable) is not judged: the guard never refuses
//! what it does not understand.
use super::shell::{Call, LEAD, basename, code, located, resolve, shlex};
use regex::Regex;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::LazyLock;

static EDITOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{LEAD}(?:\S*/)?(g?sed|perl|python3?(?:\.\d+)?)(?:\s|$)"
    ))
    .expect("editor")
});
/// a command that runs its words on another machine or in a container
static REMOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:ssh|docker|podman|kubectl)\s").expect("remote"));
static MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(?:<<<<<<<|>>>>>>>)(?: |$)").expect("marker"));
/// a quoted path, as a python script names the file it edits: one with an extension
static QUOTED_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"['"]([^'"\s]+\.[A-Za-z]\w*)['"]"#).expect("path"));
/// a heredoc's delimiter, quoted or not
static HEREDOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<<-?\s*['"]?([A-Za-z_]\w*)"#).expect("heredoc"));
/// `.replace('old',` and `re.sub(r'pattern',`, each quote kind apart (no backreference in this regex crate)
static PY: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        r#"\.replace\(\s*'((?:[^'\\\n]|\\.)*)'\s*,"#,
        r#"\.replace\(\s*"((?:[^"\\\n]|\\.)*)"\s*,"#,
        r#"\bre\.sub\(\s*(r?)'((?:[^'\\\n]|\\.)*)'\s*,"#,
        r#"\bre\.sub\(\s*(r?)"((?:[^"\\\n]|\\.)*)"\s*,"#,
    ]
    .map(|r| Regex::new(r).expect("python"))
});
/// a shell or perl variable in a pattern, whose text is not in the command, or a `\$` or a backslashed backtick, which a
/// double quote hands on bare and `shlex` keeps as typed
static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[$@][A-Za-z_{(]|\\[$`]").expect("variable"));

/// What a pattern is matched with: a literal (python's replace) or a regex as the tool reads it.
enum Pat {
    Literal(String),
    Regex(String),
}

pub fn refused(call: &Call) -> Option<String> {
    let run = code(call.command);
    let raw: Vec<char> = call.command.chars().collect();
    // a call run elsewhere edits no file of this machine
    if REMOTE.is_match(&run) {
        return None;
    }
    let mut touched: BTreeSet<PathBuf> = BTreeSet::new();
    for (span, cwd) in located(&run, call.cwd, call.home) {
        let blanked: String = run.chars().skip(span.start).take(span.len()).collect();
        let Some(tool) = EDITOR.captures(&blanked).map(|m| m[1].to_string()) else {
            continue;
        };
        let typed: String = raw[span.clone()].iter().collect();
        // comments off: sed's `s#a#b#` holds a #; an open quote is no command it can read
        let Ok(words) = shlex(&typed, false) else {
            continue;
        };
        let Some(at) = words.iter().position(|w| basename(w) == tool) else {
            continue;
        };
        let args = &words[at + 1..];
        let (name, pats, files) = match tool.as_str() {
            "sed" | "gsed" => match sed(args) {
                Some((pats, files)) => ("sed", pats, files),
                None => continue,
            },
            "perl" => match perl(args) {
                Some((pats, files)) => ("perl", pats, files),
                None => continue,
            },
            // its script: the word after -c, or the heredoc its line opens; a script file is not read
            _ => match python_script(args, &typed, &raw, span.start) {
                Some(script) => {
                    let (pats, files) = python(&script);
                    ("python", pats, files)
                }
                None => continue,
            },
        };
        // every file named, a .md or .toml beside the .rs too: a pattern found in any of them edits something
        // all of them or none: a pattern may sit in the one file it cannot read (a glob, a variable, a folder behind a
        // cd left), or in one an earlier edit of this command touched, whose text then differs from the one read here.
        // ponytail: a file another command rewrote first (`git checkout main -- a.rs && sed ...`) is judged as it is now;
        // so is a later -e of one sed that needs an earlier one's output (stricter)
        let files: BTreeSet<&String> = files.iter().collect();
        let Some(paths) = files
            .iter()
            .map(|f| resolve(cwd.as_deref(), f, call.home))
            .collect::<Option<Vec<PathBuf>>>()
        else {
            continue;
        };
        // counted, never `all`: each path is marked touched, the ones after a touched one too
        let seen = paths
            .iter()
            .filter(|p| !touched.insert((*p).clone()))
            .count();
        if seen > 0 || !files.iter().any(|f| f.ends_with(".rs")) {
            continue;
        }
        let texts: Vec<(String, String)> = files
            .iter()
            .zip(&paths)
            .filter_map(|(f, p)| Some(((*f).clone(), (call.read)(p)?)))
            .collect();
        if texts.len() < files.len() {
            continue;
        }
        for (file, text) in texts.iter().filter(|(f, _)| f.ends_with(".rs")) {
            if let Some(m) = MARKER.find(text) {
                let line = text[..m.start()].lines().count() + 1;
                return Some(format!(
                    "Refused by dash hook scripted-edit: {file} holds conflict markers (line {line}): a {name} edit there \
                     changes both sides blind. Resolve them with the Edit tool, then edit it with the Edit tool."
                ));
            }
        }
        // each pattern of sed and perl must hit; of a python script, one: its replaces may chain, or work on a path or a
        // variable it built, so only a script none of whose patterns is in the files surely changes nothing.
        // ponytail: a python script with one stale replace among good ones passes; reading python properly is the upgrade
        let judged: Vec<(String, bool)> = pats
            .iter()
            // a `$name` the shell or perl fills in (`sed -i "s/$old/$new/"`) is text the guard cannot read
            .filter(|pat| {
                let (Pat::Literal(p) | Pat::Regex(p)) = pat;
                !VARIABLE.is_match(p)
            })
            .filter_map(|pat| match pat {
                Pat::Literal(s) => {
                    Some((s.clone(), texts.iter().any(|(_, t)| t.contains(s.as_str()))))
                }
                Pat::Regex(r) => {
                    let re = Regex::new(&format!("(?m){r}")).ok()?;
                    Some((r.clone(), texts.iter().any(|(_, t)| re.is_match(t))))
                }
            })
            .collect();
        let missed = match name {
            "python" if judged.iter().any(|(_, hit)| *hit) => None,
            _ => judged.iter().find(|(_, hit)| !hit),
        };
        if let Some((shown, _)) = missed {
            let names: Vec<&str> = texts.iter().map(|(f, _)| f.as_str()).collect();
            return Some(format!(
                "Refused by dash hook scripted-edit: the {name} pattern `{shown}` matches nothing in {}, so this \
                 edit would change nothing and say nothing (cargo fmt rewraps code, so text written from memory \
                 misses). Read the file, then make the change with the Edit tool, which fails loudly when its text \
                 is not there.",
                names.join(", ")
            ));
        }
    }
    None
}

/// The script a python call runs: the word after `-c`, or the body of the heredoc its line opens (the lines after the
/// opener's line, up to the delimiter alone). None for a script file or stdin from elsewhere.
fn python_script(args: &[String], typed: &str, raw: &[char], start: usize) -> Option<String> {
    if let Some(at) = args.iter().position(|w| w == "-c") {
        return args.get(at + 1).cloned();
    }
    let delim = HEREDOC.captures(typed)?[1].to_string();
    // from the span's start: the body is blanked into the opener's own span, so it ends past the body
    let rest: String = raw[start..].iter().collect();
    let (_, body) = rest.split_once('\n')?;
    let lines: Vec<&str> = body.lines().take_while(|l| l.trim() != delim).collect();
    Some(lines.join("\n"))
}

/// `sed -i`'s scripts and files, none when it edits nothing in place or reads its script from a file.
fn sed(args: &[String]) -> Option<(Vec<Pat>, Vec<String>)> {
    let (mut in_place, mut ere, mut scripts, mut files) = (false, false, vec![], vec![]);
    let mut a = args.iter().peekable();
    while let Some(w) = a.next() {
        match w.as_str() {
            "-e" | "--expression" => scripts.extend(a.next().cloned()),
            "-f" | "--file" => return None,
            "-E" | "-r" | "--regexp-extended" => ere = true,
            // BSD's `-i ''`: the empty suffix is its own word
            "-i" | "--in-place" => {
                in_place = true;
                if a.peek().is_some_and(|n| n.is_empty()) {
                    a.next();
                }
            }
            _ if w.starts_with("--in-place=") || w.starts_with("-i") => in_place = true,
            _ if w.starts_with("--expression=") => scripts.push(w["--expression=".len()..].into()),
            _ if w.starts_with('-') && !w.starts_with("--") && w.len() > 1 => {
                in_place |= w.contains('i');
                ere |= w.contains('E') || w.contains('r');
            }
            _ if scripts.is_empty() => scripts.push(w.clone()),
            _ => files.push(w.clone()),
        }
    }
    if !in_place {
        return None;
    }
    let pats = scripts
        .iter()
        .flat_map(|s| sed_patterns(s))
        .filter_map(|(p, delim, flags)| {
            let re = if ere {
                ere_rx(&p, delim)
            } else {
                bre_rx(&p, delim)
            }?;
            Some(Pat::Regex(flagged(re, &flags)))
        })
        .collect();
    Some((pats, files))
}

/// `re` under its inline flags (i, s, x), as the tool's own flags set them.
fn flagged(re: String, flags: &str) -> String {
    match flags {
        "" => re,
        _ => format!("(?{flags}){re}"),
    }
}

/// `perl -pi -e`'s s/// patterns, read as sed -E reads them.
fn perl(args: &[String]) -> Option<(Vec<Pat>, Vec<String>)> {
    let (mut in_place, mut script, mut files) = (false, None, vec![]);
    let mut a = args.iter();
    while let Some(w) = a.next() {
        if w.starts_with('-') && w.len() > 1 && script.is_none() {
            // -M, -m and -I take a module or a folder, whose letters are no switches (-Mstrict holds an i)
            if !w.starts_with("-M") && !w.starts_with("-m") && !w.starts_with("-I") {
                in_place |= w[1..].contains('i');
            }
            if w.ends_with('e') && !w.starts_with("-M") && !w.starts_with("-m") {
                script = a.next().cloned();
            }
        } else if script.is_none() {
            script = Some(w.clone());
        } else {
            files.push(w.clone());
        }
    }
    let script = script.filter(|_| in_place)?;
    let pats = sed_patterns(&script)
        .into_iter()
        .filter_map(|(p, delim, flags)| Some(Pat::Regex(flagged(ere_rx(&p, delim)?, &flags))))
        .collect();
    Some((pats, files))
}

/// A python script's `.replace` literals, `re.sub` patterns (with the re flags the script names) and quoted paths.
fn python(script: &str) -> (Vec<Pat>, Vec<String>) {
    // an edit in place reads the file and writes it: a print of its text, or a file written from a template, is none
    if !(script.contains("read") && script.contains("write")) {
        return (vec![], vec![]);
    }
    let files = QUOTED_PATH
        .captures_iter(script)
        .map(|m| m[1].to_string())
        .collect();
    let named = |short: &str, long: &str| {
        Regex::new(&format!(r"\bre\.(?:{short}|{long})\b")).is_ok_and(|r| r.is_match(script))
    };
    let flags: String = [
        ("I", "IGNORECASE", 'i'),
        ("S", "DOTALL", 's'),
        ("X", "VERBOSE", 'x'),
    ]
    .into_iter()
    .filter(|(s, l, _)| named(s, l))
    .map(|(_, _, f)| f)
    .collect();
    let mut pats = vec![];
    for (k, re) in PY.iter().enumerate() {
        for m in re.captures_iter(script) {
            if k < 2 {
                pats.push(Pat::Literal(py_unescape(&m[1])));
            } else {
                let p = if m[1].is_empty() {
                    py_unescape(&m[2])
                } else {
                    m[2].to_string()
                };
                pats.push(Pat::Regex(flagged(p, &flags)));
            }
        }
    }
    (pats, files)
}

/// Python's escapes in a plain string: the common ones read, any other kept with its backslash, as python keeps it.
fn py_unescape(s: &str) -> String {
    let mut out = String::new();
    let mut c = s.chars();
    while let Some(ch) = c.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match c.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(q @ ('\\' | '\'' | '"')) => out.push(q),
            Some(other) => out.extend(['\\', other]),
            None => out.push('\\'),
        }
    }
    out
}

/// The regexes of a sed (or perl) script: each /address/ and each s command's pattern, with its delimiter and its flags as
/// inline ones (I or i, s, x). It stops at what it cannot read, keeping what it read before: perl's bracketing delimiters
/// (s{a}{b}) among them.
fn sed_patterns(script: &str) -> Vec<(String, char, String)> {
    let c: Vec<char> = script.chars().collect();
    let (mut out, mut i) = (vec![], 0);
    // the text up to an unescaped `delim` from `i`, a `\<delim>` kept as typed; the index past the closing one
    let delimited = |mut i: usize, delim: char| -> Option<(String, usize)> {
        let mut s = String::new();
        while i < c.len() {
            match c[i] {
                ch if ch == delim => return Some((s, i + 1)),
                '\\' if i + 1 < c.len() => {
                    s.extend([c[i], c[i + 1]]);
                    i += 2;
                }
                ch => {
                    s.push(ch);
                    i += 1;
                }
            }
        }
        None
    };
    let icase = |i: usize| c.get(i).is_some_and(|&ch| ch == 'I');
    'command: while i < c.len() {
        while i < c.len() && (c[i].is_whitespace() || c[i] == ';') {
            i += 1;
        }
        // up to two addresses
        let addressed = out.len();
        for _ in 0..2 {
            match c.get(i) {
                Some(d) if d.is_ascii_digit() || *d == '$' => {
                    while c
                        .get(i)
                        .is_some_and(|d| d.is_ascii_digit() || *d == '$' || *d == '~')
                    {
                        i += 1;
                    }
                }
                Some(&d) if d == '/' || d == '\\' => {
                    let delim = if d == '\\' {
                        i += 1;
                        *c.get(i).unwrap_or(&'/')
                    } else {
                        '/'
                    };
                    let Some((p, next)) = delimited(i + 1, delim) else {
                        break 'command;
                    };
                    out.push((p, delim, if icase(next) { "i" } else { "" }.into()));
                    i = next + usize::from(icase(next));
                }
                _ => break,
            }
            if c.get(i) != Some(&',') {
                break;
            }
            i += 1;
        }
        while c.get(i).is_some_and(|ch| ch.is_whitespace() || *ch == '!') {
            // `/keep/!d` edits the lines its address misses: one that matches nothing changes them all
            if c[i] == '!' {
                out.truncate(addressed);
            }
            i += 1;
        }
        let Some(&cmd) = c.get(i) else { break };
        i += 1;
        match cmd {
            's' => {
                let Some(&delim) = c.get(i).filter(|d| !matches!(d, '{' | '(' | '[' | '<')) else {
                    break;
                };
                let Some((p, next)) = delimited(i + 1, delim) else {
                    break;
                };
                let Some((_, mut end)) = delimited(next, delim) else {
                    break;
                };
                let mut flags = String::new();
                while let Some(&f) = c.get(end).filter(|f| f.is_ascii_alphanumeric()) {
                    let inline = match f {
                        'I' | 'i' => Some('i'),
                        's' | 'x' => Some(f),
                        _ => None,
                    };
                    flags.extend(inline.filter(|f| !flags.contains(*f)));
                    end += 1;
                    if f == 'w' {
                        break;
                    }
                }
                out.push((p, delim, flags));
                i = end;
            }
            'y' => {
                let Some(&delim) = c.get(i) else { break };
                let Some((_, next)) = delimited(i + 1, delim) else {
                    break;
                };
                let Some((_, end)) = delimited(next, delim) else {
                    break;
                };
                i = end;
            }
            '{' | '}' => {}
            // a text or a file name runs to the line's end
            'a' | 'i' | 'c' | 'r' | 'w' | ':' | 'b' | 't' => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            _ if cmd.is_ascii_alphabetic() || cmd == '=' => {}
            _ => break,
        }
    }
    out.retain(|(p, _, _)| !p.is_empty());
    out
}

/// A sed basic regex as this regex crate reads it: `\(` `\)` `\{` `\}` `\|` `\+` `\?` are operators, the bare ones literal;
/// `^` and `$` anchor only at the ends. None for what it has no form for (a backreference).
fn bre_rx(p: &str, delim: char) -> Option<String> {
    let c: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' => {
                let n = *c.get(i + 1)?;
                i += 2;
                match n {
                    '(' | ')' | '{' | '}' | '|' | '+' | '?' if n != delim => out.push(n),
                    'n' | 't' | 'w' | 'W' | 's' | 'S' | 'b' | 'B' | '<' | '>' => {
                        out.extend(['\\', n])
                    }
                    _ if n.is_ascii_digit() || n.is_alphanumeric() => return None,
                    _ => out.push_str(&regex::escape(&n.to_string())),
                }
                continue;
            }
            '[' => {
                let end = bracket_end(&c, i)?;
                out.push_str(&bracket(&c[i..=end]));
                i = end + 1;
                continue;
            }
            '(' | ')' | '{' | '}' | '|' | '+' | '?' => {
                out.push_str(&regex::escape(&c[i].to_string()))
            }
            // GNU sed anchors them inside a group or an alternation too: after `\(` or `\|`, before `\)` or `\|`
            '^' if i > 0 && !(i > 1 && c[i - 2] == '\\' && matches!(c[i - 1], '(' | '|')) => {
                out.push_str(r"\^")
            }
            '$' if i + 1 < c.len()
                && !(c[i + 1] == '\\' && matches!(c.get(i + 2), Some(')' | '|'))) =>
            {
                out.push_str(r"\$")
            }
            ch => out.push(ch),
        }
        i += 1;
    }
    Some(out)
}

/// A sed extended (or perl) regex: as typed, but `\<delim>` its literal and a bracket's backslash a literal one.
fn ere_rx(p: &str, delim: char) -> Option<String> {
    let c: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if c.get(i + 1) == Some(&delim) => {
                out.push_str(&regex::escape(&delim.to_string()));
                i += 2;
            }
            '\\' => {
                out.extend([c[i], *c.get(i + 1)?]);
                i += 2;
            }
            '[' => {
                let end = bracket_end(&c, i)?;
                out.push_str(&bracket(&c[i..=end]));
                i = end + 1;
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    Some(out)
}

/// The `]` closing the bracket opened at `i`: one right after `[` or `[^` is a member, `[:alpha:]` is read whole.
fn bracket_end(c: &[char], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if c.get(j) == Some(&'^') {
        j += 1;
    }
    if c.get(j) == Some(&']') {
        j += 1;
    }
    while j < c.len() {
        match c[j] {
            '[' if c.get(j + 1) == Some(&':') => {
                j += 2;
                while j + 1 < c.len() && !(c[j] == ':' && c[j + 1] == ']') {
                    j += 1;
                }
                j += 2;
            }
            ']' => return Some(j),
            _ => j += 1,
        }
    }
    None
}

/// A POSIX bracket for this regex crate: its backslash, `[`, `&` and `~` literals there (escaped), `[:class:]` kept.
fn bracket(b: &[char]) -> String {
    let s: String = b[1..b.len() - 1].iter().collect();
    let mut out = String::from("[");
    let mut rest = s.as_str();
    while let Some(ch) = rest.chars().next() {
        if let Some(end) = rest.strip_prefix("[:").and_then(|r| r.find(":]")) {
            out.push_str(&rest[..end + 4]);
            rest = &rest[end + 4..];
            continue;
        }
        if matches!(ch, '\\' | '[' | '&' | '~') {
            out.push('\\');
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::collections::HashMap;
    use std::path::Path;

    /// The guard on `cmd` run in /w, the files it may read given as (path, text).
    fn judge(cmd: &str, files: &[(&str, &str)]) -> Option<String> {
        let files: HashMap<PathBuf, String> = files
            .iter()
            .map(|(p, t)| (PathBuf::from(p), t.to_string()))
            .collect();
        let list = || Ok(String::new());
        let read = |p: &Path| files.get(p).cloned();
        let call = Call {
            tool: "Bash",
            command: cmd,
            file: "",
            cwd: Path::new("/w"),
            project: Path::new("/w"),
            home: Path::new("/h"),
            panes: &list,
            is_file: &|_| false,
            is_dir: &|_| false,
            read: &read,
            real: &|p| p.to_path_buf(),
        };
        refused(&call)
    }

    const RS: &str = "fn main() {\n    let x = foo(\n        a,\n        b,\n    );\n}\n";

    /// Done-when 1, sed: a pattern cargo fmt's rewrap made stale is refused, naming the Edit tool; one that matches passes.
    #[dash_test]
    fn a_sed_edit_of_a_rust_file_that_matches_nothing_is_refused() {
        let files = [("/w/src/a.rs", RS), ("/w/Cargo.toml", "version = \"1\"\n")];
        for bad in [
            "sed -i '' 's/foo(a, b)/bar(a, b)/' src/a.rs",
            "sed -i 's/let x = foo(a/let y = foo(a/g' src/a.rs",
            "cd src && sed -i.bak -e 's/main/main/' -e 's/gone/x/' a.rs",
            // a subshell's cd is left behind when it closes (shell::located, cargo-test's reader)
            "(cd src && ls); sed -i 's/gone/x/' src/a.rs",
            "sed -E -i '' 's/foo\\(a, b\\)/x/' /w/src/a.rs",
            "sed -i '/^fn helper/d' src/a.rs",
            "git status && /usr/bin/sed -i 's|a, b|c|' src/a.rs",
            "perl -pi -e 's/foo\\(a, b\\)/x/' src/a.rs",
            // a python call beside it marks nothing touched (/code-review)
            "sed -i 's/gone/x/' 'src/a.rs'; python3 -c 'print(1)'",
        ] {
            let why = judge(bad, &files).unwrap_or_else(|| panic!("passed: {bad}"));
            assert!(
                why.starts_with("Refused by dash hook scripted-edit:")
                    && why.contains("matches nothing")
                    && why.contains("Edit tool"),
                "{bad}: {why}"
            );
        }
        for good in [
            "sed -i '' 's/foo(/bar(/' src/a.rs",
            "sed -i 's/let x = foo\\(/let y = foo(/' src/a.rs",
            "sed -i -E 's/foo\\(/bar(/' src/a.rs",
            "sed -i '/^fn main/,/^}/s/a,$/c,/' src/a.rs",
            "sed -i 's/[[:space:]]b,/ c,/' src/a.rs",
            "sed -i 's/x = foo/y = foo/; s/b,/c,/' src/a.rs",
            "perl -0pi -e 's/a,\\n\\s+b/a, b/' src/a.rs",
            // not judged: no in-place edit, no .rs, a file it cannot read, a script file, a backreference, a quoted text
            "sed 's/gone/x/' src/a.rs",
            "sed -i 's/gone/x/' notes.md",
            "sed -i 's/gone/x/' src/missing.rs",
            "sed -i -f edits.sed src/a.rs",
            "sed -i 's/\\(gone\\)\\1/x/' src/a.rs",
            "git commit -m \"sed -i 's/gone/x/' src/a.rs\"",
            // the second edit reads what the first wrote
            "sed -i 's/foo/bar/' src/a.rs && sed -i 's/bar(/baz(/' src/a.rs",
            // what /code-review found refused: a pattern found in another file named, perl's bracketing delimiters,
            // its s flag, an -M option holding an i
            "sed -i 's/version = \"1\"/version = \"2\"/' Cargo.toml src/a.rs",
            "perl -pi -e 's{foo}{bar}g' src/a.rs",
            "perl -0pi -e 's/foo\\(.*?\\)/x/s' src/a.rs",
            "perl -Mstrict -ne 'print if s/gone/x/' src/a.rs",
            // the review of PR 1649: a variable the shell or perl fills in, a negated address, a file the command wrote
            // before the edit
            "sed -i \"s/$old/$new/\" src/a.rs",
            "perl -pi -e 's/$name\\(/x(/' src/a.rs",
            "sed -i '/gone/!d' src/a.rs",
            "sed -i \"s/b,\\$/c,/\" src/a.rs",
            // a file it cannot read beside one it can, a file an earlier edit touched beside a new one
            "sed -i 's/gone/x/' src/*.rs src/a.rs",
            "sed -i 's/foo/bar/' Cargo.toml && sed -i 's/gone/x/' Cargo.toml src/a.rs",
            // a folder it lost track of, a call run elsewhere, an alternation's anchors
            "cd - && sed -i 's/gone/x/' src/a.rs",
            "ssh box 'cd /srv && sed -i \"s/gone/x/\" src/a.rs'",
            "sed -i 's/^fn gone\\|^fn main/fn x/' src/a.rs",
        ] {
            assert_eq!(judge(good, &files), None, "{good}");
        }
    }

    /// Done-when 1, a python script: a `.replace` literal or a `re.sub` pattern missing from the file is refused.
    #[dash_test]
    fn a_python_edit_of_a_rust_file_that_matches_nothing_is_refused() {
        let files = [("/w/src/a.rs", RS)];
        let script = |body: &str| {
            format!(
                "python3 - <<'X'\nfrom pathlib import Path\np = Path('src/a.rs')\ns = p.read_text()\n{body}\np.write_text(s)\nX"
            )
        };
        for bad in [
            script("s = s.replace(\"foo(a, b)\", \"bar(a, b)\")"),
            script("s = s.replace('gone(', 'bar(')\ns = s.replace('    a,\\n    b,', 'a, b')"),
            script("import re\ns = re.sub(r'foo\\(a, b\\)', 'x', s)"),
            "python3 -c 's=open(\"src/a.rs\").read(); open(\"src/a.rs\",\"w\").write(s.replace(\"gone\",\"x\"))'".into(),
        ] {
            let why = judge(&bad, &files).unwrap_or_else(|| panic!("passed: {bad}"));
            assert!(
                why.contains("the python pattern") && why.contains("Edit tool"),
                "{bad}: {why}"
            );
        }
        for good in [
            script("s = s.replace('foo(\\n        a,', 'bar(\\n        a,')"),
            script("import re\ns = re.sub(r'let (\\w+) = foo', r'let \\1 = bar', s)"),
            script("s = s.replace(old, new)"),
            // one pattern found passes: replaces chain, or work on a path (/code-review); a stale one among good ones
            // passes too, the guard's known ceiling
            script("s = s.replace('foo', 'bar').replace('bar(', 'baz(')"),
            script("mod = str(p).replace('.rs', '').replace('/', '::')\ns = s.replace('foo(', 'bar(')"),
            script("s = s.replace('foo(', 'bar(')\ns = s.replace('    a,\\n    b,', 'a, b')"),
            script("import re\ns = re.sub(r'FOO\\(', 'bar(', s, flags=re.I)"),
            // a print of the file's text and a file written from a template edit nothing in place (review of PR 1649)
            "python3 -c \"print(open('src/a.rs').read().replace('gone', 'x'))\"".into(),
            "python3 -c \"open('src/a.rs','w').write(TEMPLATE.replace('NAME', n))\"".into(),
            // a python call that edits nothing, a .rs quoted elsewhere in the command
            "python3 -c \"print('a-b'.replace('-', '_'))\" && git add 'src/a.rs'".into(),
            "python3 - <<'X'\nopen('notes.md','w').write(open('notes.md').read().replace('gone','x'))\nX".into(),
        ] {
            assert_eq!(judge(&good, &files), None, "{good}");
        }
    }

    /// Done-when 1, conflict markers: a scripted edit of a conflicted .rs file is refused with the line, whatever its
    /// pattern.
    #[dash_test]
    fn a_scripted_edit_of_a_rust_file_with_conflict_markers_is_refused() {
        let conflicted = "fn a() {}\n<<<<<<< HEAD\nfn b() {}\n=======\nfn c() {}\n>>>>>>> main\n";
        let files = [("/w/a.rs", conflicted)];
        for cmd in [
            "sed -i 's/fn b/fn d/' a.rs",
            "perl -pi -e 's/fn c/fn d/' a.rs",
            "python3 -c 'p=\"a.rs\"; s=open(p).read(); open(p,\"w\").write(s.replace(\"fn a\",\"fn z\"))'",
        ] {
            let why = judge(cmd, &files).unwrap_or_else(|| panic!("passed: {cmd}"));
            assert!(
                why.contains("a.rs holds conflict markers (line 2)") && why.contains("Edit tool"),
                "{cmd}: {why}"
            );
        }
        // a marker-like line inside a string is not at a line's start
        assert_eq!(
            judge(
                "sed -i 's/fn a/fn z/' a.rs",
                &[("/w/a.rs", "fn a() { \"<<<<<<< x\"; }\n")]
            ),
            None
        );
    }

    #[dash_test]
    fn basic_regexes_read_as_sed_reads_them() {
        assert_eq!(bre_rx(r"foo(a)", '/').as_deref(), Some(r"foo\(a\)"));
        assert_eq!(bre_rx(r"\(a\|b\)\+", '/').as_deref(), Some("(a|b)+"));
        assert_eq!(bre_rx(r"a\|b", '|').as_deref(), Some(r"a\|b"));
        assert_eq!(bre_rx(r"^a$b$", '/').as_deref(), Some(r"^a\$b$"));
        assert_eq!(bre_rx(r"[\]x", '/').as_deref(), Some(r"[\\]x"));
        assert_eq!(bre_rx(r"\1", '/'), None);
        assert_eq!(sed_patterns("s/a/b/gI")[0].2, "i");
        assert_eq!(sed_patterns("s/a/b/sx")[0].2, "sx");
        assert_eq!(
            sed_patterns("s{a}{b}"),
            [],
            "perl's bracketing delimiters are not judged"
        );
        assert_eq!(ere_rx(r"a\/b", '/').as_deref(), Some("a/b"));
        assert_eq!(
            sed_patterns("/x/,/y/d; s#a\\#b#c#g\n3p")
                .iter()
                .map(|p| p.0.as_str())
                .collect::<Vec<_>>(),
            ["x", "y", "a\\#b"]
        );
    }
}
