//! What the guards read of a Bash call: the call itself, and its command as shell, quotes and heredocs blanked and split
//! into simple commands. Below the guards, which all use it (dash-gate's no_crate_holds_a_module_cycle).
use regex::Regex;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

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
    /// whether a variable holds a value in the environment the call inherits (set to nothing is unset)
    pub set: &'a dyn Fn(&str) -> bool,
}

/// A command word's lead, the words Claude Code's own rm check skips (its `xun`, 2.1.329): blanks, a subshell or group
/// opener, assignments, a word that opens with `$`, the keywords before a command, and the wrappers that run the next
/// word, by any path, past their flags, a flag's value and a timeout's seconds (rm-bare-var mirrors that check, and every
/// guard reads a command word the same way). A flag's value is tried both ways, so `sudo -n rm` and `sudo -u root rm`
/// both end on rm; the price is that `env -i echo git commit -n` reads as a commit. Never the lead of a `cd`: [`located`] has its own, since a wrapper's cd moves a child process.
pub const LEAD: &str = r"^(?:\s|[({`!]|(?:\w+=\S*|\$\S*|if|elif|while|until|coproc|do|then|else)\s+|(?:\S*/)?(?:env|exec|nohup|time|command|sudo|doas|xargs|nice|timeout|stdbuf|setsid|ionice|busybox|eval|pkexec|runuser)(?:\s+-\S+(?:\s+[^\s=-][^\s=]*)?|\s+\d\S*)*\s+)*";

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `p` folded on paper, never the file system: `.` dropped, `..` pops. The one fold the guards and the workspace rules
/// share (sccache folds each SCCACHE_BASEDIRS entry the same way, task sccache-basedirs).
pub fn folded(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in p.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `p` with its symlinks resolved as far as it exists, the rest folded on (`..` pops): os.path.realpath's reading.
pub fn real(p: &Path) -> PathBuf {
    let parts: Vec<Component> = p.components().collect();
    for k in (0..=parts.len()).rev() {
        let head: PathBuf = parts[..k].iter().collect();
        if let Ok(found) = head.canonicalize() {
            return folded(&found.join(parts[k..].iter().collect::<PathBuf>()));
        }
    }
    p.to_path_buf()
}

/// Whether `name` holds a value in this process's environment, the one Claude Code's hook gives each guard: set to
/// nothing counts as unset, as Claude Code's `mayBeUnset` reads it.
pub fn set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
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

/// A `;` that [`code`] put where the command has a quote: the edge of a string a shell runs. `run` is `code`'s text and
/// `raw` the command, char for char.
pub(super) fn string_edge(run: &[char], raw: &[char], k: usize) -> bool {
    run[k] == ';' && matches!(raw[k], '\'' | '"')
}

/// Whether `at` stands inside a string a shell runs (`bash -c '...'`): an odd count of such edges before it.
pub(super) fn in_code_string(run: &[char], raw: &[char], at: usize) -> bool {
    (0..at).filter(|&k| string_edge(run, raw, k)).count() % 2 == 1
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
    for (at, quoted, body) in bodies(c) {
        // a `<<\EOF` body stays code here, as before heredocs_of read it: a shell fed it runs it, and the guards built on
        // `code` must see it (/code-review of PR 1780's fixes)
        let backslash = c[at + 2..]
            .iter()
            .find(|ch| !matches!(ch, '-' | ' ' | '\t'))
            == Some(&'\\');
        if backslash {
            continue;
        }
        match quoted {
            true => out[body].fill('x'),
            false => {
                let kept = substitutions_kept(&c[body.clone()]);
                out[body].copy_from_slice(&kept);
            }
        }
    }
    out
}

/// A heredoc as bash reads it (review of PR 1780: the scripted-edit guard kept a regex of its own).
#[derive(Debug, PartialEq)]
pub struct Heredoc {
    /// Where its `<<` stands, in chars of the command.
    pub at: usize,
    /// Its delimiter quoted (`'EOF'`, `"EOF"` or `\EOF`): its body is as typed, never rewritten by bash.
    pub quoted: bool,
    /// The lines between its opening line and its delimiter line.
    pub body: String,
}

/// Each heredoc of `cmd`, in order; one never closed ends the list.
pub fn heredocs_of(cmd: &str) -> Vec<Heredoc> {
    let c: Vec<char> = cmd.chars().collect();
    (bodies(&c).into_iter())
        .map(|(at, quoted, body)| {
            let text: String = c[body.start + 1..body.end].iter().collect();
            let body = text.rsplit_once('\n').map_or("", |(b, _)| b).to_string();
            Heredoc { at, quoted, body }
        })
        .collect()
}

/// Each heredoc's `<<`, whether its delimiter is quoted, and its body's chars from the opening line's break to just past its
/// delimiter line's delimiter, in order. The scan stops at a body never closed: bash reads it to the end, and the .py left it
/// as code.
fn bodies(c: &[char]) -> Vec<(usize, bool, std::ops::Range<usize>)> {
    let mut found = vec![];
    // the delimiters opened on the current line, whose bodies start at its end, with their `<<` and whether each was quoted
    let mut pending: Vec<(usize, &[char], bool)> = vec![];
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
            for (open, delim, quoted) in pending.drain(..) {
                // ponytail: an unclosed body stays as code, as the .py left it, where bash reads it to the end
                let Some(end) = body_end(c, at, delim) else {
                    return found;
                };
                found.push((open, quoted, at..end));
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
            && let Some((delim, next, quoted)) = opener(c, i + 2)
        {
            pending.push((i, delim, quoted));
            i = next;
            continue;
        }
        i += 1;
    }
    found
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

/// From just after `<<`: the delimiter, where the opening line goes on, and whether it is quoted (`'EOF'`, `"EOF"`, `\EOF`).
fn opener(c: &[char], mut j: usize) -> Option<(&[char], usize, bool)> {
    if c.get(j) == Some(&'-') {
        j += 1;
    }
    while matches!(c.get(j), Some(' ' | '\t')) {
        j += 1;
    }
    let quoted = matches!(c.get(j), Some('\'' | '"' | '\\'));
    if quoted {
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
    Some((delim, j, quoted))
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

/// The first line (from 1) of `text` that opens or closes a conflict, `<<<<<<<` or `>>>>>>>` alone or before a space: the
/// one reading of the scripted-edit guard and the pre-commit's rust-fmt and conflict-markers steps.
pub fn conflict_marker(text: &str) -> Option<usize> {
    let marker = |l: &str| l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>");
    (text.lines())
        // any whitespace after it, as git's --check reads one (review of PR 1780: a tab missed)
        .position(|l| marker(l) && l.chars().nth(7).is_none_or(char::is_whitespace))
        .map(|i| i + 1)
}

/// A command's simple commands: split at `&&`, `||`, `|`, `;`, a line break and a lone `&` (not `>&`, `<&` or `&>`).
pub fn segments(code: &str) -> Vec<String> {
    let c: Vec<char> = code.chars().collect();
    spans(code)
        .into_iter()
        .map(|r| c[r].iter().collect())
        .collect()
}

/// `segments` as char ranges: [`code`] keeps each char's place, so a range also picks the command as typed, its quotes
/// unblanked.
pub fn spans(code: &str) -> Vec<std::ops::Range<usize>> {
    let c: Vec<char> = code.chars().collect();
    let (mut out, mut start, mut i) = (vec![], 0, 0);
    while i < c.len() {
        let next = c.get(i + 1).copied();
        let n = match (c[i], next) {
            ('&', Some('&')) | ('|', Some('|')) => 2,
            ('|' | ';' | '\n', _) => 1,
            ('&', _) if !(i > 0 && matches!(c[i - 1], '<' | '>')) && next != Some('>') => 1,
            _ => 0,
        };
        if n == 0 {
            i += 1;
        } else {
            out.push(start..i);
            i += n;
            start = i;
        }
    }
    out.push(start..c.len());
    out
}

/// A `cd` that moves this shell: behind blanks, a group or subshell opener, an assignment or a keyword, and the words that
/// run a builtin in place. Never [`LEAD`]'s wrappers (`nice cd /x`, `timeout 5 cd /x`, `env cd /x`, `$RUN cd /x`): those
/// move a child process, and a guard that followed them would judge the next command in a folder it does not run in
/// (review of PR 1849: cargo-test passed `nice cd /other; cargo test`).
static CD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:\s|[({!]|(?:\w+=\S*|if|elif|while|until|do|then|else|time|command|builtin|eval)\s+)*cd(\s.*)?$",
    )
    .expect("cd")
});

/// Each simple command of `code` (a [`spans`] range) with the folder it runs in: `cwd`, moved by each `cd`, back to the
/// one before a subshell when it closes; None while a path is unreadable. The one cd reader of the guards, cargo-test's
/// and scripted-edit's.
pub(super) fn located(
    code: &str,
    cwd: &Path,
    home: &Path,
) -> Vec<(std::ops::Range<usize>, Option<PathBuf>)> {
    let c: Vec<char> = code.chars().collect();
    let mut dir = Some(cwd.to_path_buf());
    // the folder before each open subshell, back when it closes
    let mut saved = vec![];
    let mut out = vec![];
    for span in spans(code) {
        let seg: String = c[span.clone()].iter().collect();
        let edge = |chars: &mut dyn Iterator<Item = char>, of: char| {
            chars
                .filter(|c| !c.is_whitespace())
                .take_while(|&c| c == of)
                .count()
        };
        for _ in 0..edge(&mut seg.chars(), '(') {
            saved.push(dir.clone());
        }
        if let Some(m) = CD.captures(&seg) {
            dir = cd(dir.as_deref(), m.get(1).map_or("", |g| g.as_str()), home);
        }
        out.push((span, dir.clone()));
        for _ in 0..edge(&mut seg.chars().rev(), ')') {
            if let Some(before) = saved.pop() {
                dir = before;
            }
        }
    }
    out
}

/// The folder after `cd <words>`: home with no path, None for `cd -` or a path it cannot read; a redirection is no path.
fn cd(dir: Option<&Path>, words: &str, home: &Path) -> Option<PathBuf> {
    let paths: Vec<&str> = words
        .split_whitespace()
        .filter(|w| !w.contains(['<', '>']) && !matches!(*w, "-P" | "-L" | "--"))
        .collect();
    match paths[..] {
        [] => Some(home.to_path_buf()),
        [path] if path != "-" => resolve(dir, path, home),
        _ => None,
    }
}

/// `path` from `dir`, `~` read as home, `.` and `..` folded; None for a path quoted or holding a variable, or relative to
/// an unknown folder.
pub(super) fn resolve(dir: Option<&Path>, path: &str, home: &Path) -> Option<PathBuf> {
    if path.contains(['$', '`', '\'', '"']) {
        return None;
    }
    let joined = match path.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ if path.starts_with('/') => PathBuf::from(path),
        _ => dir?.join(path),
    };
    Some(folded(&joined))
}

/// The last part of a command word's path: `/usr/bin/sed` is sed.
pub(super) fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

#[derive(Clone, Copy)]
enum State {
    Space,
    Word,
    Punct,
    Quote(char),
    /// a backslash, and the state it returns to
    Escape(char),
}

/// The guards' word splitter, download-guard's and scripted-edit's: Python's
/// `shlex.shlex(text, posix=True, punctuation_chars=True)` with whitespace_split, its comments on or off; Err where
/// it raises ValueError (an open quote, a backslash at the end).
pub(super) fn shlex(text: &str, comments: bool) -> Result<Vec<String>, ()> {
    let punct = |c: char| "();<>|&".contains(c);
    let ws = |c: char| " \t\r\n".contains(c);
    let mut chars = text.chars();
    let (mut out, mut tok, mut quoted) = (vec![], String::new(), false);
    let mut pushback: Option<char> = None;
    let mut state = State::Space;
    let emit = |out: &mut Vec<String>, tok: &mut String, quoted: &mut bool| {
        if !tok.is_empty() || *quoted {
            out.push(std::mem::take(tok));
        }
        *quoted = false;
    };
    loop {
        let next = pushback.take().or_else(|| chars.next());
        match state {
            State::Space => match next {
                None => return Ok(out),
                Some(c) if ws(c) => {}
                Some('#') if comments => {
                    chars.by_ref().find(|&c| c == '\n');
                }
                Some('\\') => state = State::Escape('a'),
                Some(c) if punct(c) => {
                    tok.push(c);
                    state = State::Punct;
                }
                Some(c @ ('\'' | '"')) => state = State::Quote(c),
                Some(c) => {
                    tok.push(c);
                    state = State::Word;
                }
            },
            State::Word | State::Punct => match next {
                None => {
                    emit(&mut out, &mut tok, &mut quoted);
                    return Ok(out);
                }
                Some(c) if ws(c) => {
                    state = State::Space;
                    emit(&mut out, &mut tok, &mut quoted);
                }
                Some('#') if comments => {
                    chars.by_ref().find(|&c| c == '\n');
                    state = State::Space;
                    emit(&mut out, &mut tok, &mut quoted);
                }
                Some(c) if matches!(state, State::Punct) => {
                    if punct(c) {
                        tok.push(c);
                    } else {
                        pushback = Some(c);
                        state = State::Space;
                        emit(&mut out, &mut tok, &mut quoted);
                    }
                }
                Some(c @ ('\'' | '"')) => state = State::Quote(c),
                Some('\\') => state = State::Escape('a'),
                Some(c) if !punct(c) => tok.push(c),
                Some(c) => {
                    pushback = Some(c);
                    state = State::Space;
                    emit(&mut out, &mut tok, &mut quoted);
                }
            },
            State::Quote(q) => {
                quoted = true;
                match next {
                    None => return Err(()),
                    Some(c) if c == q => state = State::Word,
                    Some('\\') if q == '"' => state = State::Escape('"'),
                    Some(c) => tok.push(c),
                }
            }
            State::Escape(back) => {
                let c = next.ok_or(())?;
                // in a quote, only the quote itself or the backslash is escaped
                if back == '"' && c != '\\' && c != '"' {
                    tok.push('\\');
                }
                tok.push(c);
                state = if back == 'a' {
                    State::Word
                } else {
                    State::Quote(back)
                };
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// What may stand before a command word, one case per word of [`LEAD`], old and new, with the flags and values Claude
    /// Code's reader skips: each guard built on LEAD runs its command behind every one (review of PR 1849).
    pub const LEADS: [&str; 46] = [
        "if",
        "elif",
        "while",
        "until",
        "coproc",
        "do",
        "then",
        "else",
        "!",
        "A=1",
        "$SUDO",
        "${SUDO}",
        "${SUDO:-}",
        "env",
        "env -i",
        "env -u A",
        "exec",
        "nohup",
        "time",
        "time -p",
        "command",
        "sudo",
        "sudo -n",
        "sudo -u root",
        "/usr/bin/sudo",
        "doas",
        "doas -u root",
        "xargs",
        "xargs -0",
        "xargs -n 1",
        "nice",
        "nice -n 10",
        "timeout 60",
        "timeout 5m",
        "timeout -s KILL 60",
        "timeout --signal=KILL 60",
        "stdbuf -oL",
        "setsid",
        "ionice -c 3",
        "busybox",
        "eval",
        "pkexec",
        "runuser -u x",
        "sudo env A=1 nice",
        "if ! sudo -n",
        "true; then",
    ];

    /// The one heredoc reader: each body, its quoting (`<<\EOF` included), by its `<<`; and a `<<\EOF` body fed to a shell
    /// stays code for the guards built on `code`.
    #[dash_test]
    fn heredocs_of_reads_each_body_and_its_quoting() {
        let cmd = "python3 - <<\\X\nprint(1)\nX\ncat <<EOF\n$HOME\nEOF";
        let docs = heredocs_of(cmd);
        assert_eq!(
            docs,
            [
                Heredoc {
                    at: 10,
                    quoted: true,
                    body: "print(1)".into()
                },
                Heredoc {
                    at: 30,
                    quoted: false,
                    body: "$HOME".into()
                },
            ]
        );
        assert!(code("bash <<\\EOF\nrm -rf x\nEOF").contains("rm -rf x"));
        // a conflict marker before any whitespace counts, a tab as a space (git's --check)
        assert_eq!(conflict_marker("a\n<<<<<<<\tHEAD\n"), Some(2));
        assert_eq!(conflict_marker(">>>>>>> main"), Some(1));
        assert_eq!(conflict_marker("<<<<<<<<< x\n"), None);
    }

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
