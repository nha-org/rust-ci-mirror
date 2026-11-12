//! `dash sandbox gate <dir>`, which `make sandbox/check` runs: whether merging the checkout at `dir` (a PR merged with main, in
//! its reviewer's worktree) needs his tap first. Ported from `Dashboard.MergeGate` (task rust-merge-gate-port; @nha, 2026-11-10:
//! « remember we move to rust. no more elixir, so if something needs fixing port and then fix »). PR 311 merged a Makefile,
//! herdr/session.sh and bin/crm-phone that launched through the approved copy's sandbox-apply with a verb only his next tap
//! brought: every launch failed until PR 317 reverted it (task merge-needs-tap-gate; @nha, 2026-10-22: « less approvals from me
//! means less time blocked for you »).
//!
//! It fails on each line of a host file that follows main (changed since the approved commit, outside the tapped set; docs,
//! tests, data and private/ aside) that calls a `sandbox-apply` verb the PR's script has and the approved one lacks, or reads a
//! `host/` path the approved copy lacks. That copy is what the approved script makes (bin/sandbox-apply host_copy): its `SET=`
//! files as at the approved commit, its own `MERGED=` list's files as at HEAD, and what host_copy itself writes into it at run
//! time (`.merged`, which bin/crm-phone reads: the Elixir gate refused every PR for it). A `MERGED=` file runs from the host
//! copy, which `sync` leaves as it is while main's tapped set differs from the approved one: its lines may call any verb of the
//! PR's own script, and a verb that script lacks is a hit; in the Makefile, which his terminal also runs from the working
//! tree, only the lines of its `ON_APPROVED_COPY` half. The diff runs from the approved commit, not from main: what the merge
//! puts in force is judged whole. Any git error fails it: a gate that cannot read says so, never OK. Here, not in
//! bin/sandbox-apply: a change there is his to tap, and main holding an untapped one stops every sync of the host copy.
//!
//! ponytail: in a host-copy line, any word after `sandbox-apply` the PR's script lacks is a hit, a log line's prose included
//! (« sandbox-apply has no launch yet »): it holds a merge rather than miss a call. A call through a variable
//! (`sandbox-apply "$verb"`), a path built in pieces (`$host/bin/$2`) or one from Elixir code (private/ is skipped but for
//! priv/commands: its prose names the verbs) is not seen. It is a reviewer's step, not CI's: the approved sha is local to his
//! Mac (.claude/run/approved), which CI's checkout has not.
//!
//! graal (a checkout whose main one is named graal, its herdr worktrees too) keeps its approved sha in ~/.config/graal/approved
//! and calls its approved set through `git show <sha>:scripts/sandbox/...`: [`graal_lacks`].
use regex::Regex;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

pub const USAGE: &str = "dash sandbox gate <dir of the worktree to merge>";

fn re(p: &str) -> Regex {
    Regex::new(p).expect("merge gate regex")
}

static VERB: LazyLock<Regex> =
    LazyLock::new(|| re(r#"sandbox-apply(?:["')]|</?string>|[ \t])+(-{0,2}[a-z][a-z-]*)"#));
/// `host/<path>` not inside a longer word: the start, or any character but a word one, a dot or a dash, before it. Word
/// characters are ASCII's `[:word:]`, as Elixir's `\w` was: the regex crate's `\w` is Unicode's (/code-review).
static HOST: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:^|[^[:word:].-])host/([[:word:].][[:word:]./-]*[[:word:]])"));
/// docs, data and tests, a Rust crate's `tests/` too (its fixtures name host files: main failed its own gate on merge_gate.rs,
/// review-lane, 2026-11-12); private/ apart from priv/commands, whose scripts and .json hooks call host files (`skipped`).
/// ponytail: a `#[cfg(test)]` module in a crate's src/ is still read; skip Rust string literals if one ever holds main.
static SKIP: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(\.md|\.html|\.edn|\.jsonl|-test|_test\.exs)$|(^|/)test/|^rust/crates/[^/]+/tests/")
});
/// the case labels of a sandbox-apply, `  sync)` or `  approve | auto) …`: its verbs
static LABELS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?m)^[ \t]*(-{0,2}[a-z][a-z-]*(?:[ \t]*\|[ \t]*-{0,2}[a-z][a-z-]*)*)\)"));
/// a whole-line comment (#, Makefile's @#, ;, //) is prose; a shell « ;; » is code
static COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(@?#|;([^;]|$)|//)"));
static TRAILING: LazyLock<Regex> = LazyLock::new(|| re(r"\s#\s.*$"));
/// what host_copy writes into its copy past the archives: stdout sent there, `echo "$m" > "$tmp/.merged"` or `>>`; never a
/// `2>` or `&>` (a log, not the copy's: /code-review)
static WRITTEN: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?:^|[^0-9&>])>>?\s*"?\$tmp/([[:word:].][[:word:]./-]*[[:word:]])"#));

