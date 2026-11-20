//! Refuses an rm or rmdir whose target, its variable empty, reaches `/` or a top-level folder: `rm -f $D/*`,
//! `rm -rf "${dir}"/`, `rm -rf $OUT/bin`, `rm -rf "${dir%/}"/*`. Claude Code stops on that shape and waits for a human to
//! answer « Dangerous rm operation on possibly-empty variable path », even in a task under bypass: a subagent left the
//! orchestrator's session on that prompt on 2026-11-20, dashboard-secrets on 2026-10-19 (task rust-rm-bare-var-guard).
//! Refused here first with the target rewritten (`"${D:?}"/*`), the agent sends that and no one is asked.
//!
//! Mirrors Claude Code 2.1.329's own check, read from its bundle. Its three shapes, each a regex there:
//! 1. a target that collapses to `/` (`jun`, and `hTr` for a positional): `$D/`, `"${D}"/*`, `${D:-$E}/x`, `"$1"/*`;
//! 2. an expansion it cannot read before a `/` (`yTr`): `"${dir%/}"/*`, `${D#x}/`, `${D/a/b}/`, `${D:+x}/`, `${D:=}/`,
//!    `${D:-/}/`, `${D?}/`, HOME aside and a variable the command itself gives a value;
//! 3. a top-level folder after a variable with no value (`kTr`): `$OUT/bin`, unless the command gives OUT a value or the
//!    environment it inherits holds one.
//!
//! `rm -rf "$dir"`, `$D/target` and `$HOME/x` reach nothing there, and run. A variable that is normally set (HOME, TMPDIR,
//! PWD) gets no `:?` rewrite: it is never empty, so the guard would not stop the rm, and the advice is a literal path, as
//! Claude Code's.
//!
//! Left to Claude Code's prompt: a variable set from `pwd`, `dirname` or `git rev-parse` in the same command, `find -exec
//! rm`, the string of a `trap`, a variable Claude Code's own process lacks but the hook's holds. Refused here though
//! Claude Code lets it run, each with a rewrite that works: a positional inside an `sh -c` string given arguments, and what
//! `shell::code` reads as code where Claude Code reads text, as for every guard built on it (a `<<\EOF` body, the string
//! ssh, watch or flock runs, a quote after one of those words on the same line). Was .claude/hooks/rm-var-guard.py
//! (dev-guards-2, removed with 21 others on 2026-11-05), which refused any `$var` after rm, a shape Claude Code never
//! stopped on.
use super::shell::{Call, LEAD, code, in_code_string, spans, string_edge};
use regex::Regex;
use std::sync::LazyLock;

/// rm or rmdir as a command word, by any path or after a `\`: at a simple command's start, or a substitution's (`code`
/// keeps an unquoted `$(` as typed), past the guards' shared lead without its `^`
static RM: LazyLock<Regex> = LazyLock::new(|| {
    let lead = LEAD.strip_prefix('^').expect("LEAD opens with ^");
    Regex::new(&format!(
        r"(?:^|\$\(|`){lead}\\?(?:\S*/)?(?:rm|rmdir)(?:\s|$)"
    ))
    .expect("rm")
});

