//! The workflows a pull request should get: each file of its checkout's `.github/workflows` whose `pull_request` trigger takes
//! the PR's changed files, by GitHub's filter rules (`paths`, `paths-ignore`, `!`, `*`, `**`). A head is judged only once
//! each of them has a run on it: a run GitHub has not listed yet is no pass (task rust-ci-wait-skipped: `dash ci wait 1879`
//! said « green on the head (1 checks) » 30 s after the PR opened, its one check Blacksmith's skipped Proof; GitHub listed
//! explainer-gate's run 77 s and superapp's 106 s after the PR opened, so a grace of seconds would have judged too).
//! Read from the files, never a list kept by hand: a workflow added or a path filter changed is waited on at once.
//! A wrong expectation costs a wait, a wrong omission a false green: what this reader cannot judge is not waited on, as
//! before this file. ponytail: the subset of YAML these files use (block keys, quoted or not, flow `[a, b]` and block
//! `- a` lists, `#` comments). Not judged: a `branches` filter, a flow map, `types` without opened and synchronize, a
//! pattern with `?`, `+` or `[` (GitHub's own meanings, not read here), a PR whose files GitHub listed only in part.
use std::path::Path;

/// A workflow a pull request may start: its name as GitHub lists it (`name:`, else its file's path), its file, its filter.
#[derive(Debug, PartialEq)]
pub struct Workflow {
    pub name: String,
    file: String,
    paths: Paths,
}

#[derive(Debug, PartialEq)]
enum Paths {
    All,
    Only(Vec<String>),
    Ignore(Vec<String>),
}

impl Workflow {
    /// Whether a pull request changing `files` starts it, as far as this checkout can tell. `more`: GitHub listed only some of
    /// its files, so a filtered workflow is not judged. A PR changing this very file runs its own version, maybe renamed or
    /// filtered otherwise: not judged either (/code-review of rust-ci-wait-skipped: main's name would wait for ever).
    pub fn takes(&self, files: &[&str], more: bool) -> bool {
        if files.contains(&self.file.as_str()) {
            return false;
        }
        match &self.paths {
            Paths::All => true,
            _ if more => false,
            Paths::Only(p) => files.iter().any(|f| filtered(p, f)),
            Paths::Ignore(p) => files.iter().any(|f| !filtered(p, f)),
        }
    }
}

/// The workflows under `dir` (a checkout's `.github/workflows`) a pull request starts, by name; none read, none.
pub fn read(dir: &Path) -> Vec<Workflow> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let yml = |p: &Path| p.extension().is_some_and(|x| x == "yml" || x == "yaml");
    let mut all: Vec<Workflow> = (entries.filter_map(Result::ok))
        .filter(|e| yml(&e.path()))
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            let path = format!(".github/workflows/{}", e.file_name().to_string_lossy());
            parse(&path, &text)
        })
        .collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

/// The workflow file `text` at `path` as a pull request trigger, or `None`: no `pull_request`, or one this reader cannot judge.
fn parse(path: &str, text: &str) -> Option<Workflow> {
    let lines: Vec<&str> = (text.lines().map(uncomment))
        .filter(|l| !l.trim().is_empty())
        .collect();
    let name = (lines.iter().find_map(|l| value(l, "name")))
        .map(unquote)
        .filter(|n| !n.is_empty())
        .unwrap_or(path)
        .to_string();
    let file = path.to_string();
    let all = |name| Workflow {
        name,
        file: file.clone(),
        paths: Paths::All,
    };
    let at = lines.iter().position(|l| value(l, "on").is_some())?;
    let inline = value(lines[at], "on").unwrap_or_default();
    let triggers = children(&lines[at + 1..], 0);
    // `on: pull_request`, `on: [push, pull_request]` or a block list of triggers: no filter
    let listed = match inline {
        "" if triggers.first()?.trim_start().starts_with("- ") => Some(block_list(&triggers, 0)),
        "" => None,
        inline => Some(items(inline)),
    };
    if let Some(listed) = listed {
        return listed
            .iter()
            .any(|w| w == "pull_request")
            .then(|| all(name));
    }
    let level = indent(triggers.first()?);
    let pr = (triggers.iter().enumerate())
        .find(|(_, l)| indent(l) == level && value(l.trim_start(), "pull_request").is_some())?;
    let said = value(pr.1.trim_start(), "pull_request").unwrap_or_default();
    if !matches!(said, "" | "{}" | "null" | "~") {
        return None;
    }
    let filters = children(&triggers[pr.0 + 1..], level);
    let (mut only, mut ignore) = (None, None);
    let level = filters.first().map(|l| indent(l));
    for (i, l) in filters.iter().enumerate() {
        // a block list's items may sit at their key's level: read by `block_list`
        if Some(indent(l)) != level || l.trim_start().starts_with("- ") {
            continue;
        }
        let (key, said) = l.trim_start().split_once(':')?;
        let list = match said.trim() {
            "" => block_list(&filters[i + 1..], indent(l)),
            said => items(said),
        };
        match unquote(key) {
            "paths" => only = Some(list),
            "paths-ignore" => ignore = Some(list),
            // the default types are opened, synchronize and reopened: a PR's push and its opening both start it
            "types"
                if ["opened", "synchronize"]
                    .iter()
                    .all(|t| list.iter().any(|w| w == t)) => {}
            _ => return None,
        }
    }
    let unread = |p: &Vec<String>| p.iter().any(|p| p.contains(['?', '+', '[']));
    if only.iter().chain(&ignore).any(unread) {
        return None;
    }
    let paths = match (only, ignore) {
        (None, None) => Paths::All,
        (Some(p), None) => Paths::Only(p),
        (None, Some(p)) => Paths::Ignore(p),
        (Some(_), Some(_)) => return None,
    };
    Some(Workflow { paths, ..all(name) })
}