fn skipped(f: &str) -> bool {
    SKIP.is_match(f)
        || (f.starts_with("private/") && !f.starts_with("private/dashboard/priv/commands/"))
}

/// The verdict for the checkout at `dir`: `Ok(line)` when the merge needs no tap, else `Err(what it lacks)`, or why it cannot
/// judge. `graal_approved` names graal's approved file (a test's).
pub fn run(dir: &Path, graal_approved: Option<&Path>) -> Result<String, String> {
    let cannot = |why: String| format!("merge gate: cannot judge: {why}");
    if !dir.is_dir() {
        return Err(cannot(format!("{} is not a folder", dir.display())));
    }
    // git first: outside a checkout, the top level and the main one both fall back to dir and it read as the main one
    let s = git(dir, &["rev-parse", "HEAD"]).map_err(cannot)?;
    let main = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(|c| {
        PathBuf::from(c)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    })
    .map_err(cannot)?;
    let top = git(dir, &["rev-parse", "--show-toplevel"]).map_err(cannot)?;
    // the main checkout's main is what he taps: there the gate would only stop the apply that fixes it
    if Path::new(&top) == main {
        return Ok(format!(
            "merge gate: {} is the main checkout, not a merge to judge",
            dir.display()
        ));
    }
    let graal = main.file_name().is_some_and(|n| n == "graal");
    let a = approved(&main, graal, graal_approved).map_err(cannot)?;
    let bad = if graal {
        graal_lacks(dir, &a, &s)
    } else {
        lacks(dir, &a, &s)
    }
    .map_err(cannot)?;
    // graal has no forge.env, and its default branch is main, as site_perso's (/code-review)
    let branch = if main.join("forge.env").is_file() {
        crate::hook::default_branch(&main)
    } else {
        "main".into()
    };
    let short = &a[..12];
    if bad.is_empty() {
        return Ok(format!(
            "merge gate OK: no file that follows {branch} calls what the approved copy ({short}) lacks"
        ));
    }
    let then = if graal {
        "Merge the sandbox change alone, let him approve it (graal's scripts/sandbox/approve.sh), then the callers"
    } else {
        "Merge the sandbox change alone, let him tap it, then the callers (PR 317 reverted PR 311 for this)"
    };
    Err(format!(
        "merge gate: merging this needs his tap first: these files follow {branch}, and the approved copy ({short}) lacks \
         what they call:\n{}\n{then}",
        bad.join("\n")
    ))
}