/// A regex of Claude Code's, its repeated parts named: `<name>` a variable's name, `<q>` a quote before or after the
/// expansion (escaped inside a shell's `-c "..."`), `<fallback>` the `:-`, `:-""` or `:-$E` of `${D:-$E}` (E in `f`),
/// `<end>` what follows the `/` of a target that is the root, `<top>` the top-level folders.
fn rx(pattern: &str) -> Regex {
    const NAME: &str = "[A-Za-z_][A-Za-z0-9_]*";
    const TOP: &str = r"bin|boot|dev|etc|home|lib|lib32|lib64|libx32|media|mnt|opt|proc|root|run|sbin|srv|sys|tmp|usr|var|snap|nix|lost\+found|private|cores|Applications|Library|System|Users|Volumes|Windows|ProgramData|cygdrive";
    let pattern = pattern
        .replace(
            "<fallback>",
            r#"(?::?-(?:["']{2}|"?\$\{?(?P<f><name>)\}?"?)?)?"#,
        )
        .replace("<q>", r#"(?:\\?["'])*"#)
        .replace("<end>", r#"(?:[*?\[{$/"'`]|$)"#)
        .replace("<top>", TOP)
        .replace("<name>", NAME);
    Regex::new(&pattern).expect("rm target")
}

/// `$D`, `${D}`, `${D:-}` or `${D:-$E}`, or a positional, `$1`, `"$@"`, then `/` and nothing, a pattern, another `/`, an
/// expansion, a substitution or a quote (`${D:?}` is none: bash stops on it empty); the name in `n` or `m`
static ROOT: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"^<q>(?P<e>\$(?:\{(?P<n><name>|[0-9]+|[@*!])<fallback>\}|(?P<m><name>|[0-9@*!])))<q>\\?/<end>",
    )
});
/// an expansion Claude Code cannot read, then the same `/`: a trim, a replacement, a slice, `:+`, `:=`, a `:-/` or a `?`
static UNREAD: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r#"^<q>(?P<e>\$\{(?P<n><name>|[0-9]+|[@*])(?:[%#/,^?](?:[^{}]|\{[^{}]*\})*|:[0-9](?:[^{}]|\{[^{}]*\})*|:?\+(?:[^{}]|\{[^{}]*\})*|:?=(?:["']{2}|"?\$\{?<name>\}?"?)?|:?[-=]["']?/[/.]*["']?)\})<q>\\?/<end>"#,
    )
});
/// a variable, never a positional, then a top-level folder, and only `/*` or `/` after it
static CHILD: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)^<q>(?P<e>\$(?:\{(?P<n><name>)<fallback>\}|(?P<m><name>)))<q>\\?/+(?:<top>)(?:/+\*+)*/?<q>$",
    )
});
/// A function, or `set -- <words>`: the positionals then hold what the command gave them, and Claude Code lets `"$1"/*` run.
static POSITIONALS_GIVEN: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r#"(?:^|[;&|(\n])[ \t]*(?:function[ \t]+<name>|<name>[ \t]*\([ \t]*\)|set[ \t]+--[ \t]+(?:["'][^"'$]|[^\s;&|$"']))"#,
    )
});
/// Claude Code's variables that are normally set: a `:?` on them stops nothing
const SET: [&str; 25] = [
    "HOME",
    "USERPROFILE",
    "HOMEPATH",
    "HOMEDRIVE",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "SYSTEMDRIVE",
    "SYSTEMROOT",
    "WINDIR",
    "PROGRAMFILES",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "HOMESHARE",
    "XDG_RUNTIME_DIR",
    "PWD",
    "OLDPWD",
    "TMPDIR",
    "TMP",
    "TEMP",
];
const WHY: &str = "Refused by dash hook rm-bare-var: Claude Code stops on rm of a possibly-empty variable path and waits for a human.";

/// What the guard knows of the command around one rm.
struct Around<'a> {
    /// whether the environment the call inherits holds a value for a variable
    set: &'a dyn Fn(&str) -> bool,
    /// whether the command gives a variable a value before the rm
    assigned: &'a dyn Fn(&str) -> bool,
    /// whether a positional may be empty: no function, no `set --`
    positional: bool,
    /// whether the rm stands in the command itself, not in a string a shell runs: there the rewrite brings no quotes of
    /// its own, which would close the string's
    top: bool,
}

pub fn refused(call: &Call) -> Option<String> {
    // what bash runs, each char in its place (`code`), so a word's range also picks it as typed, its quotes unblanked
    let raw: Vec<char> = call.command.chars().collect();
    let text = code(call.command);
    let run: Vec<char> = text.chars().collect();
    let positional = !POSITIONALS_GIVEN.is_match(&text);
    spans(&text).into_iter().find_map(|span| {
        let seg: String = run[span.clone()].iter().collect();
        RM.find_iter(&seg).find_map(|m| {
            let at = span.start + seg[..m.end()].chars().count();
            let end = args_end(&run, &raw, at);
            let around = Around {
                set: call.set,
                assigned: &|name| assigned(&run, &raw, at, name),
                positional,
                top: !in_code_string(&run, &raw, at),
            };
            let mut redirect = false;
            words(&run[at..end]).into_iter().find_map(|(from, to)| {
                let word: String = raw[at + from..at + to].iter().collect();
                // a redirection's target is no rm target: `rm -f x > $D/`
                if std::mem::replace(&mut redirect, is_redirect(&word))
                    || word.starts_with(['<', '>'])
                {
                    return None;
                }
                advice(&word, &around)
            })
        })
    })
}