/// A line without its `#` comment: a `#` at its start or after a space, outside quotes.
fn uncomment(l: &str) -> &str {
    let mut quote = None;
    let mut last = ' ';
    for (i, c) in l.char_indices() {
        match (quote, c) {
            (None, '#') if last.is_whitespace() => return &l[..i],
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            _ => {}
        }
        last = c;
    }
    l
}

/// `key:`'s value on a line that starts with it, bare or quoted (`'on':`), trimmed: `Some("")` for a key alone.
fn value<'a>(l: &'a str, key: &str) -> Option<&'a str> {
    let rest = [key.to_string(), format!("'{key}'"), format!("\"{key}\"")]
        .into_iter()
        .find_map(|k| l.strip_prefix(k.as_str()))?;
    rest.strip_prefix(':').map(str::trim)
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

/// The lines after a key at `level`, up to the next line at that level or less.
fn children<'a>(lines: &[&'a str], level: usize) -> Vec<&'a str> {
    let deeper = lines.iter().take_while(|l| indent(l) > level);
    deeper.copied().collect()
}

/// A block list's items under a key at `level`: `- a` lines, deeper or at its level, as YAML allows.
fn block_list(lines: &[&str], level: usize) -> Vec<String> {
    let item = |l: &&&str| indent(l) >= level && l.trim_start().starts_with("- ");
    let items = lines.iter().take_while(item);
    items
        .map(|l| unquote(&l.trim_start()[2..]).to_string())
        .collect()
}