/// Each « file:line: what » of `s` the approved copy at `a` lacks, in `dir`'s repository, or why it cannot read.
pub fn lacks(dir: &Path, a: &str, s: &str) -> Result<Vec<String>, String> {
    let ka = git(dir, &["show", &format!("{a}:bin/sandbox-apply")])?;
    let ks = git(dir, &["show", &format!("{s}:bin/sandbox-apply")]).unwrap_or_default();
    let set = list_of(&ka, "SET", "bin/sandbox-apply")?;
    let merged = list_of(&ka, "MERGED", "bin/sandbox-apply")?;
    let tapped: Vec<String> = set.iter().cloned().chain(list(&ks, "SET")).collect();
    let files: Vec<String> = paths(dir, &["diff", "-z", "--name-only", a, s])?
        .into_iter()
        .filter(|f| !tapped.contains(f) && !skipped(f))
        .collect();
    if files.is_empty() {
        return Ok(vec![]);
    }
    let have = copy(dir, a, s, &set, &merged, &ka)?;
    // `new`: the verbs the PR's script has and the approved one lacks, a hit from a line that follows main at once; `own`:
    // the PR's script's verbs, the only ones a host-copy line may call, since sync takes it with that script (a MERGED call
    // to a verb no script has, once a refused tap's script is reverted, is a hit: /code-review)
    let own = verbs(&ks);
    let old = verbs(&ka);
    let new: Vec<&String> = own.iter().filter(|v| !old.contains(v)).collect();
    let copy_lines = if files.iter().any(|f| f == "Makefile") {
        approved_half(&git(dir, &["show", &format!("{s}:Makefile")])?)
    } else {
        BTreeSet::new()
    };
    let mut bad = vec![];
    for (f, n, text) in grep(dir, s, "sandbox-apply|host/", &files)? {
        // A file of the approved MERGED list runs from the host copy, which `sync` leaves as it is while main's SET differs
        // from the approved one: it may call its own script's verbs (task approve-policy-manifest). The Makefile also runs
        // from the working tree (make claude at his terminal): only its ON_APPROVED_COPY half is the copy's.
        let copy_only = if f == "Makefile" {
            copy_lines.contains(&n)
        } else {
            merged.contains(&f)
        };
        let text = uncomment(&text);
        for c in VERB.captures_iter(&text) {
            let v = c[1].to_string();
            let lacked = if copy_only {
                !own.contains(&v)
            } else {
                new.contains(&&v)
            };
            if lacked {
                bad.push(format!("{f}:{n}: sandbox-apply {v}"));
            }
        }
        for c in HOST.captures_iter(&text) {
            if !have.contains(&c[1]) {
                bad.push(format!("{f}:{n}: host/{}", &c[1]));
            }
        }
    }
    Ok(bad)
}

/// The numbers of a Makefile's lines under `ifdef ON_APPROVED_COPY`, or the `else` of `ifndef ON_APPROVED_COPY`.
pub fn approved_half(text: &str) -> BTreeSet<usize> {
    let mut stack: Vec<Option<bool>> = vec![];
    let mut set = BTreeSet::new();
    for (i, l) in text.split('\n').enumerate() {
        // a recipe line (tab-led) is shell: its `if`/`else` is never make's (/code-review)
        let words: Vec<&str> = if l.starts_with('\t') {
            vec![]
        } else {
            l.split_whitespace().collect()
        };
        match words[..] {
            ["ifdef", "ON_APPROVED_COPY"] => stack.push(Some(true)),
            ["ifndef", "ON_APPROVED_COPY"] => stack.push(Some(false)),
            [w, ..] if w.starts_with("if") => stack.push(None),
            ["else", ..] => {
                if let Some(Some(b)) = stack.last_mut() {
                    *b = !*b;
                }
            }
            ["endif", ..] => {
                stack.pop();
            }
            _ => {}
        }
        if stack.contains(&Some(true)) {
            set.insert(i + 1);
        }
    }
    set
}

/// `(file, line, text)` of each line of `files` at `s` matching `pattern`; -z: a path may hold a colon.
fn grep(
    dir: &Path,
    s: &str,
    pattern: &str,
    files: &[String],
) -> Result<Vec<(String, usize, String)>, String> {
    let mut args = vec!["grep", "-I", "-n", "-z", "-E", pattern, s, "--"];
    args.extend(files.iter().map(String::as_str));
    let out = git_out(dir, &args)?;
    let found = match out.status.code() {
        Some(0) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Some(1) if out.stdout.is_empty() => String::new(),
        code => return Err(format!("git grep exited {}", code.unwrap_or(-1))),
    };
    let prefix = format!("{s}:");
    found
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut parts = l.splitn(3, '\0');
            match (
                parts.next(),
                parts.next().and_then(|n| n.parse().ok()),
                parts.next(),
            ) {
                (Some(p), Some(n), Some(t)) => Ok((
                    p.strip_prefix(&prefix).unwrap_or(p).to_string(),
                    n,
                    t.to_string(),
                )),
                _ => Err(format!("git grep printed a line it cannot read: {l}")),
            }
        })
        .collect()
}