/// Where the rm's words end: the next separator bash reads outside any parens, the `)` that closes the subshell or
/// substitution holding the rm, or the end of the string a shell runs. Never a `;` that `code` put in a substitution's
/// place (a `$(` inside a double quote), nor a `\` line break.
fn args_end(run: &[char], raw: &[char], at: usize) -> usize {
    let mut depth = 0;
    (at..run.len())
        .find(|&k| {
            match run[k] {
                '(' => depth += 1,
                ')' if depth == 0 => return true,
                ')' => depth -= 1,
                _ => {}
            }
            let separator = matches!(run[k], ';' | '|' | '&' | '\n')
                && raw[k] == run[k]
                && !(run[k] == '\n' && k > 0 && raw[k - 1] == '\\')
                && !(run[k] == '&'
                    && (matches!(run.get(k + 1), Some('>'))
                        || k > 0 && matches!(run[k - 1], '<' | '>')));
            (separator && depth == 0) || string_edge(run, raw, k)
        })
        .unwrap_or(run.len())
}

/// Each word's char range in `args`: quotes are blanked in `code`, so a space inside one never splits it, nor does one
/// inside a substitution's parens (`$D/$(basename x)` is one word).
fn words(args: &[char]) -> Vec<(usize, usize)> {
    let (mut out, mut start, mut depth) = (vec![], None, 0);
    for (i, &c) in args.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ => {}
        }
        match (c.is_whitespace() && depth == 0, start) {
            (true, Some(from)) => {
                out.push((from, i));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    out.extend(start.map(|from| (from, args.len())));
    out
}

/// `>`, `2>>`, `&>`, `<`: an operator alone, its target the next word
fn is_redirect(word: &str) -> bool {
    let op = word.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
    !op.is_empty() && op.chars().all(|c| matches!(c, '<' | '>' | '|' | '&'))
}

/// Whether the command gives `name` a value the guard can read before `at`, its last one: `OUT=/x/build; rm -rf $OUT/bin`.
/// A value holding an expansion, an empty one or `/` is none, as Claude Code counts them.
fn assigned(run: &[char], raw: &[char], at: usize, name: &str) -> bool {
    let name: Vec<char> = name.chars().chain(['=']).collect();
    let starts = |k: usize| {
        run[k..].starts_with(&name)
            && (k == 0 || run[k - 1].is_whitespace() || ";&|(".contains(run[k - 1]))
    };
    (0..at.min(run.len()))
        .rfind(|&k| starts(k))
        .is_some_and(|k| {
            let from = k + name.len();
            let to = (from..run.len())
                .find(|&j| run[j].is_whitespace() || ";&|)".contains(run[j]))
                .unwrap_or(run.len());
            let value: String = raw[from..to]
                .iter()
                .filter(|c| !"\"'".contains(**c))
                .collect();
            !value.contains(['$', '`']) && !value.trim_matches('/').is_empty()
        })
}

/// The refusal for a target that, its variable empty, reaches `/` or a top-level folder, with the target rewritten. A flag
/// and a single-quoted `$` are none, as Claude Code reads them, which also reads it past a closing paren, bracket or brace.
fn advice(typed: &str, around: &Around) -> Option<String> {
    let word = typed.trim_end_matches([')', ']', '}', '"']);
    if word.starts_with('-') || word.starts_with('\'') {
        return None;
    }
    let name = |c: &regex::Captures| {
        let n = c.name("n").or(c.name("m")).map_or("", |n| n.as_str());
        n.to_string()
    };
    let fallback = |c: &regex::Captures| c.name("f").map(|f| f.as_str().to_string());
    let may_be_empty =
        |n: &str| around.positional || n.starts_with(|c: char| c.is_alphabetic() || c == '_');
    // a top-level folder only under a variable with no value: HOME and PWD always hold one, as does a variable the
    // command or the environment it inherits gives one
    let no_value =
        |n: &str| !matches!(n, "HOME" | "PWD") && !(around.assigned)(n) && !(around.set)(n);
    let root = ROOT.captures(word).filter(|c| may_be_empty(&name(c)));
    let unread = || {
        UNREAD.captures(word).filter(|c| {
            let n = name(c);
            n != "HOME" && !(around.assigned)(&n) && may_be_empty(&n)
        })
    };
    let child = || {
        CHILD
            .captures(word)
            .filter(|c| no_value(&name(c)) && fallback(c).is_none_or(|f| no_value(&f)))
    };
    let (caps, bind) = match root {
        Some(c) => (c, false),
        None => match unread() {
            Some(c) => (c, true),
            None => (child()?, false),
        },
    };
    // as typed: bash's names keep their case, so `$tmp` is no TMP
    let (name, fallback) = (name(&caps), fallback(&caps));
    if let Some(normal) = [Some(&name), fallback.as_ref()]
        .into_iter()
        .flatten()
        .find(|n| SET.contains(&n.as_str()))
    {
        return Some(format!(
            "{WHY} ${normal} is normally set, so a :? guard would not stop it: write a literal absolute path in place of {word}."
        ));
    }
    // the rewrite is the target as typed, what the reading dropped at its end kept: `"$dir/"`, `$D/{a,b}`
    let (e, word) = (caps.name("e")?, typed);
    // what stops bash when it is empty: the variable itself, its fallback when it has one, or the variable an unread
    // expansion is first bound to
    let stops = match (bind, &fallback) {
        (true, _) => "${t:?}".to_string(),
        (false, Some(f)) => format!("${{{name}:-${{{f}:?}}}}"),
        (false, None) => format!("${{{name}:?}}"),
    };
    let guarded = match around.top && !word.contains('"') {
        true => format!("\"{stops}\""),
        false => stops,
    };
    let rewrite = format!("{}{guarded}{}", &word[..e.start()], &word[e.end()..]);
    let tail = "(or a literal path); the :? makes bash stop if it is empty.";
    Some(match bind {
        true => format!(
            "{WHY} It cannot read {}: set a variable from it before the rm, t={}, then write {rewrite} in place of {word} {tail}",
            e.as_str(),
            e.as_str()
        ),
        false => format!("{WHY} Write {rewrite} in place of {word} {tail}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::path::Path;

    /// The guard on `cmd`, the environment the call inherits holding a value for each of `set`.
    fn judge_in(cmd: &str, set: &[&str]) -> Option<String> {
        let call = Call {
            tool: "Bash",
            command: cmd,
            file: "",
            cwd: Path::new("/Users/x/repo/site_perso"),
            project: Path::new("/Users/x/repo/site_perso"),
            home: Path::new("/Users/x"),
            panes: &|| Ok(String::new()),
            is_file: &|_| false,
            is_dir: &|_| false,
            read: &|_| None,
            real: &|p| p.to_path_buf(),
            set: &|name| set.contains(&name),
        };
        refused(&call)
    }

    /// as on the Mac: HOME and TMPDIR hold a value
    fn judge(cmd: &str) -> Option<String> {
        judge_in(cmd, &["HOME", "TMPDIR"])
    }

    #[dash_test]
    fn refuses_a_variable_target_that_reaches_the_root_when_empty() {
        for bad in [
            "rm -f $D/*",
            "rm -rf \"$dir\"/",
            "rm -rf \"$dir/\"",
            "rm -rf \"${dir}\"/*",
            "cd x && rm -rf ${w}/",
            "for f in a b; do rm -f $f/*; done",
            "if true; then rmdir $d/; fi",
            "if rm -rf $D/*; then :; fi",
            "timeout 60 rm -rf $D/*",
            "sudo -n rm -rf $D/*",
            "\\rm -rf $D/*",
            "sudo /bin/rm -rf $TMPDIR/*",
            "rm -rf ${D:-}/*",
            "rm -rf ${D:-$E}/",
            "rm -rf $HOME/*",
            "rm -rf $D//x",
            "rm -rf $D/$name",
            "rm -rf \"$1\"/*",
            "rm -rf \"$@\"/",
            "rm -rf $OUT/bin",
            "rm -rf \"$OUT\"/tmp/*",
            "$SUDO rm -rf $D/*",
            "(cd x; rm -rf $D/*)",
            "echo \"$(rm -rf $D/)\"",
            "out=$(rm -rfv $D/*)",
            "echo $(rm -rf $D/)",
            "rm -f \"$(git rev-parse --show-toplevel)/x\" $D/*",
            "bash -c 'rm -rf $D/*'",
            "bash -c \"rm -rf \\\"$D\\\"/*\"",
            "rm -rf a \\\n  $D/*",
            // /code-review of PR 1849. A wrapper's flag with a value, a wrapper by its path, a `$` word of any shape:
            "ls | xargs -I {} rm -rf $D/{}",
            "sudo -u root rm -rf $D/*",
            "timeout -s KILL 5 rm -rf $D/*",
            "env -u X rm -rf $D/*",
            "/usr/bin/sudo rm -rf $D/*",
            "${SUDO:-} rm -rf $D/*",
            // an expansion Claude Code cannot read, before a `/`:
            "rm -rf \"${dir%/}\"/*",
            "rm -rf ${D#x}/",
            "rm -rf ${D/a/b}/*",
            "rm -rf ${D:+x}/",
            "rm -rf ${D:=}/*",
            "rm -rf ${D:-/}/*",
            "rm -rf ${D?}/*",
            "rm -rf \"${TMPDIR%/}\"/*",
            // a value the command gives does not make the root shape safe, nor does an empty one a top-level folder:
            "D=/x/build; rm -rf $D/*",
            "OUT=; rm -rf $OUT/bin",
            "OUT=$(pwd); rm -rf $OUT/bin",
            "OUT=/x/build; OUT=$1; rm -rf $OUT/bin",
        ] {
            let why = judge(bad).unwrap_or_else(|| panic!("passed: {bad}"));
            assert!(
                why.starts_with("Refused by dash hook rm-bare-var:"),
                "{bad}: {why}"
            );
        }
        // a variable the environment holds empty, or lacks, is none it can count on: TMPDIR too
        for (cmd, set) in [
            ("rm -rf $TMPDIR/tmp", &[][..]),
            ("rm -rf $OUT/bin", &["HOME"][..]),
        ] {
            assert!(judge_in(cmd, set).is_some(), "{cmd}");
        }
    }

    /// The refusal rewrites the target it read, the rest kept, so the agent's resend removes the same thing; a variable
    /// that is never empty gets a literal path instead, since a :? there stops nothing.
    #[dash_test]
    fn the_refusal_names_the_guarded_target_or_a_literal_path() {
        for (cmd, rewrite) in [
            ("rm -f $D/*", "Write \"${D:?}\"/* in place of $D/* (or"),
            (
                "rm -rf \"$dir/\"",
                "Write \"${dir:?}/\" in place of \"$dir/\" (or",
            ),
            ("rmdir $d/", "Write \"${d:?}\"/ in place of $d/ (or"),
            (
                "rm -rf ${D:-}/*",
                "Write \"${D:?}\"/* in place of ${D:-}/* (or",
            ),
            (
                "rm -rf $OUT/bin",
                "Write \"${OUT:?}\"/bin in place of $OUT/bin (or",
            ),
            // a lowercase name is the script's own, never TMP or HOME: bash keeps a name's case
            (
                "rm -rf $tmp/*",
                "Write \"${tmp:?}\"/* in place of $tmp/* (or",
            ),
            ("rm -rf $home/*", "Write \"${home:?}\"/* in place of"),
            // the fallback is kept, and guarded in its turn
            (
                "rm -rf ${OUT:-$BUILD}/",
                "Write \"${OUT:-${BUILD:?}}\"/ in place of ${OUT:-$BUILD}/ (or",
            ),
            // an expansion it cannot read is bound first, the `:?` on that
            (
                "rm -rf \"${dir%/}\"/*",
                "set a variable from it before the rm, t=${dir%/}, then write \"${t:?}\"/* in place of \"${dir%/}\"/* (or",
            ),
            // in a string a shell runs, no quote of the rewrite's own, and none of the string's in the target
            (
                "bash -c 'rm -rf $D/*'",
                "Write ${D:?}/* in place of $D/* (or",
            ),
            (
                "bash -c \"rm -rf $D/*\" x",
                "Write ${D:?}/* in place of $D/* (or",
            ),
            // a substitution is one word, and its closing paren ends the rm's words
            (
                "rm -rf $D/$(basename x y)",
                "in place of $D/$(basename x y) (or",
            ),
            ("(cd x; rm -rf $D/*)", "in place of $D/* (or"),
            (
                "ls | xargs -I {} rm -rf $D/{}",
                "Write \"${D:?}\"/{} in place of $D/{} (or",
            ),
        ] {
            let why = judge(cmd).unwrap_or_default();
            assert!(why.contains(rewrite), "{cmd}: {why}");
        }
        // never a `:?` on a variable that is normally set, his home first: `"${HOME:?}"/*` would still empty it
        for cmd in [
            "rm -rf $HOME/*",
            "rm -rf ${HOME}/*",
            "rm -rf \"${HOME}\"/",
            "rm -rf ${HOME:-}/*",
            "rm -rf ${HOME:-$D}/*",
            "rm -rf ${D:-$HOME}/*",
            "rm -rf \"${D:-\"$HOME\"}\"/*",
            "rm -rf \"$TMPDIR\"/",
            "rm -rf $PWD/*",
            "rm -rf $OLDPWD/",
            "rm -rf \"${TMPDIR%/}\"/*",
            "rm -rf ${PWD%/}/*",
            "bash -c 'rm -rf $HOME/*'",
            "sudo -u root rm -rf $HOME/",
        ] {
            let why = judge(cmd).unwrap_or_else(|| panic!("passed: {cmd}"));
            assert!(
                why.contains("literal absolute path") && !why.contains(":?}"),
                "{cmd}: {why}"
            );
        }
        let why = judge_in("rm -rf $TMPDIR/tmp", &[]).unwrap_or_default();
        assert!(
            why.contains("literal absolute path") && !why.contains(":?}"),
            "{why}"
        );
    }

    /// What Claude Code lets run passes too: a guarded or literal path, a variable that empty reaches no `/`, and text that
    /// only names such an rm.
    #[dash_test]
    fn passes_a_guarded_literal_or_harmless_target_and_text_naming_one() {
        for good in [
            "rm -f \"${D:?}\"/*",
            "rm -rf \"${dir:?}\"/",
            "rm -rf ${D:?no dir}/*",
            "rm -rf target/debug",
            "rm -rf /tmp/x.$$",
            "rm -rf $HOME/x",
            "rm -rf $HOME/Library",
            "rm -rf \"$dir\"",
            "rm -rf $dir",
            "cd x && rm -rf ${w}/target",
            "rm -rf ${D:-/tmp/x}/*",
            "rm -rf '$D'/*",
            "rm -rf \\$D/*",
            "rm -f -- out.log",
            "rm -f x > $D/",
            "rm -f x 2>$D/",
            "echo rm -rf $D/*",
            "farm -rf $D/*",
            "cat <<'EOF' > n.md\nrm -rf $D/*\nEOF",
            "cat <<EOF > n.md\nrm -rf $D/*\nEOF",
            "cat <<'EOF' > n.md\nrm -rf $D/* \\\nEOF",
            "git commit -m \"never rm -rf $D/* again\"",
            "git commit -m \"$(printf 'rm -rf $D/')\"",
            "gh pr create --body 'rm -f $D/* waits on a human'",
            "grep -n 'rm -rf \\$D/' f",
            // /code-review of PR 1849. The rewrites this guard advises, each run by Claude Code:
            "t=${dir%/}; rm -rf \"${t:?}\"/*",
            "rm -rf \"${OUT:-${BUILD:?}}\"/",
            "bash -c 'rm -rf ${D:?}/*'",
            // an unread expansion of HOME, of a variable the command gives a value, or with no `/` after it
            "rm -rf \"${HOME%/}\"/x",
            "dir=/x/build/; rm -rf \"${dir%/}\"/*",
            "rm -rf \"${dir%/}\"",
            // a top-level folder name under a variable that holds a value: the command's own, or the environment's
            "OUT=/Users/x/build; rm -rf $OUT/bin",
            "export W=\"/private/tmp/w\" && rm -rf \"$W\"/tmp",
            "rm -rf $TMPDIR/tmp",
            "rm -rf \"$TMPDIR\"/tmp/*",
            "rm -rf $PWD/tmp",
            // positionals the command gives, and one before a top-level folder
            "set -- /x/build; rm -rf \"$1\"/*",
            "clean() { rm -rf \"$1\"/*; }; clean /x/build",
            "rm -rf \"$1\"/tmp",
            // words after the substitution that holds the rm are no target of it
            "echo \"$(rm -f a.log)\" $D/",
            "echo $(rm -f a.log) $D/",
            "bash -c 'rm -f a.log' $D/",
        ] {
            assert_eq!(judge(good), None, "{good}");
        }
    }
}
