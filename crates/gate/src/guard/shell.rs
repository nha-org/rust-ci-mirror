//! What the guards read of a Bash call: the call itself, and its command as shell, quotes and heredocs blanked and split
//! into simple commands. Below the guards, which all use it (dash-gate's no_crate_holds_a_module_cycle).
use std::path::{Component, Path, PathBuf};

/// What a guard reads of one Bash call. Its functions are the world it looks at, so a test gives stand-ins.
pub struct Call<'a> {
    /// Claude Code's tool_name
    pub tool: &'a str,
    pub command: &'a str,
    /// an Edit or Write call's file_path (a notebook's notebook_path), "" for Bash
    pub file: &'a str,
    /// the call's folder, the hook JSON's `cwd`
    pub cwd: &'a Path,
    /// `$CLAUDE_PROJECT_DIR`, else dash's own folder
    pub project: &'a Path,
    pub home: &'a Path,
    /// `herdr pane list`'s JSON
    pub panes: &'a dyn Fn() -> Result<String, String>,
    pub is_file: &'a dyn Fn(&Path) -> bool,
    pub is_dir: &'a dyn Fn(&Path) -> bool,
    /// a file's text, None when it is no readable UTF-8 file
    pub read: &'a dyn Fn(&Path) -> Option<String>,
    /// the path with its symlinks resolved, as far as it exists
    pub real: &'a dyn Fn(&Path) -> PathBuf,
}

/// A command word's lead: blanks, a subshell or group opener, then assignments and the wrappers that run the next word.
pub const LEAD: &str =
    r"^(?:\s|[({`!]|(?:\w+=\S*|env|exec|nohup|time|command|sudo|xargs|do|then|else)\s+)*";

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `p` with its symlinks resolved as far as it exists, the rest folded on (`..` pops): os.path.realpath's reading.
pub fn real(p: &Path) -> PathBuf {
    let parts: Vec<Component> = p.components().collect();
    for k in (0..=parts.len()).rev() {
        let head: PathBuf = parts[..k].iter().collect();
        if let Ok(mut found) = head.canonicalize() {
            for part in &parts[k..] {
                match part {
                    Component::ParentDir => {
                        found.pop();
                    }
                    Component::CurDir => {}
                    other => found.push(other),
                }
            }
            return found;
        }
    }
    p.to_path_buf()
}