/// A whole-line comment or a trailing « # … » is prose, not a call.
/// ponytail: a « # » inside a quoted string, or a call split by a trailing backslash, is not seen.
fn uncomment(text: &str) -> String {
    if COMMENT.is_match(text) {
        String::new()
    } else {
        TRAILING.replace(text, "").into_owned()
    }
}

fn verbs(k: &str) -> Vec<String> {
    LABELS
        .captures_iter(k)
        .flat_map(|c| {
            c[1].split('|')
                .map(|v| v.trim().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The one-line `name=(…)` list of a sandbox-apply's text `k` (SET, MERGED, CANDS), or none.
pub fn list(k: &str, name: &str) -> Vec<String> {
    let head = format!("{name}=(");
    k.lines()
        .find_map(|l| l.strip_prefix(&head)?.strip_suffix(')'))
        .map(|l| l.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// An empty list would make ls-tree list the whole tree, and every host/ path would pass.
fn list_of(k: &str, name: &str, file: &str) -> Result<Vec<String>, String> {
    let l = list(k, name);
    if l.is_empty() {
        return Err(format!("no one-line {name}=( in the approved {file}"));
    }
    Ok(l)
}

/// The host copy's files and each folder above them ("$conf/host/sandbox" is a folder of it), and what the approved
/// host_copy writes into it at run time: bin/crm-phone reads host/.merged, which no commit holds (task rust-merge-gate-port).
fn copy(
    dir: &Path,
    a: &str,
    s: &str,
    set: &[String],
    merged: &[String],
    ka: &str,
) -> Result<BTreeSet<String>, String> {
    let tree = |at: &str, of: &[String]| {
        let mut args = vec!["ls-tree", "-r", "-z", "--name-only", at, "--"];
        args.extend(of.iter().map(String::as_str));
        paths(dir, &args)
    };
    let mut have = BTreeSet::new();
    for f in tree(a, set)?
        .into_iter()
        .chain(tree(s, merged)?)
        .chain(written(ka))
    {
        let mut at = String::new();
        for part in f.split('/') {
            if !at.is_empty() {
                at.push('/');
            }
            at.push_str(part);
            have.insert(at.clone());
        }
    }
    Ok(have)
}

/// The paths a sandbox-apply's `host_copy` writes into its copy past the archives (`.merged`): read from that function alone,
/// since another `$tmp` (mirror_init's) is no copy. The function runs from its header, `host_copy()`, `host_copy ()` or
/// `function host_copy`, to the first line opening on `}` (a trailing comment or space included: /code-review).
/// ponytail: a write through another variable or `tee` is not seen; the read is then refused, never let through.
fn written(k: &str) -> Vec<String> {
    let header = |l: &str| {
        let l = l.strip_prefix("function ").unwrap_or(l);
        l.strip_prefix("host_copy")
            .is_some_and(|r| r.trim_start().starts_with(['(', '{']) || r.is_empty())
    };
    let body: String = k
        .lines()
        .skip_while(|l| !header(l))
        .take_while(|l| !l.starts_with('}'))
        .collect::<Vec<_>>()
        .join("\n");
    WRITTEN
        .captures_iter(&body)
        .map(|c| c[1].to_string())
        .collect()
}

/// unlike SKIP, a .edn is read: graal's bb.edn reads scripts/sandbox/launch.sh at the approved sha
static GRAAL_SKIP: LazyLock<Regex> =
    LazyLock::new(|| re(r"(\.md|\.html|-test|_test\.exs)$|(^|/)test/"));
/// « <sha>:scripts/sandbox/x » (git show) runs as approved; « bash scripts/sandbox/x » runs from the tree
static GRAAL_PATH: LazyLock<Regex> =
    LazyLock::new(|| re(r":(scripts/sandbox/[[:word:].][[:word:]./-]*[[:word:]])"));
/// a whole-line comment, Clojure's « ;; » included, is prose
static GRAAL_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(#|;|//)"));

/// graal's [`lacks`]: a file outside its approved set (launch.sh's `SET=(...)` at `a`) that follows main and reads a
/// `scripts/sandbox/` file at an approved sha (`git show "$SHA:scripts/sandbox/hook.sh"`) the approved commit `a` lacks needs
/// his tap first.
///
/// ponytail: only the file's existence is judged, not a subcommand or flag the approved copy lacks (`hook.sh newverb`);
/// graal's own `sandbox-check` is the reviewer's step for that.
pub fn graal_lacks(dir: &Path, a: &str, s: &str) -> Result<Vec<String>, String> {
    let launch = "scripts/sandbox/launch.sh";
    let set = list_of(
        &git(dir, &["show", &format!("{a}:{launch}")])?,
        "SET",
        launch,
    )?;
    let have: BTreeSet<String> = paths(
        dir,
        &[
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            a,
            "--",
            "scripts/sandbox",
        ],
    )?
    .into_iter()
    .collect();
    let files: Vec<String> = paths(dir, &["diff", "-z", "--name-only", a, s])?
        .into_iter()
        .filter(|f| !set.iter().any(|p| f.starts_with(p.as_str())) && !GRAAL_SKIP.is_match(f))
        .collect();
    if files.is_empty() {
        return Ok(vec![]);
    }
    let mut bad = vec![];
    for (f, n, text) in grep(dir, s, ":scripts/sandbox/", &files)? {
        if GRAAL_COMMENT.is_match(&text) {
            continue;
        }
        for c in GRAAL_PATH.captures_iter(&TRAILING.replace(&text, "")) {
            if !have.contains(&c[1]) {
                bad.push(format!("{f}:{n}: {}", &c[1]));
            }
        }
    }
    Ok(bad)
}

/// No record, or one it cannot read, is no verdict: never OK.
fn approved(main: &Path, graal: bool, graal_approved: Option<&Path>) -> Result<String, String> {
    let f = match (graal, graal_approved) {
        (true, Some(f)) => f.to_path_buf(),
        // no HOME is no record, never a path read against the current folder (/code-review)
        (true, None) => match std::env::var("HOME") {
            Ok(h) if Path::new(&h).is_absolute() => Path::new(&h).join(".config/graal/approved"),
            _ => return Err("no approved commit on record: HOME is not set".into()),
        },
        (false, _) => main.join(".claude/run/approved"),
    };
    let a = std::fs::read_to_string(&f)
        .map_err(|e| format!("no approved commit on record: {}: {e}", f.display()))?;
    let a = a.trim();
    if a.len() == 40 && a.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(a.to_string())
    } else {
        Err(format!("{} holds no full sha", f.display()))
    }
}

/// -z output: a path git would otherwise quote (non-ASCII, a space) matches as a pathspec
fn paths(dir: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(git(dir, args)?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect())
}

/// stdout only, trailing whitespace trimmed: a warning on stderr would read as a line of output
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_out(dir, args)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        // git's own words: a « cannot judge » says why (/code-review)
        Err(format!(
            "git {} {} exited {}: {}",
            args[0],
            args.get(1).unwrap_or(&""),
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The repository is `dir`'s, whoever calls: git's hook variables (a pre-commit's GIT_INDEX_FILE, a linked worktree's GIT_DIR)
/// would aim every read at the caller's. Never the checkout's fsmonitor: git grep prints its errors over the results
/// (CLAUDE.md, environment quirk 5).
pub const GIT_HOOK_VARS: [&str; 4] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
];

fn git_out(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let mut git = Command::new("git");
    for v in GIT_HOOK_VARS {
        git.env_remove(v);
    }
    git.args(["-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git {}: {e}", args[0]))
}