/// A flow list's items, `[a, "b"]`, or a lone scalar.
fn items(said: &str) -> Vec<String> {
    let said = said.trim();
    let inner = (said.strip_prefix('[').and_then(|s| s.strip_suffix(']'))).unwrap_or(said);
    let words = inner.split(',').map(unquote).filter(|w| !w.is_empty());
    words.map(String::from).collect()
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    let quoted = |q: char| s.len() >= 2 && s.starts_with(q) && s.ends_with(q);
    if quoted('"') || quoted('\'') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Whether `patterns` take `file`: the last pattern matching it decides, a `!` one leaving it out (GitHub's rule).
fn filtered(patterns: &[String], file: &str) -> bool {
    patterns
        .iter()
        .fold(false, |hit, p| match p.strip_prefix('!') {
            Some(not) if glob(not.as_bytes(), file.as_bytes()) => false,
            None if glob(p.as_bytes(), file.as_bytes()) => true,
            _ => hit,
        })
}

/// GitHub's filter pattern: `*` any characters but `/`, `**` any at all, `**/` none or some folders (`parse` judges no
/// pattern holding `?`, `+` or `[`).
fn glob(p: &[u8], s: &[u8]) -> bool {
    match p {
        [] => s.is_empty(),
        [b'*', b'*', b'/', rest @ ..] => {
            glob(rest, s) || (0..s.len()).any(|i| s[i] == b'/' && glob(rest, &s[i + 1..]))
        }
        [b'*', b'*', rest @ ..] => (0..=s.len()).any(|i| glob(rest, &s[i..])),
        [b'*', rest @ ..] => (0..=s.len())
            .take_while(|&i| i == 0 || s[i - 1] != b'/')
            .any(|i| glob(rest, &s[i..])),
        [c, rest @ ..] => s.first() == Some(c) && glob(rest, &s[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// This checkout's own workflows: every pull request gets explainer-gate, one touching dash/ superapp too, one changing only
    /// dash/PLAN.md not superapp, and a push-only workflow none.
    #[dash_test]
    fn this_repository_s_workflows_are_read_from_their_files() {
        // the public mirror holds rust/ alone: there the test stops
        let Some(repo) = dash_meter::monorepo() else {
            return;
        };
        let all = read(&repo.join(".github/workflows"));
        let names: Vec<&str> = all.iter().map(|w| w.name.as_str()).collect();
        assert!(
            names.contains(&"explainer-gate") && names.contains(&"superapp"),
            "{names:?}"
        );
        assert!(
            !names.contains(&"approval-number"),
            "a push-only workflow: {names:?}"
        );
        let started = |files: &[&str], more| -> Vec<&str> {
            let on = all.iter().filter(|w| w.takes(files, more));
            on.map(|w| w.name.as_str()).collect()
        };
        let forge = "rust/crates/forge/src/lib.rs";
        assert_eq!(started(&[forge], false), ["explainer-gate"]);
        assert_eq!(
            started(&["dash/core/src/lib.rs"], false),
            ["explainer-gate", "superapp"]
        );
        assert_eq!(started(&["dash/PLAN.md"], false), ["explainer-gate"]);
        assert_eq!(
            started(&["site/index.html"], false),
            ["explainer-gate", "site"]
        );
        // GitHub listed only some of its files: a filtered workflow is not judged
        assert_eq!(started(&[forge], true), ["explainer-gate"]);
        // a PR changing a workflow runs its own version: that one is not judged
        let own = ".github/workflows/superapp.yml";
        assert_eq!(
            started(&["dash/core/src/lib.rs", own], false),
            ["explainer-gate"]
        );
    }

    #[dash_test]
    fn a_trigger_is_read_in_each_form_and_one_not_judged_is_none() {
        let w = |t: &str| parse(".github/workflows/x.yml", t);
        let with = |name: &str, paths| {
            Some(Workflow {
                name: name.into(),
                file: ".github/workflows/x.yml".into(),
                paths,
            })
        };
        let all = |name: &str| with(name, Paths::All);
        assert_eq!(
            w("on: pull_request\njobs:\n"),
            all(".github/workflows/x.yml")
        );
        assert_eq!(w("name: x\non: [push, pull_request]\n"), all("x"));
        assert_eq!(w("name: x\non:\n  - push\n  - pull_request\n"), all("x"));
        assert_eq!(w("name: x\n'on':\n  \"pull_request\":\n"), all("x"));
        assert_eq!(w("name: x\non: [push]\n"), None);
        assert_eq!(w("name: x\non:\n  pull_request_target:\n"), None);
        assert_eq!(
            w("name: x\non:\n  push:\n  pull_request:\njobs:\n  a:\n"),
            all("x")
        );
        // comments: a whole line, after a key, after a list item; a `#` inside quotes or a word stays
        let block = "name: 'x' # its CI\non:\n  pull_request: # why\n    # why\n    paths-ignore:\n    - \"docs/**\" # docs\n    - '*.md'\n    - 'a#b'\n  push:\n";
        let ignore = Paths::Ignore(vec!["docs/**".into(), "*.md".into(), "a#b".into()]);
        assert_eq!(w(block), with("x", ignore));
        assert_eq!(uncomment("a#b # c"), "a#b ");
        assert_eq!(
            w("name: x\non:\n  pull_request:\n    branches: [main]\n"),
            None
        );
        assert_eq!(
            w("name: x\non:\n  pull_request:\n    types: [labeled]\n"),
            None
        );
        assert_eq!(w("name: x\non:\n  pull_request: {paths: [a]}\n"), None);
        let both = "name: x\non:\n  pull_request:\n    paths: [a]\n    paths-ignore: [b]\n";
        assert_eq!(w(both), None);
        // GitHub's `?` and `+` are not read here
        assert_eq!(
            w("name: x\non:\n  pull_request:\n    paths: [docs?/**]\n"),
            None
        );
    }

    #[dash_test]
    fn a_pattern_reads_as_github_s() {
        let on =
            |p: &[&str], f: &str| filtered(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>(), f);
        assert!(on(&["dash/**"], "dash/a/b.rs") && !on(&["dash/**"], "dashboard/a"));
        assert!(on(&["*.md"], "a.md") && !on(&["*.md"], "docs/a.md"));
        assert!(
            on(&["**.md"], "docs/a.md") && on(&["**/a.md"], "a.md") && on(&["**/a.md"], "x/y/a.md")
        );
        assert!(
            on(&["bin/sandbox-apply"], "bin/sandbox-apply")
                && !on(&["bin/sandbox-apply"], "bin/sandbox-applyx")
        );
        // the last match decides
        assert!(!on(&["dash/**", "!dash/PLAN.md"], "dash/PLAN.md"));
        assert!(on(
            &["dash/**", "!dash/rfcs/**", "dash/rfcs/keep.md"],
            "dash/rfcs/keep.md"
        ));
        let ignore = Workflow {
            name: "x".into(),
            file: ".github/workflows/x.yml".into(),
            paths: Paths::Ignore(vec!["docs/**".into()]),
        };
        assert!(!ignore.takes(&["docs/a"], false) && ignore.takes(&["docs/a", "src/b"], false));
    }
}