/// A file's first MiB as text, an invalid byte replaced, as the .py hooks read a script (`open(...).read(1 << 20)`): a
/// guard never loads a whole binary or log, and a `#!` script with a Latin-1 byte still reads as one.
pub fn read(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(p)
        .ok()?
        .take(1 << 20)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `cmd` with heredoc bodies and quoted strings blanked, each blank keeping its length in chars: an `x` per char of a body
/// (its line breaks included) or of a quote's inside, a space per char of a comment. As bash reads them: the bodies of a
/// line's heredocs follow it in order, each ending at a line that is its delimiter alone (leading blanks allowed, a `)`
/// after it for `$(cat <<EOF ... EOF)`); the .py's regex took a later line starting with the word, hiding the lines between.
/// What bash runs stays code (task hook-reads-command-not-text: no-pkill refused a heredoc and a PR body naming pkill): a
/// `$(...)` or backtick inside a double quote or an unquoted heredoc, and the string a shell's `-c`, `eval`, `watch` or
/// `ssh` runs. Each such span opens with a `;` in place of its `$`, backtick or quote (a code string closes with one too),
/// so `segments` gives the command inside as a simple command of its own.
pub fn code(cmd: &str) -> String {
    quotes(&heredocs(&cmd.chars().collect::<Vec<_>>()))
}

/// `text` blanked as a double quote's inside or an unquoted heredoc's body: an `x` per char, but each `$(...)` and
/// backtick substitution kept, its opening `$` or backtick a `;`, since bash runs it, and a `$NAME` or `${...}` kept as
/// written; a backslash escapes the next char.
/// ponytail: parens are counted, so a `)` quoted inside a substitution ends it early; the rest is then blanked.
fn substitutions_kept(text: &[char]) -> Vec<char> {
    let mut out = vec!['x'; text.len()];
    let mut i = 0;
    while i < text.len() {
        let end = match (text[i], text.get(i + 1)) {
            ('\\', _) => {
                i += 2;
                continue;
            }
            ('$', Some('{')) => text[i..]
                .iter()
                .position(|&ch| ch == '}')
                .map_or(text.len(), |n| i + n + 1),
            ('$', Some(&ch)) if word(ch) => (i + 1..text.len())
                .find(|&k| !word(text[k]))
                .unwrap_or(text.len()),
            ('$', Some('(')) => {
                let mut depth = 0;
                (i + 1..text.len())
                    .find(|&k| {
                        match text[k] {
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            _ => {}
                        }
                        depth == 0
                    })
                    .map_or(text.len(), |k| k + 1)
            }
            ('`', _) => text[i + 1..]
                .iter()
                .position(|&ch| ch == '`')
                .map_or(text.len(), |n| i + n + 2),
            _ => {
                i += 1;
                continue;
            }
        };
        out[i..end].copy_from_slice(&text[i..end]);
        if text[i] != '$' || text[i + 1] == '(' {
            out[i] = ';';
        }
        i = end;
    }
    out
}

/// A quote at `i` that a shell runs as code: the string after the `-c` of a shell (any flags between, `bash -o pipefail
/// -c`), su, flock or runuser; an argument of eval, watch or ssh. Read from the line's start, a `\` line break joined.
/// ponytail: another runner of a string (xargs sh -c's own quote aside, a script's own) is read as text.
fn runs_as_code(c: &[char], i: usize) -> bool {
    static RUNS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?:^|[\s;&|(`])(?:(?:(?:\S*/)?(?:ba|z|k|da)?sh|su|flock|runuser)\b[^;&|\n]*\s-\w*c|(?:eval|watch|ssh)\b[^;&|\n]*?)\s+$",
        )
        .expect("runs")
    });
    let start = (0..i)
        .rev()
        .find(|&k| c[k] == '\n' && (k == 0 || c[k - 1] != '\\'))
        .map_or(0, |n| n + 1);
    let line: String = c[start..i].iter().collect();
    RUNS.is_match(&line.replace("\\\n", "  "))
}

fn heredocs(c: &[char]) -> Vec<char> {
    let mut out = c.to_vec();
    // the delimiters opened on the current line, whose bodies start at its end, and whether each was quoted
    let mut pending: Vec<(&[char], bool)> = vec![];
    // a `<<` inside a single quote, or a double quote with no `$(` before it, is text: `grep '<<EOF' f`
    let (mut single, mut double) = (false, None::<usize>);
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if !single => {
                i += 2;
                continue;
            }
            '\'' if double.is_none() => single = !single,
            '"' if !single => double = if double.is_some() { None } else { Some(i) },
            _ => {}
        }
        if c[i] == '\n' && !pending.is_empty() {
            let mut at = i;
            for (delim, quoted) in pending.drain(..) {
                // ponytail: an unclosed body stays as code, as the .py left it, where bash reads it to the end
                let Some(end) = body_end(c, at, delim) else {
                    return out;
                };
                if quoted {
                    out[at..end].fill('x');
                } else {
                    out[at..end].copy_from_slice(&substitutions_kept(&c[at..end]));
                }
                at = end;
            }
            i = at;
            continue;
        }
        if c[i] == '<'
            && c.get(i + 1) == Some(&'<')
            && !single
            && !double.is_some_and(|d| !c[d..i].windows(2).any(|w| w == ['$', '(']))
            && (i == 0 || c[i - 1] != '<')
            && !in_arithmetic(c, i)
            && let Some((delim, next)) = opener(c, i + 2)
        {
            pending.push((delim, matches!(c[next - 1], '\'' | '"')));
            i = next;
            continue;
        }
        i += 1;
    }
    out
}

/// `$((1<<2))` shifts: an `$((` on the line before `i` that no `))` closes yet.
fn in_arithmetic(c: &[char], i: usize) -> bool {
    let start = c[..i]
        .iter()
        .rposition(|&ch| ch == '\n')
        .map_or(0, |n| n + 1);
    let line: String = c[start..i].iter().collect();
    line.rfind("$((")
        .is_some_and(|at| !line[at..].contains("))"))
}

/// From just after `<<`: the delimiter and where the opening line goes on.
fn opener(c: &[char], mut j: usize) -> Option<(&[char], usize)> {
    if c.get(j) == Some(&'-') {
        j += 1;
    }
    while matches!(c.get(j), Some(' ' | '\t')) {
        j += 1;
    }
    if matches!(c.get(j), Some('\'' | '"')) {
        j += 1;
    }
    let start = j;
    while c.get(j).copied().is_some_and(word) {
        j += 1;
    }
    if j == start {
        return None;
    }
    let delim = &c[start..j];
    if matches!(c.get(j), Some('\'' | '"')) {
        j += 1;
    }
    Some((delim, j))
}

/// The end of the body starting at the line break `at`: just past its delimiter line's delimiter.
fn body_end(c: &[char], at: usize, delim: &[char]) -> Option<usize> {
    let delim: String = delim.iter().collect();
    let mut line = at + 1;
    while line <= c.len() {
        let eol = c[line..]
            .iter()
            .position(|&ch| ch == '\n')
            .map_or(c.len(), |n| line + n);
        let text: String = c[line..eol].iter().collect();
        if let Some(rest) = text
            .trim_start_matches([' ', '\t'])
            .strip_prefix(delim.as_str())
            && (rest.trim().is_empty() || rest.trim_start().starts_with(')'))
        {
            return Some(eol - rest.chars().count());
        }
        line = eol + 1;
    }
    None
}

fn quotes(c: &[char]) -> String {
    let mut out = String::with_capacity(c.len());
    let mut i = 0;
    while i < c.len() {
        let close = match c[i] {
            '\'' => c[i + 1..]
                .iter()
                .position(|&ch| ch == '\'')
                .map(|n| i + 1 + n),
            '"' => double_quoted(c, i + 1),
            _ => None,
        };
        if let Some(j) = close {
            out.push(c[i]);
            let inside = &c[i + 1..j];
            if runs_as_code(c, i) {
                out.pop();
                out.push(';');
                out.push_str(&quotes(inside));
                out.push(';');
            } else {
                if c[i] == '"' {
                    out.push_str(&quotes(&substitutions_kept(inside)));
                } else {
                    out.extend(std::iter::repeat_n('x', inside.len()));
                }
                out.push(c[j]);
            }
            i = j + 1;
        } else if c[i] == '#' && (i == 0 || c[i - 1].is_whitespace() || ";|&(".contains(c[i - 1])) {
            let n = c[i..]
                .iter()
                .position(|&ch| ch == '\n')
                .unwrap_or(c.len() - i);
            out.extend(std::iter::repeat_n(' ', n));
            i += n;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// The closing quote of a double-quoted string opened before `j`: a backslash escapes the next char, never a line break.
fn double_quoted(c: &[char], mut j: usize) -> Option<usize> {
    while j < c.len() {
        match c[j] {
            '"' => return Some(j),
            '\\' if c.get(j + 1).is_some_and(|&ch| ch != '\n') => j += 2,
            '\\' => return None,
            _ => j += 1,
        }
    }
    None
}

/// A command's simple commands: split at `&&`, `||`, `|`, `;`, a line break and a lone `&` (not `>&`, `<&` or `&>`).
pub fn segments(code: &str) -> Vec<String> {
    let c: Vec<char> = code.chars().collect();
    let (mut out, mut cur, mut i) = (vec![], String::new(), 0);
    while i < c.len() {
        let next = c.get(i + 1).copied();
        let n = match (c[i], next) {
            ('&', Some('&')) | ('|', Some('|')) => 2,
            ('|' | ';' | '\n', _) => 1,
            ('&', _) if !(i > 0 && matches!(c[i - 1], '<' | '>')) && next != Some('>') => 1,
            _ => 0,
        };
        if n == 0 {
            cur.push(c[i]);
            i += 1;
        } else {
            out.push(std::mem::take(&mut cur));
            i += n;
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    #[dash_test]
    fn code_blanks_quotes_heredocs_and_comments_keeping_lengths() {
        assert_eq!(code("echo 'a b' \"c\\\"d\" # x"), "echo 'xxx' \"xxxx\"    ");
        let cmd = "cat <<'EOF'\ngit commit -n\nEOF\ngit status";
        let blanked = code(cmd);
        assert_eq!(blanked.chars().count(), cmd.chars().count());
        assert_eq!(segments(&blanked).last().unwrap(), "git status");
        assert!(!blanked.contains("commit"));
        // two heredocs on one line: both bodies blanked, in order; the line after them is code
        let two = code("cat <<A <<'B'\ngit commit -n\nA\ngit commit -n\nB\ngit push");
        assert_eq!(segments(&two).last().unwrap(), "git push");
        assert!(!two.contains("commit"), "{two}");
        // a body ends at its delimiter alone: a line starting with the word is still body
        let prose = code("git commit -m \"$(cat <<'EOF'\nEOF-style notes: git commit -n\nEOF\n)\"");
        assert!(!prose.contains("commit -n"), "{prose}");
        // a shift is no heredoc
        assert!(code("echo $((1<<2))\ngit status").ends_with("git status"));
        // a here-string is no heredoc; an unclosed quote stays as it is
        assert_eq!(code("cat <<< 'x'"), "cat <<< 'x'");
        assert_eq!(code("echo 'open"), "echo 'open");
    }

    /// what bash runs stays code: a substitution in a double quote or an unquoted heredoc, a shell's -c string, eval's
    #[dash_test]
    fn code_keeps_what_bash_runs_inside_quotes_and_heredocs() {
        assert_eq!(
            code("echo \"a $(ls 'b') `pwd` \\$(c)\""),
            "echo \"xx;(ls 'x')x;pwd`xxxxxx\""
        );
        assert_eq!(code("cat <<EOF\nhi $(ls)\nEOF"), "cat <<EOFxxxx;(ls)xxxx");
        assert_eq!(
            code("cat <<'EOF'\nhi $(ls)\nEOF"),
            "cat <<'xxx'xxxxxxxxxxxxx"
        );
        // a string a runner runs is code, a `;` in place of its quotes, so segments splits the command inside
        for run in [
            "bash -c 'ls x'",
            "sh -e -lc \"ls x\"",
            "/bin/zsh --norc -c 'ls x'",
            "bash -o pipefail -c 'ls x'",
            "dash -c 'ls x'",
            "ksh -c 'ls x'",
            "su root -c 'ls x'",
            "flock /tmp/l -c 'ls x'",
            "eval 'ls x'",
            "watch -n1 'ls x'",
            "ssh host 'ls x'",
            "bash -c \\\n'ls x'",
        ] {
            let c = code(run);
            assert_eq!(c.chars().count(), run.chars().count(), "{run}");
            assert!(segments(&c).iter().any(|s| s == "ls x"), "{run}: {c}");
        }
        // a -c of another program, or a quote after the -c's string, is text
        assert_eq!(code("git -c 'a=b' st"), "git -c 'xxx' st");
        assert_eq!(code("bash -c 'ls' 'x y'"), "bash -c ;ls; 'xxx'");
        // a `<<` in a quote is no heredoc: the next lines stay code
        assert!(code("grep -n '<<EOF' f\nls\nEOF").contains("\nls\n"));
        assert!(code("echo \"a <<EOF\"\nls\nEOF").contains("\nls\n"));
        // a variable stays as written, so "$HERDR_BIN" keeps its name
        assert_eq!(
            code("\"$HERDR_BIN\" x \"${A}b\""),
            "\"$HERDR_BIN\" x \"${A}x\""
        );
    }

    /// realpath's reading: a symlink resolved, `..` below a missing folder folded (/code-review of 79ddd91c3)
    #[dash_test]
    fn real_resolves_links_and_folds_below_a_missing_folder() {
        let dir = std::env::temp_dir().join(format!("guard-real-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("repo/private/dashboard")).expect("mkdir");
        let link = dir.join("link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(dir.join("repo"), &link).expect("symlink");
        let repo = dir.join("repo").canonicalize().expect("repo");
        assert_eq!(
            real(&link.join("private/dashboard/lib/x.ex")),
            repo.join("private/dashboard/lib/x.ex")
        );
        assert_eq!(
            real(&link.join("private/dashboard/new/../../notes/x.ex")),
            repo.join("private/notes/x.ex")
        );
        assert_eq!(read(&dir.join("missing")), None);
        std::fs::write(dir.join("s.sh"), b"#!/bin/sh\n\xe9\n").expect("write");
        assert!(read(&dir.join("s.sh")).is_some_and(|t| t.starts_with("#!")));
        std::fs::remove_file(&link).expect("unlink");
    }

    #[dash_test]
    fn segments_split_at_separators_but_not_redirections() {
        assert_eq!(
            segments("a && b || c | d; e\nf & g 2>&1 &>x"),
            ["a ", " b ", " c ", " d", " e", "f ", " g 2>&1 &>x"]
        );
    }
}
